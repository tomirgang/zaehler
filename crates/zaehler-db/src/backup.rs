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
        AcquisitionRecord, AdjustmentRecord, ApiTokenRecord, AssetCostRecord, BackupData,
        ChargingSessionRecord, ConsumptionProfileRecord, ContractRecord, CostPositionRecord,
        DeviceRecord, DocumentRecord, DraftRecord, HouseholdRecord, InstallmentRecord,
        InvitationRecord, MembershipRecord, MeterRecord, NotificationSettingRecord,
        PriceComponentRecord, PvSystemMeterRecord, PvSystemRecord, ReadingRecord, RegisterRecord,
        RevisionRecord, SettlementRecord, UserRecord, VehicleRecord,
    },
    port::{BackupRepository, PortResult},
};

use crate::{
    entity::{
        adjustment, ai_draft, api_token, asset_cost, charging_session, contract, cost_position,
        document, household, installment, invitation, membership, meter, meter_device,
        notification_settings, price_component, pv_system, pv_system_meter, reading,
        reading_revision, register, settlement, user, vehicle, vehicle_acquisition,
        vehicle_consumption_profile,
    },
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
                purpose: r.or("purpose", "general".to_owned())?,
                schedule_kind: r.opt("schedule_kind")?,
                schedule_value: r.opt("schedule_value")?,
                archived_at: r.opt("archived_at")?,
                season_weights: r.opt("season_weights")?,
                annual_target_milli: r.opt("annual_target_milli")?,
                target_tolerance_pct: r.opt("target_tolerance_pct")?,
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
                parent_meter_id: r.opt("parent_meter_id")?,
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
                updated_at: r.opt("updated_at")?,
                api_token_id: r.opt("api_token_id")?,
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
        invitations: records(db, "invitations", |r| {
            Ok(InvitationRecord {
                id: r.get("id")?,
                household_id: r.get("household_id")?,
                role: r.get("role")?,
                token_hash: r.get("token_hash")?,
                email: r.get("email")?,
                created_by: r.get("created_by")?,
                created_at: r.get("created_at")?,
                expires_at: r.get("expires_at")?,
                accepted_by: r.get("accepted_by")?,
                accepted_at: r.get("accepted_at")?,
                revoked_at: r.get("revoked_at")?,
            })
        })
        .await?,
        revisions: records(db, "reading_revisions", |r| {
            Ok(RevisionRecord {
                id: r.get("id")?,
                household_id: r.get("household_id")?,
                reading_id: r.get("reading_id")?,
                meter_id: r.get("meter_id")?,
                register_id: r.get("register_id")?,
                action: r.get("action")?,
                changed_by: r.get("changed_by")?,
                changed_at: r.get("changed_at")?,
                old_read_at: r.get("old_read_at")?,
                old_value_milli: r.get("old_value_milli")?,
                old_note: r.get("old_note")?,
                new_read_at: r.get("new_read_at")?,
                new_value_milli: r.get("new_value_milli")?,
                new_note: r.get("new_note")?,
            })
        })
        .await?,
        notification_settings: records(db, "notification_settings", |r| {
            Ok(NotificationSettingRecord {
                user_id: r.get("user_id")?,
                email: r.get("email")?,
                reminder: r.get("reminder")?,
                alarm: r.get("alarm")?,
                updated_at: r.get("updated_at")?,
            })
        })
        .await?,
        contracts: records(db, "contracts", |r| {
            Ok(ContractRecord {
                id: r.get("id")?,
                meter_id: r.get("meter_id")?,
                direction: r.get("direction")?,
                provider: r.get("provider")?,
                contract_number: r.get("contract_number")?,
                starts_on: r.get("starts_on")?,
                ends_on: r.get("ends_on")?,
                billing_start_month: r.get("billing_start_month")?,
                billing_start_day: r.get("billing_start_day")?,
                note: r.get("note")?,
            })
        })
        .await?,
        cost_positions: records(db, "cost_positions", |r| {
            Ok(CostPositionRecord {
                id: r.get("id")?,
                contract_id: r.get("contract_id")?,
                name: r.get("name")?,
                register_id: r.get("register_id")?,
                factor_micro: r.get("factor_micro")?,
                vat_pct_milli: r.get("vat_pct_milli")?,
                sort: r.get("sort")?,
            })
        })
        .await?,
        price_components: records(db, "price_components", |r| {
            Ok(PriceComponentRecord {
                id: r.get("id")?,
                position_id: r.get("position_id")?,
                valid_from: r.get("valid_from")?,
                base_price_micro: r.get("base_price_micro")?,
                base_interval: r.get("base_interval")?,
                unit_pricing: r.get("unit_pricing")?,
                factor_micro: r.get("factor_micro")?,
            })
        })
        .await?,
        installments: records(db, "installments", |r| {
            Ok(InstallmentRecord {
                id: r.get("id")?,
                contract_id: r.get("contract_id")?,
                valid_from: r.get("valid_from")?,
                amount_micro: r.get("amount_micro")?,
                interval_months: r.get("interval_months")?,
            })
        })
        .await?,
        adjustments: records(db, "adjustments", |r| {
            Ok(AdjustmentRecord {
                id: r.get("id")?,
                contract_id: r.get("contract_id")?,
                kind: r.get("kind")?,
                booked_on: r.get("booked_on")?,
                amount_micro: r.get("amount_micro")?,
                note: r.get("note")?,
            })
        })
        .await?,
        settlements: records(db, "settlements", |r| {
            Ok(SettlementRecord {
                id: r.get("id")?,
                contract_id: r.get("contract_id")?,
                period_start: r.get("period_start")?,
                period_end: r.get("period_end")?,
                amount_micro: r.get("amount_micro")?,
                document_id: r.get("document_id")?,
            })
        })
        .await?,
        documents: records(db, "documents", |r| {
            Ok(DocumentRecord {
                id: r.get("id")?,
                household_id: r.get("household_id")?,
                kind: r.get("kind")?,
                blob_key: r.get("blob_key")?,
                media_type: r.get("media_type")?,
                bytes: r.get("bytes")?,
                original_name: r.get("original_name")?,
                created_by: r.get("created_by")?,
                created_at: r.get("created_at")?,
                status: r.get("status")?,
                model: r.get("model")?,
                extraction: r.get("extraction")?,
                meter_id: r.get("meter_id")?,
                analysed_at: r.get("analysed_at")?,
                finished_at: r.get("finished_at")?,
            })
        })
        .await?,
        ..BackupData::default()
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
    data.invitations.sort_by_key(|x| x.id);
    data.revisions.sort_by_key(|x| x.id);
    data.notification_settings.sort_by_key(|x| x.user_id);
    data.contracts.sort_by_key(|x| x.id);
    data.cost_positions.sort_by_key(|x| x.id);
    data.price_components.sort_by_key(|x| x.id);
    data.installments.sort_by_key(|x| x.id);
    data.adjustments.sort_by_key(|x| x.id);
    data.settlements.sort_by_key(|x| x.id);
    data.documents.sort_by_key(|x| x.id);
    read_vehicles(db, &mut data).await?;
    read_pv(db, &mut data).await?;
    read_api(db, &mut data).await?;
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
                purpose: Set(m.purpose.clone()),
                schedule_kind: Set(m.schedule_kind.clone()),
                schedule_value: Set(m.schedule_value),
                archived_at: Set(m.archived_at),
                season_weights: Set(m.season_weights.clone()),
                annual_target_milli: Set(m.annual_target_milli),
                target_tolerance_pct: Set(m.target_tolerance_pct),
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
                parent_meter_id: Set(x.parent_meter_id),
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
                updated_at: Set(r.updated_at),
                api_token_id: Set(r.api_token_id),
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
    .await?;
    insert(
        txn,
        d.invitations
            .iter()
            .map(|x| invitation::ActiveModel {
                id: Set(x.id),
                household_id: Set(x.household_id),
                role: Set(x.role.clone()),
                token_hash: Set(x.token_hash.clone()),
                email: Set(x.email.clone()),
                created_by: Set(x.created_by),
                created_at: Set(x.created_at),
                expires_at: Set(x.expires_at),
                accepted_by: Set(x.accepted_by),
                accepted_at: Set(x.accepted_at),
                revoked_at: Set(x.revoked_at),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.revisions
            .iter()
            .map(|x| reading_revision::ActiveModel {
                id: Set(x.id),
                household_id: Set(x.household_id),
                reading_id: Set(x.reading_id),
                meter_id: Set(x.meter_id),
                register_id: Set(x.register_id),
                action: Set(x.action.clone()),
                changed_by: Set(x.changed_by),
                changed_at: Set(x.changed_at),
                old_read_at: Set(x.old_read_at),
                old_value_milli: Set(x.old_value_milli),
                old_note: Set(x.old_note.clone()),
                new_read_at: Set(x.new_read_at),
                new_value_milli: Set(x.new_value_milli),
                new_note: Set(x.new_note.clone()),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.notification_settings
            .iter()
            .map(|x| notification_settings::ActiveModel {
                user_id: Set(x.user_id),
                email: Set(x.email.clone()),
                reminder: Set(x.reminder),
                alarm: Set(x.alarm),
                updated_at: Set(x.updated_at),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.documents
            .iter()
            .map(|x| document::ActiveModel {
                id: Set(x.id),
                household_id: Set(x.household_id),
                kind: Set(x.kind.clone()),
                blob_key: Set(x.blob_key.clone()),
                media_type: Set(x.media_type.clone()),
                bytes: Set(x.bytes),
                original_name: Set(x.original_name.clone()),
                created_by: Set(x.created_by),
                created_at: Set(x.created_at),
                status: Set(x.status.clone()),
                model: Set(x.model.clone()),
                extraction: Set(x.extraction.clone()),
                meter_id: Set(x.meter_id),
                analysed_at: Set(x.analysed_at),
                finished_at: Set(x.finished_at),
            })
            .collect(),
    )
    .await?;
    write_contracts(txn, d).await?;
    write_vehicles(txn, d).await?;
    write_pv(txn, d).await?;
    write_api(txn, d).await
}

/// Verträge mit Positionen, Preisständen und Einträgen (ab
/// `m20261017_000011_tariffs`).
async fn write_contracts(txn: &DatabaseTransaction, d: &BackupData) -> PortResult<()> {
    insert(
        txn,
        d.contracts
            .iter()
            .map(|x| contract::ActiveModel {
                id: Set(x.id),
                meter_id: Set(x.meter_id),
                direction: Set(x.direction.clone()),
                provider: Set(x.provider.clone()),
                contract_number: Set(x.contract_number.clone()),
                starts_on: Set(x.starts_on),
                ends_on: Set(x.ends_on),
                billing_start_month: Set(x.billing_start_month),
                billing_start_day: Set(x.billing_start_day),
                note: Set(x.note.clone()),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.cost_positions
            .iter()
            .map(|x| cost_position::ActiveModel {
                id: Set(x.id),
                contract_id: Set(x.contract_id),
                name: Set(x.name.clone()),
                register_id: Set(x.register_id),
                factor_micro: Set(x.factor_micro),
                vat_pct_milli: Set(x.vat_pct_milli),
                sort: Set(x.sort),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.price_components
            .iter()
            .map(|x| price_component::ActiveModel {
                id: Set(x.id),
                position_id: Set(x.position_id),
                valid_from: Set(x.valid_from),
                base_price_micro: Set(x.base_price_micro),
                base_interval: Set(x.base_interval.clone()),
                unit_pricing: Set(x.unit_pricing.clone()),
                factor_micro: Set(x.factor_micro),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.installments
            .iter()
            .map(|x| installment::ActiveModel {
                id: Set(x.id),
                contract_id: Set(x.contract_id),
                valid_from: Set(x.valid_from),
                amount_micro: Set(x.amount_micro),
                interval_months: Set(x.interval_months),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.adjustments
            .iter()
            .map(|x| adjustment::ActiveModel {
                id: Set(x.id),
                contract_id: Set(x.contract_id),
                kind: Set(x.kind.clone()),
                booked_on: Set(x.booked_on),
                amount_micro: Set(x.amount_micro),
                note: Set(x.note.clone()),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.settlements
            .iter()
            .map(|x| settlement::ActiveModel {
                id: Set(x.id),
                contract_id: Set(x.contract_id),
                period_start: Set(x.period_start),
                period_end: Set(x.period_end),
                amount_micro: Set(x.amount_micro),
                document_id: Set(x.document_id),
            })
            .collect(),
    )
    .await
}

/// Fahrzeuge und Kostenbuch lesen (ab `m20261018_000013_vehicles`).
async fn read_vehicles(db: &DatabaseTransaction, data: &mut BackupData) -> PortResult<()> {
    data.vehicles = records(db, "vehicles", |r| {
        Ok(VehicleRecord {
            meter_id: r.get("meter_id")?,
            plate: r.get("plate")?,
            model: r.get("model")?,
            drive_type: r.get("drive_type")?,
            home_charging_meter_id: r.get("home_charging_meter_id")?,
            pv_share_override_pct_milli: r.get("pv_share_override_pct_milli")?,
        })
    })
    .await?;
    data.consumption_profiles = records(db, "vehicle_consumption_profiles", |r| {
        Ok(ConsumptionProfileRecord {
            id: r.get("id")?,
            vehicle_id: r.get("vehicle_id")?,
            valid_from: r.get("valid_from")?,
            kwh_per_100km_milli: r.get("kwh_per_100km_milli")?,
            charging_loss_pct_milli: r.get("charging_loss_pct_milli")?,
            electric_share_pct_milli: r.get("electric_share_pct_milli")?,
        })
    })
    .await?;
    data.acquisitions = records(db, "vehicle_acquisitions", |r| {
        Ok(AcquisitionRecord {
            vehicle_id: r.get("vehicle_id")?,
            purchased_on: r.get("purchased_on")?,
            price_micro: r.get("price_micro")?,
            residual_micro: r.get("residual_micro")?,
            planned_end_on: r.get("planned_end_on")?,
            sold_on: r.get("sold_on")?,
            sale_price_micro: r.get("sale_price_micro")?,
        })
    })
    .await?;
    data.charging_sessions = records(db, "charging_sessions", |r| {
        Ok(ChargingSessionRecord {
            id: r.get("id")?,
            vehicle_id: r.get("vehicle_id")?,
            charged_at: r.get("charged_at")?,
            energy_milli: r.get("energy_milli")?,
            amount_micro: r.get("amount_micro")?,
            odometer_milli: r.get("odometer_milli")?,
            provider: r.get("provider")?,
            kind: r.get("kind")?,
            source: r.get("source")?,
            document_id: r.get("document_id")?,
        })
    })
    .await?;
    data.asset_costs = records(db, "asset_costs", |r| {
        Ok(AssetCostRecord {
            id: r.get("id")?,
            household_id: r.get("household_id")?,
            asset_type: r.get("asset_type")?,
            asset_id: r.get("asset_id")?,
            booked_on: r.get("booked_on")?,
            category: r.get("category")?,
            amount_micro: r.get("amount_micro")?,
            quantity_milli: r.get("quantity_milli")?,
            odometer_milli: r.get("odometer_milli")?,
            full_tank: r.get("full_tank")?,
            recurrence_months: r.get("recurrence_months")?,
            recurrence_until: r.get("recurrence_until")?,
            document_id: r.get("document_id")?,
            note: r.get("note")?,
        })
    })
    .await?;
    data.vehicles.sort_by_key(|x| x.meter_id);
    data.consumption_profiles.sort_by_key(|x| x.id);
    data.acquisitions.sort_by_key(|x| x.vehicle_id);
    data.charging_sessions.sort_by_key(|x| x.id);
    data.asset_costs.sort_by_key(|x| x.id);
    Ok(())
}

/// Fahrzeuge und Kostenbuch schreiben (ab `m20261018_000013_vehicles`).
async fn write_vehicles(txn: &DatabaseTransaction, d: &BackupData) -> PortResult<()> {
    insert(
        txn,
        d.vehicles
            .iter()
            .map(|x| vehicle::ActiveModel {
                meter_id: Set(x.meter_id),
                plate: Set(x.plate.clone()),
                model: Set(x.model.clone()),
                drive_type: Set(x.drive_type.clone()),
                home_charging_meter_id: Set(x.home_charging_meter_id),
                pv_share_override_pct_milli: Set(x.pv_share_override_pct_milli),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.consumption_profiles
            .iter()
            .map(|x| vehicle_consumption_profile::ActiveModel {
                id: Set(x.id),
                vehicle_id: Set(x.vehicle_id),
                valid_from: Set(x.valid_from),
                kwh_per_100km_milli: Set(x.kwh_per_100km_milli),
                charging_loss_pct_milli: Set(x.charging_loss_pct_milli),
                electric_share_pct_milli: Set(x.electric_share_pct_milli),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.acquisitions
            .iter()
            .map(|x| vehicle_acquisition::ActiveModel {
                vehicle_id: Set(x.vehicle_id),
                purchased_on: Set(x.purchased_on),
                price_micro: Set(x.price_micro),
                residual_micro: Set(x.residual_micro),
                planned_end_on: Set(x.planned_end_on),
                sold_on: Set(x.sold_on),
                sale_price_micro: Set(x.sale_price_micro),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.charging_sessions
            .iter()
            .map(|x| charging_session::ActiveModel {
                id: Set(x.id),
                vehicle_id: Set(x.vehicle_id),
                charged_at: Set(x.charged_at),
                energy_milli: Set(x.energy_milli),
                amount_micro: Set(x.amount_micro),
                odometer_milli: Set(x.odometer_milli),
                provider: Set(x.provider.clone()),
                kind: Set(x.kind.clone()),
                source: Set(x.source.clone()),
                document_id: Set(x.document_id),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.asset_costs
            .iter()
            .map(|x| asset_cost::ActiveModel {
                id: Set(x.id),
                household_id: Set(x.household_id),
                asset_type: Set(x.asset_type.clone()),
                asset_id: Set(x.asset_id),
                booked_on: Set(x.booked_on),
                category: Set(x.category.clone()),
                amount_micro: Set(x.amount_micro),
                quantity_milli: Set(x.quantity_milli),
                odometer_milli: Set(x.odometer_milli),
                full_tank: Set(x.full_tank),
                recurrence_months: Set(x.recurrence_months),
                recurrence_until: Set(x.recurrence_until),
                document_id: Set(x.document_id),
                note: Set(x.note.clone()),
            })
            .collect(),
    )
    .await
}

/// PV-Anlagen lesen (ab `m20261018_000014_pv`).
async fn read_pv(db: &DatabaseTransaction, data: &mut BackupData) -> PortResult<()> {
    data.pv_systems = records(db, "pv_systems", |r| {
        Ok(PvSystemRecord {
            id: r.get("id")?,
            household_id: r.get("household_id")?,
            name: r.get("name")?,
            kind: r.get("kind")?,
            kwp_milli: r.get("kwp_milli")?,
            commissioned_on: r.get("commissioned_on")?,
            azimuth: r.get("azimuth")?,
            tilt: r.get("tilt")?,
            inverter: r.get("inverter")?,
            battery_kwh_milli: r.get("battery_kwh_milli")?,
            specific_yield_milli: r.get("specific_yield_milli")?,
            degradation_pct_milli: r.get("degradation_pct_milli")?,
            month_shares: r.get("month_shares")?,
            alarm_threshold_pct: r.get("alarm_threshold_pct")?,
        })
    })
    .await?;
    data.pv_system_meters = records(db, "pv_system_meters", |r| {
        Ok(PvSystemMeterRecord {
            pv_system_id: r.get("pv_system_id")?,
            meter_id: r.get("meter_id")?,
            role: r.get("role")?,
        })
    })
    .await?;
    data.pv_systems.sort_by_key(|x| x.id);
    data.pv_system_meters
        .sort_by_key(|x| (x.pv_system_id, x.meter_id));
    Ok(())
}

/// PV-Anlagen schreiben (ab `m20261018_000014_pv`).
async fn write_pv(txn: &DatabaseTransaction, d: &BackupData) -> PortResult<()> {
    insert(
        txn,
        d.pv_systems
            .iter()
            .map(|x| pv_system::ActiveModel {
                id: Set(x.id),
                household_id: Set(x.household_id),
                name: Set(x.name.clone()),
                kind: Set(x.kind.clone()),
                kwp_milli: Set(x.kwp_milli),
                commissioned_on: Set(x.commissioned_on),
                azimuth: Set(x.azimuth),
                tilt: Set(x.tilt),
                inverter: Set(x.inverter.clone()),
                battery_kwh_milli: Set(x.battery_kwh_milli),
                specific_yield_milli: Set(x.specific_yield_milli),
                degradation_pct_milli: Set(x.degradation_pct_milli),
                month_shares: Set(x.month_shares.clone()),
                alarm_threshold_pct: Set(x.alarm_threshold_pct),
            })
            .collect(),
    )
    .await?;
    insert(
        txn,
        d.pv_system_meters
            .iter()
            .map(|x| pv_system_meter::ActiveModel {
                pv_system_id: Set(x.pv_system_id),
                meter_id: Set(x.meter_id),
                role: Set(x.role.clone()),
            })
            .collect(),
    )
    .await
}

/// API-Tokens lesen (ab `m20261019_000015_api`).
async fn read_api(db: &DatabaseTransaction, data: &mut BackupData) -> PortResult<()> {
    data.api_tokens = records(db, "api_tokens", |r| {
        Ok(ApiTokenRecord {
            id: r.get("id")?,
            household_id: r.get("household_id")?,
            user_id: r.get("user_id")?,
            name: r.get("name")?,
            role: r.get("role")?,
            token_hash: r.get("token_hash")?,
            prefix: r.get("prefix")?,
            created_at: r.get("created_at")?,
            expires_at: r.get("expires_at")?,
            revoked_at: r.get("revoked_at")?,
            last_used_at: r.get("last_used_at")?,
        })
    })
    .await?;
    data.api_tokens.sort_by_key(|x| x.id);
    Ok(())
}

/// API-Tokens schreiben (ab `m20261019_000015_api`).
async fn write_api(txn: &DatabaseTransaction, d: &BackupData) -> PortResult<()> {
    insert(
        txn,
        d.api_tokens
            .iter()
            .map(|x| api_token::ActiveModel {
                id: Set(x.id),
                household_id: Set(x.household_id),
                user_id: Set(x.user_id),
                name: Set(x.name.clone()),
                role: Set(x.role.clone()),
                token_hash: Set(x.token_hash.clone()),
                prefix: Set(x.prefix.clone()),
                created_at: Set(x.created_at),
                expires_at: Set(x.expires_at),
                revoked_at: Set(x.revoked_at),
                last_used_at: Set(x.last_used_at),
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
