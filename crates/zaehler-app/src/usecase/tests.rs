//! Tests der Anwendungsfälle gegen den In-Memory-Adapter.

use chrono::{Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use zaehler_domain::{
    consumption,
    meter::{Direction, MeterKind},
    reading::{Plausibility, Reading as DomainReading, ReadingSource},
};

use crate::{
    AppError, Role, Tenant,
    ids::MeterId,
    model::{HouseholdData, Identity},
    testing::{memory_ports, suite},
    usecase::{
        UseCases,
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
