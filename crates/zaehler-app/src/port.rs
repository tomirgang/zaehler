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
    backup::BackupData,
    error::PortError,
    ids::{
        BlobKey, DeviceId, DraftId, HouseholdId, InvitationId, MeterId, OutboxId, ReadingId,
        RegisterId, UserId,
    },
    model::{
        DeviceData, Draft, DraftAnalysis, DraftOutcome, Household, HouseholdData, HouseholdSummary,
        Identity, Invitation, JobRun, Member, MeterAggregate, MeterData, MeterSummary,
        NewInvitation, Notification, NotificationKind, NotificationLogEntry, NotificationSettings,
        OutboxEntry, OutboxEvent, OutboxResult, Reading, ReadingUpdate, Recipient, Recognition,
        RegisterData, Revision, User,
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
    fn invitations(&mut self) -> &mut dyn InvitationRepository;
    fn revisions(&mut self) -> &mut dyn RevisionRepository;
    fn outbox(&mut self) -> &mut dyn OutboxRepository;
    fn notification_settings(&mut self) -> &mut dyn NotificationSettingsRepository;
    async fn commit(self: Box<Self>) -> PortResult<()>;
}

// @impl SPEC_TEN_ROLES
#[async_trait]
pub trait MembershipRepository: Send {
    async fn role(&mut self, user: UserId, household: HouseholdId) -> PortResult<Option<Role>>;
    /// Haushalte des Nutzers, nach Name sortiert.
    async fn households_of(&mut self, user: UserId) -> PortResult<Vec<HouseholdSummary>>;
    /// Mitglieder des Haushalts, nach Anzeigename sortiert.
    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<Member>>;
    /// Sperrt die Mitgliedschaften des Haushalts bis zum Ende der Unit of
    /// Work (PostgreSQL: Zeilensperre), damit die Prüfung auf den letzten
    /// Owner nicht parallel umgangen wird.
    async fn lock_household(&mut self, t: &Tenant) -> PortResult<()>;
    /// `PortError::NotFound`, wenn der Nutzer kein Mitglied ist.
    async fn set_role(&mut self, t: &Tenant, user: UserId, role: Role) -> PortResult<()>;
    /// `PortError::NotFound`, wenn der Nutzer kein Mitglied ist.
    async fn remove(&mut self, t: &Tenant, user: UserId) -> PortResult<()>;
    /// Legt eine Mitgliedschaft an (nur beim Einlösen einer Einladung);
    /// besteht sie bereits, bleibt die höhere Rolle.
    async fn add(&mut self, household: HouseholdId, user: UserId, role: Role) -> PortResult<()>;
}

/// Einladungen (SPEC_TEN_INVITES).
// @impl SPEC_TEN_INVITES
#[async_trait]
pub trait InvitationRepository: Send {
    async fn create(&mut self, t: &Tenant, i: &NewInvitation) -> PortResult<()>;
    /// Einladung zum Token-Hash, ohne Tenant (Einlösen).
    async fn find_by_hash(&mut self, hash: &[u8; 32]) -> PortResult<Option<Invitation>>;
    /// Bedingtes Update auf `accepted_at IS NULL AND revoked_at IS NULL`;
    /// `false`, wenn die Einladung bereits eingelöst oder widerrufen ist.
    async fn mark_accepted(
        &mut self,
        id: InvitationId,
        user: UserId,
        at: DateTime<Utc>,
    ) -> PortResult<bool>;
    /// Offene Einladungen des Haushalts, neueste zuerst.
    async fn list_open(&mut self, t: &Tenant, now: DateTime<Utc>) -> PortResult<Vec<Invitation>>;
    /// `PortError::NotFound`, wenn die Einladung nicht offen ist oder zu
    /// einem anderen Haushalt gehört.
    async fn revoke(&mut self, t: &Tenant, id: InvitationId, at: DateTime<Utc>) -> PortResult<()>;
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
    /// Messstellen mit mindestens einem Gerät, das `parent` zugeordnet ist
    /// (Zwischenzähler, SPEC_MTR_SUBMETER).
    async fn submeters_of(&mut self, t: &Tenant, parent: MeterId) -> PortResult<Vec<MeterId>>;
    /// Archiviert (`Some`) oder reaktiviert (`None`) die Messstelle.
    async fn set_archived(
        &mut self,
        t: &Tenant,
        id: MeterId,
        at: Option<DateTime<Utc>>,
    ) -> PortResult<()>;
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
    /// Ändert Zeitpunkt, Wert und Notiz; `PortError::NotFound` für fremde
    /// IDs, `PortError::Conflict` bei gleichem (Register, Gerät, Zeitpunkt).
    async fn update(&mut self, t: &Tenant, id: ReadingId, u: &ReadingUpdate) -> PortResult<()>;
}

