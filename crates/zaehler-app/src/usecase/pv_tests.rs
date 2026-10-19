//! Tests: PV-Anlagen, Sammelerfassung, Kennzahlen, Erwartung, Alarm und
//! Wirtschaftlichkeit (SPEC_PV_ASSIGNMENT, SPEC_PV_BULK_CAPTURE,
//! SPEC_PV_KPI, SPEC_PV_YIELD_EXPECTATION, SPEC_PV_MONITORING,
//! SPEC_PV_FEED_IN, SPEC_PV_ECONOMICS).

use std::sync::Arc;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use zaehler_domain::{
    assets::{AssetCategory, PvCostCategory},
    meter::{Direction, MeterKind},
    pv::{PvKind, PvRole, PvSystemData, YieldSource},
};

use super::contract_tests::{
    berlin, conflict, contract, day, field, flat, meter_input, position, record, setup,
};
use crate::{
    Tenant,
    ids::{MeterId, RegisterId},
    model::{AssetRef, Identity, Notification},
    testing::{RecordingNotifier, memory_ports},
    usecase::{
        UseCases,
        asset_cost::AssetCostInput,
        jobs::DispatchOutbox,
        pv::{BulkInput, BulkOutcome, PvSystemInput},
        reading::{ReadingsInput, RecordOutcome},
    },
};

fn data(kind: PvKind, commissioned: NaiveDate) -> PvSystemData {
    PvSystemData {
        name: "Dach".into(),
        kind,
        kwp: dec!(10),
        commissioned_on: commissioned,
        azimuth: Some(180),
        tilt: Some(30),
        inverter: None,
        battery_kwh: Some(dec!(10)),
        specific_yield: dec!(950),
        degradation_pct: dec!(0.5),
        month_shares: None,
        alarm_threshold_pct: 50,
    }
}

async fn electricity(
    uc: &UseCases,
    t: &Tenant,
    name: &str,
    registers: &[(&str, Direction)],
    start: chrono::DateTime<chrono::Utc>,
) -> MeterId {
    uc.create_meter
        .execute(
            t,
            meter_input(name, MeterKind::Electricity, registers, start),
        )
        .await
        .unwrap()
}

async fn registers(uc: &UseCases, t: &Tenant, meter: MeterId) -> Vec<RegisterId> {
    uc.get_meter
        .execute(t, meter)
        .await
        .unwrap()
        .aggregate
        .registers
        .iter()
        .map(|r| r.id)
        .collect()
}

/// Hausanschluss (Bezug, Einspeisung), Erzeugung und Speicher ab 2025-01-01.
async fn site(uc: &UseCases, t: &Tenant) -> (MeterId, MeterId, MeterId) {
    let start = berlin(2025, 1, 1);
    let grid = electricity(
        uc,
        t,
        "Hausanschluss",
        &[("1.8.0", Direction::Import), ("2.8.0", Direction::Export)],
        start,
    )
    .await;
    let generation = electricity(uc, t, "PV", &[("2.8.0", Direction::Export)], start).await;
    let battery = electricity(
        uc,
        t,
        "Speicher",
        &[
            ("charge", Direction::Import),
            ("discharge", Direction::Export),
        ],
        start,
    )
    .await;
    (grid, generation, battery)
}

