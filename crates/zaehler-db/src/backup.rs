//! Export und Import aller Datensätze (SPEC_BACKUP_DATA).
//!
//! Der Export liest jede Tabelle mit genau den tatsächlich vorhandenen
//! Spalten. Damit kann eine neue Programmversion auch ein noch
//! nicht migriertes, älteres Schema exportieren (Sicherung vor der Migration,
//! SPEC_OPS_UPGRADE); später ergänzte Spalten erhalten ihre Vorgabe.

use std::collections::HashSet;

use async_trait::async_trait;
use sea_orm::{
    AccessMode,
    ActiveValue::Set,
    ConnectionTrait, DatabaseTransaction, DbBackend, DbErr, EntityTrait, IsolationLevel,
    QueryResult, Statement, TransactionTrait, TryGetable,
    sea_query::{Alias, Query},
};
use zaehler_app::{
    PortError,
    backup::{
        BackupData, DeviceRecord, DraftRecord, HouseholdRecord, MembershipRecord, MeterRecord,
        ReadingRecord, RegisterRecord, UserRecord,
    },
    port::{BackupRepository, PortResult},
};

use crate::{
    entity::{ai_draft, household, membership, meter, meter_device, reading, register, user},
    uow::SeaOrmStore,
};

/// Zeilen je `INSERT` (SQLite begrenzt die Zahl der Parameter).
const CHUNK: usize = 200;

#[allow(clippy::needless_pass_by_value)] // als Funktion für `map_err` verwendet
fn db_err(e: DbErr) -> PortError {
    PortError::Backend(e.to_string())
}

/// Zeile mit den im Schema vorhandenen Spalten.
struct Row<'a> {
    row: &'a QueryResult,
    cols: &'a HashSet<String>,
}

impl Row<'_> {
    fn get<T: TryGetable>(&self, col: &str) -> PortResult<T> {
        self.row
            .try_get("", col)
            .map_err(|e| PortError::Backend(format!("Spalte {col}: {e}")))
    }

    /// Spalte, die ältere Schemata nicht haben; dort `None`.
    fn opt<T>(&self, col: &str) -> PortResult<Option<T>>
    where
        Option<T>: TryGetable,
    {
        if self.cols.contains(col) {
            self.get(col)
        } else {
            Ok(None)
        }
    }

    /// Spalte, die ältere Schemata nicht haben; dort `default`.
    fn or<T: TryGetable>(&self, col: &str, default: T) -> PortResult<T> {
        if self.cols.contains(col) {
            self.get(col)
        } else {
            Ok(default)
        }
    }
}

/// Spalten und Zeilen einer Tabelle; `None`, wenn die Tabelle im Schema fehlt.
/// `table` stammt immer aus einer festen Liste dieses Moduls.
async fn table<C: ConnectionTrait>(
    db: &C,
    table: &'static str,
) -> PortResult<Option<(HashSet<String>, Vec<QueryResult>)>> {
    let backend = db.get_database_backend();
    let columns = match backend {
        DbBackend::Sqlite => Statement::from_sql_and_values(
            backend,
            "SELECT name FROM pragma_table_info(?)",
            [table.into()],
        ),
        _ => Statement::from_sql_and_values(
            backend,
            "SELECT column_name AS name FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1",
            [table.into()],
        ),
    };
    let cols: HashSet<String> = db
        .query_all_raw(columns)
        .await
        .map_err(db_err)?
        .iter()
        .map(|r| r.try_get::<String>("", "name").map_err(db_err))
        .collect::<PortResult<_>>()?;
    if cols.is_empty() {
        return Ok(None);
    }
    // Explizite Spaltenliste statt `SELECT *` über den Query-Builder (quotet
    // die Bezeichner): Nach einer Migration ändert sich damit der SQL-Text,
    // und kein vorbereitetes Statement mit altem Spaltenstand wird
    // wiederverwendet (sqlx-Statement-Cache).
    let mut names: Vec<&String> = cols.iter().collect();
    names.sort();
    let query = Query::select()
        .columns(names.into_iter().map(|c| Alias::new(c.as_str())))
        .from(Alias::new(table))
        .to_owned();
    let rows = db
        .query_all_raw(backend.build(&query))
        .await
        .map_err(db_err)?;
    Ok(Some((cols, rows)))
}

/// Liest alle Zeilen einer Tabelle über `map`; fehlende Tabelle ergibt nichts.
async fn records<C: ConnectionTrait, T>(
    db: &C,
    name: &'static str,
    map: impl Fn(&Row) -> PortResult<T>,
) -> PortResult<Vec<T>> {
    let Some((cols, rows)) = table(db, name).await? else {
        return Ok(Vec::new());
    };
    rows.iter()
        .map(|row| map(&Row { row, cols: &cols }))
        .collect()
}

