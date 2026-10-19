//! Archivformat für vollständigen Export und Import (SPEC_BACKUP_FORMAT).
//!
//! Ein Archiv ist eine ZIP-Datei mit `data.json` (alle Datensätze, Werte wie in
//! der Datenbank gespeichert), den Fotos unter `blobs/<Objektschlüssel>` und
//! `manifest.json` mit Formatversion, Schemastand und SHA-256 aller Teile. Das
//! Format ist unabhängig vom Datenbankschema versioniert: Neue Felder kommen
//! mit `#[serde(default)]` hinzu, damit ältere Archive lesbar bleiben.

use std::{
    collections::{BTreeSet, HashSet},
    io::{Read, Seek, Write},
};

use bytes::Bytes;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{
    ids::{BlobKey, BlobKind},
    model::DraftStatus,
};

/// Kennung des Formats im Manifest.
pub const FORMAT: &str = "zaehler-backup";
/// Aktuelle Formatversion; Importe akzeptieren diese und ältere Versionen.
/// Version 2 (v1.1): Laufleistungsmodus, Literpreis, Energieart der Profile.
pub const FORMAT_VERSION: u32 = 2;
const MANIFEST: &str = "manifest.json";
const DATA: &str = "data.json";
const BLOB_PREFIX: &str = "blobs/";
/// Größte zulässige Datei im Archiv (entpackt), Schutz vor ZIP-Bomben.
pub const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
/// Größte zulässige Anzahl Dateien im Archiv.
pub const MAX_ENTRIES: usize = 2_000_000;

/// Umfang eines Archivs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Alle Nutzer, Haushalte und Daten der Installation
    Installation,
    /// Ein Haushalt ohne Nutzerkonten
    Household,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: Uuid,
    pub oidc_issuer: String,
    pub oidc_subject: String,
    pub email: Option<String>,
    pub display_name: String,
    pub locale: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HouseholdRecord {
    pub id: Uuid,
    pub name: String,
    pub timezone: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipRecord {
    pub household_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeterRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    pub name: String,
    pub kind: String,
    pub unit: String,
    pub reading_interval_days: Option<i32>,
    pub max_daily_rate_milli: Option<i64>,
    pub billing_start_month: Option<i16>,
    pub billing_start_day: Option<i16>,
    pub archived: bool,
    pub created_at: DateTime<Utc>,
    /// Ab `m20261016_000008_meter_v1`
    #[serde(default = "general_purpose")]
    pub purpose: String,
    #[serde(default)]
    pub schedule_kind: Option<String>,
    #[serde(default)]
    pub schedule_value: Option<i16>,
    #[serde(default)]
    pub archived_at: Option<DateTime<Utc>>,
    /// Ab `m20261016_000010_expectation`: angepasstes Profil als JSON
    #[serde(default)]
    pub season_weights: Option<String>,
    #[serde(default)]
    pub annual_target_milli: Option<i64>,
    #[serde(default)]
    pub target_tolerance_pct: Option<i16>,
}

fn general_purpose() -> String {
    "general".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub id: Uuid,
    pub meter_id: Uuid,
    pub serial_number: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub removed_at: Option<DateTime<Utc>>,
    pub rollover_digits: Option<i16>,
    pub decimal_places: i16,
    /// Übergeordnete Messstelle (ab `m20261016_000008_meter_v1`)
    #[serde(default)]
    pub parent_meter_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterRecord {
    pub id: Uuid,
    pub meter_id: Uuid,
    pub code: String,
    pub direction: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingRecord {
    pub id: Uuid,
    pub register_id: Uuid,
    pub device_id: Uuid,
    pub read_at: DateTime<Utc>,
    /// Stand in Tausendsteln (Festkomma wie in der Datenbank)
    pub value_milli: i64,
    pub source: String,
    pub photo_key: Option<String>,
    pub ai_confidence: Option<f64>,
    pub note: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    /// Letzte Korrektur (ab `m20261016_000007_reading_revisions`)
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    /// Token einer automatisch gelieferten Ablesung (ab `m20261019_000015_api`);
    /// im Haushaltsarchiv leer
    #[serde(default)]
    pub api_token_id: Option<Uuid>,
}

/// Revision einer Ablesung (ab `m20261016_000007_reading_revisions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    /// Ablesung; nach dem Löschen nicht mehr vorhanden
    pub reading_id: Uuid,
    pub meter_id: Uuid,
    pub register_id: Uuid,
    /// `update` oder `delete`
    pub action: String,
    pub changed_by: Uuid,
    pub changed_at: DateTime<Utc>,
    pub old_read_at: DateTime<Utc>,
    pub old_value_milli: i64,
    pub old_note: Option<String>,
    pub new_read_at: Option<DateTime<Utc>>,
    pub new_value_milli: Option<i64>,
    pub new_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    pub created_by: Uuid,
    pub photo_key: String,
    pub model: Option<String>,
    pub duration_ms: Option<i64>,
    /// Vorschlag als JSON wie gespeichert
    pub suggestion: Option<String>,
    pub matched_meter_id: Option<Uuid>,
    pub status: String,
    pub confirmed_reading_id: Option<Uuid>,
    pub corrected: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub taken_at: Option<DateTime<Utc>>,
    /// Ab `m20261016_000005`; fehlt in älteren Archiven
    #[serde(default)]
    pub analysis_started_at: Option<DateTime<Utc>>,
}

/// Einladung (ab `m20261016_000006_members`); nur im Installationsarchiv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvitationRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    pub role: String,
    /// SHA-256 des Tokens, hexadezimal
    pub token_hash: String,
    pub email: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub accepted_by: Option<Uuid>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// API-Token (ab `m20261019_000015_api`); nur im Installationsarchiv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiTokenRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub role: String,
    /// SHA-256 des Tokens, hexadezimal
    pub token_hash: String,
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Benachrichtigungseinstellungen eines Nutzers (ab `m20261016_000009_jobs`);
/// nur im Installationsarchiv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSettingRecord {
    pub user_id: Uuid,
    pub email: Option<String>,
    pub reminder: bool,
    pub alarm: bool,
    pub updated_at: DateTime<Utc>,
}

/// Vertrag (ab `m20261017_000011_tariffs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRecord {
    pub id: Uuid,
    pub meter_id: Uuid,
    /// `import` oder `export`
    pub direction: String,
    pub provider: String,
    pub contract_number: Option<String>,
    pub starts_on: NaiveDate,
    pub ends_on: Option<NaiveDate>,
    pub billing_start_month: i16,
    pub billing_start_day: i16,
    pub note: Option<String>,
}

