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
        BlobKey, BlobKind, DeviceId, DraftId, HouseholdId, MeterId, ReadingId, RegisterId, UserId,
    },
    model::{
        Device, DeviceData, Draft, DraftOutcome, DraftStatus, Household, HouseholdData,
        HouseholdSummary, Identity, Meter, MeterAggregate, MeterData, MeterSummary, Reading,
        Recognition, Register, RegisterData, User,
    },
    port::{
        BlobStore, Clock, DraftRepository, HouseholdRepository, ImageNormalizer,
        MaintenanceRepository, MembershipRepository, MeterRecognizer, MeterRepository,
        NormalizedJpeg, PortResult, Readiness, ReadingRepository, RecognitionError, UnitOfWork,
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
/// unverändert (1 × 1 Pixel), alles andere mit `photo.unsupported`.
pub struct StubImageNormalizer;

#[async_trait]
impl ImageNormalizer for StubImageNormalizer {
    async fn normalize(&self, bytes: Bytes) -> PortResult<NormalizedJpeg> {
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Ok(NormalizedJpeg {
                bytes,
                width: 1,
                height: 1,
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
    async fn commit(self: Box<Self>) -> PortResult<()> {
        *self.shared.lock().map_err(poisoned)? = self.state;
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
        self.state.meters.retain(|(h, _)| *h != hid);
        self.state.drafts.retain(|d| d.household != hid);
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
}
