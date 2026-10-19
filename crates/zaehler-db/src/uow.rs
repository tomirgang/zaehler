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
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, DbBackend, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    SqlErr, TransactionTrait, sea_query::Expr,
};
use tracing::Instrument;
use uuid::Uuid;
use zaehler_app::{
    PortError, Role, Tenant,
    ids::{
        BlobKey, DeviceId, DraftId, HouseholdId, InvitationId, MeterId, OutboxId, ReadingId,
        RegisterId, RevisionId, UserId,
    },
    model::{
        Device, DeviceData, Draft, DraftAnalysis, DraftOutcome, DraftStatus, Household,
        HouseholdData, HouseholdSummary, Identity, Invitation, Member, Meter, MeterAggregate,
        MeterData, MeterSummary, MeterTargets, MonthDay, NewInvitation, NotificationSettings,
        OutboxEvent, OutboxStatus, Reading, ReadingUpdate, Register, RegisterData, Revision,
        SetupDraft, User, schedule_columns, schedule_from_columns, season_weights_from_json,
        season_weights_to_json,
    },
    port::{
        ContractRepository, DraftRepository, HouseholdRepository, InvitationRepository,
        MaintenanceRepository, MembershipRepository, MeterRepository,
        NotificationSettingsRepository, OutboxRepository, PortResult, Readiness, ReadingRepository,
        RevisionRepository, SetupDraftRepository, UnitOfWork, UnitOfWorkFactory, UserRepository,
    },
    secret,
};

use crate::{
    entity::{
        ai_draft, household, invitation, membership, meter, meter_device, notification_settings,
        outbox, reading, reading_revision, register, setup_draft, user,
    },
    fixed,
};

#[allow(clippy::needless_pass_by_value)] // als Funktion für `map_err` verwendet
pub(crate) fn db_err(e: DbErr) -> PortError {
    match e.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(msg)) => PortError::Conflict(msg),
        _ => PortError::Backend(e.to_string()),
    }
}

pub(crate) fn data_err(e: impl std::fmt::Display) -> PortError {
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

    pub(crate) fn conn(&self) -> &DatabaseConnection {
        &self.db
    }
}

#[async_trait]
impl UnitOfWorkFactory for SeaOrmStore {
    // @impl SPEC_ARCH_UOW
    async fn begin(&self) -> PortResult<Box<dyn UnitOfWork>> {
        // Span über die Dauer der Transaktion, endet mit `commit` oder Abbruch
        // (SPEC_OPS_OTEL)
        let span = tracing::info_span!("db.transaction");
        let txn = self
            .db
            .begin()
            .instrument(span.clone())
            .await
            .map_err(db_err)?;
        Ok(Box::new(SeaOrmUow { txn, span }))
    }
}

#[async_trait]
impl Readiness for SeaOrmStore {
    async fn check(&self) -> PortResult<()> {
        self.db.ping().await.map_err(db_err)
    }
}

/// Offene Entwürfe (`pending`, `failed`).
/// Nicht abgeschlossene Entwürfe (Warteschlange, Analyse, Bestätigung offen).
fn open_status() -> Vec<&'static str> {
    DraftStatus::ALL
        .into_iter()
        .filter(|s| s.is_open())
        .map(DraftStatus::as_str)
        .collect()
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

    async fn expire_setup_drafts(&self, cutoff: DateTime<Utc>) -> PortResult<Vec<BlobKey>> {
        let expired = setup_draft::Entity::find()
            .filter(setup_draft::Column::UpdatedAt.lt(cutoff))
            .all(&self.db)
            .await
            .map_err(db_err)?;
        let mut keys = Vec::new();
        for row in &expired {
            let draft: SetupDraft = serde_json::from_str(&row.payload).map_err(data_err)?;
            for key in draft.photos.values() {
                let parsed = BlobKey::parse(key).ok_or_else(|| data_err("setup photo key"))?;
                if parsed.household() != HouseholdId(row.household_id) {
                    return Err(data_err("setup photo household"));
                }
                keys.push(parsed);
            }
        }
        if !expired.is_empty() {
            setup_draft::Entity::delete_many()
                .filter(
                    setup_draft::Column::HouseholdId
                        .is_in(expired.iter().map(|row| row.household_id)),
                )
                .filter(setup_draft::Column::UpdatedAt.lt(cutoff))
                .exec(&self.db)
                .await
                .map_err(db_err)?;
        }
        Ok(keys)
    }

    // @impl SPEC_OPS_HOUSEKEEPING
    async fn delete_expired_sessions(&self, now: DateTime<Utc>) -> PortResult<u64> {
        crate::session_store::delete_expired(&self.db, now)
            .await
            .map_err(db_err)
    }

    // @impl SPEC_OPS_HOUSEKEEPING, SPEC_JOB_OUTBOX
    async fn purge_outbox(&self, before: DateTime<Utc>) -> PortResult<u64> {
        let res = outbox::Entity::delete_many()
            .filter(outbox::Column::Status.eq(OutboxStatus::Done.as_str()))
            .filter(outbox::Column::FinishedAt.lt(before))
            .exec(&self.db)
            .await
            .map_err(db_err)?;
        Ok(res.rows_affected)
    }

    // @impl SPEC_OPS_HOUSEKEEPING, SPEC_DOC_UPLOADS
    async fn expire_documents(&self, cutoff: DateTime<Utc>) -> PortResult<Vec<BlobKey>> {
        self.expire_documents_impl(cutoff).await
    }
}

