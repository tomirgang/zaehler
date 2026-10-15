//! Driven-Adapter: Unit of Work und Repositories auf SeaORM (ADR_022).
//!
//! Jede Unit of Work ist eine `DatabaseTransaction`; ohne `commit` rollt
//! SeaORM beim Drop zurück. Mandantentrennung: jede Abfrage prüft über
//! `meters.household_id` gegen den `Tenant` (SPEC_SEC_TENANT_SCOPE).

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use rust_decimal::{
    Decimal,
    prelude::{FromPrimitive, ToPrimitive},
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect, SqlErr, TransactionTrait, sea_query::Expr,
};
use uuid::Uuid;
use zaehler_app::{
    PortError, Role, Tenant,
    ids::{BlobKey, DeviceId, DraftId, HouseholdId, MeterId, ReadingId, RegisterId, UserId},
    model::{
        Device, DeviceData, Draft, DraftOutcome, DraftStatus, Household, HouseholdData,
        HouseholdSummary, Identity, Meter, MeterAggregate, MeterData, MeterSummary, MonthDay,
        Reading, Register, RegisterData, User,
    },
    port::{
        DraftRepository, HouseholdRepository, MaintenanceRepository, MembershipRepository,
        MeterRepository, PortResult, Readiness, ReadingRepository, UnitOfWork, UnitOfWorkFactory,
        UserRepository,
    },
};

use crate::{
    entity::{ai_draft, household, membership, meter, meter_device, reading, register, user},
    fixed,
};

#[allow(clippy::needless_pass_by_value)] // als Funktion für `map_err` verwendet
fn db_err(e: DbErr) -> PortError {
    match e.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(msg)) => PortError::Conflict(msg),
        _ => PortError::Backend(e.to_string()),
    }
}

fn data_err(e: impl std::fmt::Display) -> PortError {
    PortError::Backend(format!("ungültige Daten: {e}"))
}

/// Fabrik für Unit of Work; zugleich Readiness-Prüfung.
#[derive(Debug, Clone)]
pub struct SeaOrmStore {
    db: DatabaseConnection,
}

impl SeaOrmStore {
    #[must_use]
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait]
impl UnitOfWorkFactory for SeaOrmStore {
    // @impl SPEC_ARCH_UOW
    async fn begin(&self) -> PortResult<Box<dyn UnitOfWork>> {
        let txn = self.db.begin().await.map_err(db_err)?;
        Ok(Box::new(SeaOrmUow { txn }))
    }
}

#[async_trait]
impl Readiness for SeaOrmStore {
    async fn check(&self) -> PortResult<()> {
        self.db.ping().await.map_err(db_err)
    }
}

/// Offene Entwürfe (`pending`, `failed`).
fn open_status() -> [&'static str; 2] {
    [DraftStatus::Pending.as_str(), DraftStatus::Failed.as_str()]
}

