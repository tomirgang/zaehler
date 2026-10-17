//! Datentypen an den Port-Grenzen. Adapter bilden ihre eigenen Typen
//! (SeaORM-Entities, Formulare) auf diese Typen ab.

use chrono::{DateTime, Days, NaiveDate, Utc};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use zaehler_domain::{
    cost::ContractTerms,
    forecast::SeasonProfile,
    meter::{Direction, MeterKind, MeterPurpose},
    period::Period,
    reading::ReadingSource,
    schedule::ReadingSchedule,
    tariff::{self, CostPosition, PriceComponent},
};

use crate::{
    ids::{
        AdjustmentId, BlobKey, ContractId, DeviceId, DraftId, HouseholdId, InstallmentId,
        InvitationId, MeterId, OutboxId, PositionId, PriceComponentId, ReadingId, RegisterId,
        RevisionId, SettlementId, UserId,
    },
    tenant::Role,
};

/// Identität aus dem Identity Provider (bzw. Dev-Login).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub display_name: String,
    /// Bevorzugte Sprache für neu angelegte Nutzer (`de` oder `en`).
    pub locale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub display_name: String,
    pub email: Option<String>,
    pub locale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HouseholdSummary {
    pub id: HouseholdId,
    pub name: String,
    pub timezone: Tz,
    pub role: Role,
}

/// Mitglied eines Haushalts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub user_id: UserId,
    pub display_name: String,
    pub role: Role,
}

/// Einladung in einen Haushalt (SPEC_TEN_INVITES). Gespeichert wird nur der
/// SHA-256 des Tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    pub id: InvitationId,
    pub household_id: HouseholdId,
    pub household_name: String,
    pub role: Role,
    pub email: Option<String>,
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub accepted_by: Option<UserId>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl Invitation {
    /// Weder angenommen noch widerrufen noch abgelaufen.
    #[must_use]
    pub fn is_open(&self, now: DateTime<Utc>) -> bool {
        self.accepted_at.is_none() && self.revoked_at.is_none() && self.expires_at > now
    }
}

/// Neue Einladung; `token_hash` ist der SHA-256 des Tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInvitation {
    pub id: InvitationId,
    pub role: Role,
    pub token_hash: [u8; 32],
    pub email: Option<String>,
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HouseholdData {
    pub name: String,
    pub timezone: Tz,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Household {
    pub id: HouseholdId,
    pub data: HouseholdData,
}

/// Monat und Tag, z.B. Beginn des Abrechnungsjahres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonthDay {
    pub month: u8,
    pub day: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeterData {
    pub name: String,
    pub kind: MeterKind,
    pub unit: String,
    /// Ableseplan (SPEC_RDG_SCHEDULE)
    pub schedule: Option<ReadingSchedule>,
    pub max_daily_rate: Option<Decimal>,
    pub billing_start: Option<MonthDay>,
    /// Verwendungszweck (SPEC_MTR_SUBMETER)
    pub purpose: MeterPurpose,
    /// Jahresziel und Saisonprofil für Prognose und Ampel
    /// (SPEC_ANA_TRAFFIC_LIGHT)
    pub targets: MeterTargets,
}

/// Vorgaben für Erwartung und Ampel einer Messstelle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeterTargets {
    /// Angepasstes Standardprofil (12 Monatsgewichte); ohne gilt das
    /// gelernte oder das Standardprofil der Art
    pub season_weights: Option<SeasonProfile>,
    /// Erwarteter Jahresverbrauch in der Einheit der Messstelle
    pub annual_target: Option<Decimal>,
    /// Toleranz um das Ziel in Prozent; ohne Angabe 20
    pub tolerance_pct: Option<u8>,
}

/// Angepasstes Saisonprofil als JSON (12 Gewichte als Zeichenketten).
#[must_use]
pub fn season_weights_to_json(profile: &SeasonProfile) -> String {
    serde_json::to_string(&profile.monthly_weights).unwrap_or_default()
}

/// Angepasstes Saisonprofil aus seinem JSON; `None` bei ungültigem Inhalt.
#[must_use]
pub fn season_weights_from_json(json: &str) -> Option<SeasonProfile> {
    serde_json::from_str(json)
        .ok()
        .map(|monthly_weights| SeasonProfile { monthly_weights })
}

