//! In-Memory-Implementierung aller Driven Ports.
//!
//! Eine Unit of Work arbeitet auf einer Kopie des Zustands und übernimmt sie
//! beim `commit` (SPEC_ARCH_UOW).

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};

use crate::{
    PortError, Role, Tenant,
    ids::{
        BlobKey, BlobKind, DeviceId, DraftId, HouseholdId, InvitationId, MeterId, OutboxId,
        ReadingId, RegisterId, UserId,
    },
    model::{
        Contract, Device, DeviceData, Draft, DraftAnalysis, DraftOutcome, DraftStatus, Household,
        HouseholdData, HouseholdSummary, Identity, Invitation, JobRun, Member, Meter,
        MeterAggregate, MeterData, MeterSummary, NewInvitation, Notification, NotificationKind,
        NotificationLogEntry, NotificationSettings, OutboxEntry, OutboxEvent, OutboxResult,
        OutboxStatus, Reading, ReadingUpdate, Recipient, Recognition, Register, RegisterData,
        Revision, User,
    },
    port::{
        BlobStore, Clock, ContractRepository, DraftRepository, HouseholdRepository,
        ImageNormalizer, InvitationRepository, JobRepository, MaintenanceRepository,
        MembershipRepository, MeterRecognizer, MeterRepository, NormalizedJpeg,
        NotificationSettingsRepository, Notifier, OutboxRepository, PhotoTime, PortResult,
        Readiness, ReadingRepository, RecognitionError, RevisionRepository, UnitOfWork,
        UnitOfWorkFactory, UserRepository,
    },
};

#[derive(Debug, Clone)]
struct StoredUser {
    user: User,
    issuer: String,
    subject: String,
}

#[derive(Debug, Clone, Default)]
struct State {
    users: Vec<StoredUser>,
    households: Vec<Household>,
    memberships: Vec<(HouseholdId, UserId, Role)>,
    meters: Vec<(HouseholdId, Meter)>,
    devices: Vec<(MeterId, Device)>,
    registers: Vec<(MeterId, Register)>,
    readings: Vec<Reading>,
    /// Ablaufzeitpunkte von Sitzungen (nur für `MaintenanceRepository`)
    sessions: Vec<DateTime<Utc>>,
    drafts: Vec<StoredDraft>,
    invitations: Vec<StoredInvitation>,
    revisions: Vec<(HouseholdId, Revision)>,
    outbox: Vec<StoredOutbox>,
    job_runs: BTreeMap<String, JobRun>,
    notification_settings: BTreeMap<UserId, (NotificationSettings, DateTime<Utc>)>,
    /// (Nutzer, Art, Schlüssel) des Versandprotokolls
    notification_log: Vec<(UserId, &'static str, String)>,
    contracts: Vec<Contract>,
    documents: Vec<(HouseholdId, crate::model::Document)>,
    vehicles: Vec<crate::model::Vehicle>,
    charging: Vec<crate::model::ChargingSession>,
    asset_costs: Vec<(HouseholdId, crate::model::AssetCost)>,
    pv_systems: Vec<(HouseholdId, crate::model::PvSystem)>,
}

/// Outbox-Eintrag mit Zustand (nur für Tests einsehbar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOutbox {
    pub entry: OutboxEntry,
    pub status: OutboxStatus,
    pub next_attempt_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct StoredInvitation {
    invitation: NewInvitation,
    household: HouseholdId,
    accepted_by: Option<UserId>,
    accepted_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

impl State {
    fn invitation(&self, s: &StoredInvitation) -> Invitation {
        let i = &s.invitation;
        Invitation {
            id: i.id,
            household_id: s.household,
            household_name: self
                .households
                .iter()
                .find(|h| h.id == s.household)
                .map(|h| h.data.name.clone())
                .unwrap_or_default(),
            role: i.role,
            email: i.email.clone(),
            created_by: i.created_by,
            created_at: i.created_at,
            expires_at: i.expires_at,
            accepted_by: s.accepted_by,
            accepted_at: s.accepted_at,
            revoked_at: s.revoked_at,
        }
    }
}

#[derive(Debug, Clone)]
struct StoredDraft {
    household: HouseholdId,
    draft: Draft,
    corrected: Option<bool>,
}

impl State {
    fn owns_meter(&self, t: &Tenant, meter: MeterId) -> bool {
        self.meters
            .iter()
            .any(|(h, m)| *h == t.household_id() && m.id == meter)
    }

    fn meter_of_register(&self, register: RegisterId) -> Option<MeterId> {
        self.registers
            .iter()
            .find(|(_, r)| r.id == register)
            .map(|(m, _)| *m)
    }

    fn meter_of_device(&self, device: DeviceId) -> Option<MeterId> {
        self.devices
            .iter()
            .find(|(_, d)| d.id == device)
            .map(|(m, _)| *m)
    }
}

/// Gemeinsamer Speicher; implementiert `UnitOfWorkFactory` und `Readiness`.
#[derive(Debug, Default)]
pub struct InMemoryStore {
    shared: Arc<Mutex<State>>,
}

fn poisoned<T>(_: T) -> PortError {
    PortError::Backend("Mutex vergiftet".into())
}

#[async_trait]
impl UnitOfWorkFactory for InMemoryStore {
    async fn begin(&self) -> PortResult<Box<dyn UnitOfWork>> {
        let state = self.shared.lock().map_err(poisoned)?.clone();
        Ok(Box::new(MemoryUow {
            shared: self.shared.clone(),
            state,
        }))
    }
}

#[async_trait]
impl Readiness for InMemoryStore {
    async fn check(&self) -> PortResult<()> {
        Ok(())
    }
}

impl InMemoryStore {
    /// Legt eine Sitzung mit diesem Ablaufzeitpunkt an.
    pub fn add_session(&self, expires_at: DateTime<Utc>) {
        if let Ok(mut s) = self.shared.lock() {
            s.sessions.push(expires_at);
        }
    }

    /// Anzahl der Sitzungen.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.shared.lock().map_or(0, |s| s.sessions.len())
    }

    /// Status und Korrekturkennzeichen eines Entwurfs.
    #[must_use]
    pub fn draft_state(&self, id: DraftId) -> Option<(DraftStatus, Option<bool>)> {
        self.shared.lock().ok().and_then(|s| {
            s.drafts
                .iter()
                .find(|d| d.draft.id == id)
                .map(|d| (d.draft.status, d.corrected))
        })
    }
}

#[async_trait]
impl MaintenanceRepository for InMemoryStore {
    async fn expire_drafts(
        &self,
        cutoff: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> PortResult<Vec<BlobKey>> {
        let mut s = self.shared.lock().map_err(poisoned)?;
        Ok(s.drafts
            .iter_mut()
            .filter(|d| d.draft.status.is_open() && d.draft.created_at < cutoff)
            .map(|d| {
                d.draft.status = DraftStatus::Discarded;
                d.draft.photo_key.clone()
            })
            .collect())
    }

    async fn delete_expired_sessions(&self, now: DateTime<Utc>) -> PortResult<u64> {
        let mut s = self.shared.lock().map_err(poisoned)?;
        let before = s.sessions.len();
        s.sessions.retain(|e| *e >= now);
        Ok((before - s.sessions.len()) as u64)
    }

    async fn purge_outbox(&self, before: DateTime<Utc>) -> PortResult<u64> {
        let mut s = self.shared.lock().map_err(poisoned)?;
        let count = s.outbox.len();
        s.outbox.retain(|o| {
            !(o.status == OutboxStatus::Done && o.finished_at.is_some_and(|f| f < before))
        });
        Ok((count - s.outbox.len()) as u64)
    }

    async fn expire_documents(&self, cutoff: DateTime<Utc>) -> PortResult<Vec<BlobKey>> {
        let mut s = self.shared.lock().map_err(poisoned)?;
        Ok(documents_impl::expire(&mut s, cutoff))
    }
}

#[async_trait]
impl JobRepository for InMemoryStore {
    async fn claim_outbox(&self, now: DateTime<Utc>, limit: u32) -> PortResult<Vec<OutboxEntry>> {
        let s = self.shared.lock().map_err(poisoned)?;
        let mut due: Vec<&StoredOutbox> = s
            .outbox
            .iter()
            .filter(|o| o.status == OutboxStatus::Pending && o.next_attempt_at <= now)
            .collect();
        due.sort_by_key(|o| (o.entry.created_at, o.entry.id));
        Ok(due
            .into_iter()
            .take(limit as usize)
            .map(|o| o.entry.clone())
            .collect())
    }

