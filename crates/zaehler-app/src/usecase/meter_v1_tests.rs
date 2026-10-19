//! Tests: Messstellen v1 (SPEC_RDG_SCHEDULE, SPEC_MTR_ARCHIVE,
//! SPEC_MTR_SUBMETER).

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use zaehler_domain::{
    calendar::Calendar,
    meter::{Direction, MeterKind, MeterPurpose},
    reading::ReadingSource,
    schedule::{Due, ReadingSchedule},
};

use crate::{
    AppError, Tenant,
    ids::MeterId,
    model::Identity,
    testing::{FixedClock, memory_ports},
    usecase::{
        UseCases,
        meter::{MeterInput, RegisterInput, SwapInput},
        reading::{ReadingsInput, RecordOutcome},
    },
};

fn berlin(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    Calendar::new(chrono_tz::Europe::Berlin)
        .start_of(chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap())
}

async fn setup() -> (UseCases, Tenant, std::sync::Arc<FixedClock>) {
    let (ports, _, clock) = memory_ports();
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
    (uc, t, clock)
}

fn input(name: &str, kind: MeterKind, unit: &str, start: DateTime<Utc>) -> MeterInput {
    MeterInput {
        name: name.into(),
        kind,
        unit: unit.into(),
        schedule: None,
        max_daily_rate: None,
        billing_start: None,
        purpose: MeterPurpose::General,
        targets: crate::model::MeterTargets::default(),
        parent_meter_id: None,
        serial_number: None,
        rollover_digits: None,
        decimal_places: 1,
        registers: vec![RegisterInput {
            id: None,
            code: "main".into(),
            direction: Direction::Import,
            start_value: Some(dec!(0)),
        }],
        start_at: start,
    }
}

async fn record(uc: &UseCases, t: &Tenant, meter: MeterId, at: DateTime<Utc>, value: Decimal) {
    let register = uc
        .get_meter
        .execute(t, meter)
        .await
        .unwrap()
        .aggregate
        .registers[0]
        .id;
    let out = uc
        .record_readings
        .execute(
            t,
            &ReadingsInput {
                meter_id: meter,
                at,
                values: vec![(register, value)],
                note: None,
            },
            true,
            ReadingSource::Manual,
        )
        .await
        .unwrap();
    assert!(matches!(out, RecordOutcome::Saved(_)));
}

// @test SPEC_MTR_ARCHIVE
#[tokio::test]
async fn archived_meters_leave_capture_and_dashboard_but_keep_their_analysis() {
    let (uc, t, _) = setup().await;
    let mut gas = input("Gas", MeterKind::Gas, "", berlin(2025, 12, 1));
    gas.schedule = Some(ReadingSchedule::EveryDays(7));
    let id = uc.create_meter.execute(&t, gas).await.unwrap();
    record(&uc, &t, id, berlin(2026, 1, 1), dec!(100)).await;
    uc.archive_meter.execute(&t, id).await.unwrap();

    let detail = uc.get_meter.execute(&t, id).await.unwrap();
    assert!(detail.aggregate.meter.archived);
    assert!(detail.aggregate.meter.archived_at.is_some());
    assert!(uc.dashboard.execute(&t).await.unwrap().is_empty());
    // neue Ablesungen werden abgelehnt
    let register = detail.aggregate.registers[0].id;
    let res = uc
        .record_readings
        .execute(
            &t,
            &ReadingsInput {
                meter_id: id,
                at: berlin(2026, 1, 10),
                values: vec![(register, dec!(110))],
                note: None,
            },
            true,
            ReadingSource::Manual,
        )
        .await;
    assert!(matches!(res, Err(AppError::Validation(e)) if !e.for_field("meter").is_empty()));
    // die Auswertung bleibt
    let analysis = uc.meter_analysis.execute(&t, id, None).await.unwrap();
    assert!(analysis.import.iter().any(|m| m.value.is_some()));

    uc.restore_meter.execute(&t, id).await.unwrap();
    let cards = uc.dashboard.execute(&t).await.unwrap();
    assert_eq!(cards.len(), 1);
    assert!(cards[0].overdue(), "nach 14 Tagen bei Plan alle 7 Tage");
}