/// Ableseplan aus den gespeicherten Spalten (`schedule_kind`,
/// `schedule_value`); ohne Art gilt der Turnus `reading_interval_days`
/// aus dem Schema vor `m20261016_000008_meter_v1`.
#[must_use]
pub fn schedule_from_columns(
    kind: Option<&str>,
    value: Option<i64>,
    legacy_interval_days: Option<i64>,
) -> Option<ReadingSchedule> {
    match (kind, value) {
        (Some(kind), Some(value)) => ReadingSchedule::from_parts(kind, u16::try_from(value).ok()?),
        _ => legacy_interval_days
            .and_then(|d| u16::try_from(d).ok())
            .map(ReadingSchedule::EveryDays),
    }
}

/// Gespeicherte Spalten eines Ableseplans: Art, Zahl und für ältere
/// Programmversionen der Turnus in Tagen.
#[must_use]
pub fn schedule_columns(
    schedule: Option<ReadingSchedule>,
) -> (Option<&'static str>, Option<u16>, Option<u16>) {
    match schedule {
        None => (None, None, None),
        Some(s @ ReadingSchedule::EveryDays(n)) => (Some(s.kind()), Some(n), Some(n)),
        Some(s) => (Some(s.kind()), Some(s.value()), None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meter {
    pub id: MeterId,
    pub data: MeterData,
    pub archived: bool,
    /// Zeitpunkt der Archivierung (SPEC_MTR_ARCHIVE)
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceData {
    pub serial_number: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub rollover_digits: Option<u8>,
    pub decimal_places: u8,
    /// Übergeordnete Messstelle während der Einbauzeit (Zwischenzähler,
    /// SPEC_MTR_SUBMETER)
    pub parent_meter_id: Option<MeterId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub data: DeviceData,
    pub removed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterData {
    pub code: String,
    pub direction: Direction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    pub id: RegisterId,
    pub data: RegisterData,
}

/// Messstelle mit Geräten (nach Einbau sortiert) und Registern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeterAggregate {
    pub meter: Meter,
    pub devices: Vec<Device>,
    pub registers: Vec<Register>,
}

impl MeterAggregate {
    /// Aktuell eingebautes Gerät.
    #[must_use]
    pub fn active_device(&self) -> Option<&Device> {
        self.devices.iter().rev().find(|d| d.removed_at.is_none())
    }

    #[must_use]
    pub fn device(&self, id: DeviceId) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    #[must_use]
    pub fn register(&self, id: RegisterId) -> Option<&Register> {
        self.registers.iter().find(|r| r.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeterSummary {
    pub meter: Meter,
    pub last_reading_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub id: ReadingId,
    pub register_id: RegisterId,
    pub device_id: DeviceId,
    pub read_at: DateTime<Utc>,
    pub value: Decimal,
    pub source: ReadingSource,
    pub note: Option<String>,
    pub photo_key: Option<BlobKey>,
    /// Konfidenz der AI-Erkennung (0..1) bei `source = photo`.
    pub ai_confidence: Option<Decimal>,
    pub created_by: Option<UserId>,
}

/// Änderbare Felder einer Ablesung (SPEC_RDG_HISTORY).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingUpdate {
    pub read_at: DateTime<Utc>,
    pub value: Decimal,
    pub note: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Art einer Änderung an einer Ablesung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionAction {
    Update,
    Delete,
}

impl RevisionAction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

impl std::str::FromStr for RevisionAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "update" => Ok(Self::Update),
            "delete" => Ok(Self::Delete),
            other => Err(format!("unbekannte Änderung: {other}")),
        }
    }
}

/// Unveränderliche Revision einer Ablesung; bleibt nach dem Löschen der
/// Ablesung erhalten (SPEC_RDG_HISTORY).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub id: RevisionId,
    pub reading_id: ReadingId,
    pub meter_id: MeterId,
    pub register_id: RegisterId,
    pub action: RevisionAction,
    pub changed_by: UserId,
    pub changed_at: DateTime<Utc>,
    pub old_read_at: DateTime<Utc>,
    pub old_value: Decimal,
    pub old_note: Option<String>,
    /// `None` beim Löschen
    pub new_read_at: Option<DateTime<Utc>>,
    pub new_value: Option<Decimal>,
    pub new_note: Option<String>,
}

/// Art einer Benachrichtigung; zugleich Ereignistyp der Einstellungen
/// (SPEC_NTF_EMAIL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationKind {
    Reminder,
    Alarm,
    Invitation,
}

impl NotificationKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reminder => "reminder",
            Self::Alarm => "alarm",
            Self::Invitation => "invitation",
        }
    }
}