    async fn finish_outbox(&self, id: OutboxId, result: OutboxResult) -> PortResult<()> {
        let mut s = self.shared.lock().map_err(poisoned)?;
        let o = s
            .outbox
            .iter_mut()
            .find(|o| o.entry.id == id)
            .ok_or(PortError::NotFound)?;
        match result {
            OutboxResult::Done { at } => {
                o.status = OutboxStatus::Done;
                o.finished_at = Some(at);
            }
            OutboxResult::Retry {
                attempts,
                next_attempt_at,
            } => {
                o.entry.attempts = attempts;
                o.next_attempt_at = next_attempt_at;
            }
            OutboxResult::Failed { attempts, at } => {
                o.entry.attempts = attempts;
                o.status = OutboxStatus::Failed;
                o.finished_at = Some(at);
            }
        }
        Ok(())
    }

    async fn pending_outbox(&self) -> PortResult<u64> {
        let s = self.shared.lock().map_err(poisoned)?;
        Ok(s.outbox
            .iter()
            .filter(|o| o.status == OutboxStatus::Pending)
            .count() as u64)
    }

    async fn record_run(&self, job: &str, run: &JobRun) -> PortResult<()> {
        let mut s = self.shared.lock().map_err(poisoned)?;
        s.job_runs.insert(job.to_owned(), run.clone());
        Ok(())
    }

    async fn last_run(&self, job: &str) -> PortResult<Option<JobRun>> {
        let s = self.shared.lock().map_err(poisoned)?;
        Ok(s.job_runs.get(job).cloned())
    }