/// Kostenposition (ab `m20261017_000011_tariffs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostPositionRecord {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub name: String,
    pub register_id: Option<Uuid>,
    pub factor_micro: i64,
    pub vat_pct_milli: Option<i64>,
    pub sort: i32,
}

/// Preisstand (ab `m20261017_000011_tariffs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceComponentRecord {
    pub id: Uuid,
    pub position_id: Uuid,
    pub valid_from: NaiveDate,
    pub base_price_micro: i64,
    /// `day`, `month` oder `year`
    pub base_interval: String,
    /// Arbeitspreis als JSON wie gespeichert
    pub unit_pricing: String,
    pub factor_micro: Option<i64>,
}

/// Abschlagsstand (ab `m20261017_000011_tariffs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallmentRecord {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub valid_from: NaiveDate,
    pub amount_micro: i64,
    pub interval_months: i16,
}

/// Bonus oder Gutschrift (ab `m20261017_000011_tariffs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjustmentRecord {
    pub id: Uuid,
    pub contract_id: Uuid,
    /// `bonus` oder `credit`
    pub kind: String,
    pub booked_on: NaiveDate,
    pub amount_micro: i64,
    pub note: Option<String>,
}

/// Erfasste Jahresabrechnung (ab `m20261017_000011_tariffs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementRecord {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub amount_micro: i64,
    pub document_id: Option<Uuid>,
}

/// Hochgeladenes Dokument mit Erkennungsergebnis (ab
/// `m20261017_000012_documents`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    /// `invoice`
    pub kind: String,
    pub blob_key: String,
    pub media_type: String,
    pub bytes: i64,
    pub original_name: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub status: String,
    pub model: Option<String>,
    /// Erkennungsergebnis als JSON wie gespeichert
    pub extraction: Option<String>,
    pub meter_id: Option<Uuid>,
    pub analysed_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Fahrzeug (ab `m20261018_000013_vehicles`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VehicleRecord {
    pub meter_id: Uuid,
    pub plate: String,
    pub model: String,
    /// `combustion`, `electric` oder `plugin_hybrid`
    pub drive_type: String,
    pub home_charging_meter_id: Option<Uuid>,
    pub pv_share_override_pct_milli: Option<i64>,
    /// €/kWh beim Heimladen (ab `m20261019_000020_home_charging_price`)
    #[serde(default)]
    pub home_charging_price_micro: Option<i64>,
}

/// Verbrauchsprofil (ab `m20261018_000013_vehicles`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumptionProfileRecord {
    pub id: Uuid,
    pub vehicle_id: Uuid,
    pub valid_from: NaiveDate,
    pub kwh_per_100km_milli: i64,
    pub charging_loss_pct_milli: i64,
    pub electric_share_pct_milli: Option<i64>,
    /// `electric` oder `fuel` (ab Format 2, `m20261019_000017_fuel_price`)
    #[serde(default = "default_energy")]
    pub energy: String,
}