/// Benachrichtigungseinstellungen eines Nutzers; ohne Eintrag sind alle
/// Ereignistypen aktiv und die Adresse aus der Anmeldung gilt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationSettings {
    /// Abweichende Adresse; `None`: E-Mail aus dem OIDC-Login
    pub email: Option<String>,
    pub reminder: bool,
    pub alarm: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            email: None,
            reminder: true,
            alarm: true,
        }
    }
}

/// Empfänger einer Benachrichtigung (nur für Jobs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    pub user_id: UserId,
    pub email: String,
    pub locale: String,
}

/// Eintrag im Versandprotokoll; `(user_id, kind, subject_key)` ist
/// eindeutig und verhindert Doppelversand (SPEC_NTF_REMINDERS,
/// SPEC_NTF_ALARMS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationLogEntry {
    pub household_id: HouseholdId,
    pub user_id: UserId,
    pub kind: NotificationKind,
    pub subject_key: String,
    pub sent_at: DateTime<Utc>,
}

/// Fällige Messstelle in einer Erinnerung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueMeter {
    pub name: String,
    pub due_on: chrono::NaiveDate,
    pub overdue: bool,
    pub link: String,
}

/// Inhalt einer Benachrichtigung mit fachlichen Daten; Texte und Sprache
/// erzeugt der Adapter (SPEC_NTF_EMAIL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    Reminder {
        household: String,
        meters: Vec<DueMeter>,
        settings_link: String,
    },
    ConsumptionAlarm {
        household: String,
        meter: String,
        unit: String,
        from: chrono::NaiveDateTime,
        until: chrono::NaiveDateTime,
        consumption: Decimal,
        expected_lower: Decimal,
        expected_upper: Decimal,
        link: String,
        settings_link: String,
    },
    Invitation {
        household: String,
        inviter: String,
        role: Role,
        link: String,
        expires_on: chrono::NaiveDate,
    },
}

impl Notification {
    #[must_use]
    pub fn kind(&self) -> NotificationKind {
        match self {
            Self::Reminder { .. } => NotificationKind::Reminder,
            Self::ConsumptionAlarm { .. } => NotificationKind::Alarm,
            Self::Invitation { .. } => NotificationKind::Invitation,
        }
    }
}

/// Ereignis für die Folgeverarbeitung, geschrieben in derselben Unit of Work
/// wie die fachliche Änderung (SPEC_JOB_OUTBOX, ADR_024).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum OutboxEvent {
    /// Neue oder korrigierte Ablesungen einer Messstelle
    ReadingsRecorded {
        meter_id: MeterId,
        reading_ids: Vec<ReadingId>,
    },
    /// Einladung mit E-Mail-Adresse; das Token steht nur bis zur Zustellung
    /// im Eintrag und ist nach Ablauf der Einladung wertlos
    InvitationCreated {
        invitation_id: InvitationId,
        token: String,
    },
}

impl OutboxEvent {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ReadingsRecorded { .. } => "readings_recorded",
            Self::InvitationCreated { .. } => "invitation_created",
        }
    }
}

/// Zustand eines Outbox-Eintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxStatus {
    Pending,
    Done,
    Failed,
}

impl OutboxStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for OutboxStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unbekannter Status: {other}")),
        }
    }
}

/// Offener Outbox-Eintrag; `payload` ist das Ereignis als JSON, damit
/// unbekannte Arten nicht schon beim Lesen scheitern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    pub id: OutboxId,
    pub household_id: HouseholdId,
    pub kind: String,
    pub payload: String,
    pub attempts: u32,
    pub created_at: DateTime<Utc>,
}

