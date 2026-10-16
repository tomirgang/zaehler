//! Driven Ports: Traits, die Adapter implementieren (ADR_022).
//!
//! Aufruf über `Arc<dyn …>` bzw. `Box<dyn UnitOfWork>`; alle Methoden sind
//! `async` über `async-trait`.

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};
use uuid::Uuid;
use zaehler_domain::calendar::Calendar;

use crate::{
    error::PortError,
    ids::{BlobKey, DeviceId, DraftId, HouseholdId, MeterId, ReadingId, RegisterId, UserId},
    model::{
        DeviceData, Draft, DraftOutcome, Household, HouseholdData, HouseholdSummary, Identity,
        MeterAggregate, MeterData, MeterSummary, Reading, Recognition, RegisterData, User,
    },
    tenant::{Role, Tenant},
};

pub type PortResult<T> = Result<T, PortError>;

/// Erzeugt Transaktionen.
// @impl SPEC_ARCH_UOW
#[async_trait]
pub trait UnitOfWorkFactory: Send + Sync {
    async fn begin(&self) -> PortResult<Box<dyn UnitOfWork>>;
}

/// Eine Transaktion. Ohne `commit` werden alle Änderungen verworfen.
// @impl SPEC_ARCH_UOW
#[async_trait]
pub trait UnitOfWork: Send {
    fn memberships(&mut self) -> &mut dyn MembershipRepository;
    fn households(&mut self) -> &mut dyn HouseholdRepository;
    fn users(&mut self) -> &mut dyn UserRepository;
    fn meters(&mut self) -> &mut dyn MeterRepository;
    fn readings(&mut self) -> &mut dyn ReadingRepository;
    fn drafts(&mut self) -> &mut dyn DraftRepository;
    async fn commit(self: Box<Self>) -> PortResult<()>;
}

#[async_trait]
pub trait MembershipRepository: Send {
    async fn role(&mut self, user: UserId, household: HouseholdId) -> PortResult<Option<Role>>;
    /// Haushalte des Nutzers, nach Name sortiert.
    async fn households_of(&mut self, user: UserId) -> PortResult<Vec<HouseholdSummary>>;
}

#[async_trait]
pub trait HouseholdRepository: Send {
    /// Legt den Haushalt an und macht `owner` zum Owner.
    async fn create(
        &mut self,
        id: HouseholdId,
        owner: UserId,
        data: &HouseholdData,
        now: DateTime<Utc>,
    ) -> PortResult<()>;
    async fn get(&mut self, t: &Tenant) -> PortResult<Household>;
    async fn update(&mut self, t: &Tenant, data: &HouseholdData) -> PortResult<()>;
    /// Löscht den Haushalt mit allen abhängigen Daten.
    async fn delete(&mut self, t: &Tenant) -> PortResult<()>;
}

#[async_trait]
pub trait UserRepository: Send {
    /// Liefert den Nutzer zu (issuer, subject) oder legt ihn mit `new_id` an;
    /// aktualisiert Anzeigename, E-Mail und `last_login_at`.
    async fn upsert_oidc(
        &mut self,
        new_id: UserId,
        identity: &Identity,
        now: DateTime<Utc>,
    ) -> PortResult<User>;
    async fn get(&mut self, id: UserId) -> PortResult<Option<User>>;
    async fn set_locale(&mut self, id: UserId, locale: &str) -> PortResult<()>;
}