    async fn households_with_schedules(&self) -> PortResult<Vec<HouseholdId>> {
        let s = self.shared.lock().map_err(poisoned)?;
        let mut out: Vec<HouseholdId> = s
            .meters
            .iter()
            .filter(|(_, m)| !m.archived && m.data.schedule.is_some())
            .map(|(h, _)| *h)
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    async fn recipients(
        &self,
        household: HouseholdId,
        kind: NotificationKind,
    ) -> PortResult<Vec<Recipient>> {
        let s = self.shared.lock().map_err(poisoned)?;
        let mut out: Vec<Recipient> = s
            .memberships
            .iter()
            .filter(|(h, _, role)| {
                *h == household && (kind != NotificationKind::Reminder || *role >= Role::Member)
            })
            .filter_map(|(_, u, _)| {
                let user = s.users.iter().find(|x| x.user.id == *u)?;
                let settings = s
                    .notification_settings
                    .get(u)
                    .map(|x| x.0.clone())
                    .unwrap_or_default();
                let wanted = match kind {
                    NotificationKind::Reminder => settings.reminder,
                    NotificationKind::Alarm => settings.alarm,
                    NotificationKind::Invitation => true,
                };
                let email = settings.email.or_else(|| user.user.email.clone())?;
                wanted.then(|| Recipient {
                    user_id: *u,
                    email,
                    locale: user.user.locale.clone(),
                })
            })
            .collect();
        out.sort_by_key(|r| r.user_id);
        Ok(out)
    }

    async fn log_once(&self, entry: &NotificationLogEntry) -> PortResult<bool> {
        let mut s = self.shared.lock().map_err(poisoned)?;
        let key = (
            entry.user_id,
            entry.kind.as_str(),
            entry.subject_key.clone(),
        );
        if s.notification_log.contains(&key) {
            return Ok(false);
        }
        s.notification_log.push(key);
        Ok(true)
    }

    async fn release_log(&self, entry: &NotificationLogEntry) -> PortResult<()> {
        let mut s = self.shared.lock().map_err(poisoned)?;
        s.notification_log.retain(|(u, k, key)| {
            !(*u == entry.user_id && *k == entry.kind.as_str() && *key == entry.subject_key)
        });
        Ok(())
    }
}

#[async_trait]
impl NotificationSettingsRepository for MemoryUow {
    async fn get(&mut self, user: UserId) -> PortResult<Option<NotificationSettings>> {
        Ok(self
            .state
            .notification_settings
            .get(&user)
            .map(|x| x.0.clone()))
    }

    async fn set(
        &mut self,
        user: UserId,
        settings: &NotificationSettings,
        now: DateTime<Utc>,
    ) -> PortResult<()> {
        self.state
            .notification_settings
            .insert(user, (settings.clone(), now));
        Ok(())
    }
}

/// Aufzeichnender Notifier für Tests; `fail` lässt jeden Versand scheitern.
#[derive(Debug, Default)]
pub struct RecordingNotifier {
    pub sent: Mutex<Vec<(String, Notification, String)>>,
    pub fail: std::sync::atomic::AtomicBool,
}

impl RecordingNotifier {
    /// Versendete Benachrichtigungen: (Adresse, Inhalt, Sprache).
    #[must_use]
    pub fn sent(&self) -> Vec<(String, Notification, String)> {
        self.sent.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl Notifier for RecordingNotifier {
    fn enabled(&self) -> bool {
        true
    }

    async fn send(&self, to: &str, notification: &Notification, locale: &str) -> PortResult<()> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(PortError::Backend("SMTP nicht erreichbar".into()));
        }
        self.sent.lock().map_err(poisoned)?.push((
            to.to_owned(),
            notification.clone(),
            locale.to_owned(),
        ));
        Ok(())
    }
}

impl InMemoryStore {
    /// Macht den Inhalt eines Outbox-Eintrags unlesbar (Fehlerpfad im Test).
    pub fn corrupt_outbox_payload(&self, id: OutboxId) {
        if let Ok(mut s) = self.shared.lock()
            && let Some(o) = s.outbox.iter_mut().find(|o| o.entry.id == id)
        {
            "{".clone_into(&mut o.entry.payload);
        }
    }

    /// Alle Outbox-Einträge in Reihenfolge der Erzeugung.
    #[must_use]
    pub fn outbox(&self) -> Vec<StoredOutbox> {
        self.shared
            .lock()
            .map(|s| s.outbox.clone())
            .unwrap_or_default()
    }
}

#[async_trait]
impl OutboxRepository for MemoryUow {
    async fn enqueue(
        &mut self,
        t: &Tenant,
        id: OutboxId,
        event: &OutboxEvent,
        now: DateTime<Utc>,
    ) -> PortResult<()> {
        let payload =
            serde_json::to_string(event).map_err(|e| PortError::Backend(e.to_string()))?;
        self.state.outbox.push(StoredOutbox {
            entry: OutboxEntry {
                id,
                household_id: t.household_id(),
                kind: event.kind().to_owned(),
                payload,
                attempts: 0,
                created_at: now,
            },
            status: OutboxStatus::Pending,
            next_attempt_at: now,
            finished_at: None,
        });
        Ok(())
    }
}

/// Objektspeicher im Speicher; Zeitstempel kommen von der übergebenen Uhr.
pub struct InMemoryBlobStore {
    objects: Mutex<BTreeMap<String, (Bytes, DateTime<Utc>)>>,
    clock: Arc<dyn Clock>,
}

impl InMemoryBlobStore {
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            objects: Mutex::default(),
            clock,
        }
    }

    /// Alle Schlüssel, sortiert.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.objects
            .lock()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    }
}

#[async_trait]
impl BlobStore for InMemoryBlobStore {
    async fn put(&self, key: &BlobKey, bytes: Bytes) -> PortResult<()> {
        let now = self.clock.now();
        self.objects
            .lock()
            .map_err(poisoned)?
            .insert(key.to_string(), (bytes, now));
        Ok(())
    }