/// Ergebnis der Verarbeitung eines Outbox-Eintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxResult {
    Done {
        at: DateTime<Utc>,
    },
    /// Erneuter Versuch ab `next_attempt_at`
    Retry {
        attempts: u32,
        next_attempt_at: DateTime<Utc>,
    },
    /// Endgültig fehlgeschlagen
    Failed {
        attempts: u32,
        at: DateTime<Utc>,
    },
}

/// Ergebnis eines Job-Laufs für `job_runs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRun {
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    /// `ok` oder `error`
    pub outcome: String,
}

/// Vorschlag der Zählerstandserkennung; nicht lesbare Felder sind `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recognition {
    pub value: Option<Decimal>,
    pub serial_number: Option<String>,
    pub register: Option<String>,
    pub unit: Option<String>,
    pub meter_kind: Option<MeterKind>,
    /// Selbsteinschätzung 0..1
    pub confidence: Decimal,
}

/// Zustand eines Entwurfs der Foto-Erfassung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftStatus {
    /// Foto gespeichert, Analyse steht aus (SPEC_AI_MULTI)
    Queued,
    /// Analyse läuft
    Analysing,
    /// Vorschlag liegt vor, Bestätigung offen
    Pending,
    /// Erkennung fehlgeschlagen oder nicht konfiguriert; manuelle Eingabe offen
    Failed,
    Confirmed,
    Discarded,
}

impl DraftStatus {
    pub const ALL: [Self; 6] = [
        Self::Queued,
        Self::Analysing,
        Self::Pending,
        Self::Failed,
        Self::Confirmed,
        Self::Discarded,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Analysing => "analysing",
            Self::Pending => "pending",
            Self::Failed => "failed",
            Self::Confirmed => "confirmed",
            Self::Discarded => "discarded",
        }
    }

    /// Entwurf ist nicht abgeschlossen (wartet auf Analyse, Bestätigung
    /// oder Verwerfen).
    #[must_use]
    pub fn is_open(self) -> bool {
        !matches!(self, Self::Confirmed | Self::Discarded)
    }

    /// Analyse abgeschlossen; der Nutzer kann bestätigen.
    #[must_use]
    pub fn is_ready(self) -> bool {
        matches!(self, Self::Pending | Self::Failed)
    }
}

impl std::str::FromStr for DraftStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| format!("unbekannter Status: {s}"))
    }
}

/// Entwurf der Foto-Erfassung (SPEC_AI_DRAFTS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub id: DraftId,
    pub created_by: UserId,
    pub photo_key: BlobKey,
    /// Modell der Erkennung; `None` ohne AI-Konfiguration
    pub model: Option<String>,
    pub duration_ms: Option<i64>,
    pub suggestion: Option<Recognition>,
    pub matched_meter_id: Option<MeterId>,
    pub status: DraftStatus,
    pub created_at: DateTime<Utc>,
    /// Aufnahmezeit aus den Bildmetadaten (SPEC_IMG_CAPTURE_TIME)
    pub taken_at: Option<DateTime<Utc>>,
    /// Beginn der laufenden Analyse (SPEC_AI_MULTI)
    pub analysis_started_at: Option<DateTime<Utc>>,
}

/// Ergebnis der Analyse eines Entwurfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftAnalysis {
    pub model: Option<String>,
    pub duration_ms: Option<i64>,
    pub suggestion: Option<Recognition>,
    pub matched_meter_id: Option<MeterId>,
    /// `Pending` mit Vorschlag, sonst `Failed`
    pub status: DraftStatus,
}

/// Abschluss eines Entwurfs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftOutcome {
    Confirmed {
        reading: ReadingId,
        /// Wert weicht vom Vorschlag ab; `None` ohne Vorschlag
        corrected: Option<bool>,
    },
    Discarded,
}

/// Preisstand einer Kostenposition (SPEC_TAR_CONTRACTS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceComponentRow {
    pub id: PriceComponentId,
    pub component: PriceComponent,
}