/// Revisionen von Ablesungen; nur anfügen, nie ändern (SPEC_RDG_HISTORY).
// @impl SPEC_RDG_HISTORY
#[async_trait]
pub trait RevisionRepository: Send {
    /// Messstelle und Register müssen zum Haushalt gehören.
    async fn append(&mut self, t: &Tenant, r: &Revision) -> PortResult<()>;
    /// Revisionen der Messstelle, neueste zuerst.
    async fn for_meter(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Vec<Revision>>;
}

/// Ereignisse für die Folgeverarbeitung, geschrieben in der Unit of Work
/// der fachlichen Änderung (SPEC_JOB_OUTBOX).
// @impl SPEC_JOB_OUTBOX
#[async_trait]
pub trait OutboxRepository: Send {
    async fn enqueue(
        &mut self,
        t: &Tenant,
        id: OutboxId,
        event: &OutboxEvent,
        now: DateTime<Utc>,
    ) -> PortResult<()>;
}

/// Abfragen ohne Haushaltsbezug für Jobs; nur der Scheduler mit Leader-Lock
/// ruft sie auf (SPEC_JOB_SCHEDULER, SPEC_JOB_OUTBOX).
#[async_trait]
pub trait JobRepository: Send + Sync {
    /// Offene Einträge mit `next_attempt_at <= now`, älteste zuerst.
    async fn claim_outbox(&self, now: DateTime<Utc>, limit: u32) -> PortResult<Vec<OutboxEntry>>;
    async fn finish_outbox(&self, id: OutboxId, result: OutboxResult) -> PortResult<()>;
    /// Anzahl offener Einträge (Metrik).
    async fn pending_outbox(&self) -> PortResult<u64>;
    /// Protokolliert einen Lauf in `job_runs`.
    async fn record_run(&self, job: &str, run: &JobRun) -> PortResult<()>;
    /// Letzter Lauf eines Jobs, auch nach einem Neustart.
    async fn last_run(&self, job: &str) -> PortResult<Option<JobRun>>;
    /// Haushalte mit mindestens einer aktiven Messstelle mit Ableseplan.
    async fn households_with_schedules(&self) -> PortResult<Vec<HouseholdId>>;
    /// Empfänger mit Adresse und aktivem Ereignistyp: Erinnerungen an
    /// Mitglieder ab der Rolle `Member`, Alarme an alle Mitglieder.
    async fn recipients(
        &self,
        household: HouseholdId,
        kind: NotificationKind,
    ) -> PortResult<Vec<Recipient>>;
    /// Trägt den Versand ein; `false`, wenn er bereits eingetragen ist.
    async fn log_once(&self, entry: &NotificationLogEntry) -> PortResult<bool>;
    /// Entfernt einen Eintrag nach einem fehlgeschlagenen Versand.
    async fn release_log(&self, entry: &NotificationLogEntry) -> PortResult<()>;
}

/// Benachrichtigungseinstellungen je Nutzer (SPEC_NTF_EMAIL).
#[async_trait]
pub trait NotificationSettingsRepository: Send {
    async fn get(&mut self, user: UserId) -> PortResult<Option<NotificationSettings>>;
    async fn set(
        &mut self,
        user: UserId,
        settings: &NotificationSettings,
        now: DateTime<Utc>,
    ) -> PortResult<()>;
}

/// E-Mail-Versand (SPEC_NTF_EMAIL, ADR_011). Texte und Sprache erzeugt der
/// Adapter; Adressen erscheinen nicht in Logs.
#[async_trait]
pub trait Notifier: Send + Sync {
    /// Versand ist konfiguriert.
    fn enabled(&self) -> bool;
    async fn send(&self, to: &str, notification: &Notification, locale: &str) -> PortResult<()>;
}

/// Ohne SMTP-Konfiguration: nichts wird versendet.
pub struct DisabledNotifier;

#[async_trait]
impl Notifier for DisabledNotifier {
    fn enabled(&self) -> bool {
        false
    }