fn default_energy() -> String {
    "electric".into()
}

fn default_mode() -> String {
    "time".into()
}

/// Anschaffung (ab `m20261018_000013_vehicles`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquisitionRecord {
    pub vehicle_id: Uuid,
    pub purchased_on: NaiveDate,
    pub price_micro: i64,
    pub residual_micro: Option<i64>,
    pub planned_end_on: Option<NaiveDate>,
    pub sold_on: Option<NaiveDate>,
    pub sale_price_micro: Option<i64>,
    /// `time` oder `mileage` (ab Format 2, `m20261019_000016_vehicle_mileage`)
    #[serde(default = "default_mode")]
    pub depreciation_mode: String,
    #[serde(default)]
    pub odometer_at_purchase_milli: Option<i64>,
    #[serde(default)]
    pub lifetime_km_milli: Option<i64>,
    #[serde(default)]
    pub end_value_micro: Option<i64>,
    /// Erstzulassung (ab `m20261019_000019_vehicle_registration`)
    #[serde(default)]
    pub first_registration_on: Option<NaiveDate>,
}

impl AcquisitionRecord {
    /// Gespeicherte Spalten einer Anschaffung (SPEC_CAR_MILEAGE_UI).
    ///
    /// # Panics
    /// Wenn die Anschaffung nicht auf gültige Festkommawerte geprüft wurde.
    #[must_use]
    pub fn from_holding(vehicle_id: Uuid, h: &zaehler_domain::vehicle::Holding) -> Self {
        Self {
            vehicle_id,
            purchased_on: h.purchased_on,
            price_micro: scaled(h.price, 6),
            residual_micro: h.residual_value.map(|v| scaled(v, 6)),
            planned_end_on: h.planned_end,
            sold_on: h.sold_on,
            sale_price_micro: h.sale_price.map(|p| scaled(p, 6)),
            depreciation_mode: h.mode.as_str().into(),
            odometer_at_purchase_milli: h.odometer_at_purchase.map(|v| scaled(v, 3)),
            lifetime_km_milli: h.lifetime_km.map(|v| scaled(v, 3)),
            end_value_micro: h.end_value.map(|v| scaled(v, 6)),
            first_registration_on: h.first_registration,
        }
    }

    /// Anschaffung aus den gespeicherten Spalten.
    pub fn to_holding(&self) -> Result<zaehler_domain::vehicle::Holding, String> {
        use rust_decimal::Decimal;
        Ok(zaehler_domain::vehicle::Holding {
            purchased_on: self.purchased_on,
            price: Decimal::new(self.price_micro, 6),
            mode: self.depreciation_mode.parse()?,
            first_registration: self.first_registration_on,
            residual_value: self.residual_micro.map(|v| Decimal::new(v, 6)),
            planned_end: self.planned_end_on,
            odometer_at_purchase: self.odometer_at_purchase_milli.map(|v| Decimal::new(v, 3)),
            lifetime_km: self.lifetime_km_milli.map(|v| Decimal::new(v, 3)),
            end_value: self.end_value_micro.map(|v| Decimal::new(v, 6)),
            sold_on: self.sold_on,
            sale_price: self.sale_price_micro.map(|p| Decimal::new(p, 6)),
        })
    }
}

/// Externer Ladevorgang (ab `m20261018_000013_vehicles`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChargingSessionRecord {
    pub id: Uuid,
    pub vehicle_id: Uuid,
    pub charged_at: DateTime<Utc>,
    pub energy_milli: i64,
    pub amount_micro: i64,
    pub odometer_milli: Option<i64>,
    pub provider: Option<String>,
    /// `ac` oder `dc`
    pub kind: String,
    /// `form` oder `receipt`
    pub source: String,
    pub document_id: Option<Uuid>,
}