/// Kostenposition eines Vertrags; `register_id = None` gilt für alle
/// Register der Richtung des Vertrags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub id: PositionId,
    pub name: String,
    pub register_id: Option<RegisterId>,
    /// Umrechnungsfaktor Mess- auf Abrechnungseinheit (Standard 1)
    pub factor: Decimal,
    /// MwSt-Satz in Prozent, nur für die Netto-Anzeige (REQ_TAR_010)
    pub vat_pct: Option<Decimal>,
    /// Preisstände, aufsteigend nach `valid_from`
    pub components: Vec<PriceComponentRow>,
}

/// Änderbare Daten eines Vertrags; Positionen und Preisstände werden beim
/// Ändern vollständig ersetzt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractData {
    pub direction: Direction,
    pub provider: String,
    pub contract_number: Option<String>,
    pub starts_on: NaiveDate,
    /// Letzter Tag (einschließlich); `None` = unbefristet
    pub ends_on: Option<NaiveDate>,
    /// Beginn des Abrechnungszeitraums (12 Monate)
    pub billing_start: MonthDay,
    pub note: Option<String>,
    pub positions: Vec<Position>,
}

/// Abschlagsstand; gilt bis zum nächsten Stand (REQ_TAR_008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallmentRow {
    pub id: InstallmentId,
    pub installment: tariff::Installment,
}

/// Bonus oder Gutschrift (REQ_TAR_005).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjustmentRow {
    pub id: AdjustmentId,
    pub adjustment: tariff::Adjustment,
    pub note: Option<String>,
}

/// Betrag einer tatsächlichen Jahresabrechnung (SPEC_TAR_ACTUAL_SETTLEMENT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementRow {
    pub id: SettlementId,
    pub period_start: NaiveDate,
    /// Letzter Tag (einschließlich)
    pub period_end: NaiveDate,
    pub amount: Decimal,
    /// Beleg (Rechnungsimport, WP_V1_10)
    pub document_id: Option<uuid::Uuid>,
}

impl SettlementRow {
    /// Abgerechneter Zeitraum als halboffener Zeitraum.
    #[must_use]
    pub fn period(&self) -> Period {
        Period::new(self.period_start, self.period_end + Days::new(1))
    }
}

/// Eintrag eines Vertrags, der einzeln gelöscht werden kann.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractEntry {
    Installment(InstallmentId),
    Adjustment(AdjustmentId),
    Settlement(SettlementId),
}

/// Vertrag mit Positionen, Preisständen, Abschlägen, Anpassungen und
/// Abrechnungen; Listen nach Datum aufsteigend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub id: ContractId,
    pub meter_id: MeterId,
    pub data: ContractData,
    pub installments: Vec<InstallmentRow>,
    pub adjustments: Vec<AdjustmentRow>,
    pub settlements: Vec<SettlementRow>,
}

impl Contract {
    /// Positionen im Domain-Modell, in derselben Reihenfolge.
    #[must_use]
    pub fn cost_positions(&self) -> Vec<CostPosition> {
        self.data
            .positions
            .iter()
            .map(|p| CostPosition {
                name: p.name.clone(),
                conversion_factor: p.factor,
                components: p.components.iter().map(|c| c.component.clone()).collect(),
            })
            .collect()
    }

    #[must_use]
    pub fn adjustment_values(&self) -> Vec<tariff::Adjustment> {
        self.adjustments
            .iter()
            .map(|a| a.adjustment.clone())
            .collect()
    }

    #[must_use]
    pub fn installment_values(&self) -> Vec<tariff::Installment> {
        self.installments
            .iter()
            .map(|i| i.installment.clone())
            .collect()
    }