#[async_trait]
impl MaintenanceRepository for SeaOrmStore {
    // @impl SPEC_OPS_HOUSEKEEPING
    async fn expire_drafts(
        &self,
        cutoff: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> PortResult<Vec<BlobKey>> {
        let expired = ai_draft::Entity::find()
            .filter(ai_draft::Column::Status.is_in(open_status()))
            .filter(ai_draft::Column::CreatedAt.lt(cutoff))
            .all(&self.db)
            .await
            .map_err(db_err)?;
        if expired.is_empty() {
            return Ok(Vec::new());
        }
        // Bedingung auf den Status: ein paralleler Lauf ändert dieselben
        // Zeilen nicht doppelt; die Fotos löschen beide (idempotent).
        ai_draft::Entity::update_many()
            .col_expr(
                ai_draft::Column::Status,
                Expr::value(DraftStatus::Discarded.as_str()),
            )
            .col_expr(ai_draft::Column::FinishedAt, Expr::value(now))
            .filter(ai_draft::Column::Id.is_in(expired.iter().map(|d| d.id)))
            .filter(ai_draft::Column::Status.is_in(open_status()))
            .exec(&self.db)
            .await
            .map_err(db_err)?;
        expired
            .into_iter()
            .map(|d| BlobKey::parse(&d.photo_key).ok_or_else(|| data_err("photo_key")))
            .collect()
    }

    // @impl SPEC_OPS_HOUSEKEEPING
    async fn delete_expired_sessions(&self, now: DateTime<Utc>) -> PortResult<u64> {
        crate::session_store::delete_expired(&self.db, now)
            .await
            .map_err(db_err)
    }
}

struct SeaOrmUow {
    txn: DatabaseTransaction,
}

#[async_trait]
impl UnitOfWork for SeaOrmUow {
    fn memberships(&mut self) -> &mut dyn MembershipRepository {
        self
    }
    fn households(&mut self) -> &mut dyn HouseholdRepository {
        self
    }
    fn users(&mut self) -> &mut dyn UserRepository {
        self
    }
    fn meters(&mut self) -> &mut dyn MeterRepository {
        self
    }
    fn readings(&mut self) -> &mut dyn ReadingRepository {
        self
    }
    fn drafts(&mut self) -> &mut dyn DraftRepository {
        self
    }
    async fn commit(self: Box<Self>) -> PortResult<()> {
        self.txn.commit().await.map_err(db_err)
    }
}

// ---------------------------------------------------------------- Abbildung

fn to_user(m: user::Model) -> User {
    User {
        id: UserId(m.id),
        display_name: m.display_name,
        email: m.email,
        locale: m.locale,
    }
}

fn to_household(m: household::Model) -> PortResult<Household> {
    Ok(Household {
        id: HouseholdId(m.id),
        data: HouseholdData {
            name: m.name,
            timezone: m.timezone.parse::<Tz>().map_err(data_err)?,
        },
    })
}

fn to_meter(m: meter::Model) -> PortResult<Meter> {
    let billing_start = match (m.billing_start_month, m.billing_start_day) {
        (Some(month), Some(day)) => Some(MonthDay {
            month: u8::try_from(month).map_err(data_err)?,
            day: u8::try_from(day).map_err(data_err)?,
        }),
        _ => None,
    };
    Ok(Meter {
        id: MeterId(m.id),
        data: MeterData {
            name: m.name,
            kind: m.kind.parse().map_err(data_err)?,
            unit: m.unit,
            reading_interval_days: m
                .reading_interval_days
                .map(u16::try_from)
                .transpose()
                .map_err(data_err)?,
            max_daily_rate: m.max_daily_rate_milli.map(fixed::quantity_from_db),
            billing_start,
        },
        archived: m.archived,
        created_at: m.created_at,
    })
}

fn to_device(m: meter_device::Model) -> PortResult<Device> {
    Ok(Device {
        id: DeviceId(m.id),
        data: DeviceData {
            serial_number: m.serial_number,
            installed_at: m.installed_at,
            rollover_digits: m
                .rollover_digits
                .map(u8::try_from)
                .transpose()
                .map_err(data_err)?,
            decimal_places: u8::try_from(m.decimal_places).map_err(data_err)?,
        },
        removed_at: m.removed_at,
    })
}

fn to_register(m: register::Model) -> PortResult<Register> {
    Ok(Register {
        id: RegisterId(m.id),
        data: RegisterData {
            code: m.code,
            direction: m.direction.parse().map_err(data_err)?,
        },
    })
}

fn to_reading(m: reading::Model) -> PortResult<Reading> {
    Ok(Reading {
        id: ReadingId(m.id),
        register_id: RegisterId(m.register_id),
        device_id: DeviceId(m.device_id),
        read_at: m.read_at,
        value: fixed::quantity_from_db(m.value_milli),
        source: m.source.parse().map_err(data_err)?,
        note: m.note,
        photo_key: m
            .photo_key
            .map(|k| BlobKey::parse(&k).ok_or_else(|| data_err("photo_key")))
            .transpose()?,
        ai_confidence: m
            .ai_confidence
            .and_then(Decimal::from_f64)
            .map(|c| c.round_dp(3)),
        created_by: m.created_by.map(UserId),
    })
}

/// Spalten einer Messstelle: Tagesrate (milli), Abrechnungsmonat, -tag, Ableseturnus.
type MeterColumns = (Option<i64>, Option<i16>, Option<i16>, Option<i32>);

fn meter_columns(data: &MeterData) -> PortResult<MeterColumns> {
    let rate = data
        .max_daily_rate
        .map(fixed::quantity_to_db)
        .transpose()
        .map_err(data_err)?;
    let (month, day) = data.billing_start.map_or((None, None), |md| {
        (Some(i16::from(md.month)), Some(i16::from(md.day)))
    });
    Ok((rate, month, day, data.reading_interval_days.map(i32::from)))
}

// ------------------------------------------------------------- Hilfsabfragen

impl SeaOrmUow {
    async fn owns_meter(&self, t: &Tenant, meter_id: Uuid) -> PortResult<bool> {
        Ok(meter::Entity::find_by_id(meter_id)
            .filter(meter::Column::HouseholdId.eq(t.household_id().0))
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .is_some())
    }

