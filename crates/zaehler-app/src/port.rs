//! Driven Ports: Traits, die Adapter implementieren (ADR_022).
//!
//! Aufruf über `Arc<dyn …>` bzw. `Box<dyn UnitOfWork>`; alle Methoden sind
//! `async` über `async-trait`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    error::PortError,
    ids::{DeviceId, HouseholdId, MeterId, ReadingId, RegisterId, UserId},
    model::{
        DeviceData, Household, HouseholdData, HouseholdSummary, Identity, MeterAggregate,
        MeterData, MeterSummary, Reading, RegisterData, User,
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
