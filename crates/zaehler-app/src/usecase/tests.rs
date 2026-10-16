//! Tests der Anwendungsfälle gegen den In-Memory-Adapter.

use chrono::{Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use zaehler_domain::{
    consumption,
    meter::{Direction, MeterKind},
    reading::{Plausibility, Reading as DomainReading, ReadingSource},
};

use std::sync::Arc;

use bytes::Bytes;

use crate::{
    AppError, Role, Tenant,
    ids::{BlobKey, MeterId, ReadingId},
    model::{DraftStatus, HouseholdData, Identity, Reading, Recognition},
    port::{Clock, MeterRecognizer, RecognitionError, SystemClock},
    testing::{
        InMemoryBlobStore, InMemoryStore, StubRecognizer, memory_ports, memory_ports_with_blobs,
        suite,
    },
    usecase::{
        UseCases,
        capture::normalize_serial,
        housekeeping::{Housekeeping, HousekeepingReport},
        meter::{MeterInput, RegisterInput, SwapInput},
        reading::{ReadingsInput, RecordOutcome},
    },
};

fn identity(subject: &str) -> Identity {
    Identity {
        issuer: "dev".into(),
        subject: subject.into(),
        email: None,
        display_name: "Anna".into(),
        locale: "de".into(),
    }
}

fn start() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap()
}

fn meter_input(name: &str) -> MeterInput {
    MeterInput {
        name: name.into(),
        kind: MeterKind::Electricity,
        unit: String::new(),
        reading_interval_days: Some(30),
        max_daily_rate: Some(dec!(50)),
        billing_start: None,
        serial_number: Some("1 emh-001".into()),
        rollover_digits: Some(5),
        decimal_places: 1,
        registers: vec![
            RegisterInput {
                id: None,
                code: "HT".into(),
                direction: Direction::Import,
                start_value: Some(dec!(1000)),
            },
            RegisterInput {
                id: None,
                code: "NT".into(),
                direction: Direction::Import,
                start_value: Some(dec!(500)),
            },
        ],
        start_at: start(),
    }
}

async fn setup() -> (UseCases, Tenant, std::sync::Arc<crate::testing::FixedClock>) {
    let (ports, _store, clock) = memory_ports();
    let uc = UseCases::new(&ports);
    let signed = uc.sign_in.execute(&identity("anna")).await.unwrap();
    let resolved = uc
        .resolve_tenant
        .execute(signed.user.id, None)
        .await
        .unwrap();
    (uc, resolved.tenant, clock)
}

// @test SPEC_ARCH_UOW, SPEC_SEC_TENANT_SCOPE
#[tokio::test]
async fn memory_adapter_passes_port_suite() {
    let (ports, _, _) = memory_ports();
    suite::run(ports.uow.as_ref()).await;
}

// @test SPEC_TEN_HOUSEHOLDS
#[tokio::test]
async fn first_sign_in_creates_one_household() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let first = uc.sign_in.execute(&identity("x")).await.unwrap();
    assert_eq!(first.households.len(), 1);
    assert_eq!(first.households[0].name, "Anna");
    assert_eq!(first.households[0].role, Role::Owner);
    let second = uc.sign_in.execute(&identity("x")).await.unwrap();
    assert_eq!(second.user.id, first.user.id);
    assert_eq!(second.households, first.households);
}

// @test SPEC_SEC_TENANT_SCOPE, SPEC_TEN_HOUSEHOLDS
#[tokio::test]
async fn resolve_tenant_prefers_member_household_only() {
    let (uc, tenant, _) = setup().await;
    let user = tenant.user_id();
    let other = uc
        .create_household
        .execute(
            user,
            HouseholdData {
                name: "Ferienwohnung".into(),
                timezone: chrono_tz::Europe::Berlin,
            },
        )
        .await
        .unwrap();
    let r = uc.resolve_tenant.execute(user, Some(other)).await.unwrap();
    assert_eq!(r.tenant.household_id(), other);
    assert_eq!(r.households.len(), 2);

    // Haushalt eines anderen Nutzers wird nicht gewählt
    let stranger = uc.sign_in.execute(&identity("bob")).await.unwrap();
    let r = uc
        .resolve_tenant
        .execute(user, Some(stranger.active))
        .await
        .unwrap();
    assert_ne!(r.tenant.household_id(), stranger.active);
    assert!(matches!(
        uc.resolve_tenant
            .execute(crate::ids::UserId(uuid::Uuid::now_v7()), None)
            .await,
        Err(AppError::Unauthenticated)
    ));
}

// @test SPEC_PRIV_HOUSEHOLD_DELETE
#[tokio::test]
async fn delete_household_requires_name_and_owner() {
    let (uc, tenant, _) = setup().await;
    uc.create_meter
        .execute(&tenant, meter_input("Strom"))
        .await
        .unwrap();
    assert!(matches!(
        uc.delete_household.execute(&tenant, "falsch").await,
        Err(AppError::Validation(_))
    ));
    let viewer = Tenant::new(tenant.user_id(), tenant.household_id(), Role::Viewer);
    assert!(matches!(
        uc.delete_household.execute(&viewer, "Anna").await,
        Err(AppError::Forbidden)
    ));
    uc.delete_household
        .execute(&tenant, " Anna ")
        .await
        .unwrap();
    // nächster Request legt einen neuen, leeren Haushalt an
    let r = uc
        .resolve_tenant
        .execute(tenant.user_id(), Some(tenant.household_id()))
        .await
        .unwrap();
    assert_ne!(r.tenant.household_id(), tenant.household_id());
    assert!(uc.list_meters.execute(&r.tenant).await.unwrap().is_empty());
}