    async fn require_meter(&self, t: &Tenant, meter_id: Uuid) -> PortResult<()> {
        if self.owns_meter(t, meter_id).await? {
            Ok(())
        } else {
            Err(PortError::NotFound)
        }
    }

    async fn meter_of_register(&self, register_id: Uuid) -> PortResult<Option<Uuid>> {
        Ok(register::Entity::find_by_id(register_id)
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|r| r.meter_id))
    }

    async fn meter_of_device(&self, device_id: Uuid) -> PortResult<Option<Uuid>> {
        Ok(meter_device::Entity::find_by_id(device_id)
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|d| d.meter_id))
    }

    /// Gerät, das zu einer Messstelle des Tenants gehört.
    async fn require_device(&self, t: &Tenant, device_id: Uuid) -> PortResult<()> {
        let meter_id = self
            .meter_of_device(device_id)
            .await?
            .ok_or(PortError::NotFound)?;
        self.require_meter(t, meter_id).await
    }

    async fn register_ids(&self, meter_ids: &[Uuid]) -> PortResult<Vec<(Uuid, Uuid)>> {
        register::Entity::find()
            .select_only()
            .column(register::Column::Id)
            .column(register::Column::MeterId)
            .filter(register::Column::MeterId.is_in(meter_ids.iter().copied()))
            .into_tuple::<(Uuid, Uuid)>()
            .all(&self.txn)
            .await
            .map_err(db_err)
    }
}

// ----------------------------------------------------------- Repositories

#[async_trait]
impl MembershipRepository for SeaOrmUow {
    async fn role(&mut self, user: UserId, household: HouseholdId) -> PortResult<Option<Role>> {
        membership::Entity::find_by_id((household.0, user.0))
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|m| m.role.parse::<Role>().map_err(data_err))
            .transpose()
    }

    async fn households_of(&mut self, user: UserId) -> PortResult<Vec<HouseholdSummary>> {
        let memberships = membership::Entity::find()
            .filter(membership::Column::UserId.eq(user.0))
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let roles: HashMap<Uuid, String> = memberships
            .into_iter()
            .map(|m| (m.household_id, m.role))
            .collect();
        let households = household::Entity::find()
            .filter(household::Column::Id.is_in(roles.keys().copied()))
            .order_by_asc(household::Column::Name)
            .order_by_asc(household::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        households
            .into_iter()
            .map(|h| {
                Ok(HouseholdSummary {
                    id: HouseholdId(h.id),
                    role: roles
                        .get(&h.id)
                        .map_or("viewer", String::as_str)
                        .parse::<Role>()
                        .map_err(data_err)?,
                    timezone: h.timezone.parse::<Tz>().map_err(data_err)?,
                    name: h.name,
                })
            })
            .collect()
    }
}