#[async_trait]
pub trait MeterRepository: Send {
    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<MeterSummary>>;
    async fn get(&mut self, t: &Tenant, id: MeterId) -> PortResult<Option<MeterAggregate>>;
    async fn create(
        &mut self,
        t: &Tenant,
        id: MeterId,
        data: &MeterData,
        now: DateTime<Utc>,
    ) -> PortResult<()>;
    async fn update(&mut self, t: &Tenant, id: MeterId, data: &MeterData) -> PortResult<()>;
    async fn add_register(
        &mut self,
        t: &Tenant,
        meter: MeterId,
        id: RegisterId,
        data: &RegisterData,
    ) -> PortResult<()>;
    async fn rename_register(&mut self, t: &Tenant, id: RegisterId, code: &str) -> PortResult<()>;
    async fn add_device(
        &mut self,
        t: &Tenant,
        meter: MeterId,
        id: DeviceId,
        data: &DeviceData,
    ) -> PortResult<()>;
    async fn update_device(
        &mut self,
        t: &Tenant,
        id: DeviceId,
        data: &DeviceData,
    ) -> PortResult<()>;
    async fn retire_device(
        &mut self,
        t: &Tenant,
        id: DeviceId,
        at: DateTime<Utc>,
    ) -> PortResult<()>;
    /// Aktives Gerät mit dieser Zählernummer im Haushalt.
    async fn find_by_serial(
        &mut self,
        t: &Tenant,
        serial: &str,
    ) -> PortResult<Option<(MeterId, DeviceId)>>;
    async fn find_by_name(&mut self, t: &Tenant, name: &str) -> PortResult<Option<MeterId>>;
}

#[async_trait]
pub trait ReadingRepository: Send {
    /// Alle Ablesungen der Messstelle, chronologisch.
    async fn list_for_meter(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Vec<Reading>>;
    /// Fügt Ablesungen ein; Register müssen zum Haushalt gehören.
    /// `PortError::Conflict` bei gleichem (Register, Gerät, Zeitpunkt).
    async fn insert(&mut self, t: &Tenant, readings: &[Reading]) -> PortResult<()>;
    /// Ablesung mit zugehöriger Messstelle.
    async fn get(&mut self, t: &Tenant, id: ReadingId) -> PortResult<Option<(MeterId, Reading)>>;
    async fn delete(&mut self, t: &Tenant, id: ReadingId) -> PortResult<()>;
}

/// Entwürfe der Foto-Erfassung (SPEC_AI_DRAFTS).
// @impl SPEC_AI_DRAFTS
#[async_trait]
pub trait DraftRepository: Send {
    /// Legt den Entwurf im Haushalt des Tenants an.
    async fn create(&mut self, t: &Tenant, draft: &Draft) -> PortResult<()>;
    /// Offener Entwurf (`pending` oder `failed`) des Haushalts.
    async fn get_open(&mut self, t: &Tenant, id: DraftId) -> PortResult<Option<Draft>>;
    /// Schließt einen offenen Entwurf ab; `PortError::NotFound`, wenn er nicht
    /// (mehr) offen ist.
    async fn finish(
        &mut self,
        t: &Tenant,
        id: DraftId,
        outcome: DraftOutcome,
        at: DateTime<Utc>,
    ) -> PortResult<()>;
}

/// Abfragen ohne Haushaltsbezug; verwendet nur der Anwendungsfall
/// `Housekeeping`. Alle Operationen sind idempotent.
// @impl SPEC_OPS_HOUSEKEEPING
#[async_trait]
pub trait MaintenanceRepository: Send + Sync {
    /// Markiert offene Entwürfe, angelegt vor `cutoff`, als `discarded` und
    /// liefert ihre Fotos.
    async fn expire_drafts(
        &self,
        cutoff: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> PortResult<Vec<BlobKey>>;
    /// Entfernt Sitzungen mit Ablauf vor `now`; liefert die Anzahl.
    async fn delete_expired_sessions(&self, now: DateTime<Utc>) -> PortResult<u64>;
}

/// Objektspeicher für Fotos und temporäre Dateien. Die Operationen sind
/// nicht transaktional (SPEC_ARCH_UOW).
// @impl SPEC_STORE_BLOBS
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Schreibt das Objekt; ein vorhandenes wird ersetzt.
    async fn put(&self, key: &BlobKey, bytes: Bytes) -> PortResult<()>;
    /// `PortError::NotFound`, wenn das Objekt fehlt.
    async fn get(&self, key: &BlobKey) -> PortResult<Bytes>;
    async fn copy(&self, from: &BlobKey, to: &BlobKey) -> PortResult<()>;
    /// Ein fehlendes Objekt ist kein Fehler.
    async fn delete(&self, key: &BlobKey) -> PortResult<()>;
    /// Löscht alle Objekte unter `households/{household}/`.
    async fn delete_household(&self, household: HouseholdId) -> PortResult<()>;
    /// Temporäre Objekte aller Haushalte, zuletzt geschrieben vor `cutoff`.
    async fn list_tmp_older_than(&self, cutoff: DateTime<Utc>) -> PortResult<Vec<BlobKey>>;
}

/// Foto nach der Normalisierung: JPEG ohne Metadaten, Ausrichtung angewendet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedJpeg {
    pub bytes: Bytes,
    pub width: u32,
    pub height: u32,
    /// Aufnahmezeit aus den Metadaten des Originals
    pub taken_at: Option<PhotoTime>,
}