// @test SPEC_MTR_MANAGE
#[tokio::test]
async fn create_meter_validates_and_enforces_uniqueness() {
    let (uc, tenant, _) = setup().await;
    let mut bad = meter_input("");
    bad.registers.clear();
    bad.decimal_places = 4;
    let Err(AppError::Validation(errors)) = uc.create_meter.execute(&tenant, bad).await else {
        panic!("Validierung erwartet");
    };
    let codes: Vec<_> = errors.0.iter().map(|e| e.code).collect();
    assert!(codes.contains(&"meter.name_invalid"));
    assert!(codes.contains(&"meter.registers_required"));
    assert!(codes.contains(&"meter.decimals_invalid"));

    let id = uc
        .create_meter
        .execute(&tenant, meter_input("Strom"))
        .await
        .unwrap();
    let detail = uc.get_meter.execute(&tenant, id).await.unwrap();
    assert_eq!(
        detail.aggregate.meter.data.unit, "kWh",
        "Standardeinheit aus der Art"
    );
    assert_eq!(detail.aggregate.registers.len(), 2);
    assert_eq!(detail.readings.len(), 2, "Startstände");
    assert_eq!(
        detail
            .aggregate
            .active_device()
            .unwrap()
            .data
            .serial_number
            .as_deref(),
        Some("1EMH001"),
        "Zählernummer normalisiert"
    );

    let mut same_serial = meter_input("Strom 2");
    same_serial.serial_number = Some("1EMH-001".into());
    let Err(AppError::Validation(errors)) = uc.create_meter.execute(&tenant, same_serial).await
    else {
        panic!("Konflikt Zählernummer erwartet");
    };
    assert_eq!(errors.0[0].code, "meter.serial_taken");
    let Err(AppError::Validation(errors)) =
        uc.create_meter.execute(&tenant, meter_input("Strom")).await
    else {
        panic!("Konflikt Name erwartet");
    };
    assert_eq!(errors.0[0].code, "meter.name_taken");

    let viewer = Tenant::new(tenant.user_id(), tenant.household_id(), Role::Member);
    assert!(matches!(
        uc.create_meter.execute(&viewer, meter_input("X")).await,
        Err(AppError::Forbidden)
    ));
}

// @test SPEC_MTR_MANAGE
#[tokio::test]
async fn update_meter_locks_kind_after_readings_and_keeps_registers() {
    let (uc, tenant, _) = setup().await;
    let id = uc
        .create_meter
        .execute(&tenant, meter_input("Strom"))
        .await
        .unwrap();
    let detail = uc.get_meter.execute(&tenant, id).await.unwrap();
    let mut input = meter_input("Hausstrom");
    input.registers = detail
        .aggregate
        .registers
        .iter()
        .map(|r| RegisterInput {
            id: Some(r.id),
            code: format!("{}1", r.data.code),
            direction: r.data.direction,
            start_value: None,
        })
        .collect();
    input.registers.push(RegisterInput {
        id: None,
        code: "Einspeisung".into(),
        direction: Direction::Export,
        start_value: None,
    });
    uc.update_meter
        .execute(&tenant, id, input.clone())
        .await
        .unwrap();
    let detail = uc.get_meter.execute(&tenant, id).await.unwrap();
    assert_eq!(detail.aggregate.meter.data.name, "Hausstrom");
    assert_eq!(detail.aggregate.registers.len(), 3);

    let mut changed_kind = input.clone();
    changed_kind.kind = MeterKind::Gas;
    changed_kind.registers.truncate(2);
    let Err(AppError::Validation(errors)) =
        uc.update_meter.execute(&tenant, id, changed_kind).await
    else {
        panic!("Validierung erwartet");
    };
    let codes: Vec<_> = errors.0.iter().map(|e| e.code).collect();
    assert!(codes.contains(&"meter.kind_locked"));
    assert!(
        codes.contains(&"meter.register_locked"),
        "Register können nicht entfernt werden"
    );
}

fn values(
    detail: &crate::usecase::meter::MeterDetail,
    v: &[Decimal],
) -> Vec<(crate::ids::RegisterId, Decimal)> {
    detail
        .aggregate
        .registers
        .iter()
        .zip(v)
        .map(|(r, x)| (r.id, *x))
        .collect()
}

// @test SPEC_RDG_FORM, SPEC_RDG_PLAUSIBILITY
#[tokio::test]
async fn record_readings_with_plausibility_and_confirmation() {
    let (uc, tenant, _) = setup().await;
    let id = uc
        .create_meter
        .execute(&tenant, meter_input("Strom"))
        .await
        .unwrap();
    let detail = uc.get_meter.execute(&tenant, id).await.unwrap();
    let at = start() + Duration::days(10);
    let ok = ReadingsInput {
        meter_id: id,
        at,
        values: values(&detail, &[dec!(1100), dec!(600)]),
        note: None,
    };
    let RecordOutcome::Saved(ids) = uc
        .record_readings
        .execute(&tenant, &ok, false, ReadingSource::Manual)
        .await
        .unwrap()
    else {
        panic!("gespeichert erwartet");
    };
    assert_eq!(ids.len(), 2);

    // gleicher Zeitpunkt erneut: Duplikat
    assert!(matches!(
        uc.record_readings
            .execute(&tenant, &ok, true, ReadingSource::Manual)
            .await,
        Err(AppError::Conflict("reading.duplicate"))
    ));

    // sinkender Wert: Bestätigung nötig, dann speicherbar
    let lower = ReadingsInput {
        at: at + Duration::days(1),
        values: values(&detail, &[dec!(1050)]),
        ..ok.clone()
    };
    let RecordOutcome::NeedsConfirmation(checks) = uc
        .record_readings
        .execute(&tenant, &lower, false, ReadingSource::Manual)
        .await
        .unwrap()
    else {
        panic!("Bestätigung erwartet");
    };
    assert_eq!(checks[0].plausibility, Plausibility::Decreasing);
    assert!(matches!(
        uc.record_readings
            .execute(&tenant, &lower, true, ReadingSource::Manual)
            .await
            .unwrap(),
        RecordOutcome::Saved(_)
    ));

    // zu hohe Tagesrate (max 50/Tag) wird gemeldet, ohne zu speichern
    let high = ReadingsInput {
        at: at + Duration::days(2),
        values: values(&detail, &[dec!(2000)]),
        ..ok.clone()
    };
    let checks = uc.check_readings.execute(&tenant, &high).await.unwrap();
    assert_eq!(checks[0].plausibility, Plausibility::RateTooHigh);

    // zu viele Nachkommastellen und Zukunft
    let precise = ReadingsInput {
        values: values(&detail, &[dec!(2000.25)]),
        ..high.clone()
    };
    assert!(matches!(
        uc.check_readings.execute(&tenant, &precise).await,
        Err(AppError::Validation(_))
    ));
    let future = ReadingsInput {
        at: Utc::now() + Duration::days(400),
        ..high.clone()
    };
    assert!(matches!(
        uc.record_readings
            .execute(&tenant, &future, true, ReadingSource::Manual)
            .await,
        Err(AppError::Validation(_))
    ));
}