    async fn get(&self, key: &BlobKey) -> PortResult<Bytes> {
        self.objects
            .lock()
            .map_err(poisoned)?
            .get(key.as_str())
            .map(|(b, _)| b.clone())
            .ok_or(PortError::NotFound)
    }

    async fn copy(&self, from: &BlobKey, to: &BlobKey) -> PortResult<()> {
        let bytes = self.get(from).await?;
        self.put(to, bytes).await
    }

    async fn delete(&self, key: &BlobKey) -> PortResult<()> {
        self.objects.lock().map_err(poisoned)?.remove(key.as_str());
        Ok(())
    }

    async fn delete_household(&self, household: HouseholdId) -> PortResult<()> {
        let prefix = BlobKey::household_prefix(household);
        self.objects
            .lock()
            .map_err(poisoned)?
            .retain(|k, _| !k.starts_with(&prefix));
        Ok(())
    }

    async fn list_tmp_older_than(&self, cutoff: DateTime<Utc>) -> PortResult<Vec<BlobKey>> {
        Ok(self
            .objects
            .lock()
            .map_err(poisoned)?
            .iter()
            .filter(|(_, (_, at))| *at < cutoff)
            .filter_map(|(k, _)| BlobKey::parse(k))
            .filter(|k| k.kind() == BlobKind::Tmp)
            .collect())
    }
}

#[async_trait]
impl DraftRepository for MemoryUow {
    async fn create(&mut self, t: &Tenant, draft: &Draft) -> PortResult<()> {
        if draft.photo_key.household() != t.household_id() {
            return Err(PortError::NotFound);
        }
        self.state.drafts.push(StoredDraft {
            household: t.household_id(),
            draft: draft.clone(),
            corrected: None,
        });
        Ok(())
    }

    async fn get_open(&mut self, t: &Tenant, id: DraftId) -> PortResult<Option<Draft>> {
        Ok(self
            .state
            .drafts
            .iter()
            .find(|d| {
                d.household == t.household_id() && d.draft.id == id && d.draft.status.is_open()
            })
            .map(|d| d.draft.clone()))
    }

    async fn list_open(&mut self, t: &Tenant) -> PortResult<Vec<Draft>> {
        let mut open: Vec<Draft> = self
            .state
            .drafts
            .iter()
            .filter(|d| d.household == t.household_id() && d.draft.status.is_open())
            .map(|d| d.draft.clone())
            .collect();
        open.sort_by_key(|d| (d.created_at, d.id));
        Ok(open)
    }

    async fn claim(
        &mut self,
        t: &Tenant,
        id: DraftId,
        now: DateTime<Utc>,
        stale_before: DateTime<Utc>,
    ) -> PortResult<bool> {
        let Some(d) = self
            .state
            .drafts
            .iter_mut()
            .find(|d| d.household == t.household_id() && d.draft.id == id)
        else {
            return Ok(false);
        };
        let claimable = d.draft.status == DraftStatus::Queued
            || (d.draft.status == DraftStatus::Analysing
                && d.draft.analysis_started_at.is_none_or(|s| s < stale_before));
        if claimable {
            d.draft.status = DraftStatus::Analysing;
            d.draft.analysis_started_at = Some(now);
        }
        Ok(claimable)
    }

    async fn record_analysis(
        &mut self,
        t: &Tenant,
        id: DraftId,
        a: &DraftAnalysis,
    ) -> PortResult<()> {
        let d = self
            .state
            .drafts
            .iter_mut()
            .find(|d| {
                d.household == t.household_id()
                    && d.draft.id == id
                    && d.draft.status == DraftStatus::Analysing
            })
            .ok_or(PortError::NotFound)?;
        d.draft.model.clone_from(&a.model);
        d.draft.duration_ms = a.duration_ms;
        d.draft.suggestion.clone_from(&a.suggestion);
        d.draft.matched_meter_id = a.matched_meter_id;
        d.draft.status = a.status;
        Ok(())
    }

    async fn finish(
        &mut self,
        t: &Tenant,
        id: DraftId,
        outcome: DraftOutcome,
        _at: DateTime<Utc>,
    ) -> PortResult<()> {
        let d = self
            .state
            .drafts
            .iter_mut()
            .find(|d| {
                d.household == t.household_id() && d.draft.id == id && d.draft.status.is_open()
            })
            .ok_or(PortError::NotFound)?;
        match outcome {
            DraftOutcome::Confirmed { corrected, .. } => {
                d.draft.status = DraftStatus::Confirmed;
                d.corrected = corrected;
            }
            DraftOutcome::Discarded => d.draft.status = DraftStatus::Discarded,
        }
        Ok(())
    }
}

/// Ersatz für die Erkennung mit festem Ergebnis.
pub struct StubRecognizer(pub Result<Recognition, RecognitionError>);

#[async_trait]
impl MeterRecognizer for StubRecognizer {
    fn model(&self) -> &'static str {
        "stub"
    }

    async fn recognize(&self, _: &NormalizedJpeg) -> Result<Recognition, RecognitionError> {
        self.0.clone()
    }
}

