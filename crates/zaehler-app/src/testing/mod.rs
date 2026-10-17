//! Test-Unterstützung (Feature `testing`): In-Memory-Adapter für alle
//! Driven Ports und eine Port-Testsuite, die jede Adapter-Implementierung
//! bestehen muss.

pub mod memory;
pub mod suite;

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use chrono::{DateTime, Duration, TimeZone, Utc};
use uuid::Uuid;

use crate::{
    port::{Clock, IdGenerator},
    usecase::Ports,
};

pub use memory::{
    InMemoryBlobStore, InMemoryStore, RecordingNotifier, StoredOutbox, StubExtractor,
    StubImageNormalizer, StubRecognizer,
};

/// Uhr mit fester, manuell verschiebbarer Zeit.
pub struct FixedClock(AtomicI64);

impl FixedClock {
    #[must_use]
    pub fn new(at: DateTime<Utc>) -> Self {
        Self(AtomicI64::new(at.timestamp()))
    }

    pub fn advance(&self, by: Duration) {
        self.0.fetch_add(by.num_seconds(), Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_opt(self.0.load(Ordering::SeqCst), 0)
            .single()
            .unwrap_or_default()
    }
}

/// Zufällige UUIDv7.
pub struct RandomIds;

impl IdGenerator for RandomIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::now_v7()
    }
}

/// Ports mit In-Memory-Speicher und fester Uhr (2026-01-15 12:00 UTC).
#[must_use]
pub fn memory_ports() -> (Ports, Arc<InMemoryStore>, Arc<FixedClock>) {
    let (ports, store, clock, _) = memory_ports_with_blobs();
    (ports, store, clock)
}

/// Wie [`memory_ports`], zusätzlich mit Zugriff auf den Objektspeicher.
#[must_use]
pub fn memory_ports_with_blobs() -> (
    Ports,
    Arc<InMemoryStore>,
    Arc<FixedClock>,
    Arc<InMemoryBlobStore>,
) {
    let store = Arc::new(InMemoryStore::default());
    let clock = Arc::new(FixedClock::new(
        Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0)
            .single()
            .unwrap_or_default(),
    ));
    let blobs = Arc::new(InMemoryBlobStore::new(clock.clone()));
    let ports = Ports {
        uow: store.clone(),
        clock: clock.clone(),
        ids: Arc::new(RandomIds),
        blobs: blobs.clone(),
        images: Arc::new(StubImageNormalizer::default()),
        recognizer: None,
        extractor: None,
        backup: store.clone(),
        jobs: store.clone(),
        notifier: Arc::new(crate::port::DisabledNotifier),
        settings: crate::usecase::Settings::default(),
    };
    (ports, store, clock, blobs)
}