// @test SPEC_MTR_SWAP_FLOW, SPEC_CONS_DEVICE_SWAP, SPEC_RDG_DELETE
#[tokio::test]
async fn device_swap_keeps_consumption_series_and_locks_swap_readings() {
    let (uc, tenant, clock) = setup().await;
    let mut input = meter_input("Wasser");
    input.registers.truncate(1);
    let id = uc.create_meter.execute(&tenant, input).await.unwrap();
    let detail = uc.get_meter.execute(&tenant, id).await.unwrap();
    let reg = detail.aggregate.registers[0].id;
    let swap_at = start() + Duration::days(10);
    let swap = SwapInput {
        at: swap_at,
        end_values: vec![(reg, dec!(1100))],
        serial_number: Some("NEU".into()),
        rollover_digits: None,
        decimal_places: 3,
        start_values: vec![(reg, dec!(0))],
    };
    uc.swap_device
        .execute(&tenant, id, swap.clone())
        .await
        .unwrap();
    clock.advance(Duration::days(30));
    let after = ReadingsInput {
        meter_id: id,
        at: swap_at + Duration::days(10),
        values: vec![(reg, dec!(50.125))],
        note: None,
    };
    uc.record_readings
        .execute(&tenant, &after, false, ReadingSource::Manual)
        .await
        .unwrap();

    let detail = uc.get_meter.execute(&tenant, id).await.unwrap();
    assert_eq!(detail.aggregate.devices.len(), 2);
    let domain: Vec<DomainReading> = detail
        .readings
        .iter()
        .map(|r| {
            let d = detail.aggregate.device(r.device_id).unwrap();
            DomainReading {
                at: r.read_at,
                value: r.value,
                device_id: d.id.0,
                rollover_digits: d.data.rollover_digits,
            }
        })
        .collect();
    let total: Decimal = consumption::intervals(&domain)
        .unwrap()
        .iter()
        .map(|i| i.amount)
        .sum();
    assert_eq!(total, dec!(150.125), "100 vor und 50,125 nach dem Wechsel");

    // Ablesungen des Wechsels sind gesperrt, die übrigen löschbar
    let end = detail
        .readings
        .iter()
        .find(|r| r.read_at == swap_at && r.value == dec!(1100))
        .unwrap();
    assert!(matches!(
        uc.delete_reading.execute(&tenant, end.id).await,
        Err(AppError::Conflict("reading.part_of_swap"))
    ));
    let last = detail.readings.last().unwrap();
    assert_eq!(
        uc.delete_reading.execute(&tenant, last.id).await.unwrap(),
        id
    );

    // Wechsel vor letzter Ablesung unzulässig
    assert!(matches!(
        uc.swap_device.execute(&tenant, id, swap).await,
        Err(AppError::Validation(_))
    ));
}

/// Messstelle mit einer Ablesung, deren Belegfoto im Objektspeicher liegt.
async fn reading_with_photo(
    ports: &crate::usecase::Ports,
    uc: &UseCases,
    tenant: &Tenant,
) -> (crate::ids::ReadingId, BlobKey) {
    let meter = uc
        .create_meter
        .execute(tenant, meter_input("Strom"))
        .await
        .unwrap();
    let detail = uc.get_meter.execute(tenant, meter).await.unwrap();
    let id = ReadingId(uuid::Uuid::now_v7());
    let key = BlobKey::reading_photo(tenant.household_id(), id);
    ports
        .blobs
        .put(&key, Bytes::from_static(b"\xFF\xD8\xFFjpeg"))
        .await
        .unwrap();
    let mut uow = ports.uow.begin().await.unwrap();
    uow.readings()
        .insert(
            tenant,
            &[Reading {
                id,
                register_id: detail.aggregate.registers[0].id,
                device_id: detail.aggregate.devices[0].id,
                read_at: start() + Duration::days(3),
                value: dec!(1010),
                source: ReadingSource::Manual,
                note: None,
                photo_key: Some(key.clone()),
                ai_confidence: None,
                created_by: Some(tenant.user_id()),
            }],
        )
        .await
        .unwrap();
    uow.commit().await.unwrap();
    (id, key)
}

async fn setup_with_blobs() -> (
    crate::usecase::Ports,
    UseCases,
    Tenant,
    Arc<InMemoryBlobStore>,
) {
    let (ports, _store, _clock, blobs) = memory_ports_with_blobs();
    let uc = UseCases::new(&ports);
    let signed = uc.sign_in.execute(&identity("anna")).await.unwrap();
    let tenant = uc
        .resolve_tenant
        .execute(signed.user.id, None)
        .await
        .unwrap()
        .tenant;
    (ports, uc, tenant, blobs)
}

// @test SPEC_STORE_BLOBS
#[tokio::test]
async fn memory_blob_store_passes_port_suite() {
    suite::blob_store(&InMemoryBlobStore::new(Arc::new(SystemClock))).await;
}

// @test SPEC_SEC_UPLOADS, SPEC_SEC_TENANT_SCOPE
#[tokio::test]
async fn reading_photo_only_for_own_household() {
    let (ports, uc, tenant, _) = setup_with_blobs().await;
    let (id, _) = reading_with_photo(&ports, &uc, &tenant).await;
    let photo = uc.get_reading_photo.execute(&tenant, id).await.unwrap();
    assert_eq!(photo, &b"\xFF\xD8\xFFjpeg"[..]);

    let other = uc.sign_in.execute(&identity("bob")).await.unwrap();
    let other = uc
        .resolve_tenant
        .execute(other.user.id, None)
        .await
        .unwrap()
        .tenant;
    assert!(matches!(
        uc.get_reading_photo.execute(&other, id).await,
        Err(AppError::NotFound)
    ));
    // Ablesung ohne Foto
    let detail = uc
        .get_meter
        .execute(
            &tenant,
            uc.list_meters.execute(&tenant).await.unwrap()[0].meter.id,
        )
        .await
        .unwrap();
    let without = detail
        .readings
        .iter()
        .find(|r| r.photo_key.is_none())
        .unwrap();
    assert!(matches!(
        uc.get_reading_photo.execute(&tenant, without.id).await,
        Err(AppError::NotFound)
    ));
}

// @test SPEC_RDG_DELETE, SPEC_STORE_BLOBS
#[tokio::test]
async fn deleting_a_reading_deletes_its_photo() {
    let (ports, uc, tenant, blobs) = setup_with_blobs().await;
    let (id, key) = reading_with_photo(&ports, &uc, &tenant).await;
    assert!(blobs.keys().contains(&key.to_string()));
    uc.delete_reading.execute(&tenant, id).await.unwrap();
    assert!(!blobs.keys().contains(&key.to_string()));
}

// @test SPEC_PRIV_HOUSEHOLD_DELETE, SPEC_STORE_BLOBS
#[tokio::test]
async fn deleting_a_household_deletes_its_objects() {
    let (ports, uc, tenant, blobs) = setup_with_blobs().await;
    reading_with_photo(&ports, &uc, &tenant).await;
    let foreign = BlobKey::tmp(
        crate::ids::HouseholdId(uuid::Uuid::now_v7()),
        uuid::Uuid::now_v7(),
    );
    ports.blobs.put(&foreign, Bytes::new()).await.unwrap();
    uc.delete_household.execute(&tenant, "Anna").await.unwrap();
    assert_eq!(blobs.keys(), vec![foreign.to_string()]);
}