/// Kostenbuch-Eintrag (ab `m20261018_000013_vehicles`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetCostRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    /// `vehicle` oder `pv_system`
    pub asset_type: String,
    pub asset_id: Uuid,
    pub booked_on: NaiveDate,
    pub category: String,
    pub amount_micro: i64,
    pub quantity_milli: Option<i64>,
    /// Preis pro Liter (ab Format 2, `m20261019_000017_fuel_price`)
    #[serde(default)]
    pub unit_price_micro: Option<i64>,
    pub odometer_milli: Option<i64>,
    pub full_tank: Option<bool>,
    pub recurrence_months: Option<i16>,
    pub recurrence_until: Option<NaiveDate>,
    pub document_id: Option<Uuid>,
    pub note: Option<String>,
}

/// PV-Anlage (ab `m20261018_000014_pv`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PvSystemRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    pub name: String,
    /// `surplus`, `full_feed_in` oder `plug_in`
    pub kind: String,
    pub kwp_milli: i64,
    pub commissioned_on: NaiveDate,
    pub azimuth: Option<i16>,
    pub tilt: Option<i16>,
    pub inverter: Option<String>,
    pub battery_kwh_milli: Option<i64>,
    pub specific_yield_milli: i64,
    pub degradation_pct_milli: i64,
    /// Monatsanteile als JSON wie gespeichert
    pub month_shares: Option<String>,
    pub alarm_threshold_pct: i16,
}

/// Zuordnung einer Messstelle zu einer PV-Anlage (ab `m20261018_000014_pv`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PvSystemMeterRecord {
    pub pv_system_id: Uuid,
    pub meter_id: Uuid,
    /// `grid`, `generation` oder `battery`
    pub role: String,
}

/// Alle Datensätze eines Archivs. Sitzungen und temporäre Dateien sind nicht
/// enthalten.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupData {
    #[serde(default)]
    pub users: Vec<UserRecord>,
    #[serde(default)]
    pub households: Vec<HouseholdRecord>,
    #[serde(default)]
    pub memberships: Vec<MembershipRecord>,
    #[serde(default)]
    pub meters: Vec<MeterRecord>,
    #[serde(default)]
    pub devices: Vec<DeviceRecord>,
    #[serde(default)]
    pub registers: Vec<RegisterRecord>,
    #[serde(default)]
    pub readings: Vec<ReadingRecord>,
    #[serde(default)]
    pub drafts: Vec<DraftRecord>,
    #[serde(default)]
    pub invitations: Vec<InvitationRecord>,
    #[serde(default)]
    pub revisions: Vec<RevisionRecord>,
    #[serde(default)]
    pub notification_settings: Vec<NotificationSettingRecord>,
    #[serde(default)]
    pub contracts: Vec<ContractRecord>,
    #[serde(default)]
    pub cost_positions: Vec<CostPositionRecord>,
    #[serde(default)]
    pub price_components: Vec<PriceComponentRecord>,
    #[serde(default)]
    pub installments: Vec<InstallmentRecord>,
    #[serde(default)]
    pub adjustments: Vec<AdjustmentRecord>,
    #[serde(default)]
    pub settlements: Vec<SettlementRecord>,
    #[serde(default)]
    pub documents: Vec<DocumentRecord>,
    #[serde(default)]
    pub vehicles: Vec<VehicleRecord>,
    #[serde(default)]
    pub consumption_profiles: Vec<ConsumptionProfileRecord>,
    #[serde(default)]
    pub acquisitions: Vec<AcquisitionRecord>,
    #[serde(default)]
    pub charging_sessions: Vec<ChargingSessionRecord>,
    #[serde(default)]
    pub asset_costs: Vec<AssetCostRecord>,
    #[serde(default)]
    pub pv_systems: Vec<PvSystemRecord>,
    #[serde(default)]
    pub pv_system_meters: Vec<PvSystemMeterRecord>,
    #[serde(default)]
    pub api_tokens: Vec<ApiTokenRecord>,
}

impl BackupData {
    /// Objektschlüssel aller vorhandenen Dateien, sortiert: Belegfotos,
    /// Fotos offener Entwürfe und Dokumente. Abgeschlossene Entwürfe
    /// verweisen auf ein planmäßig gelöschtes Foto (die Ablesung hat eine
    /// Kopie).
    #[must_use]
    pub fn blob_keys(&self) -> Vec<String> {
        let keys: BTreeSet<String> = self
            .readings
            .iter()
            .filter_map(|r| r.photo_key.clone())
            .chain(
                self.drafts
                    .iter()
                    .filter(|d| {
                        d.status
                            .parse::<DraftStatus>()
                            .is_ok_and(DraftStatus::is_open)
                    })
                    .map(|d| d.photo_key.clone()),
            )
            .chain(self.documents.iter().map(|d| d.blob_key.clone()))
            .collect();
        keys.into_iter().collect()
    }