    /// Vertragsbedingungen für die Kostenrechnung.
    #[must_use]
    pub fn terms<'a>(
        &self,
        positions: &'a [CostPosition],
        adjustments: &'a [tariff::Adjustment],
    ) -> ContractTerms<'a> {
        ContractTerms {
            starts_on: self.data.starts_on,
            ends_on: self.data.ends_on,
            billing_start: (
                u32::from(self.data.billing_start.month),
                u32::from(self.data.billing_start.day),
            ),
            positions,
            adjustments,
        }
    }

    /// Laufzeit `[starts_on, ends_on + 1)`; unbefristet bis `NaiveDate::MAX`.
    #[must_use]
    pub fn validity(&self) -> Period {
        Period::new(
            self.data.starts_on,
            self.data
                .ends_on
                .map_or(NaiveDate::MAX, |d| d + Days::new(1)),
        )
    }

    /// Der Vertrag gilt am Tag `date`.
    #[must_use]
    pub fn active_on(&self, date: NaiveDate) -> bool {
        let v = self.validity();
        date >= v.start && date < v.end
    }

    /// Abschlagsstand, der am Tag `date` gilt.
    #[must_use]
    pub fn installment_on(&self, date: NaiveDate) -> Option<&InstallmentRow> {
        self.installments
            .iter()
            .rev()
            .find(|i| i.installment.valid_from <= date)
    }
}

/// Dateiart eines Dokuments (SPEC_DOC_UPLOADS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentType {
    Pdf,
    /// Normalisiertes Foto
    Jpeg,
}

impl DocumentType {
    #[must_use]
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Jpeg => "image/jpeg",
        }
    }

    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Jpeg => "jpg",
        }
    }

    #[must_use]
    pub fn from_media_type(s: &str) -> Option<Self> {
        match s {
            "application/pdf" => Some(Self::Pdf),
            "image/jpeg" => Some(Self::Jpeg),
            _ => None,
        }
    }
}

/// Zustand eines Dokuments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentStatus {
    /// Gespeichert, Erkennung steht aus
    Uploaded,
    /// Vorschlag liegt vor
    Analysed,
    /// Erkennung fehlgeschlagen oder nicht konfiguriert
    Failed,
    /// Vorschläge übernommen
    Confirmed,
    Discarded,
}

impl DocumentStatus {
    pub const ALL: [Self; 5] = [
        Self::Uploaded,
        Self::Analysed,
        Self::Failed,
        Self::Confirmed,
        Self::Discarded,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uploaded => "uploaded",
            Self::Analysed => "analysed",
            Self::Failed => "failed",
            Self::Confirmed => "confirmed",
            Self::Discarded => "discarded",
        }
    }

    /// Weder übernommen noch verworfen.
    #[must_use]
    pub fn is_open(self) -> bool {
        !matches!(self, Self::Confirmed | Self::Discarded)
    }
}

impl std::str::FromStr for DocumentStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|x| x.as_str() == s)
            .ok_or_else(|| format!("unbekannter Status: {s}"))
    }
}

/// Zählerstand aus einer Rechnung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceReading {
    pub serial_number: Option<String>,
    pub register: Option<String>,
    pub date: NaiveDate,
    pub value: Decimal,
}

/// Preisstand einer Position aus einer Rechnung (brutto).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoicePrice {
    pub position: String,
    pub valid_from: Option<NaiveDate>,
    pub base_price: Option<Decimal>,
    pub base_interval: Option<tariff::BasePriceInterval>,
    pub unit_price: Option<Decimal>,
}

/// Neuer Abschlag aus einer Rechnung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceInstallment {
    pub amount: Decimal,
    pub every_months: u8,
    pub valid_from: Option<NaiveDate>,
}

/// Vorschlag der Rechnungserkennung (SPEC_AI_INVOICE); nicht lesbare
/// Felder sind leer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceExtraction {
    pub provider: Option<String>,
    pub contract_number: Option<String>,
    pub period_start: Option<NaiveDate>,
    pub period_end: Option<NaiveDate>,
    pub readings: Vec<InvoiceReading>,
    pub prices: Vec<InvoicePrice>,
    pub installment: Option<InvoiceInstallment>,
    /// Rechnungsbetrag (Kosten des Zeitraums, brutto)
    pub total_amount: Option<Decimal>,
    /// Selbsteinschätzung 0..1
    pub confidence: Decimal,
}