// @test SPEC_OPS_HOUSEKEEPING
#[tokio::test]
async fn housekeeping_removes_old_tmp_and_sessions_idempotently() {
    let (ports, store, clock, blobs) = memory_ports_with_blobs();
    let h = crate::ids::HouseholdId(uuid::Uuid::now_v7());
    let old_tmp = BlobKey::tmp(h, uuid::Uuid::now_v7());
    let photo = BlobKey::reading_photo(h, ReadingId(uuid::Uuid::now_v7()));
    ports.blobs.put(&old_tmp, Bytes::new()).await.unwrap();
    ports.blobs.put(&photo, Bytes::new()).await.unwrap();
    clock.advance(Duration::hours(23));
    let new_tmp = BlobKey::tmp(h, uuid::Uuid::now_v7());
    ports.blobs.put(&new_tmp, Bytes::new()).await.unwrap();
    clock.advance(Duration::hours(2));
    store.add_session(clock.now() - Duration::minutes(1));
    store.add_session(clock.now() + Duration::hours(1));

    let hk = Housekeeping::new(
        store.clone(),
        ports.blobs.clone(),
        clock.clone(),
        Duration::hours(24),
    );
    let (a, b) = suite::join(hk.execute(), hk.execute()).await;
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(a.tmp + b.tmp >= 1 && a.sessions + b.sessions == 1);
    let mut keys = vec![new_tmp.to_string(), photo.to_string()];
    keys.sort();
    assert_eq!(blobs.keys(), keys);
    assert_eq!(store.session_count(), 1);
    // erneuter Lauf ändert nichts
    assert_eq!(hk.execute().await.unwrap(), HousekeepingReport::default());
}

const JPEG: &[u8] = b"\xFF\xD8\xFFfoto";

fn recognition(value: Decimal, serial: Option<&str>) -> Recognition {
    Recognition {
        value: Some(value),
        serial_number: serial.map(Into::into),
        register: Some("ht".into()),
        unit: Some("kWh".into()),
        meter_kind: Some(MeterKind::Electricity),
        confidence: dec!(0.9),
    }
}

struct Capture {
    uc: UseCases,
    tenant: Tenant,
    store: Arc<InMemoryStore>,
    blobs: Arc<InMemoryBlobStore>,
    clock: Arc<crate::testing::FixedClock>,
}

async fn capture_setup(result: Option<Result<Recognition, RecognitionError>>) -> Capture {
    let (mut ports, store, clock, blobs) = memory_ports_with_blobs();
    ports.recognizer = result.map(|r| Arc::new(StubRecognizer(r)) as Arc<dyn MeterRecognizer>);
    let uc = UseCases::new(&ports);
    let signed = uc.sign_in.execute(&identity("anna")).await.unwrap();
    let tenant = uc
        .resolve_tenant
        .execute(signed.user.id, None)
        .await
        .unwrap()
        .tenant;
    Capture {
        uc,
        tenant,
        store,
        blobs,
        clock,
    }
}

fn confirm_input(
    view: &crate::usecase::capture::DraftView,
    value: Decimal,
    at: chrono::DateTime<Utc>,
) -> ReadingsInput {
    let meter = view.meter.as_ref().unwrap();
    ReadingsInput {
        meter_id: meter.aggregate.meter.id,
        at,
        values: vec![(view.register.unwrap(), value)],
        note: None,
    }
}

// @test SPEC_AI_FLOW, SPEC_AI_DRAFTS, SPEC_RDG_PLAUSIBILITY
#[tokio::test]
async fn photo_capture_matches_by_serial_and_saves_after_confirmation() {
    let c = capture_setup(Some(Ok(recognition(dec!(1100), Some("1-EMH 001"))))).await;
    let meter =
        c.uc.create_meter
            .execute(&c.tenant, meter_input("Strom"))
            .await
            .unwrap();
    let mut other = meter_input("Zweitz\u{e4}hler");
    other.serial_number = Some("XYZ".into());
    c.uc.create_meter.execute(&c.tenant, other).await.unwrap();

    let draft =
        c.uc.analyse_photo
            .execute(&c.tenant, Bytes::from_static(JPEG))
            .await
            .unwrap();
    let draft_key = BlobKey::draft_photo(c.tenant.household_id(), draft);
    assert_eq!(c.blobs.keys(), vec![draft_key.to_string()]);
    assert_eq!(
        c.uc.get_draft_photo
            .execute(&c.tenant, draft)
            .await
            .unwrap(),
        JPEG
    );
    let view =
        c.uc.get_draft
            .execute(&c.tenant, draft, None)
            .await
            .unwrap();
    assert_eq!(view.draft.status, DraftStatus::Pending);
    assert_eq!(view.draft.model.as_deref(), Some("stub"));
    let detail = view.meter.as_ref().expect("zugeordnet");
    assert_eq!(detail.aggregate.meter.id, meter);
    let ht = detail
        .aggregate
        .registers
        .iter()
        .find(|r| r.data.code == "HT")
        .unwrap();
    assert_eq!(
        view.register,
        Some(ht.id),
        "Register \u{fc}ber erkannten Code"
    );
    assert_eq!(view.plausibility, Some(Plausibility::Ok));
    assert_eq!(view.meters.len(), 2);

    // sinkender Wert: Best\u{e4}tigung n\u{f6}tig, nichts gespeichert
    let now = c.clock.now();
    let lower = confirm_input(&view, dec!(900), now);
    assert!(matches!(
        c.uc.confirm_draft
            .execute(&c.tenant, draft, &lower, false)
            .await
            .unwrap(),
        RecordOutcome::NeedsConfirmation(_)
    ));
    assert_eq!(
        c.store.draft_state(draft),
        Some((DraftStatus::Pending, None))
    );

    let input = confirm_input(&view, dec!(1100), now);
    let RecordOutcome::Saved(ids) =
        c.uc.confirm_draft
            .execute(&c.tenant, draft, &input, false)
            .await
            .unwrap()
    else {
        panic!("gespeichert erwartet");
    };
    let reading =
        c.uc.get_meter
            .execute(&c.tenant, meter)
            .await
            .unwrap()
            .readings
            .into_iter()
            .find(|r| r.id == ids[0])
            .unwrap();
    assert_eq!(reading.source, ReadingSource::Photo);
    assert_eq!(reading.ai_confidence, Some(dec!(0.9)));
    let photo = BlobKey::reading_photo(c.tenant.household_id(), reading.id);
    assert_eq!(reading.photo_key, Some(photo.clone()));
    assert_eq!(
        c.blobs.keys(),
        vec![photo.to_string()],
        "Entwurfsfoto verschoben"
    );
    assert_eq!(
        c.store.draft_state(draft),
        Some((DraftStatus::Confirmed, Some(false)))
    );
    // abgeschlossener Entwurf ist nicht mehr erreichbar
    assert!(matches!(
        c.uc.confirm_draft
            .execute(&c.tenant, draft, &input, true)
            .await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        c.uc.get_draft_photo.execute(&c.tenant, draft).await,
        Err(AppError::NotFound)
    ));
}