#[allow(clippy::too_many_lines)] // eine Abbildung je Tabelle
async fn read_all(db: &DatabaseTransaction) -> PortResult<BackupData> {
    let mut data = BackupData {
        users: records(db, "users", |r| {
            Ok(UserRecord {
                id: r.get("id")?,
                oidc_issuer: r.get("oidc_issuer")?,
                oidc_subject: r.get("oidc_subject")?,
                email: r.get("email")?,
                display_name: r.get("display_name")?,
                locale: r.get("locale")?,
                created_at: r.get("created_at")?,
                last_login_at: r.opt("last_login_at")?,
            })
        })
        .await?,
        households: records(db, "households", |r| {
            Ok(HouseholdRecord {
                id: r.get("id")?,
                name: r.get("name")?,
                timezone: r.get("timezone")?,
                created_at: r.get("created_at")?,
            })
        })
        .await?,
        memberships: records(db, "memberships", |r| {
            Ok(MembershipRecord {
                household_id: r.get("household_id")?,
                user_id: r.get("user_id")?,
                role: r.get("role")?,
            })
        })
        .await?,
        meters: records(db, "meters", |r| {
            Ok(MeterRecord {
                id: r.get("id")?,
                household_id: r.get("household_id")?,
                name: r.get("name")?,
                kind: r.get("kind")?,
                unit: r.get("unit")?,
                reading_interval_days: r.get("reading_interval_days")?,
                max_daily_rate_milli: r.get("max_daily_rate_milli")?,
                billing_start_month: r.opt("billing_start_month")?,
                billing_start_day: r.opt("billing_start_day")?,
                archived: r.get("archived")?,
                created_at: r.get("created_at")?,
            })
        })
        .await?,
        devices: records(db, "meter_devices", |r| {
            Ok(DeviceRecord {
                id: r.get("id")?,
                meter_id: r.get("meter_id")?,
                serial_number: r.get("serial_number")?,
                installed_at: r.get("installed_at")?,
                removed_at: r.get("removed_at")?,
                rollover_digits: r.get("rollover_digits")?,
                decimal_places: r.or("decimal_places", 0)?,
            })
        })
        .await?,
        registers: records(db, "registers", |r| {
            Ok(RegisterRecord {
                id: r.get("id")?,
                meter_id: r.get("meter_id")?,
                code: r.get("code")?,
                direction: r.get("direction")?,
            })
        })
        .await?,
        readings: records(db, "readings", |r| {
            Ok(ReadingRecord {
                id: r.get("id")?,
                register_id: r.get("register_id")?,
                device_id: r.get("device_id")?,
                read_at: r.get("read_at")?,
                value_milli: r.get("value_milli")?,
                source: r.get("source")?,
                photo_key: r.get("photo_key")?,
                ai_confidence: r.get("ai_confidence")?,
                note: r.get("note")?,
                created_by: r.get("created_by")?,
                created_at: r.get("created_at")?,
            })
        })
        .await?,
        drafts: records(db, "ai_drafts", |r| {
            Ok(DraftRecord {
                id: r.get("id")?,
                household_id: r.get("household_id")?,
                created_by: r.get("created_by")?,
                photo_key: r.get("photo_key")?,
                model: r.get("model")?,
                duration_ms: r.get("duration_ms")?,
                suggestion: r.get("suggestion")?,
                matched_meter_id: r.get("matched_meter_id")?,
                status: r.get("status")?,
                confirmed_reading_id: r.get("confirmed_reading_id")?,
                corrected: r.get("corrected")?,
                created_at: r.get("created_at")?,
                finished_at: r.get("finished_at")?,
                taken_at: r.opt("taken_at")?,
                analysis_started_at: r.opt("analysis_started_at")?,
            })
        })
        .await?,
    };
    // feste Reihenfolge, damit gleiche Daten gleiche Archive ergeben
    data.users.sort_by_key(|x| x.id);
    data.households.sort_by_key(|x| x.id);
    data.memberships
        .sort_by_key(|x| (x.household_id, x.user_id));
    data.meters.sort_by_key(|x| x.id);
    data.devices.sort_by_key(|x| x.id);
    data.registers.sort_by_key(|x| x.id);
    data.readings.sort_by_key(|x| x.id);
    data.drafts.sort_by_key(|x| x.id);
    Ok(data)
}