struct SeaOrmUow {
    txn: DatabaseTransaction,
    span: tracing::Span,
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
    fn invitations(&mut self) -> &mut dyn InvitationRepository {
        self
    }
    fn revisions(&mut self) -> &mut dyn RevisionRepository {
        self
    }
    fn outbox(&mut self) -> &mut dyn OutboxRepository {
        self
    }
    fn notification_settings(&mut self) -> &mut dyn NotificationSettingsRepository {
        self
    }
    fn contracts(&mut self) -> &mut dyn ContractRepository {
        self
    }
    fn vehicles(&mut self) -> &mut dyn zaehler_app::port::VehicleRepository {
        self
    }
    fn pv(&mut self) -> &mut dyn zaehler_app::port::PvRepository {
        self
    }
    fn asset_costs(&mut self) -> &mut dyn zaehler_app::port::AssetCostRepository {
        self
    }
    fn documents(&mut self) -> &mut dyn zaehler_app::port::DocumentRepository {
        self
    }
    fn api_tokens(&mut self) -> &mut dyn zaehler_app::port::ApiTokenRepository {
        self
    }
    fn setup_drafts(&mut self) -> &mut dyn SetupDraftRepository {
        self
    }
    async fn commit(self: Box<Self>) -> PortResult<()> {
        let Self { txn, span } = *self;
        txn.commit().instrument(span).await.map_err(db_err)
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
            schedule: schedule_from_columns(
                m.schedule_kind.as_deref(),
                m.schedule_value.map(i64::from),
                m.reading_interval_days.map(i64::from),
            ),
            max_daily_rate: m.max_daily_rate_milli.map(fixed::quantity_from_db),
            billing_start,
            purpose: m.purpose.parse().map_err(data_err)?,
            targets: MeterTargets {
                season_weights: m
                    .season_weights
                    .as_deref()
                    .map(|w| season_weights_from_json(w).ok_or_else(|| data_err("season_weights")))
                    .transpose()?,
                annual_target: m.annual_target_milli.map(fixed::quantity_from_db),
                tolerance_pct: m
                    .target_tolerance_pct
                    .map(u8::try_from)
                    .transpose()
                    .map_err(data_err)?,
            },
        },
        archived: m.archived,
        archived_at: m.archived_at,
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
            parent_meter_id: m.parent_meter_id.map(MeterId),
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

/// Spalten einer Messstelle, die aus `MeterData` abgeleitet werden.
struct MeterColumns {
    rate: Option<i64>,
    month: Option<i16>,
    day: Option<i16>,
    /// Turnus für ältere Programmversionen (nur `EveryDays`)
    interval: Option<i32>,
    schedule_kind: Option<String>,
    schedule_value: Option<i16>,
    season_weights: Option<String>,
    annual_target: Option<i64>,
    tolerance: Option<i16>,
}

fn meter_columns(data: &MeterData) -> PortResult<MeterColumns> {
    let rate = data
        .max_daily_rate
        .map(fixed::quantity_to_db)
        .transpose()
        .map_err(data_err)?;
    let (month, day) = data.billing_start.map_or((None, None), |md| {
        (Some(i16::from(md.month)), Some(i16::from(md.day)))
    });
    let (kind, value, legacy) = schedule_columns(data.schedule);
    Ok(MeterColumns {
        rate,
        month,
        day,
        interval: legacy.map(i32::from),
        schedule_kind: kind.map(str::to_owned),
        schedule_value: value.map(i16::try_from).transpose().map_err(data_err)?,
        season_weights: data
            .targets
            .season_weights
            .as_ref()
            .map(season_weights_to_json),
        annual_target: data
            .targets
            .annual_target
            .map(fixed::quantity_to_db)
            .transpose()
            .map_err(data_err)?,
        tolerance: data.targets.tolerance_pct.map(i16::from),
    })
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

    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<Member>> {
        let memberships = membership::Entity::find()
            .filter(membership::Column::HouseholdId.eq(t.household_id().0))
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let names: HashMap<Uuid, String> = user::Entity::find()
            .filter(user::Column::Id.is_in(memberships.iter().map(|m| m.user_id)))
            .all(&self.txn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|u| (u.id, u.display_name))
            .collect();
        let mut out = memberships
            .into_iter()
            .map(|m| {
                Ok(Member {
                    user_id: UserId(m.user_id),
                    display_name: names.get(&m.user_id).cloned().unwrap_or_default(),
                    role: m.role.parse::<Role>().map_err(data_err)?,
                })
            })
            .collect::<PortResult<Vec<_>>>()?;
        out.sort_by(|a, b| {
            a.display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase())
                .then(a.user_id.cmp(&b.user_id))
        });
        Ok(out)
    }