/// Hochgeladenes Dokument mit Erkennungsergebnis (SPEC_DOC_UPLOADS,
/// SPEC_AI_INVOICE).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub id: crate::ids::DocumentId,
    pub blob_key: BlobKey,
    pub kind: DocumentKind,
    pub media_type: DocumentType,
    pub bytes: i64,
    /// Dateiname beim Upload, nur zur Anzeige
    pub original_name: String,
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
    pub status: DocumentStatus,
    /// Modell der Erkennung; `None` ohne AI-Konfiguration
    pub model: Option<String>,
    pub extraction: Option<InvoiceExtraction>,
    /// Vorschlag bei Belegen
    pub receipt: Option<ReceiptExtraction>,
    /// Über die Zählernummer zugeordnete Messstelle
    pub meter_id: Option<MeterId>,
    pub analysed_at: Option<DateTime<Utc>>,
}

/// Ergebnis der Erkennung eines Dokuments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentAnalysis {
    pub model: Option<String>,
    pub extraction: Option<InvoiceExtraction>,
    /// Vorschlag bei Belegen
    pub receipt: Option<ReceiptExtraction>,
    pub meter_id: Option<MeterId>,
    /// `Analysed` mit Vorschlag, sonst `Failed`
    pub status: DocumentStatus,
    pub at: DateTime<Utc>,
}

/// Stammdaten eines Fahrzeugs; die Messstelle der Art `vehicle` führt den
/// Kilometerstand (SPEC_CAR_VEHICLES).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VehicleData {
    pub plate: String,
    pub model: String,
    pub drive_type: zaehler_domain::vehicle::DriveType,
    /// Heimlade-Messstelle: Wallbox-Zwischenzähler (gemessen) oder
    /// Hausstrom-Messstelle (Schätzung)
    pub home_charging_meter_id: Option<MeterId>,
    /// Bekannter PV-Anteil beim Heimladen in Prozent (ab M8)
    pub pv_share_pct: Option<Decimal>,
}

/// Verbrauchsprofil mit ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRow {
    pub id: crate::ids::ProfileId,
    pub profile: zaehler_domain::vehicle::ConsumptionProfile,
}

/// Fahrzeug mit Profilen und Anschaffung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vehicle {
    pub meter_id: MeterId,
    pub data: VehicleData,
    /// Aufsteigend nach `valid_from`
    pub profiles: Vec<ProfileRow>,
    pub holding: Option<zaehler_domain::vehicle::Holding>,
}

impl Vehicle {
    #[must_use]
    pub fn consumption_profiles(&self) -> Vec<zaehler_domain::vehicle::ConsumptionProfile> {
        self.profiles.iter().map(|p| p.profile.clone()).collect()
    }
}

/// Ladeart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargingKind {
    Ac,
    Dc,
}

impl ChargingKind {
    pub const ALL: [Self; 2] = [Self::Ac, Self::Dc];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ac => "ac",
            Self::Dc => "dc",
        }
    }
}

impl std::str::FromStr for ChargingKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|x| x.as_str() == s)
            .ok_or_else(|| format!("unbekannte Ladeart: {s}"))
    }
}

/// Herkunft eines Eintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySource {
    Form,
    /// Aus der Belegerkennung (SPEC_AI_RECEIPT)
    Receipt,
}

impl EntrySource {
    pub const ALL: [Self; 2] = [Self::Form, Self::Receipt];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Form => "form",
            Self::Receipt => "receipt",
        }
    }
}

impl std::str::FromStr for EntrySource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|x| x.as_str() == s)
            .ok_or_else(|| format!("unbekannte Herkunft: {s}"))
    }
}

/// Externer Ladevorgang (REQ_CAR_010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChargingSession {
    pub id: crate::ids::ChargingId,
    pub vehicle: MeterId,
    pub charged_at: DateTime<Utc>,
    /// kWh
    pub energy: Decimal,
    /// Betrag brutto, 0 bei kostenlosem Laden
    pub amount: Decimal,
    pub odometer: Option<Decimal>,
    pub provider: Option<String>,
    pub kind: ChargingKind,
    pub source: EntrySource,
    pub document_id: Option<crate::ids::DocumentId>,
}