// @test SPEC_AI_DRAFTS
#[tokio::test]
async fn corrected_value_is_recorded() {
    let c = capture_setup(Some(Ok(recognition(dec!(1100), Some("1EMH001"))))).await;
    c.uc.create_meter
        .execute(&c.tenant, meter_input("Strom"))
        .await
        .unwrap();
    let draft =
        c.uc.analyse_photo
            .execute(&c.tenant, Bytes::from_static(JPEG))
            .await
            .unwrap();
    let view =
        c.uc.get_draft
            .execute(&c.tenant, draft, None)
            .await
            .unwrap();
    let input = confirm_input(&view, dec!(1101.5), c.clock.now());
    c.uc.confirm_draft
        .execute(&c.tenant, draft, &input, false)
        .await
        .unwrap();
    assert_eq!(
        c.store.draft_state(draft),
        Some((DraftStatus::Confirmed, Some(true)))
    );
}

// @test SPEC_AI_DRAFTS
#[tokio::test]
async fn matching_by_kind_and_range_needs_a_unique_meter() {
    // ohne Z\u{e4}hlernummer: eine Messstelle passt (1100 \u{fc}ber 1000), eine nicht
    let c = capture_setup(Some(Ok(recognition(dec!(1100), None)))).await;
    let strom =
        c.uc.create_meter
            .execute(&c.tenant, meter_input("Strom"))
            .await
            .unwrap();
    let mut high = meter_input("Hoch");
    high.serial_number = None;
    for r in &mut high.registers {
        r.start_value = Some(dec!(50000));
    }
    c.uc.create_meter.execute(&c.tenant, high).await.unwrap();
    let draft =
        c.uc.analyse_photo
            .execute(&c.tenant, Bytes::from_static(JPEG))
            .await
            .unwrap();
    let view =
        c.uc.get_draft
            .execute(&c.tenant, draft, None)
            .await
            .unwrap();
    assert_eq!(view.draft.matched_meter_id, Some(strom));

    // zwei passende Messstellen: keine Zuordnung, Auswahl durch den Nutzer
    let mut second = meter_input("Strom 2");
    second.serial_number = None;
    c.uc.create_meter.execute(&c.tenant, second).await.unwrap();
    let draft =
        c.uc.analyse_photo
            .execute(&c.tenant, Bytes::from_static(JPEG))
            .await
            .unwrap();
    let view =
        c.uc.get_draft
            .execute(&c.tenant, draft, None)
            .await
            .unwrap();
    assert_eq!(view.draft.matched_meter_id, None);
    assert!(view.meter.is_none());
    let chosen =
        c.uc.get_draft
            .execute(&c.tenant, draft, Some(strom))
            .await
            .unwrap();
    assert_eq!(chosen.meter.unwrap().aggregate.meter.id, strom);
}

// @test SPEC_AI_DRAFTS, SPEC_IMG_PROCESSING
#[tokio::test]
async fn without_ai_or_on_errors_the_draft_is_open_for_manual_entry() {
    for result in [
        None,
        Some(Err(RecognitionError::Timeout)),
        Some(Err(RecognitionError::Invalid("kein JSON".into()))),
        Some(Err(RecognitionError::Failed("HTTP 500".into()))),
    ] {
        let enabled = result.is_some();
        let c = capture_setup(result).await;
        assert_eq!(c.uc.analyse_photo.ai_enabled(), enabled);
        let draft =
            c.uc.analyse_photo
                .execute(&c.tenant, Bytes::from_static(JPEG))
                .await
                .unwrap();
        let view =
            c.uc.get_draft
                .execute(&c.tenant, draft, None)
                .await
                .unwrap();
        assert_eq!(view.draft.status, DraftStatus::Failed);
        assert_eq!(view.draft.suggestion, None);
        assert_eq!(
            c.blobs.keys().len(),
            1,
            "Foto bleibt f\u{fc}r die manuelle Eingabe"
        );
    }
    let c = capture_setup(None).await;
    assert!(matches!(
        c.uc.analyse_photo
            .execute(&c.tenant, Bytes::from_static(b"kein Bild"))
            .await,
        Err(AppError::Validation(_))
    ));
    assert!(c.blobs.keys().is_empty());
}

// @test SPEC_AI_DRAFTS, SPEC_SEC_TENANT_SCOPE, SPEC_OPS_HOUSEKEEPING
#[tokio::test]
async fn drafts_can_be_discarded_and_expire() {
    let c = capture_setup(None).await;
    let draft =
        c.uc.analyse_photo
            .execute(&c.tenant, Bytes::from_static(JPEG))
            .await
            .unwrap();
    let bob = c.uc.sign_in.execute(&identity("bob")).await.unwrap();
    let bob =
        c.uc.resolve_tenant
            .execute(bob.user.id, None)
            .await
            .unwrap()
            .tenant;
    assert!(matches!(
        c.uc.get_draft.execute(&bob, draft, None).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        c.uc.discard_draft.execute(&bob, draft).await,
        Err(AppError::NotFound)
    ));
    c.uc.discard_draft.execute(&c.tenant, draft).await.unwrap();
    assert!(c.blobs.keys().is_empty());
    assert_eq!(
        c.store.draft_state(draft),
        Some((DraftStatus::Discarded, None))
    );

    // Ablauf nach 24 Stunden
    let old =
        c.uc.analyse_photo
            .execute(&c.tenant, Bytes::from_static(JPEG))
            .await
            .unwrap();
    c.clock.advance(Duration::hours(25));
    let hk = Housekeeping::new(
        c.store.clone(),
        c.blobs.clone(),
        c.clock.clone(),
        Duration::hours(24),
    );
    assert_eq!(hk.execute().await.unwrap().drafts, 1);
    assert!(c.blobs.keys().is_empty());
    assert_eq!(
        c.store.draft_state(old),
        Some((DraftStatus::Discarded, None))
    );
    assert_eq!(hk.execute().await.unwrap().drafts, 0);
}