/// Ersatz für den Bild-Adapter: akzeptiert Daten mit JPEG-Signatur
/// unverändert (1 × 1 Pixel) mit der festgelegten Aufnahmezeit, alles andere
/// mit `photo.unsupported`.
#[derive(Debug, Clone, Copy, Default)]
pub struct StubImageNormalizer {
    pub taken_at: Option<PhotoTime>,
}

#[async_trait]
impl ImageNormalizer for StubImageNormalizer {
    async fn normalize(&self, bytes: Bytes) -> PortResult<NormalizedJpeg> {
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Ok(NormalizedJpeg {
                bytes,
                width: 1,
                height: 1,
                taken_at: self.taken_at,
            })
        } else {
            Err(PortError::Invalid("photo.unsupported"))
        }
    }
}

struct MemoryUow {
    shared: Arc<Mutex<State>>,
    state: State,
}

#[async_trait]
impl UnitOfWork for MemoryUow {
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
    fn vehicles(&mut self) -> &mut dyn crate::port::VehicleRepository {
        self
    }
    fn pv(&mut self) -> &mut dyn crate::port::PvRepository {
        self
    }
    fn asset_costs(&mut self) -> &mut dyn crate::port::AssetCostRepository {
        self
    }
    fn documents(&mut self) -> &mut dyn crate::port::DocumentRepository {
        self
    }
    async fn commit(self: Box<Self>) -> PortResult<()> {
        let mut shared = self.shared.lock().map_err(poisoned)?;
        // Einlösen wie das bedingte Update der Datenbank: eine parallel
        // eingelöste Einladung lässt den späteren Commit scheitern.
        for mine in &self.state.invitations {
            let accepted_elsewhere = shared.invitations.iter().any(|other| {
                other.invitation.id == mine.invitation.id
                    && other.accepted_at.is_some()
                    && other.accepted_by != mine.accepted_by
            });
            if mine.accepted_at.is_some() && accepted_elsewhere {
                return Err(PortError::Conflict("invitation".into()));
            }
        }
        *shared = self.state;
        Ok(())
    }
}

#[async_trait]
impl InvitationRepository for MemoryUow {
    async fn create(&mut self, t: &Tenant, i: &NewInvitation) -> PortResult<()> {
        if self
            .state
            .invitations
            .iter()
            .any(|x| x.invitation.token_hash == i.token_hash)
        {
            return Err(PortError::Conflict("token".into()));
        }
        self.state.invitations.push(StoredInvitation {
            invitation: i.clone(),
            household: t.household_id(),
            accepted_by: None,
            accepted_at: None,
            revoked_at: None,
        });
        Ok(())
    }

    async fn find_by_hash(&mut self, hash: &[u8; 32]) -> PortResult<Option<Invitation>> {
        Ok(self
            .state
            .invitations
            .iter()
            .find(|x| &x.invitation.token_hash == hash)
            .map(|x| self.state.invitation(x)))
    }

    async fn mark_accepted(
        &mut self,
        id: InvitationId,
        user: UserId,
        at: DateTime<Utc>,
    ) -> PortResult<bool> {
        let Some(x) =
            self.state.invitations.iter_mut().find(|x| {
                x.invitation.id == id && x.accepted_at.is_none() && x.revoked_at.is_none()
            })
        else {
            return Ok(false);
        };
        x.accepted_by = Some(user);
        x.accepted_at = Some(at);
        Ok(true)
    }

    async fn list_open(&mut self, t: &Tenant, now: DateTime<Utc>) -> PortResult<Vec<Invitation>> {
        let mut out: Vec<Invitation> = self
            .state
            .invitations
            .iter()
            .filter(|x| x.household == t.household_id())
            .map(|x| self.state.invitation(x))
            .filter(|i| i.is_open(now))
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        Ok(out)
    }

    async fn revoke(&mut self, t: &Tenant, id: InvitationId, at: DateTime<Utc>) -> PortResult<()> {
        let x = self
            .state
            .invitations
            .iter_mut()
            .find(|x| {
                x.household == t.household_id()
                    && x.invitation.id == id
                    && x.accepted_at.is_none()
                    && x.revoked_at.is_none()
            })
            .ok_or(PortError::NotFound)?;
        x.revoked_at = Some(at);
        Ok(())
    }
}

#[async_trait]
impl MembershipRepository for MemoryUow {
    async fn role(&mut self, user: UserId, household: HouseholdId) -> PortResult<Option<Role>> {
        Ok(self
            .state
            .memberships
            .iter()
            .find(|(h, u, _)| *h == household && *u == user)
            .map(|(_, _, r)| *r))
    }

