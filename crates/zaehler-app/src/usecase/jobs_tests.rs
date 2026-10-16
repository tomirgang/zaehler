//! Tests: Outbox und Jobs (SPEC_JOB_OUTBOX).

use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use rust_decimal_macros::dec;
use zaehler_domain::{
    meter::{Direction, MeterKind, MeterPurpose},
    reading::ReadingSource,
};

use crate::{
    Tenant,
    ids::{MeterId, OutboxId},
    model::{Identity, OutboxEvent, OutboxStatus},
    port::MaintenanceRepository,
    testing::{FixedClock, InMemoryStore, memory_ports, suite},
    usecase::{
        UseCases,
        import::{DateFormat, DecimalSeparator, ImportMapping},
        jobs::{DispatchOutbox, OUTBOX_MAX_ATTEMPTS, backoff},
        meter::{MeterInput, RegisterInput},
        reading::{CorrectionInput, ReadingsInput, RecordOutcome},
    },
};

// @test SPEC_JOB_OUTBOX, SPEC_JOB_SCHEDULER
#[tokio::test]
async fn memory_adapter_passes_job_suite() {
    let (ports, store, _) = memory_ports();
    suite::jobs(ports.uow.as_ref(), store.as_ref(), store.as_ref()).await;
}

type Setup = (
    UseCases,
    Tenant,
    MeterId,
    Arc<InMemoryStore>,
    crate::usecase::Ports,
    Arc<FixedClock>,
);

async fn setup() -> Setup {
    let (ports, store, clock) = memory_ports();
    let uc = UseCases::new(&ports);
    let identity = Identity {
        issuer: "dev".into(),
        subject: "anna".into(),
        email: None,
        display_name: "Anna".into(),
        locale: "de".into(),
    };
    let user = uc.sign_in.execute(&identity).await.unwrap().user.id;
    let t = uc.resolve_tenant.execute(user, None).await.unwrap().tenant;
    let start: DateTime<Utc> = ports.clock.now() - Duration::days(10);
    let meter = uc
        .create_meter
        .execute(
            &t,
            MeterInput {
                name: "Strom".into(),
                kind: MeterKind::Electricity,
                unit: String::new(),
                schedule: None,
                max_daily_rate: None,
                billing_start: None,
                purpose: MeterPurpose::General,
                targets: crate::model::MeterTargets::default(),
                parent_meter_id: None,
                serial_number: None,
                rollover_digits: None,
                decimal_places: 0,
                registers: vec![RegisterInput {
                    id: None,
                    code: "main".into(),
                    direction: Direction::Import,
                    start_value: Some(dec!(100)),
                }],
                start_at: start,
            },
        )
        .await
        .unwrap();
    (uc, t, meter, store, ports, clock)
}

/// Erfassung und Korrektur schreiben einen Eintrag, ein abgelehnter Vorgang
/// und der CSV-Import nicht.
// @test SPEC_JOB_OUTBOX
#[tokio::test]
async fn outbox_entries_only_from_successful_capture() {
    let (uc, t, meter, store, ports, _) = setup().await;
    assert!(store.outbox().is_empty(), "Startstände ohne Eintrag");
    let register = uc
        .get_meter
        .execute(&t, meter)
        .await
        .unwrap()
        .aggregate
        .registers[0]
        .id;
    let input = |days: i64, value| ReadingsInput {
        meter_id: meter,
        at: ports.clock.now() - Duration::days(days),
        values: vec![(register, value)],
        note: None,
    };
    let RecordOutcome::Saved(ids) = uc
        .record_readings
        .execute(&t, &input(5, dec!(150)), false, ReadingSource::Manual)
        .await
        .unwrap()
    else {
        panic!("gespeichert erwartet");
    };
    let entries = store.outbox();
    assert_eq!(entries.len(), 1);
    let event: OutboxEvent = serde_json::from_str(&entries[0].entry.payload).unwrap();
    assert_eq!(
        event,
        OutboxEvent::ReadingsRecorded {
            meter_id: meter,
            reading_ids: ids.clone(),
        }
    );
    // Warnung ohne Bestätigung, doppelter Zeitpunkt: kein Eintrag
    uc.record_readings
        .execute(&t, &input(4, dec!(50)), false, ReadingSource::Manual)
        .await
        .unwrap();
    assert!(
        uc.record_readings
            .execute(&t, &input(5, dec!(160)), true, ReadingSource::Manual)
            .await
            .is_err()
    );
    assert_eq!(store.outbox().len(), 1);
    // Korrektur schreibt einen weiteren
    uc.correct_reading
        .execute(
            &t,
            ids[0],
            &CorrectionInput {
                at: ports.clock.now() - Duration::days(5),
                value: dec!(151),
                note: None,
            },
            false,
        )
        .await
        .unwrap();
    assert_eq!(store.outbox().len(), 2);

    // CSV-Import: keine Alarme aus historischen Daten
    let key = uc
        .upload_csv
        .execute(&t, Bytes::from_static(b"2026-01-14;170\n2026-01-15;171\n"))
        .await
        .unwrap();
    let report = uc
        .commit_import
        .execute(
            &t,
            &key,
            &ImportMapping {
                meter_id: meter,
                has_header: false,
                date: 0,
                time: None,
                value: 1,
                register: None,
                serial: None,
                note: None,
                date_format: DateFormat::YearMonthDay,
                decimal: DecimalSeparator::Comma,
                default_register: None,
            },
            true,
        )
        .await
        .unwrap();
    assert!(report.inserted > 0);
    assert_eq!(store.outbox().len(), 2);
}