    /// Nur die Daten eines Haushalts; Nutzerkonten, Einladungen, API-Tokens
    /// und Benachrichtigungseinstellungen entfallen.
    #[must_use]
    #[allow(clippy::too_many_lines)] // ein Filter je Tabelle
    pub fn household(self, id: Uuid) -> Self {
        let meters: HashSet<Uuid> = self
            .meters
            .iter()
            .filter(|m| m.household_id == id)
            .map(|m| m.id)
            .collect();
        let registers: HashSet<Uuid> = self
            .registers
            .iter()
            .filter(|r| meters.contains(&r.meter_id))
            .map(|r| r.id)
            .collect();
        let contracts: HashSet<Uuid> = self
            .contracts
            .iter()
            .filter(|c| meters.contains(&c.meter_id))
            .map(|c| c.id)
            .collect();
        let positions: HashSet<Uuid> = self
            .cost_positions
            .iter()
            .filter(|p| contracts.contains(&p.contract_id))
            .map(|p| p.id)
            .collect();
        Self {
            contracts: self
                .contracts
                .into_iter()
                .filter(|c| contracts.contains(&c.id))
                .collect(),
            cost_positions: self
                .cost_positions
                .into_iter()
                .filter(|p| positions.contains(&p.id))
                .collect(),
            price_components: self
                .price_components
                .into_iter()
                .filter(|c| positions.contains(&c.position_id))
                .collect(),
            installments: self
                .installments
                .into_iter()
                .filter(|x| contracts.contains(&x.contract_id))
                .collect(),
            adjustments: self
                .adjustments
                .into_iter()
                .filter(|x| contracts.contains(&x.contract_id))
                .collect(),
            settlements: self
                .settlements
                .into_iter()
                .filter(|x| contracts.contains(&x.contract_id))
                .collect(),
            users: Vec::new(),
            households: self.households.into_iter().filter(|h| h.id == id).collect(),
            memberships: self
                .memberships
                .into_iter()
                .filter(|m| m.household_id == id)
                .collect(),
            meters: self
                .meters
                .into_iter()
                .filter(|m| m.household_id == id)
                .collect(),
            devices: self
                .devices
                .into_iter()
                .filter(|d| meters.contains(&d.meter_id))
                .collect(),
            registers: self
                .registers
                .into_iter()
                .filter(|r| meters.contains(&r.meter_id))
                .collect(),
            readings: self
                .readings
                .into_iter()
                .filter(|r| registers.contains(&r.register_id))
                .map(|r| ReadingRecord {
                    api_token_id: None,
                    ..r
                })
                .collect(),
            drafts: self
                .drafts
                .into_iter()
                .filter(|d| d.household_id == id)
                .collect(),
            documents: self
                .documents
                .into_iter()
                .filter(|d| d.household_id == id)
                .collect(),
            invitations: Vec::new(),
            api_tokens: Vec::new(),
            revisions: self
                .revisions
                .into_iter()
                .filter(|r| r.household_id == id)
                .collect(),
            notification_settings: Vec::new(),
            vehicles: self
                .vehicles
                .into_iter()
                .filter(|v| meters.contains(&v.meter_id))
                .collect(),
            consumption_profiles: self
                .consumption_profiles
                .into_iter()
                .filter(|p| meters.contains(&p.vehicle_id))
                .collect(),
            acquisitions: self
                .acquisitions
                .into_iter()
                .filter(|a| meters.contains(&a.vehicle_id))
                .collect(),
            charging_sessions: self
                .charging_sessions
                .into_iter()
                .filter(|c| meters.contains(&c.vehicle_id))
                .collect(),
            pv_system_meters: {
                let systems: HashSet<Uuid> = self
                    .pv_systems
                    .iter()
                    .filter(|s| s.household_id == id)
                    .map(|s| s.id)
                    .collect();
                self.pv_system_meters
                    .into_iter()
                    .filter(|m| systems.contains(&m.pv_system_id))
                    .collect()
            },
            pv_systems: self
                .pv_systems
                .into_iter()
                .filter(|s| s.household_id == id)
                .collect(),
            asset_costs: self
                .asset_costs
                .into_iter()
                .filter(|c| c.household_id == id)
                .collect(),
        }
    }