// @test SPEC_MTR_DELETE
#[tokio::test]
async fn deleting_a_meter_removes_it_unless_a_submeter_refers_to_it() {
    let (uc, t, _) = setup().await;
    let grid = uc
        .create_meter
        .execute(
            &t,
            input("Strom", MeterKind::Electricity, "", berlin(2025, 12, 1)),
        )
        .await
        .unwrap();
    let mut heat_pump = input(
        "Wärmepumpe",
        MeterKind::Electricity,
        "",
        berlin(2025, 12, 1),
    );
    heat_pump.parent_meter_id = Some(grid);
    let sub = uc.create_meter.execute(&t, heat_pump).await.unwrap();
    record(&uc, &t, sub, berlin(2026, 1, 1), dec!(100)).await;

    assert!(matches!(
        uc.delete_meter.execute(&t, grid).await,
        Err(AppError::Conflict("meter.has_submeters"))
    ));
    uc.delete_meter.execute(&t, sub).await.unwrap();
    assert!(matches!(
        uc.get_meter.execute(&t, sub).await,
        Err(AppError::NotFound)
    ));
    uc.delete_meter.execute(&t, grid).await.unwrap();
    assert!(uc.list_meters.execute(&t).await.unwrap().is_empty());
    assert!(matches!(
        uc.delete_meter.execute(&t, grid).await,
        Err(AppError::NotFound)
    ));
}

// @test SPEC_RDG_SCHEDULE, SPEC_UI_DASHBOARD
#[tokio::test]
async fn dashboard_due_follows_the_schedule() {
    let (uc, t, clock) = setup().await;
    // Uhr: 15.01.2026 12:00 UTC
    let mut monthly = input("Strom", MeterKind::Electricity, "", berlin(2026, 1, 13));
    monthly.schedule = Some(ReadingSchedule::MonthlyOnDay(1));
    uc.create_meter.execute(&t, monthly).await.unwrap();
    let mut never = input("Wasser", MeterKind::Water, "", berlin(2026, 1, 13));
    never.registers[0].start_value = None;
    never.schedule = Some(ReadingSchedule::EveryDays(30));
    uc.create_meter.execute(&t, never).await.unwrap();
    uc.create_meter
        .execute(
            &t,
            input("Ohne Plan", MeterKind::Heat, "", berlin(2026, 1, 1)),
        )
        .await
        .unwrap();

    let cards = uc.dashboard.execute(&t).await.unwrap();
    let due: Vec<(&str, Option<Due>)> = cards
        .iter()
        .map(|c| (c.meter.data.name.as_str(), c.due))
        .collect();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
    assert_eq!(
        due,
        [
            ("Wasser", Some(Due::Due(today))),
            ("Ohne Plan", None),
            (
                "Strom",
                Some(Due::Upcoming(
                    chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()
                ))
            ),
        ],
        "ohne Ablesung sofort fällig, fällige vor den übrigen"
    );
    clock.advance(Duration::days(18));
    let cards = uc.dashboard.execute(&t).await.unwrap();
    assert_eq!(cards[0].meter.data.name, "Strom");
    assert!(cards[0].overdue());
}