async fn insert<E, A>(txn: &DatabaseTransaction, models: Vec<A>) -> PortResult<()>
where
    E: EntityTrait,
    A: sea_orm::ActiveModelTrait<Entity = E> + Send,
{
    let mut models = models.into_iter().peekable();
    while models.peek().is_some() {
        let chunk: Vec<A> = models.by_ref().take(CHUNK).collect();
        E::insert_many(chunk)
            .exec_without_returning(txn)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // eine Abbildung je Tabelle
async fn write_all(txn: &DatabaseTransaction, d: &BackupData) -> PortResult<()> {
    insert(
        txn,
        d.users
            .iter()
            .map(|u| user::ActiveModel {
                id: Set(u.id),
                oidc_issuer: Set(u.oidc_issuer.clone()),
                oidc_subject: Set(u.oidc_subject.clone()),
                email: Set(u.email.clone()),
                display_name: Set(u.display_name.clone()),
                locale: Set(u.locale.clone()),
                created_at: Set(u.created_at),
                last_login_at: Set(u.last_login_at),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.households
            .iter()
            .map(|h| household::ActiveModel {
                id: Set(h.id),
                name: Set(h.name.clone()),
                timezone: Set(h.timezone.clone()),
                created_at: Set(h.created_at),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.memberships
            .iter()
            .map(|m| membership::ActiveModel {
                household_id: Set(m.household_id),
                user_id: Set(m.user_id),
                role: Set(m.role.clone()),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.meters
            .iter()
            .map(|m| meter::ActiveModel {
                id: Set(m.id),
                household_id: Set(m.household_id),
                name: Set(m.name.clone()),
                kind: Set(m.kind.clone()),
                unit: Set(m.unit.clone()),
                reading_interval_days: Set(m.reading_interval_days),
                max_daily_rate_milli: Set(m.max_daily_rate_milli),
                billing_start_month: Set(m.billing_start_month),
                billing_start_day: Set(m.billing_start_day),
                archived: Set(m.archived),
                created_at: Set(m.created_at),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.devices
            .iter()
            .map(|x| meter_device::ActiveModel {
                id: Set(x.id),
                meter_id: Set(x.meter_id),
                serial_number: Set(x.serial_number.clone()),
                installed_at: Set(x.installed_at),
                removed_at: Set(x.removed_at),
                rollover_digits: Set(x.rollover_digits),
                decimal_places: Set(x.decimal_places),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.registers
            .iter()
            .map(|r| register::ActiveModel {
                id: Set(r.id),
                meter_id: Set(r.meter_id),
                code: Set(r.code.clone()),
                direction: Set(r.direction.clone()),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.readings
            .iter()
            .map(|r| reading::ActiveModel {
                id: Set(r.id),
                register_id: Set(r.register_id),
                device_id: Set(r.device_id),
                read_at: Set(r.read_at),
                value_milli: Set(r.value_milli),
                source: Set(r.source.clone()),
                photo_key: Set(r.photo_key.clone()),
                ai_confidence: Set(r.ai_confidence),
                note: Set(r.note.clone()),
                created_by: Set(r.created_by),
                created_at: Set(r.created_at),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.drafts
            .iter()
            .map(|x| ai_draft::ActiveModel {
                id: Set(x.id),
                household_id: Set(x.household_id),
                created_by: Set(x.created_by),
                photo_key: Set(x.photo_key.clone()),
                model: Set(x.model.clone()),
                duration_ms: Set(x.duration_ms),
                suggestion: Set(x.suggestion.clone()),
                matched_meter_id: Set(x.matched_meter_id),
                status: Set(x.status.clone()),
                confirmed_reading_id: Set(x.confirmed_reading_id),
                corrected: Set(x.corrected),
                created_at: Set(x.created_at),
                finished_at: Set(x.finished_at),
                taken_at: Set(x.taken_at),
                analysis_started_at: Set(x.analysis_started_at),
            })
            .collect(),
    )
    .await
}

// @impl SPEC_BACKUP_DATA
#[async_trait]
impl BackupRepository for SeaOrmStore {
    async fn schema(&self) -> PortResult<Vec<String>> {
        let mut versions = records(self.conn(), "seaql_migrations", |r| {
            r.get::<String>("version")
        })
        .await?;
        versions.sort();
        Ok(versions)
    }

    async fn export(&self) -> PortResult<BackupData> {
        let db = self.conn();
        // ein konsistenter Stand über alle Tabellen
        let txn = if db.get_database_backend() == DbBackend::Postgres {
            db.begin_with_config(
                Some(IsolationLevel::RepeatableRead),
                Some(AccessMode::ReadOnly),
            )
            .await
        } else {
            db.begin().await
        }
        .map_err(db_err)?;
        let data = read_all(&txn).await?;
        txn.commit().await.map_err(db_err)?;
        Ok(data)
    }

    async fn is_empty(&self) -> PortResult<bool> {
        let db = self.conn();
        Ok(user::Entity::find()
            .one(db)
            .await
            .map_err(db_err)?
            .is_none()
            && household::Entity::find()
                .one(db)
                .await
                .map_err(db_err)?
                .is_none())
    }

    async fn import(&self, data: &BackupData) -> PortResult<()> {
        let txn = self.conn().begin().await.map_err(db_err)?;
        write_all(&txn, data).await?;
        txn.commit().await.map_err(db_err)
    }
}