    // @impl SPEC_TEN_ROLES
    async fn lock_household(&mut self, t: &Tenant) -> PortResult<()> {
        // SQLite serialisiert schreibende Transaktionen ohnehin
        if self.txn.get_database_backend() == DbBackend::Postgres {
            membership::Entity::find()
                .filter(membership::Column::HouseholdId.eq(t.household_id().0))
                .lock_exclusive()
                .all(&self.txn)
                .await
                .map_err(db_err)?;
        }
        Ok(())
    }

    async fn set_role(&mut self, t: &Tenant, user: UserId, role: Role) -> PortResult<()> {
        let res = membership::Entity::update_many()
            .col_expr(membership::Column::Role, Expr::value(role.as_str()))
            .filter(membership::Column::HouseholdId.eq(t.household_id().0))
            .filter(membership::Column::UserId.eq(user.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }

    async fn remove(&mut self, t: &Tenant, user: UserId) -> PortResult<()> {
        let res = membership::Entity::delete_many()
            .filter(membership::Column::HouseholdId.eq(t.household_id().0))
            .filter(membership::Column::UserId.eq(user.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }

    async fn add(&mut self, household: HouseholdId, user: UserId, role: Role) -> PortResult<()> {
        match MembershipRepository::role(self, user, household).await? {
            Some(existing) if existing >= role => Ok(()),
            Some(_) => {
                membership::Entity::update_many()
                    .col_expr(membership::Column::Role, Expr::value(role.as_str()))
                    .filter(membership::Column::HouseholdId.eq(household.0))
                    .filter(membership::Column::UserId.eq(user.0))
                    .exec(&self.txn)
                    .await
                    .map_err(db_err)?;
                Ok(())
            }
            None => {
                membership::Entity::insert(membership::ActiveModel {
                    household_id: Set(household.0),
                    user_id: Set(user.0),
                    role: Set(role.as_str().to_owned()),
                })
                .exec_without_returning(&self.txn)
                .await
                .map_err(db_err)?;
                Ok(())
            }
        }
    }
}

fn to_invitation(m: invitation::Model, household_name: String) -> PortResult<Invitation> {
    Ok(Invitation {
        id: InvitationId(m.id),
        household_id: HouseholdId(m.household_id),
        household_name,
        role: m.role.parse().map_err(data_err)?,
        email: m.email,
        created_by: UserId(m.created_by),
        created_at: m.created_at,
        expires_at: m.expires_at,
        accepted_by: m.accepted_by.map(UserId),
        accepted_at: m.accepted_at,
        revoked_at: m.revoked_at,
    })
}

// @impl SPEC_TEN_INVITES, SPEC_SEC_TENANT_SCOPE
#[async_trait]
impl InvitationRepository for SeaOrmUow {
    async fn create(&mut self, t: &Tenant, i: &NewInvitation) -> PortResult<()> {
        invitation::Entity::insert(invitation::ActiveModel {
            id: Set(i.id.0),
            household_id: Set(t.household_id().0),
            role: Set(i.role.as_str().to_owned()),
            token_hash: Set(secret::to_hex(&i.token_hash)),
            email: Set(i.email.clone()),
            created_by: Set(i.created_by.0),
            created_at: Set(i.created_at),
            expires_at: Set(i.expires_at),
            accepted_by: Set(None),
            accepted_at: Set(None),
            revoked_at: Set(None),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn find_by_hash(&mut self, hash: &[u8; 32]) -> PortResult<Option<Invitation>> {
        let Some(m) = invitation::Entity::find()
            .filter(invitation::Column::TokenHash.eq(secret::to_hex(hash)))
            .one(&self.txn)
            .await
            .map_err(db_err)?
        else {
            return Ok(None);
        };
        let name = household::Entity::find_by_id(m.household_id)
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|h| h.name)
            .unwrap_or_default();
        to_invitation(m, name).map(Some)
    }

    async fn mark_accepted(
        &mut self,
        id: InvitationId,
        user: UserId,
        at: DateTime<Utc>,
    ) -> PortResult<bool> {
        // Bedingtes Update: von parallelen Einlösungen ändert nur eine die Zeile
        let res = invitation::Entity::update_many()
            .col_expr(invitation::Column::AcceptedBy, Expr::value(Some(user.0)))
            .col_expr(invitation::Column::AcceptedAt, Expr::value(Some(at)))
            .filter(invitation::Column::Id.eq(id.0))
            .filter(invitation::Column::AcceptedAt.is_null())
            .filter(invitation::Column::RevokedAt.is_null())
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(res.rows_affected == 1)
    }

    async fn list_open(&mut self, t: &Tenant, now: DateTime<Utc>) -> PortResult<Vec<Invitation>> {
        let name = HouseholdRepository::get(self, t).await?.data.name;
        invitation::Entity::find()
            .filter(invitation::Column::HouseholdId.eq(t.household_id().0))
            .filter(invitation::Column::AcceptedAt.is_null())
            .filter(invitation::Column::RevokedAt.is_null())
            .filter(invitation::Column::ExpiresAt.gt(now))
            .order_by_desc(invitation::Column::CreatedAt)
            .order_by_desc(invitation::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|m| to_invitation(m, name.clone()))
            .collect()
    }

    async fn revoke(&mut self, t: &Tenant, id: InvitationId, at: DateTime<Utc>) -> PortResult<()> {
        let res = invitation::Entity::update_many()
            .col_expr(invitation::Column::RevokedAt, Expr::value(Some(at)))
            .filter(invitation::Column::Id.eq(id.0))
            .filter(invitation::Column::HouseholdId.eq(t.household_id().0))
            .filter(invitation::Column::AcceptedAt.is_null())
            .filter(invitation::Column::RevokedAt.is_null())
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
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
        let c = meter_columns(data)?;
        meter::Entity::insert(meter::ActiveModel {
            id: Set(id.0),
            household_id: Set(t.household_id().0),
            name: Set(data.name.clone()),
            kind: Set(data.kind.as_str().to_owned()),
            unit: Set(data.unit.clone()),
            reading_interval_days: Set(c.interval),
            max_daily_rate_milli: Set(c.rate),
            billing_start_month: Set(c.month),
            billing_start_day: Set(c.day),
            archived: Set(false),
            created_at: Set(now),
            purpose: Set(data.purpose.as_str().to_owned()),
            schedule_kind: Set(c.schedule_kind),
            schedule_value: Set(c.schedule_value),
            archived_at: Set(None),
            season_weights: Set(c.season_weights),
            annual_target_milli: Set(c.annual_target),
            target_tolerance_pct: Set(c.tolerance),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn update(&mut self, t: &Tenant, id: MeterId, data: &MeterData) -> PortResult<()> {
        let c = meter_columns(data)?;
        let result = meter::Entity::update_many()
            .col_expr(meter::Column::Name, Expr::value(data.name.clone()))
            .col_expr(
                meter::Column::Kind,
                Expr::value(data.kind.as_str().to_owned()),
            )
            .col_expr(meter::Column::Unit, Expr::value(data.unit.clone()))
            .col_expr(meter::Column::ReadingIntervalDays, Expr::value(c.interval))
            .col_expr(meter::Column::MaxDailyRateMilli, Expr::value(c.rate))
            .col_expr(meter::Column::BillingStartMonth, Expr::value(c.month))
            .col_expr(meter::Column::BillingStartDay, Expr::value(c.day))
            .col_expr(
                meter::Column::Purpose,
                Expr::value(data.purpose.as_str().to_owned()),
            )
            .col_expr(meter::Column::ScheduleKind, Expr::value(c.schedule_kind))
            .col_expr(meter::Column::ScheduleValue, Expr::value(c.schedule_value))
            .col_expr(meter::Column::SeasonWeights, Expr::value(c.season_weights))
            .col_expr(
                meter::Column::AnnualTargetMilli,
                Expr::value(c.annual_target),
            )
            .col_expr(meter::Column::TargetTolerancePct, Expr::value(c.tolerance))
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
            parent_meter_id: Set(data.parent_meter_id.map(|m| m.0)),
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
            .col_expr(
                meter_device::Column::ParentMeterId,
                Expr::value(data.parent_meter_id.map(|m| m.0)),
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

    // @impl SPEC_MTR_SUBMETER
    async fn submeters_of(&mut self, t: &Tenant, parent: MeterId) -> PortResult<Vec<MeterId>> {
        if !self.owns_meter(t, parent.0).await? {
            return Ok(Vec::new());
        }
        let mut ids: Vec<Uuid> = meter_device::Entity::find()
            .select_only()
            .column(meter_device::Column::MeterId)
            .filter(meter_device::Column::ParentMeterId.eq(parent.0))
            .into_tuple()
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        ids.sort();
        ids.dedup();
        let mut out = Vec::new();
        for id in ids {
            if self.owns_meter(t, id).await? {
                out.push(MeterId(id));
            }
        }
        Ok(out)
    }

    // @impl SPEC_MTR_ARCHIVE
    async fn set_archived(
        &mut self,
        t: &Tenant,
        id: MeterId,
        at: Option<DateTime<Utc>>,
    ) -> PortResult<()> {
        let result = meter::Entity::update_many()
            .col_expr(meter::Column::Archived, Expr::value(at.is_some()))
            .col_expr(meter::Column::ArchivedAt, Expr::value(at))
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

    // @impl SPEC_MTR_DELETE
    async fn delete(&mut self, t: &Tenant, id: MeterId) -> PortResult<()> {
        if !self.owns_meter(t, id.0).await? {
            return Err(PortError::NotFound);
        }
        let hid = t.household_id().0;
        // Spalten ohne Fremdschlüssel; der Rest folgt über ON DELETE CASCADE
        reading_revision::Entity::delete_many()
            .filter(reading_revision::Column::HouseholdId.eq(hid))
            .filter(reading_revision::Column::MeterId.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        crate::entity::document::Entity::update_many()
            .col_expr(
                crate::entity::document::Column::MeterId,
                Expr::value(Option::<Uuid>::None),
            )
            .filter(crate::entity::document::Column::HouseholdId.eq(hid))
            .filter(crate::entity::document::Column::MeterId.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        ai_draft::Entity::update_many()
            .col_expr(
                ai_draft::Column::MatchedMeterId,
                Expr::value(Option::<Uuid>::None),
            )
            .filter(ai_draft::Column::HouseholdId.eq(hid))
            .filter(ai_draft::Column::MatchedMeterId.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        meter::Entity::delete_many()
            .filter(meter::Column::Id.eq(id.0))
            .filter(meter::Column::HouseholdId.eq(hid))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
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
            reading::Entity::insert(reading_model(r, None)?)
                .exec_without_returning(&self.txn)
                .await
                .map_err(db_err)?;
        }
        Ok(())
    }

    async fn upsert_daily(
        &mut self,
        t: &Tenant,
        r: &Reading,
        token: zaehler_app::ids::ApiTokenId,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> PortResult<zaehler_app::model::DailyOutcome> {
        let meter_id = self
            .meter_of_register(r.register_id.0)
            .await?
            .ok_or(PortError::NotFound)?;
        if !self.owns_meter(t, meter_id).await?
            || self.meter_of_device(r.device_id.0).await? != Some(meter_id)
        {
            return Err(PortError::NotFound);
        }
        self.upsert_daily_in(r, token, from, until).await
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

    async fn update(&mut self, t: &Tenant, id: ReadingId, u: &ReadingUpdate) -> PortResult<()> {
        if ReadingRepository::get(self, t, id).await?.is_none() {
            return Err(PortError::NotFound);
        }
        reading::Entity::update_many()
            .col_expr(reading::Column::ReadAt, Expr::value(u.read_at))
            .col_expr(
                reading::Column::ValueMilli,
                Expr::value(fixed::quantity_to_db(u.value).map_err(data_err)?),
            )
            .col_expr(reading::Column::Note, Expr::value(u.note.clone()))
            .col_expr(reading::Column::UpdatedAt, Expr::value(Some(u.updated_at)))
            .filter(reading::Column::Id.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

// @impl SPEC_JOB_OUTBOX
#[async_trait]
impl OutboxRepository for SeaOrmUow {
    async fn enqueue(
        &mut self,
        t: &Tenant,
        id: OutboxId,
        event: &OutboxEvent,
        now: DateTime<Utc>,
    ) -> PortResult<()> {
        outbox::Entity::insert(outbox::ActiveModel {
            id: Set(id.0),
            household_id: Set(t.household_id().0),
            kind: Set(event.kind().to_owned()),
            payload: Set(serde_json::to_string(event).map_err(data_err)?),
            status: Set(OutboxStatus::Pending.as_str().to_owned()),
            attempts: Set(0),
            next_attempt_at: Set(now),
            created_at: Set(now),
            finished_at: Set(None),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }
}

// @impl SPEC_NTF_EMAIL
#[async_trait]
impl NotificationSettingsRepository for SeaOrmUow {
    async fn get(&mut self, user: UserId) -> PortResult<Option<NotificationSettings>> {
        Ok(notification_settings::Entity::find_by_id(user.0)
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|m| NotificationSettings {
                email: m.email,
                reminder: m.reminder,
                alarm: m.alarm,
            }))
    }

    async fn set(
        &mut self,
        user: UserId,
        settings: &NotificationSettings,
        now: DateTime<Utc>,
    ) -> PortResult<()> {
        notification_settings::Entity::insert(notification_settings::ActiveModel {
            user_id: Set(user.0),
            email: Set(settings.email.clone()),
            reminder: Set(settings.reminder),
            alarm: Set(settings.alarm),
            updated_at: Set(now),
        })
        .on_conflict(
            sea_orm::sea_query::OnConflict::column(notification_settings::Column::UserId)
                .update_columns([
                    notification_settings::Column::Email,
                    notification_settings::Column::Reminder,
                    notification_settings::Column::Alarm,
                    notification_settings::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }
}

fn to_revision(m: reading_revision::Model) -> PortResult<Revision> {
    Ok(Revision {
        id: RevisionId(m.id),
        reading_id: ReadingId(m.reading_id),
        meter_id: MeterId(m.meter_id),
        register_id: RegisterId(m.register_id),
        action: m.action.parse().map_err(data_err)?,
        changed_by: UserId(m.changed_by),
        changed_at: m.changed_at,
        old_read_at: m.old_read_at,
        old_value: fixed::quantity_from_db(m.old_value_milli),
        old_note: m.old_note,
        new_read_at: m.new_read_at,
        new_value: m.new_value_milli.map(fixed::quantity_from_db),
        new_note: m.new_note,
    })
}

// @impl SPEC_RDG_HISTORY, SPEC_SEC_TENANT_SCOPE
#[async_trait]
impl RevisionRepository for SeaOrmUow {
    async fn append(&mut self, t: &Tenant, r: &Revision) -> PortResult<()> {
        if !self.owns_meter(t, r.meter_id.0).await?
            || self.meter_of_register(r.register_id.0).await? != Some(r.meter_id.0)
        {
            return Err(PortError::NotFound);
        }
        reading_revision::Entity::insert(reading_revision::ActiveModel {
            id: Set(r.id.0),
            household_id: Set(t.household_id().0),
            reading_id: Set(r.reading_id.0),
            meter_id: Set(r.meter_id.0),
            register_id: Set(r.register_id.0),
            action: Set(r.action.as_str().to_owned()),
            changed_by: Set(r.changed_by.0),
            changed_at: Set(r.changed_at),
            old_read_at: Set(r.old_read_at),
            old_value_milli: Set(fixed::quantity_to_db(r.old_value).map_err(data_err)?),
            old_note: Set(r.old_note.clone()),
            new_read_at: Set(r.new_read_at),
            new_value_milli: Set(r
                .new_value
                .map(fixed::quantity_to_db)
                .transpose()
                .map_err(data_err)?),
            new_note: Set(r.new_note.clone()),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn for_meter(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Vec<Revision>> {
        reading_revision::Entity::find()
            .filter(reading_revision::Column::HouseholdId.eq(t.household_id().0))
            .filter(reading_revision::Column::MeterId.eq(meter.0))
            .order_by_desc(reading_revision::Column::ChangedAt)
            .order_by_desc(reading_revision::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(to_revision)
            .collect()
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
        taken_at: m.taken_at,
        analysis_started_at: m.analysis_started_at,
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
            taken_at: Set(d.taken_at),
            analysis_started_at: Set(d.analysis_started_at),
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

    async fn list_open(&mut self, t: &Tenant) -> PortResult<Vec<Draft>> {
        ai_draft::Entity::find()
            .filter(ai_draft::Column::HouseholdId.eq(t.household_id().0))
            .filter(ai_draft::Column::Status.is_in(open_status()))
            .order_by_asc(ai_draft::Column::CreatedAt)
            .order_by_asc(ai_draft::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(to_draft)
            .collect()
    }

    // @impl SPEC_AI_MULTI
    async fn claim(
        &mut self,
        t: &Tenant,
        id: DraftId,
        now: DateTime<Utc>,
        stale_before: DateTime<Utc>,
    ) -> PortResult<bool> {
        // Bedingtes Update: nur ein paralleler Lauf ändert die Zeile
        let claimable = Condition::any()
            .add(ai_draft::Column::Status.eq(DraftStatus::Queued.as_str()))
            .add(
                Condition::all()
                    .add(ai_draft::Column::Status.eq(DraftStatus::Analysing.as_str()))
                    .add(
                        Condition::any()
                            .add(ai_draft::Column::AnalysisStartedAt.is_null())
                            .add(ai_draft::Column::AnalysisStartedAt.lt(stale_before)),
                    ),
            );
        let res = ai_draft::Entity::update_many()
            .col_expr(
                ai_draft::Column::Status,
                Expr::value(DraftStatus::Analysing.as_str()),
            )
            .col_expr(ai_draft::Column::AnalysisStartedAt, Expr::value(now))
            .filter(ai_draft::Column::Id.eq(id.0))
            .filter(ai_draft::Column::HouseholdId.eq(t.household_id().0))
            .filter(claimable)
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(res.rows_affected == 1)
    }

    async fn record_analysis(
        &mut self,
        t: &Tenant,
        id: DraftId,
        a: &DraftAnalysis,
    ) -> PortResult<()> {
        if let Some(m) = a.matched_meter_id {
            self.require_meter(t, m.0).await?;
        }
        let suggestion = a
            .suggestion
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(data_err)?;
        let res = ai_draft::Entity::update_many()
            .col_expr(ai_draft::Column::Model, Expr::value(a.model.clone()))
            .col_expr(ai_draft::Column::DurationMs, Expr::value(a.duration_ms))
            .col_expr(ai_draft::Column::Suggestion, Expr::value(suggestion))
            .col_expr(
                ai_draft::Column::MatchedMeterId,
                Expr::value(a.matched_meter_id.map(|m| m.0)),
            )
            .col_expr(ai_draft::Column::Status, Expr::value(a.status.as_str()))
            .filter(ai_draft::Column::Id.eq(id.0))
            .filter(ai_draft::Column::HouseholdId.eq(t.household_id().0))
            .filter(ai_draft::Column::Status.eq(DraftStatus::Analysing.as_str()))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
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

#[path = "contracts.rs"]
mod contracts;

#[path = "documents.rs"]
mod documents;

#[path = "vehicles.rs"]
mod vehicles;

#[path = "pv.rs"]
mod pv;

#[path = "api.rs"]
mod api;

#[path = "setup.rs"]
mod setup;

/// Neue Zeile einer Ablesung; `token` bei automatisch gelieferten Ständen.
fn reading_model(r: &Reading, token: Option<Uuid>) -> PortResult<reading::ActiveModel> {
    Ok(reading::ActiveModel {
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
        updated_at: Set(None),
        api_token_id: Set(token),
    })
}