/// Ein nicht verarbeitbarer Eintrag wird mit Backoff wiederholt und nach
/// `OUTBOX_MAX_ATTEMPTS` Versuchen `failed`; gültige werden erledigt.
// @test SPEC_JOB_OUTBOX
#[tokio::test]
async fn dispatch_retries_with_backoff_and_gives_up() {
    let (uc, t, meter, store, ports, clock) = setup().await;
    let register = uc
        .get_meter
        .execute(&t, meter)
        .await
        .unwrap()
        .aggregate
        .registers[0]
        .id;
    uc.record_readings
        .execute(
            &t,
            &ReadingsInput {
                meter_id: meter,
                at: ports.clock.now() - Duration::days(1),
                values: vec![(register, dec!(150))],
                note: None,
            },
            false,
            ReadingSource::Manual,
        )
        .await
        .unwrap();
    // Eintrag, dessen Inhalt nicht lesbar ist
    let broken = OutboxId(uuid::Uuid::now_v7());
    {
        let mut uow = ports.uow.begin().await.unwrap();
        uow.outbox()
            .enqueue(
                &t,
                broken,
                &OutboxEvent::InvitationCreated {
                    invitation_id: crate::ids::InvitationId(uuid::Uuid::now_v7()),
                    token: "x".into(),
                },
                ports.clock.now(),
            )
            .await
            .unwrap();
        uow.commit().await.unwrap();
    }
    store.corrupt_outbox_payload(broken);

    let job = DispatchOutbox::new(ports.clone());
    let report = job.execute().await.unwrap();
    assert_eq!((report.done, report.retried, report.failed), (1, 1, 0));
    let status = |id| {
        store
            .outbox()
            .into_iter()
            .find(|o| o.entry.id == id)
            .unwrap()
    };
    let b = status(broken);
    assert_eq!(b.status, OutboxStatus::Pending);
    assert_eq!(b.entry.attempts, 1);
    assert_eq!(b.next_attempt_at, ports.clock.now() + backoff(1));
    // vor Ablauf des Backoffs nichts
    assert_eq!(job.execute().await.unwrap().retried, 0);
    for _ in 1..OUTBOX_MAX_ATTEMPTS {
        clock.advance(status(broken).next_attempt_at - ports.clock.now());
        job.execute().await.unwrap();
    }
    let b = status(broken);
    assert_eq!(b.status, OutboxStatus::Failed);
    assert_eq!(b.entry.attempts, OUTBOX_MAX_ATTEMPTS);
    // erledigte Einträge entfernt das Housekeeping nach 30 Tagen
    let removed = store
        .purge_outbox(ports.clock.now() + Duration::days(31))
        .await
        .unwrap();
    assert_eq!(removed, 1);
    assert_eq!(store.outbox().len(), 1, "fehlgeschlagener bleibt");
}