// @test SPEC_AI_DRAFTS, SPEC_SEC_TENANT_SCOPE
#[tokio::test]
async fn memory_adapter_passes_draft_suite() {
    let (ports, store, _, _) = memory_ports_with_blobs();
    suite::drafts(ports.uow.as_ref(), store.as_ref()).await;
}

#[test]
fn serials_are_normalized() {
    assert_eq!(normalize_serial(" 1 emh-0012 "), "1EMH0012");
}

// @test SPEC_SEC_TENANT_SCOPE
#[tokio::test]
async fn foreign_meter_is_not_found() {
    let (uc, tenant, _) = setup().await;
    let id = uc
        .create_meter
        .execute(&tenant, meter_input("Strom"))
        .await
        .unwrap();
    let bob = uc.sign_in.execute(&identity("bob")).await.unwrap();
    let bob_tenant = uc
        .resolve_tenant
        .execute(bob.user.id, None)
        .await
        .unwrap()
        .tenant;
    assert!(matches!(
        uc.get_meter.execute(&bob_tenant, id).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        uc.get_meter
            .execute(&tenant, MeterId(uuid::Uuid::now_v7()))
            .await,
        Err(AppError::NotFound)
    ));
}

fn berlin_day(y: i32, m: u32, d: u32) -> chrono::DateTime<Utc> {
    zaehler_domain::calendar::Calendar::new(chrono_tz::Europe::Berlin)
        .start_of(chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap())
}

/// Messstelle mit HT/NT ab 01.01.2026 (lokal), Zählerwechsel am 01.03.,
/// Ablesungen zum Monatsbeginn bis 01.05.; Uhr am 10.05.2026.
async fn analysis_setup() -> (UseCases, Tenant, MeterId) {
    let (uc, tenant, clock) = setup().await;
    clock.advance(Duration::days(115));
    let mut input = meter_input("Strom");
    input.start_at = berlin_day(2026, 1, 1);
    let id = uc.create_meter.execute(&tenant, input).await.unwrap();
    let agg = uc.get_meter.execute(&tenant, id).await.unwrap().aggregate;
    let reg = |code: &str| {
        agg.registers
            .iter()
            .find(|r| r.data.code == code)
            .unwrap()
            .id
    };
    let (ht, nt) = (reg("HT"), reg("NT"));
    let record = |at, h: Decimal, n: Decimal| ReadingsInput {
        meter_id: id,
        at,
        values: vec![(ht, h), (nt, n)],
        note: None,
    };
    for input in [record(berlin_day(2026, 2, 1), dec!(1100), dec!(550))] {
        uc.record_readings
            .execute(&tenant, &input, false, ReadingSource::Manual)
            .await
            .unwrap();
    }
    uc.swap_device
        .execute(
            &tenant,
            id,
            SwapInput {
                at: berlin_day(2026, 3, 1),
                end_values: vec![(ht, dec!(1190)), (nt, dec!(600))],
                serial_number: Some("NEU-1".into()),
                rollover_digits: Some(5),
                decimal_places: 1,
                start_values: vec![(ht, dec!(0)), (nt, dec!(0))],
            },
        )
        .await
        .unwrap();
    for input in [
        record(berlin_day(2026, 4, 1), dec!(95), dec!(40)),
        record(berlin_day(2026, 5, 1), dec!(200), dec!(90)),
    ] {
        uc.record_readings
            .execute(&tenant, &input, false, ReadingSource::Manual)
            .await
            .unwrap();
    }
    (uc, tenant, id)
}

// @test SPEC_ANA_MONTHLY_SERIES, SPEC_ANA_REGISTER_AGG, SPEC_ANA_COMPARE_VIEW, SPEC_FC_CURRENT_YEAR, SPEC_CONS_DEVICE_SWAP
#[tokio::test]
async fn meter_analysis_sums_registers_across_a_device_swap() {
    use chrono::Datelike;
    use zaehler_domain::analysis::PeriodForecast;
    let (uc, tenant, id) = analysis_setup().await;
    let a = uc.meter_analysis.execute(&tenant, id, None).await.unwrap();
    assert_eq!(a.primary, Direction::Import);
    assert!(a.export.is_empty());
    let values: Vec<Option<Decimal>> = a.import.iter().map(|m| m.value).collect();
    assert_eq!(
        values,
        [
            Some(dec!(150)),
            Some(dec!(140)),
            Some(dec!(135)),
            Some(dec!(155)),
            None
        ]
    );
    assert_eq!(a.registers.len(), 2);
    assert_eq!(
        a.registers[0].months[2].value,
        Some(dec!(95)),
        "HT März über den Wechsel"
    );

    let april = a.month.unwrap();
    assert_eq!(
        april.start,
        chrono::NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
    );
    assert_eq!(a.complete_months.len(), 4);
    let prev = a.comparisons[0].comparison.as_ref().unwrap();
    assert_eq!(prev.abs_diff, dec!(20));
    assert!(a.comparisons[1].comparison.is_none(), "kein Vorjahr");
    assert!(a.comparisons[2].comparison.is_some(), "3 Monate");
    assert!(
        a.comparisons[3].comparison.is_none(),
        "6 Monate unvollständig"
    );
    let ytd = a.year_to_date.unwrap();
    assert_eq!((ytd.current, ytd.previous), (Some(dec!(580)), None));

    assert_eq!(a.forecasts.len(), 1, "kein Abrechnungsjahr hinterlegt");
    let PeriodForecast::Projected(f) = &a.forecasts[0].forecast else {
        panic!("Prognose erwartet");
    };
    assert_eq!((f.observed, f.projected_total), (dec!(580), dec!(1764.167)));

    let chart = &a.chart;
    assert_eq!(chart.year, 2026);
    assert_eq!(chart.current[1], Some(dec!(140)));
    assert!(chart.previous.iter().all(Option::is_none));
    assert_eq!(chart.cumulative[3], Some(dec!(580)));
    assert_eq!(chart.cumulative[4], None);
    assert_eq!(chart.forecast[2], None);
    assert_eq!(
        chart.forecast[3],
        Some(dec!(580)),
        "Anschluss an den Messwert"
    );
    assert_eq!(chart.forecast[11], Some(dec!(1764.167)));
    let json = uc.meter_chart_data.execute(&tenant, id).await.unwrap();
    assert_eq!(json.cumulative, chart.cumulative);

    // Bezugsmonat wählbar, nur vollständige Monate
    let feb = chrono::NaiveDate::from_ymd_opt(2026, 2, 10);
    let a = uc.meter_analysis.execute(&tenant, id, feb).await.unwrap();
    assert_eq!(a.month.unwrap().start.month(), 2);
    let may = chrono::NaiveDate::from_ymd_opt(2026, 5, 3);
    let a = uc.meter_analysis.execute(&tenant, id, may).await.unwrap();
    assert_eq!(a.month.unwrap().start.month(), 4);
}