    /// Anzahl der Datensätze je Art.
    #[must_use]
    pub fn counts(&self) -> Counts {
        Counts {
            users: self.users.len(),
            households: self.households.len(),
            meters: self.meters.len(),
            readings: self.readings.len(),
            drafts: self.drafts.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub users: usize,
    pub households: usize,
    pub meters: usize,
    pub readings: usize,
    pub drafts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobEntry {
    pub key: String,
    pub size: u64,
    pub sha256: String,
}

/// Beschreibung des Archivs; wird zuletzt geschrieben und zuerst gelesen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub scope: Scope,
    /// Programmversion der Quelle
    pub app_version: String,
    /// Angewendete Migrationen der Quelle
    pub schema: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub counts: Counts,
    pub data_sha256: String,
    pub blobs: Vec<BlobEntry>,
    /// Referenzierte Fotos, die in der Quelle fehlten
    #[serde(default)]
    pub missing_blobs: Vec<String>,
}

/// Fehler beim Schreiben oder Lesen eines Archivs; `code` ist ein
/// Übersetzungsschlüssel.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("ungültiges Archiv: {0}")]
    Invalid(String),
    #[error("Archiv einer neueren Programmversion (Format {0})")]
    NewerFormat(u32),
    #[error("Prüfsumme stimmt nicht: {0}")]
    Checksum(String),
    #[error("Ein-/Ausgabefehler: {0}")]
    Io(String),
}

impl ArchiveError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "backup.invalid",
            Self::NewerFormat(_) => "backup.newer_format",
            Self::Checksum(_) => "backup.checksum",
            Self::Io(_) => "backup.io",
        }
    }
}

fn io<E: std::fmt::Display>(e: E) -> ArchiveError {
    ArchiveError::Io(e.to_string())
}

fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Zulässiger Objektschlüssel im Archiv: Foto einer Ablesung oder eines
/// Entwurfs, Dokument.
fn valid_blob_key(key: &str) -> bool {
    BlobKey::parse(key).is_some_and(|k| BlobKind::archived(k.kind()))
}

/// Schreibt ein Archiv: zuerst `data.json`, dann die Fotos, zuletzt das Manifest.
pub struct ArchiveWriter<W: Write + Seek> {
    zip: ZipWriter<W>,
    blobs: Vec<BlobEntry>,
}

impl<W: Write + Seek> ArchiveWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            zip: ZipWriter::new(out),
            blobs: Vec::new(),
        }
    }

    fn put(
        &mut self,
        name: &str,
        bytes: &[u8],
        method: CompressionMethod,
    ) -> Result<(), ArchiveError> {
        let options = SimpleFileOptions::default()
            .compression_method(method)
            .large_file(bytes.len() as u64 >= u64::from(u32::MAX));
        self.zip.start_file(name, options).map_err(io)?;
        self.zip.write_all(bytes).map_err(io)
    }

    /// Schreibt die Datensätze und liefert ihre Prüfsumme.
    ///
    /// # Errors
    /// Bei Schreibfehlern.
    pub fn data(&mut self, data: &BackupData) -> Result<String, ArchiveError> {
        let json = serde_json::to_vec(data).map_err(io)?;
        self.put(DATA, &json, CompressionMethod::Deflated)?;
        Ok(sha256(&json))
    }

    /// Schreibt ein Foto (JPEG, ohne erneute Kompression).
    ///
    /// # Errors
    /// Bei ungültigem Schlüssel oder Schreibfehlern.
    pub fn blob(&mut self, key: &str, bytes: &[u8]) -> Result<(), ArchiveError> {
        if !valid_blob_key(key) {
            return Err(ArchiveError::Invalid(format!("Objektschlüssel {key}")));
        }
        self.put(
            &format!("{BLOB_PREFIX}{key}"),
            bytes,
            CompressionMethod::Stored,
        )?;
        self.blobs.push(BlobEntry {
            key: key.to_owned(),
            size: bytes.len() as u64,
            sha256: sha256(bytes),
        });
        Ok(())
    }

    /// Schreibt das Manifest; `blobs` wird aus den geschriebenen Fotos gefüllt.
    ///
    /// # Errors
    /// Bei Schreibfehlern.
    pub fn finish(mut self, mut manifest: Manifest) -> Result<W, ArchiveError> {
        manifest.blobs = std::mem::take(&mut self.blobs);
        let json = serde_json::to_vec_pretty(&manifest).map_err(io)?;
        self.put(MANIFEST, &json, CompressionMethod::Deflated)?;
        self.zip.finish().map_err(io)
    }
}

/// Liest ein Archiv; prüft Aufbau und Prüfsummen.
pub struct ArchiveReader<R: Read + Seek> {
    zip: ZipArchive<R>,
    pub manifest: Manifest,
}