/// Aufnahmezeit aus den Bildmetadaten (EXIF `DateTimeOriginal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhotoTime {
    /// Uhrzeit der Kamera
    pub local: NaiveDateTime,
    /// Abweichung von UTC (`OffsetTimeOriginal`), falls angegeben
    pub offset: Option<FixedOffset>,
}

impl PhotoTime {
    /// Zeitpunkt in UTC; ohne Offset gilt die Zeitzone des Haushalts.
    // @impl SPEC_IMG_CAPTURE_TIME
    #[must_use]
    pub fn to_utc(&self, cal: Calendar) -> DateTime<Utc> {
        match self.offset {
            Some(offset) => (self.local - offset).and_utc(),
            None => cal.to_utc(self.local),
        }
    }
}

/// Prüft, richtet aus, verkleinert und kodiert ein hochgeladenes Foto.
// @impl SPEC_IMG_PROCESSING
#[async_trait]
pub trait ImageNormalizer: Send + Sync {
    /// `PortError::Invalid` für nicht unterstützte oder nicht dekodierbare
    /// Dateien.
    async fn normalize(&self, bytes: Bytes) -> PortResult<NormalizedJpeg>;
}

/// Fehlschlag der Erkennung; bestimmt die Metrik `outcome`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecognitionError {
    /// Zeitüberschreitung
    Timeout,
    /// Antwort entspricht nicht dem Schema
    Invalid(String),
    /// Netzwerk- oder Dienstfehler
    Failed(String),
}

impl RecognitionError {
    #[must_use]
    pub fn outcome(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Invalid(_) => "invalid",
            Self::Failed(_) => "error",
        }
    }
}

impl std::fmt::Display for RecognitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => f.write_str("Zeitüberschreitung"),
            Self::Invalid(m) => write!(f, "ungültige Antwort: {m}"),
            Self::Failed(m) => write!(f, "Fehler: {m}"),
        }
    }
}

/// Erkennt den Zählerstand auf einem Foto (AI). Übertragen wird nur das
/// normalisierte Bild, keine Nutzer- oder Haushaltsdaten (REQ_AI_006).
#[async_trait]
pub trait MeterRecognizer: Send + Sync {
    /// Bezeichnung des Modells (für Entwürfe und Metriken).
    fn model(&self) -> &str;
    async fn recognize(&self, jpeg: &NormalizedJpeg) -> Result<Recognition, RecognitionError>;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub trait IdGenerator: Send + Sync {
    fn new_uuid(&self) -> Uuid;
}

/// Bereitschaft der Infrastruktur für `/readyz`.
#[async_trait]
pub trait Readiness: Send + Sync {
    async fn check(&self) -> PortResult<()>;
}

/// Systemuhr.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// UUIDv7 (zeitlich sortierbar).
pub struct UuidV7;

impl IdGenerator for UuidV7 {
    fn new_uuid(&self) -> Uuid {
        Uuid::now_v7()
    }
}