// @test SPEC_PV_ASSIGNMENT
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn assignment_rules() {
    let (uc, t, _) = setup().await;
    let (grid, generation, battery) = site(&uc, &t).await;
    let input = |meters: Vec<(MeterId, PvRole)>, kind| PvSystemInput {
        data: data(kind, day(2025, 1, 1)),
        meters,
    };
    let first = uc
        .create_pv_system
        .execute(
            &t,
            input(
                vec![
                    (grid, PvRole::Grid),
                    (generation, PvRole::Generation),
                    (battery, PvRole::Battery),
                ],
                PvKind::Surplus,
            ),
        )
        .await
        .unwrap();
    // Erzeugungszähler gehört zu genau einer Anlage
    conflict(
        uc.create_pv_system
            .execute(
                &t,
                input(
                    vec![(grid, PvRole::Grid), (generation, PvRole::Generation)],
                    PvKind::Surplus,
                ),
            )
            .await,
        "pv.meter_assigned",
    );
    // zweite Anlage am selben Hausanschluss
    let second_gen = electricity(
        &uc,
        &t,
        "PV Garage",
        &[("2.8.0", Direction::Export)],
        berlin(2025, 1, 1),
    )
    .await;
    uc.create_pv_system
        .execute(
            &t,
            input(
                vec![(grid, PvRole::Grid), (second_gen, PvRole::Generation)],
                PvKind::Surplus,
            ),
        )
        .await
        .unwrap();
    assert_eq!(uc.list_pv_systems.execute(&t).await.unwrap().len(), 2);

    // ohne Einspeiseregister nur Steckersolargerät
    let house = electricity(
        &uc,
        &t,
        "Wohnung",
        &[("1.8.0", Direction::Import)],
        berlin(2025, 1, 1),
    )
    .await;
    let balcony = electricity(
        &uc,
        &t,
        "Balkon",
        &[("2.8.0", Direction::Export)],
        berlin(2025, 1, 1),
    )
    .await;
    let plug = |kind| {
        input(
            vec![(house, PvRole::Grid), (balcony, PvRole::Generation)],
            kind,
        )
    };
    field(
        uc.create_pv_system.execute(&t, plug(PvKind::Surplus)).await,
        "kind",
        "pv.export_required",
    );
    uc.create_pv_system
        .execute(&t, plug(PvKind::PlugIn))
        .await
        .unwrap();

    // Speicher ohne die Register charge/discharge, fehlende Erzeugung,
    // falsche Art
    let wrong = electricity(
        &uc,
        &t,
        "Akku",
        &[("main", Direction::Import)],
        berlin(2025, 1, 1),
    )
    .await;
    let water = uc
        .create_meter
        .execute(
            &t,
            meter_input(
                "Wasser",
                MeterKind::Water,
                &[("main", Direction::Import)],
                berlin(2025, 1, 1),
            ),
        )
        .await
        .unwrap();
    let mut bad = input(
        vec![
            (grid, PvRole::Grid),
            (wrong, PvRole::Battery),
            (water, PvRole::Generation),
        ],
        PvKind::Surplus,
    );
    let r = uc.create_pv_system.execute(&t, bad.clone()).await;
    field(r, "battery", "pv.battery_registers");
    field(
        uc.create_pv_system.execute(&t, bad.clone()).await,
        "generation",
        "pv.meter_kind_invalid",
    );
    bad.meters = vec![(grid, PvRole::Grid)];
    field(
        uc.create_pv_system.execute(&t, bad).await,
        "generation",
        "pv.generation_required",
    );
    // Löschen
    uc.delete_pv_system.execute(&t, first).await.unwrap();
    assert_eq!(uc.list_pv_systems.execute(&t).await.unwrap().len(), 2);
}