// @test SPEC_UI_DASHBOARD
#[tokio::test]
async fn dashboard_lists_overdue_meters_first() {
    use chrono::Datelike;
    use zaehler_domain::analysis::PeriodForecast;
    let (uc, tenant, _) = analysis_setup().await;
    let mut water = meter_input("Wasser");
    water.kind = MeterKind::Water;
    water.serial_number = None;
    water.reading_interval_days = Some(7);
    water.registers.truncate(1);
    water.start_at = berlin_day(2026, 5, 1);
    uc.create_meter.execute(&tenant, water).await.unwrap();

    let cards = uc.dashboard.execute(&tenant).await.unwrap();
    let names: Vec<&str> = cards.iter().map(|c| c.meter.data.name.as_str()).collect();
    assert_eq!(names, ["Wasser", "Strom"]);
    assert!(cards[0].overdue && !cards[1].overdue);
    let strom = &cards[1];
    assert_eq!(strom.last_values.len(), 2);
    assert_eq!(strom.last_values[0].value, dec!(200));
    let last = strom.last_month.unwrap();
    assert_eq!((last.month.start.month(), last.value), (4, Some(dec!(155))));
    assert_eq!(strom.previous_month.as_ref().unwrap().reference, dec!(135));
    assert!(matches!(strom.forecast, PeriodForecast::Projected(_)));
    assert!(matches!(cards[0].forecast, PeriodForecast::NoReading));
}

fn import_mapping(meter: MeterId) -> crate::usecase::import::ImportMapping {
    use crate::usecase::import::{DateFormat, DecimalSeparator, ImportMapping};
    ImportMapping {
        meter_id: meter,
        has_header: true,
        date: 0,
        time: None,
        value: 1,
        register: None,
        serial: None,
        note: None,
        date_format: DateFormat::DayMonthYear,
        decimal: DecimalSeparator::Comma,
        default_register: None,
    }
}

/// Messstelle mit einem Register `HT`, Startstand 46 000 am 01.01.2026
/// (setzt die importierte Reihe mit 300 je Monat fort).
async fn import_setup() -> (UseCases, Tenant, MeterId) {
    let (uc, tenant, _) = setup().await;
    let mut input = meter_input("Strom");
    input.registers.truncate(1);
    input.registers[0].start_value = Some(dec!(46000));
    input.start_at = berlin_day(2026, 1, 1);
    let id = uc.create_meter.execute(&tenant, input).await.unwrap();
    (uc, tenant, id)
}

// @test SPEC_IMP_CSV, SPEC_IMP_CSV_FORMAT
#[tokio::test]
async fn ten_years_of_monthly_values_import_once_then_only_duplicates() {
    use std::fmt::Write as _;
    let (uc, tenant, meter) = import_setup().await;
    let mut csv = String::from("Datum;Zählerstand\n");
    for i in 0..120 {
        let (year, month) = (2016 + i / 12, i % 12 + 1);
        writeln!(csv, "01.{month:02}.{year};{},0", 10_000 + i * 300).unwrap();
    }
    let key = uc
        .upload_csv
        .execute(&tenant, Bytes::from(csv.clone()))
        .await
        .unwrap();
    let sheet = uc.preview_import.sheet(&tenant, &key).await.unwrap();
    assert_eq!((sheet.rows, sheet.columns), (120, 2));
    assert!(sheet.suggestion.has_header);
    let mapping = import_mapping(meter);
    let preview = uc
        .preview_import
        .execute(&tenant, &key, &mapping)
        .await
        .unwrap();
    assert_eq!((preview.ok, preview.errors, preview.warnings), (120, 0, 0));

    let report = uc
        .commit_import
        .execute(&tenant, &key, &mapping, false)
        .await
        .unwrap();
    assert_eq!(report.inserted, 120);
    let detail = uc.get_meter.execute(&tenant, meter).await.unwrap();
    assert_eq!(detail.readings.len(), 121);
    assert!(
        detail
            .readings
            .iter()
            .take(120)
            .all(|r| r.source == ReadingSource::CsvImport)
    );
    assert_eq!(
        detail.aggregate.devices[0].data.installed_at,
        berlin_day(2016, 1, 1),
        "Einbau vorverlegt"
    );
    // Datei nach dem Import gelöscht
    assert!(matches!(
        uc.preview_import.sheet(&tenant, &key).await,
        Err(AppError::NotFound)
    ));
    let analysis = uc
        .meter_analysis
        .execute(&tenant, meter, None)
        .await
        .unwrap();
    assert!(analysis.complete_months.len() >= 119);

    // derselbe Inhalt erneut: nur Duplikate
    let again = uc
        .upload_csv
        .execute(&tenant, Bytes::from(csv))
        .await
        .unwrap();
    let preview = uc
        .preview_import
        .execute(&tenant, &again, &mapping)
        .await
        .unwrap();
    assert_eq!((preview.ok, preview.duplicates), (0, 120));
    let report = uc
        .commit_import
        .execute(&tenant, &again, &mapping, false)
        .await
        .unwrap();
    assert_eq!((report.inserted, report.duplicates), (0, 120));
}