/// Wärmepumpe zunächst als Zwischenzähler des Hausstroms, später per
/// Zählerwechsel mit eigenem Zähler; die Reihe der Wärmepumpe bleibt ohne
/// Bruch, der Hausstrom ohne Zwischenzähler zieht sie nur bis zum Wechsel ab.
// @test SPEC_MTR_SUBMETER
#[tokio::test]
async fn heat_pump_as_submeter_and_switch_to_own_meter() {
    let (uc, t, clock) = setup().await;
    clock.advance(Duration::days(120));
    let house = uc
        .create_meter
        .execute(
            &t,
            input("Hausstrom", MeterKind::Electricity, "", berlin(2026, 1, 1)),
        )
        .await
        .unwrap();
    let mut pump_input = input("Wärmepumpe", MeterKind::Electricity, "", berlin(2026, 1, 1));
    pump_input.purpose = MeterPurpose::HeatPump;
    pump_input.parent_meter_id = Some(house);
    let pump = uc.create_meter.execute(&t, pump_input).await.unwrap();

    // Ketten und falsche Zuordnungen
    let mut chain = input("Kette", MeterKind::Electricity, "", berlin(2026, 1, 1));
    chain.parent_meter_id = Some(pump);
    assert!(matches!(
        uc.create_meter.execute(&t, chain).await,
        Err(AppError::Validation(e)) if !e.for_field("parent_meter_id").is_empty()
    ));
    let mut water = input("Wasser", MeterKind::Water, "", berlin(2026, 1, 1));
    water.parent_meter_id = Some(house);
    water.purpose = MeterPurpose::HeatPump;
    match uc.create_meter.execute(&t, water).await {
        Err(AppError::Validation(e)) => {
            assert!(!e.for_field("parent_meter_id").is_empty());
            assert!(!e.for_field("purpose").is_empty());
        }
        other => panic!("{other:?}"),
    }
    // eine Messstelle mit Zwischenzählern wird nicht selbst zugeordnet
    let detail = uc.get_meter.execute(&t, house).await.unwrap();
    let mut own = input("Hausstrom", MeterKind::Electricity, "", berlin(2026, 1, 1));
    own.registers[0].id = Some(detail.aggregate.registers[0].id);
    own.parent_meter_id = Some(pump);
    assert!(uc.update_meter.execute(&t, house, own).await.is_err());

    for (m, house_value, pump_value) in [(2, 500, 200), (3, 900, 350)] {
        record(
            &uc,
            &t,
            house,
            berlin(2026, m, 1),
            Decimal::from(house_value),
        )
        .await;
        record(&uc, &t, pump, berlin(2026, m, 1), Decimal::from(pump_value)).await;
    }
    // Wechsel auf eigenen Zähler am 01.03.
    let register = uc
        .get_meter
        .execute(&t, pump)
        .await
        .unwrap()
        .aggregate
        .registers[0]
        .id;
    uc.swap_device
        .execute(
            &t,
            pump,
            SwapInput {
                at: berlin(2026, 3, 1) + Duration::hours(1),
                end_values: vec![(register, dec!(350))],
                serial_number: Some("WP-EIGEN".into()),
                rollover_digits: None,
                decimal_places: 1,
                start_values: vec![(register, dec!(0))],
                parent_meter_id: None,
            },
        )
        .await
        .unwrap();
    record(&uc, &t, house, berlin(2026, 4, 1), dec!(1300)).await;
    record(&uc, &t, pump, berlin(2026, 4, 1), dec!(100)).await;

    // Reihe der Wärmepumpe ohne Bruch
    let p = uc.meter_analysis.execute(&t, pump, None).await.unwrap();
    let pump_months: Vec<Option<Decimal>> = p.import.iter().take(3).map(|m| m.value).collect();
    assert_eq!(
        pump_months,
        [Some(dec!(200)), Some(dec!(150)), Some(dec!(100))]
    );

    let h = uc
        .meter_analysis
        .execute_with(&t, house, None, true)
        .await
        .unwrap();
    assert_eq!(h.submeters.len(), 1);
    assert!(h.excluding_submeters);
    let net: Vec<Option<Decimal>> = h.import_net.iter().take(3).map(|m| m.value).collect();
    assert_eq!(
        net,
        [Some(dec!(300)), Some(dec!(250)), Some(dec!(400))],
        "März ohne Abzug, die Wärmepumpe hat eigenen Zähler"
    );
    let with = uc.meter_analysis.execute(&t, house, None).await.unwrap();
    assert!(!with.excluding_submeters);
    assert_eq!(with.import[0].value, Some(dec!(500)));
}

// @test SPEC_MTR_SUBMETER, SPEC_MTR_KINDS
#[tokio::test]
async fn custom_meters_with_litres_and_kilograms() {
    let (uc, t, _) = setup().await;
    for (name, unit) in [("Heizöl", "l"), ("Pellets", "kg")] {
        let id = uc
            .create_meter
            .execute(&t, input(name, MeterKind::Custom, unit, berlin(2026, 1, 1)))
            .await
            .unwrap();
        let detail = uc.get_meter.execute(&t, id).await.unwrap();
        assert_eq!(detail.aggregate.meter.data.unit, unit);
        assert_eq!(detail.aggregate.meter.data.purpose, MeterPurpose::General);
    }
    // ohne Einheit nicht zulässig
    assert!(matches!(
        uc.create_meter
            .execute(&t, input("Holz", MeterKind::Custom, "", berlin(2026, 1, 1)))
            .await,
        Err(AppError::Validation(_))
    ));
}