/// Bezug 200, Einspeisung 400, Erzeugung 1000, Ladung 300, Entladung 270 im
/// Januar 2025.
async fn january(uc: &UseCases, t: &Tenant, grid: MeterId, generation: MeterId, battery: MeterId) {
    let at = berlin(2025, 2, 1);
    let g = registers(uc, t, grid).await;
    let b = registers(uc, t, battery).await;
    let values = vec![
        (grid, g[0], dec!(200)),
        (grid, g[1], dec!(400)),
        (
            generation,
            registers(uc, t, generation).await[0],
            dec!(1000),
        ),
        (battery, b[0], dec!(300)),
        (battery, b[1], dec!(270)),
    ];
    let out = uc
        .record_bulk_readings
        .execute(
            t,
            grid,
            BulkInput {
                at,
                values,
                note: None,
                confirm_warnings: false,
                source: zaehler_domain::reading::ReadingSource::Manual,
                drafts: Vec::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(out, BulkOutcome::Saved(5));
}

async fn pv_site(uc: &UseCases, t: &Tenant) -> (MeterId, MeterId, MeterId, crate::ids::PvSystemId) {
    let (grid, generation, battery) = site(uc, t).await;
    let id = uc
        .create_pv_system
        .execute(
            t,
            PvSystemInput {
                data: data(PvKind::Surplus, day(2025, 1, 1)),
                meters: vec![
                    (grid, PvRole::Grid),
                    (generation, PvRole::Generation),
                    (battery, PvRole::Battery),
                ],
            },
        )
        .await
        .unwrap();
    (grid, generation, battery, id)
}

// @test SPEC_PV_BULK_CAPTURE, SPEC_PV_KPI, SPEC_PV_BATTERY
#[tokio::test]
async fn bulk_capture_and_grid_kpis() {
    let (uc, t, _) = setup().await;
    let (grid, generation, battery, _) = pv_site(&uc, &t).await;
    let meters = uc.record_bulk_readings.meters(&t, grid).await.unwrap();
    assert_eq!(meters.len(), 3);
    assert_eq!(meters[0].0.meter.id, grid);

    // Rückgang am Erzeugungszähler: Warnung, nichts gespeichert
    let g = registers(&uc, &t, generation).await[0];
    record(&uc, &t, generation, berlin(2025, 1, 10), dec!(50)).await;
    let out = uc
        .record_bulk_readings
        .execute(
            &t,
            grid,
            BulkInput {
                at: berlin(2025, 1, 20),
                values: vec![
                    (grid, registers(&uc, &t, grid).await[0], dec!(100)),
                    (generation, g, dec!(10)),
                ],
                note: None,
                confirm_warnings: false,
                source: zaehler_domain::reading::ReadingSource::Manual,
                drafts: Vec::new(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(out, BulkOutcome::NeedsConfirmation(ref w) if w[0].0 == generation));
    let grid_readings = uc.get_meter.execute(&t, grid).await.unwrap().readings;
    assert_eq!(grid_readings.len(), 2, "nur Startstände");

    january(&uc, &t, grid, generation, battery).await;
    // gemeinsamer Zeitpunkt
    for m in [grid, generation, battery] {
        let readings = uc.get_meter.execute(&t, m).await.unwrap().readings;
        assert_eq!(readings.last().unwrap().read_at, berlin(2025, 2, 1));
    }

    let view = uc.grid_analysis.execute(&t, grid).await.unwrap();
    let jan = &view.months[0];
    assert_eq!(jan.kpi.self_consumption, Some(dec!(570)));
    assert_eq!(jan.kpi.consumption, Some(dec!(770)));
    assert_eq!(jan.kpi.autarky, Some(dec!(0.7403)));
    let battery_kpi = jan.battery.unwrap();
    assert_eq!(battery_kpi.efficiency, Some(dec!(0.9)));
    assert_eq!(battery_kpi.full_cycles, Some(dec!(27)));
    let year = view.years.last().unwrap();
    assert!(year.partial);
    assert_eq!(year.kpi.consumption, Some(dec!(770)));
    let y = &view.yields[view.yields.len() - 1];
    assert_eq!(y.specific, Some(dec!(100)), "1000 kWh / 10 kWp");

    // Auswertung des Hausanschlusses auf den Gesamtverbrauch (REQ_PV_006)
    let a = uc.meter_analysis.execute(&t, grid, None).await.unwrap();
    assert!(a.uses_total);
    assert_eq!(a.total_consumption[0].value, Some(dec!(770)));
    assert_eq!(a.import[0].value, Some(dec!(200)));
}

// @test SPEC_PV_YIELD_EXPECTATION, SPEC_PV_FEED_IN, SPEC_PV_ECONOMICS
#[tokio::test]
async fn report_with_feed_in_savings_and_break_even() {
    let (uc, t, _) = setup().await;
    let (grid, generation, battery, id) = pv_site(&uc, &t).await;
    january(&uc, &t, grid, generation, battery).await;
    // Bezug 0,30 €/kWh; Einspeisung 0,08 €/kWh als negativer Arbeitspreis
    uc.create_contract
        .execute(
            &t,
            grid,
            contract(
                day(2025, 1, 1),
                vec![position(
                    "Strom",
                    None,
                    vec![flat(day(2025, 1, 1), dec!(0), dec!(0.30))],
                )],
            ),
        )
        .await
        .unwrap();
    let mut feed_in = contract(
        day(2025, 1, 1),
        vec![position(
            "Einspeisung",
            None,
            vec![flat(day(2025, 1, 1), dec!(0), dec!(-0.08))],
        )],
    );
    feed_in.direction = Direction::Export;
    uc.create_contract.execute(&t, grid, feed_in).await.unwrap();
    let invest = AssetCostInput {
        booked_on: day(2025, 1, 1),
        category: AssetCategory::Pv(PvCostCategory::Investment),
        amount: Some(dec!(10000)),
        quantity: None,
        unit_price: None,
        odometer: None,
        full_tank: false,
        recurrence: None,
        note: None,
        odometer_as_reading: false,
        confirm_warnings: false,
    };
    uc.add_asset_cost
        .execute(&t, AssetRef::pv(id), invest)
        .await
        .unwrap();

    let r = uc.pv_system_report.execute(&t, id).await.unwrap();
    let jan = r.months[0];
    assert!(!jan.projected);
    // Vergütung 400 · 0,08, Ersparnis 570 · 0,30
    assert_eq!(jan.economics.feed_in, dec!(32));
    assert_eq!(jan.economics.savings, dec!(171));
    assert_eq!(jan.economics.advantage, dec!(125.4));
    assert_eq!(jan.cumulative, dec!(203) - dec!(10000));
    assert!(r.months[1].projected, "danach fortgeschrieben");
    let (date, projected) = r.break_even.expect("Amortisation");
    assert!(projected && date > day(2026, 1, 1));
    assert_eq!(r.investments, dec!(10000));
    // Ertragserwartung im ersten Jahr: Startwert
    assert_eq!(r.yields[0].source, YieldSource::Start);
    assert_eq!(r.yields[0].expected, dec!(285));
    assert!(r.forecast.is_some());
    // Rendite: (203 + 10000 - 10000) · 12 / 10000
    assert_eq!(r.simple_return, Some(dec!(0.2436)));
}

/// Erzeugung weit unter der Erwartung: genau ein Alarm je Ablesung.
// @test SPEC_PV_MONITORING, SPEC_NTF_ALARMS
#[tokio::test]
async fn yield_alarm_once_per_reading() {
    let (mut ports, _, _) = memory_ports();
    let mail = Arc::new(RecordingNotifier::default());
    ports.notifier = mail.clone();
    let uc = UseCases::new(&ports);
    let user = uc
        .sign_in
        .execute(&Identity {
            issuer: "dev".into(),
            subject: "anna".into(),
            email: Some("anna@example.test".into()),
            display_name: "Anna".into(),
            locale: "de".into(),
        })
        .await
        .unwrap()
        .user
        .id;
    let t = uc.resolve_tenant.execute(user, None).await.unwrap().tenant;
    let start = berlin(2026, 1, 1);
    let grid = electricity(
        &uc,
        &t,
        "Hausanschluss",
        &[("1.8.0", Direction::Import), ("2.8.0", Direction::Export)],
        start,
    )
    .await;
    let generation = electricity(&uc, &t, "PV", &[("2.8.0", Direction::Export)], start).await;
    let system = uc
        .create_pv_system
        .execute(
            &t,
            PvSystemInput {
                data: data(PvKind::Surplus, day(2025, 1, 1)),
                meters: vec![(grid, PvRole::Grid), (generation, PvRole::Generation)],
            },
        )
        .await
        .unwrap();
    let dispatch = DispatchOutbox::new(ports.clone());
    dispatch.execute().await.unwrap();
    let g = registers(&uc, &t, generation).await[0];
    let read = |at, value: Decimal| ReadingsInput {
        meter_id: generation,
        at,
        values: vec![(g, value)],
        note: None,
    };
    // 10 Tage im Januar, Erwartung 10 · 950 · 3 % · 0,995 · 10/31 ≈ 92 kWh
    let out = uc
        .record_readings
        .execute(
            &t,
            &read(berlin(2026, 1, 11), dec!(5)),
            true,
            zaehler_domain::reading::ReadingSource::Manual,
        )
        .await
        .unwrap();
    assert!(matches!(out, RecordOutcome::Saved(_)));
    dispatch.execute().await.unwrap();
    dispatch.execute().await.unwrap();
    let sent = mail.sent();
    assert_eq!(sent.len(), 1, "genau ein Alarm");
    let Notification::PvYieldAlarm {
        generation: g_value,
        expected,
        link,
        ..
    } = &sent[0].1
    else {
        panic!("PV-Alarm erwartet");
    };
    assert_eq!(*g_value, dec!(5));
    assert!(*expected > dec!(90) && *expected < dec!(93), "{expected}");
    assert!(link.ends_with(&format!("/pv/{system}")));
    // Erzeugung im Rahmen: kein weiterer Alarm
    uc.record_readings
        .execute(
            &t,
            &read(berlin(2026, 1, 14), dec!(40)),
            true,
            zaehler_domain::reading::ReadingSource::Manual,
        )
        .await
        .unwrap();
    dispatch.execute().await.unwrap();
    assert_eq!(mail.sent().len(), 1);
}

/// Zwei Leistungsanteile mit Satzwechsel nach Ende der festen Vergütung und
/// eine bereits erreichte Amortisation (SPEC_PV_FEED_IN, SPEC_PV_ECONOMICS).
// @test SPEC_PV_FEED_IN, SPEC_PV_ECONOMICS
#[tokio::test]
async fn feed_in_shares_with_rate_change_and_past_break_even() {
    let (uc, t, _) = setup().await;
    let (grid, generation, battery, id) = pv_site(&uc, &t).await;
    january(&uc, &t, grid, generation, battery).await;
    // 10 von 15 kWp: Faktor 0,6667 und 0,3333; ab 16.01. einheitlich 0,04 €
    let share = |factor: Decimal, price: Decimal| {
        let mut p = position(
            "Einspeisung",
            None,
            vec![
                flat(day(2025, 1, 1), dec!(0), -price),
                flat(day(2025, 1, 16), dec!(0), dec!(-0.04)),
            ],
        );
        p.factor = factor;
        p
    };
    let mut feed_in = contract(
        day(2025, 1, 1),
        vec![
            share(dec!(0.6667), dec!(0.082)),
            share(dec!(0.3333), dec!(0.071)),
        ],
    );
    feed_in.direction = Direction::Export;
    feed_in.positions[1].name = "Einspeisung über 10 kWp".into();
    uc.create_contract.execute(&t, grid, feed_in).await.unwrap();
    uc.add_asset_cost
        .execute(
            &t,
            AssetRef::pv(id),
            AssetCostInput {
                booked_on: day(2025, 1, 1),
                category: AssetCategory::Pv(PvCostCategory::Investment),
                amount: Some(dec!(10)),
                quantity: None,
                unit_price: None,
                odometer: None,
                full_tank: false,
                recurrence: None,
                note: None,
                odometer_as_reading: false,
                confirm_warnings: false,
            },
        )
        .await
        .unwrap();
    let r = uc.pv_system_report.execute(&t, id).await.unwrap();
    // 400 kWh gleichmäßig: 15 Tage zum Mischsatz, 16 Tage zu 0,04 €
    let mixed = dec!(0.6667) * dec!(0.082) + dec!(0.3333) * dec!(0.071);
    let expected = dec!(400) * (dec!(15) * mixed + dec!(16) * dec!(0.04)) / dec!(31);
    let jan = r.months[0];
    assert!(
        (jan.economics.feed_in - expected).abs() < dec!(0.02),
        "{}",
        jan.economics.feed_in
    );
    // ohne Bezugsvertrag keine Ersparnis; Investition 10 € im Januar gedeckt
    assert_eq!(jan.economics.savings, Decimal::ZERO);
    let (date, projected) = r.break_even.expect("Amortisation");
    assert!(!projected, "in der Vergangenheit");
    assert!(date >= day(2025, 1, 1) && date < day(2025, 2, 1), "{date}");
}

/// Heimladen über die Wallbox hinter einem Hausanschluss mit PV: Mischpreis
/// aus Einspeisevergütung und Arbeitspreis, als Schätzung gekennzeichnet
/// (REQ_CAR_011).
// @test SPEC_CAR_ENERGY_COST, SPEC_PV_FEED_IN
#[tokio::test]
async fn charging_price_with_pv_is_estimated() {
    use zaehler_domain::{meter::MeterPurpose, vehicle::DriveType};

    use crate::usecase::vehicle::{NewVehicle, VehicleInput};

    let (uc, t, _) = setup().await;
    let (grid, generation, battery, _) = pv_site(&uc, &t).await;
    january(&uc, &t, grid, generation, battery).await;
    uc.create_contract
        .execute(
            &t,
            grid,
            contract(
                day(2025, 1, 1),
                vec![position(
                    "Strom",
                    None,
                    vec![flat(day(2025, 1, 1), dec!(0), dec!(0.30))],
                )],
            ),
        )
        .await
        .unwrap();
    let mut wb = meter_input(
        "Wallbox",
        MeterKind::Electricity,
        &[("1.8.0", Direction::Import)],
        berlin(2025, 1, 1),
    );
    wb.purpose = MeterPurpose::Wallbox;
    wb.parent_meter_id = Some(grid);
    let wallbox = uc.create_meter.execute(&t, wb).await.unwrap();
    record(&uc, &t, wallbox, berlin(2025, 2, 1), dec!(100)).await;
    let car = uc
        .create_vehicle
        .execute(
            &t,
            NewVehicle {
                name: "Stromer".into(),
                vehicle: VehicleInput {
                    plate: String::new(),
                    model: String::new(),
                    drive_type: DriveType::Electric,
                    home_charging_meter_id: Some(wallbox),
                    // bekannter PV-Anteil aus der Wallbox: 50 %
                    pv_share_pct: Some(dec!(50)),
                },
                odometer: dec!(0),
                start_at: berlin(2025, 1, 1),
            },
        )
        .await
        .unwrap();
    record(&uc, &t, car, berlin(2025, 2, 1), dec!(600)).await;
    let r = uc.vehicle_report.execute(&t, car).await.unwrap();
    let e = r
        .years
        .iter()
        .find(|y| y.year == Some(2025))
        .and_then(|y| y.energy.clone())
        .unwrap();
    let kwh = e.home_kwh.unwrap();
    let cost = e.home_cost.unwrap();
    assert!(cost.estimated, "Mischpreis ist eine Schätzung");
    // ohne Einspeisevertrag v_ein = 0: halber Arbeitspreis
    assert_eq!(cost.value, (kwh.value * dec!(0.15)).round_dp(2));
}

/// Fotos in der Sammelerfassung: Entwurf je Messstelle, Belegfoto je
/// Ablesung, Entwurf abgeschlossen, Alarmprüfung über die Outbox.
// @test SPEC_PV_BULK_CAPTURE
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn bulk_capture_with_photos() {
    use bytes::Bytes;
    use zaehler_domain::reading::ReadingSource;

    use crate::{
        model::{OutboxEvent, Recognition},
        testing::{StubRecognizer, memory_ports_with_blobs},
    };

    let (mut ports, store, _, blobs) = memory_ports_with_blobs();
    ports.recognizer = Some(Arc::new(StubRecognizer(Ok(Recognition {
        value: Some(dec!(1000)),
        serial_number: None,
        register: Some("2.8.0".into()),
        unit: Some("kWh".into()),
        meter_kind: Some(MeterKind::Electricity),
        confidence: dec!(0.9),
    }))));
    let uc = UseCases::new(&ports);
    let signed = uc
        .sign_in
        .execute(&Identity {
            issuer: "dev".into(),
            subject: "anna".into(),
            email: None,
            display_name: "Anna".into(),
            locale: "de".into(),
        })
        .await
        .unwrap();
    let t = uc
        .resolve_tenant
        .execute(signed.user.id, None)
        .await
        .unwrap()
        .tenant;
    let (grid, generation, _, _) = pv_site(&uc, &t).await;
    let jpeg = Bytes::from_static(b"\xFF\xD8\xFFfoto");
    let draft = uc.analyse_photo.execute(&t, jpeg.clone()).await.unwrap();
    let listed = uc.record_bulk_readings.drafts(&t, grid).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].draft.id, draft);
    assert!(
        listed[0].meter.is_none(),
        "ohne Zählernummer keine Zuordnung"
    );

    let g = registers(&uc, &t, grid).await;
    let gen_register = registers(&uc, &t, generation).await[0];
    let other = electricity(
        &uc,
        &t,
        "Garage",
        &[("1.8.0", Direction::Import)],
        berlin(2025, 1, 1),
    )
    .await;
    let input = |drafts: Vec<(crate::ids::DraftId, MeterId)>| BulkInput {
        at: berlin(2025, 2, 1),
        values: vec![
            (grid, g[0], dec!(200)),
            (grid, g[1], dec!(400)),
            (generation, gen_register, dec!(1000)),
        ],
        note: None,
        confirm_warnings: false,
        source: zaehler_domain::reading::ReadingSource::Manual,
        drafts,
    };
    // Entwurf für eine Messstelle außerhalb des Hausanschlusses
    assert!(matches!(
        uc.record_bulk_readings
            .execute(&t, grid, input(vec![(draft, other)]))
            .await,
        Err(crate::AppError::NotFound)
    ));
    let before = store.outbox().len();
    let out = uc
        .record_bulk_readings
        .execute(&t, grid, input(vec![(draft, generation)]))
        .await
        .unwrap();
    assert_eq!(out, BulkOutcome::Saved(3));

    let readings = uc.get_meter.execute(&t, generation).await.unwrap().readings;
    let photo = readings
        .iter()
        .find(|r| r.read_at == berlin(2025, 2, 1))
        .unwrap();
    assert_eq!(photo.source, ReadingSource::Photo);
    let key = photo.photo_key.clone().expect("Belegfoto");
    assert!(blobs.keys().contains(&key.to_string()));
    assert_eq!(
        uc.get_reading_photo.execute(&t, photo.id).await.unwrap(),
        jpeg
    );
    let grid_readings = uc.get_meter.execute(&t, grid).await.unwrap().readings;
    assert!(
        grid_readings
            .iter()
            .filter(|r| r.read_at == berlin(2025, 2, 1))
            .all(|r| r.source == ReadingSource::Manual && r.photo_key.is_none())
    );
    // Entwurf abgeschlossen, Entwurfsfoto gelöscht
    assert!(
        uc.record_bulk_readings
            .drafts(&t, grid)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !blobs.keys().iter().any(|k| k.contains("/drafts/")),
        "{:?}",
        blobs.keys()
    );
    // Alarmprüfung je Messstelle
    let events: Vec<OutboxEvent> = store.outbox()[before..]
        .iter()
        .map(|o| serde_json::from_str(&o.entry.payload).unwrap())
        .collect();
    assert_eq!(events.len(), 2, "Hausanschluss und Erzeugung");
    assert!(events.iter().any(|e| matches!(
        e,
        OutboxEvent::ReadingsRecorded { meter_id, reading_ids } if *meter_id == generation && reading_ids == &vec![photo.id]
    )));
}