    async fn households_of(&mut self, user: UserId) -> PortResult<Vec<HouseholdSummary>> {
        let mut out: Vec<HouseholdSummary> = self
            .state
            .memberships
            .iter()
            .filter(|(_, u, _)| *u == user)
            .filter_map(|(h, _, role)| {
                self.state
                    .households
                    .iter()
                    .find(|x| x.id == *h)
                    .map(|x| HouseholdSummary {
                        id: x.id,
                        name: x.data.name.clone(),
                        timezone: x.data.timezone,
                        role: *role,
                    })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<Member>> {
        let mut out: Vec<Member> = self
            .state
            .memberships
            .iter()
            .filter(|(h, _, _)| *h == t.household_id())
            .map(|(_, u, role)| Member {
                user_id: *u,
                display_name: self
                    .state
                    .users
                    .iter()
                    .find(|x| x.user.id == *u)
                    .map(|x| x.user.display_name.clone())
                    .unwrap_or_default(),
                role: *role,
            })
            .collect();
        out.sort_by(|a, b| {
            a.display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase())
                .then(a.user_id.cmp(&b.user_id))
        });
        Ok(out)
    }

    async fn lock_household(&mut self, _t: &Tenant) -> PortResult<()> {
        // Eine Unit of Work arbeitet auf einer eigenen Kopie
        Ok(())
    }

    async fn set_role(&mut self, t: &Tenant, user: UserId, role: Role) -> PortResult<()> {
        let m = self
            .state
            .memberships
            .iter_mut()
            .find(|(h, u, _)| *h == t.household_id() && *u == user)
            .ok_or(PortError::NotFound)?;
        m.2 = role;
        Ok(())
    }

    async fn remove(&mut self, t: &Tenant, user: UserId) -> PortResult<()> {
        let before = self.state.memberships.len();
        self.state
            .memberships
            .retain(|(h, u, _)| !(*h == t.household_id() && *u == user));
        if before == self.state.memberships.len() {
            return Err(PortError::NotFound);
        }
        Ok(())
    }

    async fn add(&mut self, household: HouseholdId, user: UserId, role: Role) -> PortResult<()> {
        if !self.state.households.iter().any(|h| h.id == household) {
            return Err(PortError::NotFound);
        }
        match self
            .state
            .memberships
            .iter_mut()
            .find(|(h, u, _)| *h == household && *u == user)
        {
            Some(m) => m.2 = m.2.max(role),
            None => self.state.memberships.push((household, user, role)),
        }
        Ok(())
    }
}

#[async_trait]
impl HouseholdRepository for MemoryUow {
    async fn create(
        &mut self,
        id: HouseholdId,
        owner: UserId,
        data: &HouseholdData,
        _now: DateTime<Utc>,
    ) -> PortResult<()> {
        self.state.households.push(Household {
            id,
            data: data.clone(),
        });
        self.state.memberships.push((id, owner, Role::Owner));
        Ok(())
    }

    async fn get(&mut self, t: &Tenant) -> PortResult<Household> {
        self.state
            .households
            .iter()
            .find(|h| h.id == t.household_id())
            .cloned()
            .ok_or(PortError::NotFound)
    }

    async fn update(&mut self, t: &Tenant, data: &HouseholdData) -> PortResult<()> {
        let h = self
            .state
            .households
            .iter_mut()
            .find(|h| h.id == t.household_id())
            .ok_or(PortError::NotFound)?;
        h.data = data.clone();
        Ok(())
    }

    async fn delete(&mut self, t: &Tenant) -> PortResult<()> {
        let hid = t.household_id();
        let meters: Vec<MeterId> = self
            .state
            .meters
            .iter()
            .filter(|(h, _)| *h == hid)
            .map(|(_, m)| m.id)
            .collect();
        let registers: Vec<RegisterId> = self
            .state
            .registers
            .iter()
            .filter(|(m, _)| meters.contains(m))
            .map(|(_, r)| r.id)
            .collect();
        self.state
            .readings
            .retain(|r| !registers.contains(&r.register_id));
        self.state.registers.retain(|(m, _)| !meters.contains(m));
        self.state.devices.retain(|(m, _)| !meters.contains(m));
        self.state
            .contracts
            .retain(|c| !meters.contains(&c.meter_id));
        self.state.meters.retain(|(h, _)| *h != hid);
        self.state.drafts.retain(|d| d.household != hid);
        self.state.documents.retain(|(h, _)| *h != hid);
        vehicles_impl::delete_household(&mut self.state, hid, &meters);
        pv_impl::delete_household(&mut self.state, hid);
        self.state.invitations.retain(|i| i.household != hid);
        self.state.revisions.retain(|(h, _)| *h != hid);
        self.state.outbox.retain(|o| o.entry.household_id != hid);
        self.state.memberships.retain(|(h, _, _)| *h != hid);
        self.state.households.retain(|h| h.id != hid);
        Ok(())
    }
}

#[async_trait]
impl UserRepository for MemoryUow {
    async fn upsert_oidc(
        &mut self,
        new_id: UserId,
        identity: &Identity,
        _now: DateTime<Utc>,
    ) -> PortResult<User> {
        if let Some(stored) = self
            .state
            .users
            .iter_mut()
            .find(|u| u.issuer == identity.issuer && u.subject == identity.subject)
        {
            stored.user.display_name.clone_from(&identity.display_name);
            stored.user.email.clone_from(&identity.email);
            return Ok(stored.user.clone());
        }
        let user = User {
            id: new_id,
            display_name: identity.display_name.clone(),
            email: identity.email.clone(),
            locale: identity.locale.clone(),
        };
        self.state.users.push(StoredUser {
            user: user.clone(),
            issuer: identity.issuer.clone(),
            subject: identity.subject.clone(),
        });
        Ok(user)
    }

    async fn get(&mut self, id: UserId) -> PortResult<Option<User>> {
        Ok(self
            .state
            .users
            .iter()
            .find(|u| u.user.id == id)
            .map(|u| u.user.clone()))
    }

    async fn set_locale(&mut self, id: UserId, locale: &str) -> PortResult<()> {
        let u = self
            .state
            .users
            .iter_mut()
            .find(|u| u.user.id == id)
            .ok_or(PortError::NotFound)?;
        locale.clone_into(&mut u.user.locale);
        Ok(())
    }
}

#[async_trait]
impl MeterRepository for MemoryUow {
    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<MeterSummary>> {
        let mut out: Vec<MeterSummary> = self
            .state
            .meters
            .iter()
            .filter(|(h, _)| *h == t.household_id())
            .map(|(_, m)| {
                let registers: Vec<RegisterId> = self
                    .state
                    .registers
                    .iter()
                    .filter(|(mid, _)| *mid == m.id)
                    .map(|(_, r)| r.id)
                    .collect();
                let last_reading_at = self
                    .state
                    .readings
                    .iter()
                    .filter(|r| registers.contains(&r.register_id))
                    .map(|r| r.read_at)
                    .max();
                MeterSummary {
                    meter: m.clone(),
                    last_reading_at,
                }
            })
            .collect();
        out.sort_by(|a, b| a.meter.data.name.cmp(&b.meter.data.name));
        Ok(out)
    }

    async fn get(&mut self, t: &Tenant, id: MeterId) -> PortResult<Option<MeterAggregate>> {
        let Some((_, meter)) = self
            .state
            .meters
            .iter()
            .find(|(h, m)| *h == t.household_id() && m.id == id)
        else {
            return Ok(None);
        };
        let mut devices: Vec<Device> = self
            .state
            .devices
            .iter()
            .filter(|(m, _)| *m == id)
            .map(|(_, d)| d.clone())
            .collect();
        devices.sort_by_key(|d| d.data.installed_at);
        let mut registers: Vec<Register> = self
            .state
            .registers
            .iter()
            .filter(|(m, _)| *m == id)
            .map(|(_, r)| r.clone())
            .collect();
        registers.sort_by(|a, b| a.data.code.cmp(&b.data.code));
        Ok(Some(MeterAggregate {
            meter: meter.clone(),
            devices,
            registers,
        }))
    }

    async fn create(
        &mut self,
        t: &Tenant,
        id: MeterId,
        data: &MeterData,
        now: DateTime<Utc>,
    ) -> PortResult<()> {
        self.state.meters.push((
            t.household_id(),
            Meter {
                id,
                data: data.clone(),
                archived: false,
                archived_at: None,
                created_at: now,
            },
        ));
        Ok(())
    }

    async fn update(&mut self, t: &Tenant, id: MeterId, data: &MeterData) -> PortResult<()> {
        let (_, m) = self
            .state
            .meters
            .iter_mut()
            .find(|(h, m)| *h == t.household_id() && m.id == id)
            .ok_or(PortError::NotFound)?;
        m.data = data.clone();
        Ok(())
    }

    async fn add_register(
        &mut self,
        t: &Tenant,
        meter: MeterId,
        id: RegisterId,
        data: &RegisterData,
    ) -> PortResult<()> {
        if !self.state.owns_meter(t, meter) {
            return Err(PortError::NotFound);
        }
        if self
            .state
            .registers
            .iter()
            .any(|(m, r)| *m == meter && r.data.code == data.code)
        {
            return Err(PortError::Conflict("register code".into()));
        }
        self.state.registers.push((
            meter,
            Register {
                id,
                data: data.clone(),
            },
        ));
        Ok(())
    }

    async fn rename_register(&mut self, t: &Tenant, id: RegisterId, code: &str) -> PortResult<()> {
        let meter = self
            .state
            .meter_of_register(id)
            .ok_or(PortError::NotFound)?;
        if !self.state.owns_meter(t, meter) {
            return Err(PortError::NotFound);
        }
        if let Some((_, r)) = self.state.registers.iter_mut().find(|(_, r)| r.id == id) {
            code.clone_into(&mut r.data.code);
        }
        Ok(())
    }

    async fn add_device(
        &mut self,
        t: &Tenant,
        meter: MeterId,
        id: DeviceId,
        data: &DeviceData,
    ) -> PortResult<()> {
        if !self.state.owns_meter(t, meter) {
            return Err(PortError::NotFound);
        }
        self.state.devices.push((
            meter,
            Device {
                id,
                data: data.clone(),
                removed_at: None,
            },
        ));
        Ok(())
    }

    async fn update_device(
        &mut self,
        t: &Tenant,
        id: DeviceId,
        data: &DeviceData,
    ) -> PortResult<()> {
        let meter = self.state.meter_of_device(id).ok_or(PortError::NotFound)?;
        if !self.state.owns_meter(t, meter) {
            return Err(PortError::NotFound);
        }
        if let Some((_, d)) = self.state.devices.iter_mut().find(|(_, d)| d.id == id) {
            d.data = data.clone();
        }
        Ok(())
    }

    async fn retire_device(
        &mut self,
        t: &Tenant,
        id: DeviceId,
        at: DateTime<Utc>,
    ) -> PortResult<()> {
        let meter = self.state.meter_of_device(id).ok_or(PortError::NotFound)?;
        if !self.state.owns_meter(t, meter) {
            return Err(PortError::NotFound);
        }
        if let Some((_, d)) = self.state.devices.iter_mut().find(|(_, d)| d.id == id) {
            d.removed_at = Some(at);
        }
        Ok(())
    }

    async fn find_by_serial(
        &mut self,
        t: &Tenant,
        serial: &str,
    ) -> PortResult<Option<(MeterId, DeviceId)>> {
        Ok(self
            .state
            .devices
            .iter()
            .filter(|(m, d)| self.state.owns_meter(t, *m) && d.removed_at.is_none())
            .find(|(_, d)| d.data.serial_number.as_deref() == Some(serial))
            .map(|(m, d)| (*m, d.id)))
    }

    async fn submeters_of(&mut self, t: &Tenant, parent: MeterId) -> PortResult<Vec<MeterId>> {
        let mut out: Vec<MeterId> = self
            .state
            .devices
            .iter()
            .filter(|(m, d)| d.data.parent_meter_id == Some(parent) && self.state.owns_meter(t, *m))
            .map(|(m, _)| *m)
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    async fn set_archived(
        &mut self,
        t: &Tenant,
        id: MeterId,
        at: Option<DateTime<Utc>>,
    ) -> PortResult<()> {
        let (_, m) = self
            .state
            .meters
            .iter_mut()
            .find(|(h, m)| *h == t.household_id() && m.id == id)
            .ok_or(PortError::NotFound)?;
        m.archived = at.is_some();
        m.archived_at = at;
        Ok(())
    }

    async fn find_by_name(&mut self, t: &Tenant, name: &str) -> PortResult<Option<MeterId>> {
        Ok(self
            .state
            .meters
            .iter()
            .find(|(h, m)| *h == t.household_id() && m.data.name == name)
            .map(|(_, m)| m.id))
    }
}

#[async_trait]
impl ReadingRepository for MemoryUow {
    async fn list_for_meter(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Vec<Reading>> {
        if !self.state.owns_meter(t, meter) {
            return Ok(Vec::new());
        }
        let mut out: Vec<Reading> = self
            .state
            .readings
            .iter()
            .filter(|r| self.state.meter_of_register(r.register_id) == Some(meter))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.read_at.cmp(&b.read_at).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    async fn insert(&mut self, t: &Tenant, readings: &[Reading]) -> PortResult<()> {
        for r in readings {
            let meter = self
                .state
                .meter_of_register(r.register_id)
                .ok_or(PortError::NotFound)?;
            if !self.state.owns_meter(t, meter)
                || self.state.meter_of_device(r.device_id) != Some(meter)
            {
                return Err(PortError::NotFound);
            }
            let duplicate = self.state.readings.iter().any(|x| {
                x.register_id == r.register_id
                    && x.device_id == r.device_id
                    && x.read_at == r.read_at
            });
            if duplicate {
                return Err(PortError::Conflict("reading".into()));
            }
            self.state.readings.push(r.clone());
        }
        Ok(())
    }

    async fn get(&mut self, t: &Tenant, id: ReadingId) -> PortResult<Option<(MeterId, Reading)>> {
        Ok(self
            .state
            .readings
            .iter()
            .find(|r| r.id == id)
            .and_then(|r| {
                let meter = self.state.meter_of_register(r.register_id)?;
                self.state.owns_meter(t, meter).then(|| (meter, r.clone()))
            }))
    }

    async fn delete(&mut self, t: &Tenant, id: ReadingId) -> PortResult<()> {
        let Some((_, _)) = ReadingRepository::get(self, t, id).await? else {
            return Err(PortError::NotFound);
        };
        self.state.readings.retain(|r| r.id != id);
        Ok(())
    }

    async fn update(&mut self, t: &Tenant, id: ReadingId, u: &ReadingUpdate) -> PortResult<()> {
        let Some((_, current)) = ReadingRepository::get(self, t, id).await? else {
            return Err(PortError::NotFound);
        };
        let duplicate = self.state.readings.iter().any(|x| {
            x.id != id
                && x.register_id == current.register_id
                && x.device_id == current.device_id
                && x.read_at == u.read_at
        });
        if duplicate {
            return Err(PortError::Conflict("reading".into()));
        }
        if let Some(r) = self.state.readings.iter_mut().find(|r| r.id == id) {
            r.read_at = u.read_at;
            r.value = u.value;
            r.note.clone_from(&u.note);
        }
        Ok(())
    }
}

#[async_trait]
impl RevisionRepository for MemoryUow {
    async fn append(&mut self, t: &Tenant, r: &Revision) -> PortResult<()> {
        if !self.state.owns_meter(t, r.meter_id)
            || self.state.meter_of_register(r.register_id) != Some(r.meter_id)
        {
            return Err(PortError::NotFound);
        }
        self.state.revisions.push((t.household_id(), r.clone()));
        Ok(())
    }

    async fn for_meter(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Vec<Revision>> {
        let mut out: Vec<Revision> = self
            .state
            .revisions
            .iter()
            .filter(|(h, r)| *h == t.household_id() && r.meter_id == meter)
            .map(|(_, r)| r.clone())
            .collect();
        out.sort_by(|a, b| b.changed_at.cmp(&a.changed_at).then(b.id.cmp(&a.id)));
        Ok(out)
    }
}

#[path = "memory_backup.rs"]
mod backup_impl;

#[path = "memory_contracts.rs"]
mod contracts_impl;

#[path = "memory_documents.rs"]
mod documents_impl;

#[path = "memory_vehicles.rs"]
mod vehicles_impl;

#[path = "memory_pv.rs"]
mod pv_impl;

pub use documents_impl::{StubExtractor, StubExtractors};