/// Anlage eines Kostenbuch-Eintrags (ADR_020).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetRef {
    pub asset_type: zaehler_domain::assets::AssetType,
    /// Fahrzeug: ID der Messstelle
    pub id: uuid::Uuid,
}

impl AssetRef {
    #[must_use]
    pub fn vehicle(meter: MeterId) -> Self {
        Self {
            asset_type: zaehler_domain::assets::AssetType::Vehicle,
            id: meter.0,
        }
    }
}

/// Eintrag im Kostenbuch (SPEC_ASSET_COSTS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetCost {
    pub id: crate::ids::AssetCostId,
    pub asset: AssetRef,
    pub booked_on: NaiveDate,
    pub category: zaehler_domain::assets::AssetCategory,
    pub amount: Decimal,
    /// Menge (l) bei Kraftstoff
    pub quantity: Option<Decimal>,
    pub odometer: Option<Decimal>,
    pub full_tank: Option<bool>,
    pub recurrence: Option<zaehler_domain::assets::Recurrence>,
    pub document_id: Option<crate::ids::DocumentId>,
    pub note: Option<String>,
}

impl AssetCost {
    /// Summe der Buchungen im Zeitraum.
    #[must_use]
    pub fn amount_in(&self, period: Period) -> Decimal {
        let n = zaehler_domain::assets::occurrences(self.booked_on, self.recurrence, period).len();
        self.amount * Decimal::from(n)
    }
}

/// Art eines Dokuments: Rechnung eines Versorgers oder Beleg
/// (SPEC_AI_INVOICE, SPEC_AI_RECEIPT).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DocumentKind {
    #[default]
    Invoice,
    Receipt,
}

impl DocumentKind {
    pub const ALL: [Self; 2] = [Self::Invoice, Self::Receipt];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invoice => "invoice",
            Self::Receipt => "receipt",
        }
    }
}

impl std::str::FromStr for DocumentKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|x| x.as_str() == s)
            .ok_or_else(|| format!("unbekannte Dokumentart: {s}"))
    }
}

/// Vorschlag der Belegerkennung (SPEC_AI_RECEIPT); nicht lesbare Felder
/// sind leer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptExtraction {
    pub date: Option<NaiveDate>,
    /// Betrag brutto
    pub total_amount: Option<Decimal>,
    pub category: Option<zaehler_domain::assets::VehicleCostCategory>,
    pub quantity: Option<Decimal>,
    /// `l` oder `kWh`
    pub unit: Option<String>,
    pub vendor: Option<String>,
    pub odometer: Option<Decimal>,
    /// `ac` oder `dc` bei Ladebelegen
    pub charging_kind: Option<String>,
    pub confidence: Decimal,
}

impl ReceiptExtraction {
    /// Ladebeleg: Kategorie Laden oder Menge in kWh.
    #[must_use]
    pub fn is_charging(&self) -> bool {
        self.category == Some(zaehler_domain::assets::VehicleCostCategory::Charging)
            || self
                .unit
                .as_deref()
                .is_some_and(|u| u.eq_ignore_ascii_case("kwh"))
    }
}

/// Erkennungsergebnis als JSON wie gespeichert; die Art des Dokuments
/// bestimmt das Schema.
#[must_use]
pub fn extraction_json(
    invoice: Option<&InvoiceExtraction>,
    receipt: Option<&ReceiptExtraction>,
) -> Option<String> {
    match (invoice, receipt) {
        (Some(x), _) => serde_json::to_string(x).ok(),
        (None, Some(x)) => serde_json::to_string(x).ok(),
        (None, None) => None,
    }
}

/// Gespeichertes Erkennungsergebnis lesen.
///
/// # Errors
///
/// Wenn das JSON nicht zum Schema der Dokumentart passt.
pub fn parse_extraction(
    kind: DocumentKind,
    json: Option<&str>,
) -> Result<(Option<InvoiceExtraction>, Option<ReceiptExtraction>), serde_json::Error> {
    let Some(json) = json else {
        return Ok((None, None));
    };
    Ok(match kind {
        DocumentKind::Invoice => (Some(serde_json::from_str(json)?), None),
        DocumentKind::Receipt => (None, Some(serde_json::from_str(json)?)),
    })
}