impl<R: Read + Seek> ArchiveReader<R> {
    fn read_entry(zip: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>, ArchiveError> {
        let file = zip
            .by_name(name)
            .map_err(|_| ArchiveError::Invalid(format!("{name} fehlt")))?;
        if file.size() > MAX_ENTRY_BYTES {
            return Err(ArchiveError::Invalid(format!("{name} zu groß")));
        }
        let mut out = Vec::new();
        file.take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut out)
            .map_err(|e| ArchiveError::Invalid(format!("{name}: {e}")))?;
        if out.len() as u64 > MAX_ENTRY_BYTES {
            return Err(ArchiveError::Invalid(format!("{name} zu groß")));
        }
        Ok(out)
    }

    /// Öffnet das Archiv und prüft Manifest, Formatversion und Dateinamen.
    ///
    /// # Errors
    /// Bei ungültigem Aufbau oder neuerer Formatversion.
    pub fn open(source: R) -> Result<Self, ArchiveError> {
        let mut zip = ZipArchive::new(source).map_err(|e| ArchiveError::Invalid(e.to_string()))?;
        if zip.len() > MAX_ENTRIES {
            return Err(ArchiveError::Invalid("zu viele Dateien".into()));
        }
        let raw = Self::read_entry(&mut zip, MANIFEST)?;
        let manifest: Manifest = serde_json::from_slice(&raw)
            .map_err(|e| ArchiveError::Invalid(format!("manifest.json: {e}")))?;
        if manifest.format != FORMAT {
            return Err(ArchiveError::Invalid(format!("Format {}", manifest.format)));
        }
        if manifest.format_version > FORMAT_VERSION {
            return Err(ArchiveError::NewerFormat(manifest.format_version));
        }
        let mut expected: HashSet<String> = manifest
            .blobs
            .iter()
            .map(|b| format!("{BLOB_PREFIX}{}", b.key))
            .collect();
        if expected.len() != manifest.blobs.len()
            || manifest.blobs.iter().any(|b| !valid_blob_key(&b.key))
        {
            return Err(ArchiveError::Invalid("Fotoliste".into()));
        }
        expected.insert(MANIFEST.into());
        expected.insert(DATA.into());
        let names: Vec<String> = zip
            .file_names()
            .map(|n| n.map(std::borrow::Cow::into_owned))
            .collect::<Result<_, _>>()
            .map_err(|e| ArchiveError::Invalid(e.to_string()))?;
        let unique: HashSet<&String> = names.iter().collect();
        if unique.len() != names.len()
            || names.len() != expected.len()
            || names.iter().any(|n| !expected.contains(n))
        {
            return Err(ArchiveError::Invalid(
                "Dateien passen nicht zum Manifest".into(),
            ));
        }
        Ok(Self { zip, manifest })
    }

    /// Datensätze; die Prüfsumme muss zum Manifest passen.
    ///
    /// # Errors
    /// Bei abweichender Prüfsumme oder ungültigem JSON.
    pub fn data(&mut self) -> Result<BackupData, ArchiveError> {
        let raw = Self::read_entry(&mut self.zip, DATA)?;
        if sha256(&raw) != self.manifest.data_sha256 {
            return Err(ArchiveError::Checksum(DATA.into()));
        }
        serde_json::from_slice(&raw).map_err(|e| ArchiveError::Invalid(format!("data.json: {e}")))
    }

    /// Foto zum Objektschlüssel; Größe und Prüfsumme müssen zum Manifest passen.
    ///
    /// # Errors
    /// Bei unbekanntem Schlüssel oder abweichender Prüfsumme.
    pub fn blob(&mut self, key: &str) -> Result<Bytes, ArchiveError> {
        let entry = self
            .manifest
            .blobs
            .iter()
            .find(|b| b.key == key)
            .cloned()
            .ok_or_else(|| ArchiveError::Invalid(format!("{key} nicht im Manifest")))?;
        let raw = Self::read_entry(&mut self.zip, &format!("{BLOB_PREFIX}{key}"))?;
        if raw.len() as u64 != entry.size || sha256(&raw) != entry.sha256 {
            return Err(ArchiveError::Checksum(key.into()));
        }
        Ok(Bytes::from(raw))
    }
}