    async fn send(&self, _: &str, _: &Notification, _: &str) -> PortResult<()> {
        Err(PortError::Backend(
            "E-Mail-Versand nicht konfiguriert".into(),
        ))
    }
}

/// Genau eine aktive Instanz für Jobs (SPEC_JOB_SCHEDULER,
/// SPEC_OPS_REPLICAS).
#[async_trait]
pub trait LeaderLock: Send + Sync {
    /// Hält oder erwirbt den Lock; `false` ohne Lock oder bei Fehlern der
    /// Verbindung (der nächste Takt versucht es erneut).
    async fn is_leader(&self) -> bool;
}

/// Ohne mehrere Instanzen (SQLite) immer Leader.
pub struct SingleInstance;

#[async_trait]
impl LeaderLock for SingleInstance {
    async fn is_leader(&self) -> bool {
        true
    }
}

/// Entwürfe der Foto-Erfassung (SPEC_AI_DRAFTS).
// @impl SPEC_AI_DRAFTS
#[async_trait]
pub trait DraftRepository: Send {
    /// Legt den Entwurf im Haushalt des Tenants an.
    async fn create(&mut self, t: &Tenant, draft: &Draft) -> PortResult<()>;
    /// Offener (nicht abgeschlossener) Entwurf des Haushalts.
    async fn get_open(&mut self, t: &Tenant, id: DraftId) -> PortResult<Option<Draft>>;
    /// Offene Entwürfe des Haushalts, älteste zuerst (Warteschlange).
    async fn list_open(&mut self, t: &Tenant) -> PortResult<Vec<Draft>>;
    /// Übernimmt einen Entwurf zur Analyse: `queued` oder eine abgebrochene
    /// Analyse (`analysing` mit Beginn vor `stale_before`) wird `analysing`
    /// mit Beginn `now`. `false`, wenn ein anderer Lauf ihn bereits hat.
    async fn claim(
        &mut self,
        t: &Tenant,
        id: DraftId,
        now: DateTime<Utc>,
        stale_before: DateTime<Utc>,
    ) -> PortResult<bool>;
    /// Speichert das Ergebnis der Analyse eines Entwurfs im Status `analysing`.
    async fn record_analysis(
        &mut self,
        t: &Tenant,
        id: DraftId,
        analysis: &DraftAnalysis,
    ) -> PortResult<()>;
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
    /// Entfernt erledigte Outbox-Einträge, abgeschlossen vor `before`.
    async fn purge_outbox(&self, before: DateTime<Utc>) -> PortResult<u64>;
}

/// Vollständiger Export und Import aller Datensätze (SPEC_BACKUP_DATA). Ohne
/// Haushaltsbezug; Berechtigungen prüfen die Anwendungsfälle.
// @impl SPEC_BACKUP_DATA
#[async_trait]
pub trait BackupRepository: Send + Sync {
    /// Angewendete Migrationen (Schemastand), aufsteigend.
    async fn schema(&self) -> PortResult<Vec<String>>;
    /// Alle Datensätze außer Sitzungen. Liest auch ein älteres, noch nicht
    /// migriertes Schema: später ergänzte Spalten erhalten ihre Vorgabe.
    async fn export(&self) -> PortResult<BackupData>;
    /// Weder Nutzer noch Haushalte vorhanden.
    async fn is_empty(&self) -> PortResult<bool>;
    /// Fügt alle Datensätze mit ihren IDs in einer Transaktion ein.
    async fn import(&self, data: &BackupData) -> PortResult<()>;
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