#[async_trait]
impl HouseholdRepository for SeaOrmUow {
    async fn create(
        &mut self,
        id: HouseholdId,
        owner: UserId,
        data: &HouseholdData,
        now: DateTime<Utc>,
    ) -> PortResult<()> {
        household::Entity::insert(household::ActiveModel {
            id: Set(id.0),
            name: Set(data.name.clone()),
            timezone: Set(data.timezone.name().to_owned()),
            created_at: Set(now),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        membership::Entity::insert(membership::ActiveModel {
            household_id: Set(id.0),
            user_id: Set(owner.0),
            role: Set(Role::Owner.as_str().to_owned()),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get(&mut self, t: &Tenant) -> PortResult<Household> {
        let m = household::Entity::find_by_id(t.household_id().0)
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .ok_or(PortError::NotFound)?;
        to_household(m)
    }

    async fn update(&mut self, t: &Tenant, data: &HouseholdData) -> PortResult<()> {
        let result = household::Entity::update_many()
            .col_expr(household::Column::Name, Expr::value(data.name.clone()))
            .col_expr(
                household::Column::Timezone,
                Expr::value(data.timezone.name().to_owned()),
            )
            .filter(household::Column::Id.eq(t.household_id().0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if result.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }

    async fn delete(&mut self, t: &Tenant) -> PortResult<()> {
        // Abhängige Zeilen über ON DELETE CASCADE (Migration 1).
        household::Entity::delete_by_id(t.household_id().0)
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

#[async_trait]
impl UserRepository for SeaOrmUow {
    async fn upsert_oidc(
        &mut self,
        new_id: UserId,
        identity: &Identity,
        now: DateTime<Utc>,
    ) -> PortResult<User> {
        let existing = user::Entity::find()
            .filter(user::Column::OidcIssuer.eq(identity.issuer.clone()))
            .filter(user::Column::OidcSubject.eq(identity.subject.clone()))
            .one(&self.txn)
            .await
            .map_err(db_err)?;
        if let Some(m) = existing {
            user::Entity::update_many()
                .col_expr(
                    user::Column::DisplayName,
                    Expr::value(identity.display_name.clone()),
                )
                .col_expr(user::Column::Email, Expr::value(identity.email.clone()))
                .col_expr(user::Column::LastLoginAt, Expr::value(Some(now)))
                .filter(user::Column::Id.eq(m.id))
                .exec(&self.txn)
                .await
                .map_err(db_err)?;
            return Ok(User {
                id: UserId(m.id),
                display_name: identity.display_name.clone(),
                email: identity.email.clone(),
                locale: m.locale,
            });
        }
        user::Entity::insert(user::ActiveModel {
            id: Set(new_id.0),
            oidc_issuer: Set(identity.issuer.clone()),
            oidc_subject: Set(identity.subject.clone()),
            email: Set(identity.email.clone()),
            display_name: Set(identity.display_name.clone()),
            locale: Set(identity.locale.clone()),
            created_at: Set(now),
            last_login_at: Set(Some(now)),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(User {
            id: new_id,
            display_name: identity.display_name.clone(),
            email: identity.email.clone(),
            locale: identity.locale.clone(),
        })
    }

    async fn get(&mut self, id: UserId) -> PortResult<Option<User>> {
        Ok(user::Entity::find_by_id(id.0)
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(to_user))
    }

    async fn set_locale(&mut self, id: UserId, locale: &str) -> PortResult<()> {
        user::Entity::update_many()
            .col_expr(user::Column::Locale, Expr::value(locale.to_owned()))
            .filter(user::Column::Id.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

#[async_trait]
impl MeterRepository for SeaOrmUow {
    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<MeterSummary>> {
        let meters = meter::Entity::find()
            .filter(meter::Column::HouseholdId.eq(t.household_id().0))
            .order_by_asc(meter::Column::Name)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let meter_ids: Vec<Uuid> = meters.iter().map(|m| m.id).collect();
        let registers: HashMap<Uuid, Uuid> =
            self.register_ids(&meter_ids).await?.into_iter().collect();
        let times: Vec<(Uuid, DateTime<Utc>)> = reading::Entity::find()
            .select_only()
            .column(reading::Column::RegisterId)
            .column(reading::Column::ReadAt)
            .filter(reading::Column::RegisterId.is_in(registers.keys().copied()))
            .into_tuple()
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let mut last: HashMap<Uuid, DateTime<Utc>> = HashMap::new();
        for (register_id, at) in times {
            if let Some(meter_id) = registers.get(&register_id) {
                let entry = last.entry(*meter_id).or_insert(at);
                *entry = (*entry).max(at);
            }
        }
        meters
            .into_iter()
            .map(|m| {
                let last_reading_at = last.get(&m.id).copied();
                Ok(MeterSummary {
                    meter: to_meter(m)?,
                    last_reading_at,
                })
            })
            .collect()
    }

    async fn get(&mut self, t: &Tenant, id: MeterId) -> PortResult<Option<MeterAggregate>> {
        let Some(m) = meter::Entity::find_by_id(id.0)
            .filter(meter::Column::HouseholdId.eq(t.household_id().0))
            .one(&self.txn)
            .await
            .map_err(db_err)?
        else {
            return Ok(None);
        };
        let devices = meter_device::Entity::find()
            .filter(meter_device::Column::MeterId.eq(id.0))
            .order_by_asc(meter_device::Column::InstalledAt)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let registers = register::Entity::find()
            .filter(register::Column::MeterId.eq(id.0))
            .order_by_asc(register::Column::Code)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(Some(MeterAggregate {
            meter: to_meter(m)?,
            devices: devices
                .into_iter()
                .map(to_device)
                .collect::<PortResult<_>>()?,
            registers: registers
                .into_iter()
                .map(to_register)
                .collect::<PortResult<_>>()?,
        }))
    }

    async fn create(
        &mut self,
        t: &Tenant,
        id: MeterId,
        data: &MeterData,
        now: DateTime<Utc>,
    ) -> PortResult<()> {
        let (rate, month, day, interval) = meter_columns(data)?;
        meter::Entity::insert(meter::ActiveModel {
            id: Set(id.0),
            household_id: Set(t.household_id().0),
            name: Set(data.name.clone()),
            kind: Set(data.kind.as_str().to_owned()),
            unit: Set(data.unit.clone()),
            reading_interval_days: Set(interval),
            max_daily_rate_milli: Set(rate),
            billing_start_month: Set(month),
            billing_start_day: Set(day),
            archived: Set(false),
            created_at: Set(now),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn update(&mut self, t: &Tenant, id: MeterId, data: &MeterData) -> PortResult<()> {
        let (rate, month, day, interval) = meter_columns(data)?;
        let result = meter::Entity::update_many()
            .col_expr(meter::Column::Name, Expr::value(data.name.clone()))
            .col_expr(
                meter::Column::Kind,
                Expr::value(data.kind.as_str().to_owned()),
            )
            .col_expr(meter::Column::Unit, Expr::value(data.unit.clone()))
            .col_expr(meter::Column::ReadingIntervalDays, Expr::value(interval))
            .col_expr(meter::Column::MaxDailyRateMilli, Expr::value(rate))
            .col_expr(meter::Column::BillingStartMonth, Expr::value(month))
            .col_expr(meter::Column::BillingStartDay, Expr::value(day))
            .filter(meter::Column::Id.eq(id.0))
            .filter(meter::Column::HouseholdId.eq(t.household_id().0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if result.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }

    async fn add_register(
        &mut self,
        t: &Tenant,
        meter_id: MeterId,
        id: RegisterId,
        data: &RegisterData,
    ) -> PortResult<()> {
        self.require_meter(t, meter_id.0).await?;
        register::Entity::insert(register::ActiveModel {
            id: Set(id.0),
            meter_id: Set(meter_id.0),
            code: Set(data.code.clone()),
            direction: Set(data.direction.as_str().to_owned()),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn rename_register(&mut self, t: &Tenant, id: RegisterId, code: &str) -> PortResult<()> {
        let meter_id = self
            .meter_of_register(id.0)
            .await?
            .ok_or(PortError::NotFound)?;
        self.require_meter(t, meter_id).await?;
        register::Entity::update_many()
            .col_expr(register::Column::Code, Expr::value(code.to_owned()))
            .filter(register::Column::Id.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn add_device(
        &mut self,
        t: &Tenant,
        meter_id: MeterId,
        id: DeviceId,
        data: &DeviceData,
    ) -> PortResult<()> {
        self.require_meter(t, meter_id.0).await?;
        meter_device::Entity::insert(meter_device::ActiveModel {
            id: Set(id.0),
            meter_id: Set(meter_id.0),
            serial_number: Set(data.serial_number.clone()),
            installed_at: Set(data.installed_at),
            removed_at: Set(None),
            rollover_digits: Set(data.rollover_digits.map(i16::from)),
            decimal_places: Set(i16::from(data.decimal_places)),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn update_device(
        &mut self,
        t: &Tenant,
        id: DeviceId,
        data: &DeviceData,
    ) -> PortResult<()> {
        self.require_device(t, id.0).await?;
        meter_device::Entity::update_many()
            .col_expr(
                meter_device::Column::SerialNumber,
                Expr::value(data.serial_number.clone()),
            )
            .col_expr(
                meter_device::Column::InstalledAt,
                Expr::value(data.installed_at),
            )
            .col_expr(
                meter_device::Column::RolloverDigits,
                Expr::value(data.rollover_digits.map(i16::from)),
            )
            .col_expr(
                meter_device::Column::DecimalPlaces,
                Expr::value(i16::from(data.decimal_places)),
            )
            .filter(meter_device::Column::Id.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn retire_device(
        &mut self,
        t: &Tenant,
        id: DeviceId,
        at: DateTime<Utc>,
    ) -> PortResult<()> {
        self.require_device(t, id.0).await?;
        meter_device::Entity::update_many()
            .col_expr(meter_device::Column::RemovedAt, Expr::value(Some(at)))
            .filter(meter_device::Column::Id.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn find_by_serial(
        &mut self,
        t: &Tenant,
        serial: &str,
    ) -> PortResult<Option<(MeterId, DeviceId)>> {
        let meter_ids: Vec<Uuid> = meter::Entity::find()
            .select_only()
            .column(meter::Column::Id)
            .filter(meter::Column::HouseholdId.eq(t.household_id().0))
            .into_tuple()
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(meter_device::Entity::find()
            .filter(meter_device::Column::SerialNumber.eq(serial.to_owned()))
            .filter(meter_device::Column::RemovedAt.is_null())
            .filter(meter_device::Column::MeterId.is_in(meter_ids))
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|d| (MeterId(d.meter_id), DeviceId(d.id))))
    }

    async fn find_by_name(&mut self, t: &Tenant, name: &str) -> PortResult<Option<MeterId>> {
        Ok(meter::Entity::find()
            .filter(meter::Column::HouseholdId.eq(t.household_id().0))
            .filter(meter::Column::Name.eq(name.to_owned()))
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|m| MeterId(m.id)))
    }
}

#[async_trait]
impl ReadingRepository for SeaOrmUow {
    async fn list_for_meter(&mut self, t: &Tenant, meter_id: MeterId) -> PortResult<Vec<Reading>> {
        if !self.owns_meter(t, meter_id.0).await? {
            return Ok(Vec::new());
        }
        let register_ids: Vec<Uuid> = self
            .register_ids(&[meter_id.0])
            .await?
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        reading::Entity::find()
            .filter(reading::Column::RegisterId.is_in(register_ids))
            .order_by_asc(reading::Column::ReadAt)
            .order_by_asc(reading::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(to_reading)
            .collect()
    }

    async fn insert(&mut self, t: &Tenant, readings: &[Reading]) -> PortResult<()> {
        let mut checked: HashSet<(Uuid, Uuid)> = HashSet::new();
        for r in readings {
            if !checked.contains(&(r.register_id.0, r.device_id.0)) {
                let meter_id = self
                    .meter_of_register(r.register_id.0)
                    .await?
                    .ok_or(PortError::NotFound)?;
                if !self.owns_meter(t, meter_id).await?
                    || self.meter_of_device(r.device_id.0).await? != Some(meter_id)
                {
                    return Err(PortError::NotFound);
                }
                checked.insert((r.register_id.0, r.device_id.0));
            }
            reading::Entity::insert(reading::ActiveModel {
                id: Set(r.id.0),
                register_id: Set(r.register_id.0),
                device_id: Set(r.device_id.0),
                read_at: Set(r.read_at),
                value_milli: Set(fixed::quantity_to_db(r.value).map_err(data_err)?),
                source: Set(r.source.as_str().to_owned()),
                photo_key: Set(r.photo_key.as_ref().map(ToString::to_string)),
                ai_confidence: Set(r.ai_confidence.and_then(|c| c.to_f64())),
                note: Set(r.note.clone()),
                created_by: Set(r.created_by.map(|u| u.0)),
                created_at: Set(Utc::now()),
            })
            .exec_without_returning(&self.txn)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    async fn get(&mut self, t: &Tenant, id: ReadingId) -> PortResult<Option<(MeterId, Reading)>> {
        let Some(m) = reading::Entity::find_by_id(id.0)
            .one(&self.txn)
            .await
            .map_err(db_err)?
        else {
            return Ok(None);
        };
        let Some(meter_id) = self.meter_of_register(m.register_id).await? else {
            return Ok(None);
        };
        if !self.owns_meter(t, meter_id).await? {
            return Ok(None);
        }
        Ok(Some((MeterId(meter_id), to_reading(m)?)))
    }

    async fn delete(&mut self, t: &Tenant, id: ReadingId) -> PortResult<()> {
        if ReadingRepository::get(self, t, id).await?.is_none() {
            return Err(PortError::NotFound);
        }
        reading::Entity::delete_by_id(id.0)
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

fn to_draft(m: ai_draft::Model) -> PortResult<Draft> {
    Ok(Draft {
        id: DraftId(m.id),
        created_by: UserId(m.created_by),
        photo_key: BlobKey::parse(&m.photo_key).ok_or_else(|| data_err("photo_key"))?,
        model: m.model,
        duration_ms: m.duration_ms,
        suggestion: m
            .suggestion
            .map(|s| serde_json::from_str(&s).map_err(data_err))
            .transpose()?,
        matched_meter_id: m.matched_meter_id.map(MeterId),
        status: m.status.parse().map_err(data_err)?,
        created_at: m.created_at,
    })
}

// @impl SPEC_AI_DRAFTS, SPEC_SEC_TENANT_SCOPE
#[async_trait]
impl DraftRepository for SeaOrmUow {
    async fn create(&mut self, t: &Tenant, d: &Draft) -> PortResult<()> {
        if d.photo_key.household() != t.household_id() {
            return Err(PortError::NotFound);
        }
        if let Some(m) = d.matched_meter_id {
            self.require_meter(t, m.0).await?;
        }
        let suggestion = d
            .suggestion
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(data_err)?;
        ai_draft::Entity::insert(ai_draft::ActiveModel {
            id: Set(d.id.0),
            household_id: Set(t.household_id().0),
            created_by: Set(d.created_by.0),
            photo_key: Set(d.photo_key.to_string()),
            model: Set(d.model.clone()),
            duration_ms: Set(d.duration_ms),
            suggestion: Set(suggestion),
            matched_meter_id: Set(d.matched_meter_id.map(|m| m.0)),
            status: Set(d.status.as_str().to_owned()),
            confirmed_reading_id: Set(None),
            corrected: Set(None),
            created_at: Set(d.created_at),
            finished_at: Set(None),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_open(&mut self, t: &Tenant, id: DraftId) -> PortResult<Option<Draft>> {
        ai_draft::Entity::find_by_id(id.0)
            .filter(ai_draft::Column::HouseholdId.eq(t.household_id().0))
            .filter(ai_draft::Column::Status.is_in(open_status()))
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(to_draft)
            .transpose()
    }

    async fn finish(
        &mut self,
        t: &Tenant,
        id: DraftId,
        outcome: DraftOutcome,
        at: DateTime<Utc>,
    ) -> PortResult<()> {
        let (status, reading, corrected) = match outcome {
            DraftOutcome::Confirmed { reading, corrected } => {
                (DraftStatus::Confirmed, Some(reading.0), corrected)
            }
            DraftOutcome::Discarded => (DraftStatus::Discarded, None, None),
        };
        let res = ai_draft::Entity::update_many()
            .col_expr(ai_draft::Column::Status, Expr::value(status.as_str()))
            .col_expr(ai_draft::Column::ConfirmedReadingId, Expr::value(reading))
            .col_expr(ai_draft::Column::Corrected, Expr::value(corrected))
            .col_expr(ai_draft::Column::FinishedAt, Expr::value(at))
            .filter(ai_draft::Column::Id.eq(id.0))
            .filter(ai_draft::Column::HouseholdId.eq(t.household_id().0))
            .filter(ai_draft::Column::Status.is_in(open_status()))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }
}