/// Festkommawert einer bereits validierten Anschaffung.
fn scaled(d: rust_decimal::Decimal, scale: u32) -> i64 {
    use rust_decimal::prelude::ToPrimitive;
    d.checked_mul(rust_decimal::Decimal::from(10_i64.pow(scale)))
        .and_then(|v| v.trunc().to_i64())
        .expect("validated acquisition fits fixed-point storage")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{HouseholdId, ReadingId};
    use std::io::Cursor;

    // @test SPEC_BACKUP_FORMAT, SPEC_CAR_MILEAGE_UI
    #[test]
    fn v1_acquisition_and_archive_remain_readable() {
        let record: AcquisitionRecord = serde_json::from_value(serde_json::json!({
            "vehicle_id": Uuid::nil(),
            "purchased_on": "2025-01-01",
            "price_micro": 30_000_000_000_i64,
            "residual_micro": 5_000_000_000_i64,
            "planned_end_on": "2030-01-01",
            "sold_on": null,
            "sale_price_micro": null
        }))
        .unwrap();
        let holding = record.to_holding().unwrap();
        assert_eq!(
            holding.mode,
            zaehler_domain::vehicle::DepreciationMode::Time
        );
        assert_eq!(
            holding.residual_value,
            Some(rust_decimal::Decimal::from(5000))
        );
        assert_eq!(
            AcquisitionRecord::from_holding(Uuid::nil(), &holding),
            record
        );
        let mut data = BackupData::default();
        data.acquisitions.push(record);
        let raw = archive(&data, &[], |m| m.format_version = 1);
        let mut reader = ArchiveReader::open(Cursor::new(raw)).unwrap();
        assert_eq!(reader.data().unwrap(), data);
    }

    fn manifest() -> Manifest {
        Manifest {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION,
            scope: Scope::Installation,
            app_version: "test".into(),
            schema: vec!["m1".into()],
            created_at: Utc::now(),
            counts: Counts::default(),
            data_sha256: String::new(),
            blobs: Vec::new(),
            missing_blobs: Vec::new(),
        }
    }

    fn archive(
        data: &BackupData,
        blobs: &[(&str, &[u8])],
        edit: impl FnOnce(&mut Manifest),
    ) -> Vec<u8> {
        let mut w = ArchiveWriter::new(Cursor::new(Vec::new()));
        let sha = w.data(data).unwrap();
        for (k, b) in blobs {
            w.blob(k, b).unwrap();
        }
        let mut m = manifest();
        m.data_sha256 = sha;
        edit(&mut m);
        w.finish(m).unwrap().into_inner()
    }

    // @test SPEC_BACKUP_FORMAT
    #[test]
    fn archive_roundtrip_checks_manifest_and_checksums() {
        let key = BlobKey::reading_photo(HouseholdId(Uuid::now_v7()), ReadingId(Uuid::now_v7()))
            .to_string();
        let data = BackupData {
            households: vec![HouseholdRecord {
                id: Uuid::now_v7(),
                name: "Haus".into(),
                timezone: "Europe/Berlin".into(),
                created_at: Utc::now(),
            }],
            ..BackupData::default()
        };
        let bytes = archive(&data, &[(&key, b"\xFF\xD8\xFFjpeg")], |_| {});
        let mut r = ArchiveReader::open(Cursor::new(bytes.clone())).unwrap();
        assert_eq!(r.manifest.blobs.len(), 1);
        assert_eq!(r.data().unwrap(), data);
        assert_eq!(&r.blob(&key).unwrap()[..], b"\xFF\xD8\xFFjpeg");

        // neuere Formatversion wird abgelehnt
        let newer = archive(&data, &[], |m| m.format_version = FORMAT_VERSION + 1);
        assert!(matches!(
            ArchiveReader::open(Cursor::new(newer)),
            Err(ArchiveError::NewerFormat(_))
        ));
        // falsche Prüfsumme der Daten
        let bad = archive(&data, &[], |m| m.data_sha256 = "0".repeat(64));
        assert!(matches!(
            ArchiveReader::open(Cursor::new(bad)).unwrap().data(),
            Err(ArchiveError::Checksum(_))
        ));
        // Datei im Archiv, die das Manifest nicht nennt
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let json = serde_json::to_vec(&data).unwrap();
        let mut m = manifest();
        m.data_sha256 = sha256(&json);
        for (name, bytes) in [
            (DATA.to_owned(), json),
            (format!("{BLOB_PREFIX}{key}"), b"x".to_vec()),
            (MANIFEST.to_owned(), serde_json::to_vec(&m).unwrap()),
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        let foreign = zip.finish().unwrap().into_inner();
        assert!(matches!(
            ArchiveReader::open(Cursor::new(foreign)),
            Err(ArchiveError::Invalid(_))
        ));
        // Pfad außerhalb der erlaubten Schlüssel
        let mut w = ArchiveWriter::new(Cursor::new(Vec::new()));
        assert!(w.blob("../../etc/passwd", b"x").is_err());
        assert!(ArchiveReader::open(Cursor::new(b"kein zip".to_vec())).is_err());
    }
}