// @test SPEC_IMP_CSV, SPEC_IMP_CSV_FORMAT, SPEC_SEC_TENANT_SCOPE
#[tokio::test]
async fn import_reports_errors_and_saves_warnings_only_after_confirmation() {
    use crate::usecase::import::RowStatus;
    let (uc, tenant, meter) = import_setup().await;
    let csv = "Datum;Stand;Register\n\
               32.01.2026;1,0;HT\n\
               01.01.2026;abc;HT\n\
               01.12.2026;1,0;HT\n\
               02.01.2026;1,0;XX\n\
               03.01.2026;46000,25;HT\n\
               05.01.2026;100,0;ht\n";
    let mut mapping = import_mapping(meter);
    mapping.register = Some(2);
    let key = uc
        .upload_csv
        .execute(&tenant, Bytes::from_static(csv.as_bytes()))
        .await
        .unwrap();
    let preview = uc
        .preview_import
        .execute(&tenant, &key, &mapping)
        .await
        .unwrap();
    let codes: Vec<&str> = preview
        .rows
        .iter()
        .map(|r| match &r.status {
            RowStatus::Error(e) => e.code,
            RowStatus::Warning(_) => "warning",
            RowStatus::Ok => "ok",
            RowStatus::Duplicate => "duplicate",
        })
        .collect();
    assert_eq!(
        codes,
        [
            "import.date_invalid",
            "import.value_invalid",
            "reading.in_future",
            "import.register_unknown",
            "reading.too_many_decimals",
            "warning"
        ]
    );
    assert_eq!(preview.rows[5].line, 7);
    assert_eq!(
        preview.rows[5].status,
        RowStatus::Warning(Plausibility::Decreasing)
    );

    // fremder Haushalt sieht die Datei nicht
    let bob = uc.sign_in.execute(&identity("bob")).await.unwrap();
    let other = uc
        .resolve_tenant
        .execute(bob.user.id, None)
        .await
        .unwrap()
        .tenant;
    assert!(matches!(
        uc.preview_import.sheet(&other, &key).await,
        Err(AppError::NotFound)
    ));

    let report = uc
        .commit_import
        .execute(&tenant, &key, &mapping, false)
        .await
        .unwrap();
    assert_eq!(
        (report.inserted, report.warnings_skipped, report.errors),
        (0, 1, 5)
    );
    let key = uc
        .upload_csv
        .execute(&tenant, Bytes::from_static(csv.as_bytes()))
        .await
        .unwrap();
    let report = uc
        .commit_import
        .execute(&tenant, &key, &mapping, true)
        .await
        .unwrap();
    assert_eq!(report.inserted, 1, "Hinweis bestätigt");

    for bad in [&b"\xFF\xD8\xFFjpeg"[..], b"Datum;Stand\n"] {
        assert!(matches!(
            uc.upload_csv
                .execute(&tenant, Bytes::copy_from_slice(bad))
                .await,
            Err(AppError::Validation(_))
        ));
    }
}

// @test SPEC_IMG_CAPTURE_TIME
#[tokio::test]
async fn capture_time_from_photo_metadata_is_stored_with_the_draft() {
    use crate::{port::PhotoTime, testing::StubImageNormalizer};
    use chrono::FixedOffset;

    let local = chrono::NaiveDate::from_ymd_opt(2026, 1, 10)
        .unwrap()
        .and_hms_opt(8, 15, 0)
        .unwrap();
    let analyse = |taken_at: Option<PhotoTime>| async move {
        let (mut ports, _store, _clock, _blobs) = memory_ports_with_blobs();
        ports.images = Arc::new(StubImageNormalizer { taken_at });
        let uc = UseCases::new(&ports);
        let signed = uc.sign_in.execute(&identity("anna")).await.unwrap();
        let tenant = uc
            .resolve_tenant
            .execute(signed.user.id, None)
            .await
            .unwrap()
            .tenant;
        let id = uc
            .analyse_photo
            .execute(&tenant, Bytes::from_static(JPEG))
            .await
            .unwrap();
        uc.get_draft
            .execute(&tenant, id, None)
            .await
            .unwrap()
            .draft
            .taken_at
    };
    // ohne Offset: Zeitzone des Haushalts (Europe/Berlin, UTC+1)
    let berlin = analyse(Some(PhotoTime {
        local,
        offset: None,
    }))
    .await;
    assert_eq!(berlin.unwrap().to_rfc3339(), "2026-01-10T07:15:00+00:00");
    // mit Offset exakt
    let tokyo = analyse(Some(PhotoTime {
        local,
        offset: FixedOffset::east_opt(9 * 3600),
    }))
    .await;
    assert_eq!(tokyo.unwrap().to_rfc3339(), "2026-01-09T23:15:00+00:00");
    // Kamera-Uhr in der Zukunft (Uhr der Tests: 15.01.2026 12:00 UTC)
    let future = local + Duration::days(30);
    assert_eq!(
        analyse(Some(PhotoTime {
            local: future,
            offset: None
        }))
        .await,
        None
    );
    assert_eq!(analyse(None).await, None);
}

// @test SPEC_AI_NEW_METER
#[tokio::test]
async fn new_meter_from_photo_uses_the_draft_as_start_reading() {
    let c = capture_setup(Some(Ok(recognition(dec!(1100), Some("9 ABC 1"))))).await;
    let draft =
        c.uc.analyse_photo
            .execute(&c.tenant, Bytes::from_static(JPEG))
            .await
            .unwrap();
    let view =
        c.uc.get_draft
            .execute(&c.tenant, draft, None)
            .await
            .unwrap();
    assert!(view.meter.is_none(), "keine Messstelle vorhanden");

    let mut input = meter_input("Strom");
    input.serial_number = Some("9 ABC 1".into());
    input.registers.truncate(1);
    input.registers[0].start_value = None;
    let Err(AppError::Validation(errors)) =
        c.uc.create_meter
            .execute_from_draft(&c.tenant, input.clone(), draft)
            .await
    else {
        panic!("Startstand erforderlich");
    };
    assert_eq!(errors.0[0].code, "meter.photo_start_required");

    input.registers[0].start_value = Some(dec!(1100.5));
    let member = Tenant::new(c.tenant.user_id(), c.tenant.household_id(), Role::Member);
    assert!(matches!(
        c.uc.create_meter
            .execute_from_draft(&member, input.clone(), draft)
            .await,
        Err(AppError::Forbidden)
    ));

    let meter =
        c.uc.create_meter
            .execute_from_draft(&c.tenant, input.clone(), draft)
            .await
            .unwrap();
    let detail = c.uc.get_meter.execute(&c.tenant, meter).await.unwrap();
    assert_eq!(detail.readings.len(), 1);
    let start = &detail.readings[0];
    assert_eq!(start.source, ReadingSource::Photo);
    assert_eq!(start.value, dec!(1100.5));
    assert_eq!(start.ai_confidence, Some(dec!(0.9)));
    let photo = BlobKey::reading_photo(c.tenant.household_id(), start.id);
    assert_eq!(start.photo_key, Some(photo.clone()));
    assert_eq!(
        c.blobs.keys(),
        vec![photo.to_string()],
        "Entwurfsfoto verschoben"
    );
    assert_eq!(
        c.store.draft_state(draft),
        Some((DraftStatus::Confirmed, Some(true))),
        "Wert korrigiert"
    );

    // der Entwurf ist verbraucht
    input.name = "Strom 2".into();
    input.serial_number = None;
    assert!(matches!(
        c.uc.create_meter
            .execute_from_draft(&c.tenant, input, draft)
            .await,
        Err(AppError::NotFound)
    ));
}
