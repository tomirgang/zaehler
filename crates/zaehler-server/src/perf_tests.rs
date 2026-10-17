//! Leistungsbudget (SPEC_PERF_BUDGET): Seed-Daten über die Anwendungsfälle,
//! Messung serverseitig über den vollständigen Router mit allen Adaptern.
//!
//! Ausführung im Release-Profil, PostgreSQL nur mit
//! `ZAEHLER_TEST_DATABASE_URL`:
//!
//! ```text
//! cargo test --release -p zaehler-server perf_ -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use chrono::{DateTime, Months, TimeZone, Utc};
use rust_decimal::Decimal;
use tower::ServiceExt;
use zaehler_app::{
    ids::MeterId,
    model::Identity,
    usecase::{
        UseCases,
        contract::{ComponentInput, ContractInput, InstallmentInput, PositionInput},
        meter::{MeterInput, RegisterInput},
        reading::ReadingsInput,
    },
};
use zaehler_domain::{
    meter::{Direction, MeterKind},
    reading::ReadingSource,
    tariff::{BasePriceInterval, UnitPricing},
};

use crate::config::{Config, StorageConfig};

const METERS: usize = 20;
const MONTHS: u32 = 120;
const WARMUP: usize = 20;
const REQUESTS: usize = 200;
const BUDGET: Duration = Duration::from_millis(300);

/// Ein Haushalt, 20 Messstellen mit je 2 Registern, 10 Jahre monatliche
/// Ablesungen (4 800 Ablesungen) und je ein Vertrag mit zwei Preiswechseln
/// je Jahr.
struct Seeded {
    meter: MeterId,
    vehicle: MeterId,
    grid: MeterId,
    pv: zaehler_app::ids::PvSystemId,
    token: String,
}

async fn seed(uc: &UseCases, name: &str) -> Seeded {
    let identity = Identity {
        issuer: "dev".into(),
        subject: name.into(),
        email: None,
        display_name: name.into(),
        locale: "de".into(),
    };
    let user = uc.sign_in.execute(&identity).await.unwrap().user.id;
    let tenant = uc.resolve_tenant.execute(user, None).await.unwrap().tenant;
    let start: DateTime<Utc> = Utc.with_ymd_and_hms(2016, 1, 1, 0, 0, 0).unwrap();
    let mut first = None;
    for m in 0..METERS {
        let id = uc
            .create_meter
            .execute(
                &tenant,
                MeterInput {
                    name: format!("Messstelle {m:02}"),
                    kind: MeterKind::Electricity,
                    unit: "kWh".into(),
                    schedule: Some(zaehler_domain::schedule::ReadingSchedule::EveryDays(30)),
                    purpose: zaehler_domain::meter::MeterPurpose::General,
                    targets: zaehler_app::model::MeterTargets::default(),
                    parent_meter_id: None,
                    max_daily_rate: None,
                    billing_start: None,
                    serial_number: Some(format!("PERF-{m:02}")),
                    rollover_digits: None,
                    decimal_places: 1,
                    registers: ["HT", "NT"]
                        .into_iter()
                        .map(|code| RegisterInput {
                            id: None,
                            code: code.into(),
                            direction: Direction::Import,
                            start_value: Some(Decimal::ZERO),
                        })
                        .collect(),
                    start_at: start,
                },
            )
            .await
            .unwrap();
        let detail = uc.get_meter.execute(&tenant, id).await.unwrap();
        let registers: Vec<_> = detail.aggregate.registers.iter().map(|r| r.id).collect();
        let (mut ht, mut nt) = (Decimal::ZERO, Decimal::ZERO);
        for k in 1..=MONTHS {
            ht += Decimal::from(150 + (k % 12) * 10);
            nt += Decimal::from(80 + (k % 5) * 3);
            let input = ReadingsInput {
                meter_id: id,
                at: start + Months::new(k),
                values: vec![(registers[0], ht), (registers[1], nt)],
                note: None,
            };
            uc.record_readings
                .execute(&tenant, &input, true, ReadingSource::Manual)
                .await
                .unwrap();
        }
        seed_contract(uc, &tenant, id, &registers).await;
        first.get_or_insert(id);
    }
    let house = first.unwrap();
    let vehicle = seed_vehicle(uc, &tenant, house).await;
    let (grid, pv) = seed_pv(uc, &tenant, start).await;
    let token = uc
        .create_api_token
        .execute(
            &tenant,
            zaehler_app::usecase::api::ApiTokenInput {
                name: "Perf".into(),
                role: zaehler_app::Role::Member,
                expires_at: None,
            },
        )
        .await
        .unwrap()
        .token;
    Seeded {
        meter: house,
        vehicle,
        grid,
        pv,
        token,
    }
}

/// Hausanschluss mit Erzeugungszähler und Speicher, 10 Jahre monatliche
/// Sammelerfassung, Anlage mit Investition (SPEC_PERF_BUDGET, SPEC_PV_CHARTS).
#[allow(clippy::too_many_lines)] // vollständiger Datensatz
async fn seed_pv(
    uc: &UseCases,
    tenant: &zaehler_app::Tenant,
    start: DateTime<Utc>,
) -> (MeterId, zaehler_app::ids::PvSystemId) {
    use zaehler_app::{
        model::AssetRef,
        usecase::{
            asset_cost::AssetCostInput,
            pv::{BulkInput, PvSystemInput},
        },
    };
    use zaehler_domain::{
        assets::{AssetCategory, PvCostCategory},
        pv::{PvKind, PvRole, PvSystemData},
    };

    let meter = |name: &str, registers: &[(&str, Direction)]| MeterInput {
        name: name.into(),
        kind: MeterKind::Electricity,
        unit: "kWh".into(),
        schedule: None,
        purpose: zaehler_domain::meter::MeterPurpose::General,
        targets: zaehler_app::model::MeterTargets::default(),
        parent_meter_id: None,
        max_daily_rate: None,
        billing_start: None,
        serial_number: None,
        rollover_digits: None,
        decimal_places: 1,
        registers: registers
            .iter()
            .map(|(code, direction)| RegisterInput {
                id: None,
                code: (*code).into(),
                direction: *direction,
                start_value: Some(Decimal::ZERO),
            })
            .collect(),
        start_at: start,
    };
    let mut ids = Vec::new();
    for (name, registers) in [
        (
            "PV Hausanschluss",
            vec![("1.8.0", Direction::Import), ("2.8.0", Direction::Export)],
        ),
        ("PV Erzeugung", vec![("2.8.0", Direction::Export)]),
        (
            "PV Speicher",
            vec![
                ("charge", Direction::Import),
                ("discharge", Direction::Export),
            ],
        ),
    ] {
        let id = uc
            .create_meter
            .execute(tenant, meter(name, &registers))
            .await
            .unwrap();
        let regs: Vec<_> = uc
            .get_meter
            .execute(tenant, id)
            .await
            .unwrap()
            .aggregate
            .registers
            .iter()
            .map(|r| r.id)
            .collect();
        ids.push((id, regs));
    }
    let system = uc
        .create_pv_system
        .execute(
            tenant,
            PvSystemInput {
                data: PvSystemData {
                    name: "Dach".into(),
                    kind: PvKind::Surplus,
                    kwp: Decimal::from(10),
                    commissioned_on: start.date_naive(),
                    azimuth: Some(180),
                    tilt: Some(30),
                    inverter: None,
                    battery_kwh: Some(Decimal::from(10)),
                    specific_yield: Decimal::from(950),
                    degradation_pct: Decimal::new(5, 1),
                    month_shares: None,
                    alarm_threshold_pct: 50,
                },
                meters: vec![
                    (ids[0].0, PvRole::Grid),
                    (ids[1].0, PvRole::Generation),
                    (ids[2].0, PvRole::Battery),
                ],
            },
        )
        .await
        .unwrap();
    let mut totals = [Decimal::ZERO; 5];
    for k in 1..=MONTHS {
        let season = Decimal::from(40 + (k % 12) * 15);
        let deltas = [
            Decimal::from(250) - season,
            season * Decimal::from(2),
            season * Decimal::from(4),
            Decimal::from(120),
            Decimal::from(110),
        ];
        for (t, d) in totals.iter_mut().zip(deltas) {
            *t += d;
        }
        let values = vec![
            (ids[0].0, ids[0].1[0], totals[0]),
            (ids[0].0, ids[0].1[1], totals[1]),
            (ids[1].0, ids[1].1[0], totals[2]),
            (ids[2].0, ids[2].1[0], totals[3]),
            (ids[2].0, ids[2].1[1], totals[4]),
        ];
        uc.record_bulk_readings
            .execute(
                tenant,
                ids[0].0,
                BulkInput {
                    at: start + Months::new(k),
                    values,
                    note: None,
                    confirm_warnings: true,
                    drafts: Vec::new(),
                },
            )
            .await
            .unwrap();
    }
    uc.add_asset_cost
        .execute(
            tenant,
            AssetRef::pv(system),
            AssetCostInput {
                booked_on: start.date_naive(),
                category: AssetCategory::Pv(PvCostCategory::Investment),
                amount: Decimal::from(14_500),
                quantity: None,
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
    (ids[0].0, system)
}

/// Vertrag HT/NT ab 2016 mit zwei Preiswechseln je Jahr und jährlich neuem
/// Abschlag (SPEC_PERF_BUDGET, SPEC_TAR_COST_VIEW).
async fn seed_contract(
    uc: &UseCases,
    tenant: &zaehler_app::Tenant,
    meter: MeterId,
    registers: &[zaehler_app::ids::RegisterId],
) {
    let day = |y: i32, m: u32| chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap();
    let components = |base: i64, cents: i64| -> Vec<ComponentInput> {
        (2016..2026)
            .flat_map(|y| [day(y, 1), day(y, 7)])
            .enumerate()
            .map(|(i, valid_from)| ComponentInput {
                id: None,
                valid_from,
                base_price: Decimal::from(base),
                base_interval: BasePriceInterval::Year,
                unit_pricing: UnitPricing::Flat {
                    price: Decimal::new(cents + i64::try_from(i).unwrap(), 2),
                },
                factor: None,
            })
            .collect()
    };
    let position = |name: &str, register, base, cents| PositionInput {
        id: None,
        name: name.into(),
        register_id: Some(register),
        factor: Decimal::ONE,
        vat_pct: Some(Decimal::from(19)),
        components: components(base, cents),
    };
    let contract = uc
        .create_contract
        .execute(
            tenant,
            meter,
            ContractInput {
                direction: Direction::Import,
                provider: "Stadtwerke".into(),
                contract_number: None,
                starts_on: day(2016, 1),
                ends_on: None,
                billing_start: None,
                note: None,
                positions: vec![
                    position("HT", registers[0], 120, 25),
                    position("NT", registers[1], 0, 20),
                ],
            },
        )
        .await
        .unwrap();
    for y in 2016..=2026 {
        uc.set_installment
            .execute(
                tenant,
                contract,
                InstallmentInput {
                    valid_from: day(y, 2),
                    amount: Decimal::from(60 + (y - 2016) * 2),
                    every_months: 1,
                },
            )
            .await
            .unwrap();
    }
}

/// Plug-in-Hybrid mit 10 Jahren Kilometerständen, monatlichem Tanken und
/// Laden, Parken und jährlicher Versicherung; Heimladen über die erste
/// Messstelle geschätzt (SPEC_PERF_BUDGET, SPEC_CAR_REPORT).
#[allow(clippy::too_many_lines)] // vollständiger Datensatz
async fn seed_vehicle(uc: &UseCases, tenant: &zaehler_app::Tenant, house: MeterId) -> MeterId {
    use zaehler_app::{
        model::{AssetRef, ChargingKind},
        usecase::{
            asset_cost::AssetCostInput,
            vehicle::{ChargingInput, NewVehicle, VehicleInput},
        },
    };
    use zaehler_domain::{
        assets::{AssetCategory, Recurrence, VehicleCostCategory},
        vehicle::{ConsumptionProfile, DriveType},
    };
    let start: DateTime<Utc> = Utc.with_ymd_and_hms(2016, 1, 1, 0, 0, 0).unwrap();
    let car = uc
        .create_vehicle
        .execute(
            tenant,
            NewVehicle {
                name: "Hybrid".into(),
                vehicle: VehicleInput {
                    plate: "M-PF 1".into(),
                    model: "Kombi".into(),
                    drive_type: DriveType::PluginHybrid,
                    home_charging_meter_id: Some(house),
                    pv_share_pct: None,
                },
                odometer: Decimal::ZERO,
                start_at: start,
            },
        )
        .await
        .unwrap();
    uc.set_consumption_profile
        .execute(
            tenant,
            car,
            ConsumptionProfile {
                valid_from: start.date_naive(),
                kwh_per_100km: Decimal::from(18),
                charging_loss_pct: Decimal::from(10),
                electric_share_pct: Some(Decimal::from(40)),
            },
        )
        .await
        .unwrap();
    let mut insurance = AssetCostInput {
        booked_on: start.date_naive(),
        category: AssetCategory::Vehicle(VehicleCostCategory::Insurance),
        amount: Decimal::from(600),
        quantity: None,
        odometer: None,
        full_tank: false,
        recurrence: Some(Recurrence {
            every_months: 12,
            until: None,
        }),
        note: None,
        odometer_as_reading: false,
        confirm_warnings: false,
    };
    uc.add_asset_cost
        .execute(tenant, AssetRef::vehicle(car), insurance.clone())
        .await
        .unwrap();
    insurance.recurrence = None;
    for k in 1..=MONTHS {
        let at = start + Months::new(k);
        let km = Decimal::from(k * 1200);
        uc.add_asset_cost
            .execute(
                tenant,
                AssetRef::vehicle(car),
                AssetCostInput {
                    booked_on: at.date_naive(),
                    category: AssetCategory::Vehicle(VehicleCostCategory::Fuel),
                    amount: Decimal::from(75),
                    quantity: Some(Decimal::from(42)),
                    odometer: Some(km),
                    full_tank: true,
                    odometer_as_reading: true,
                    confirm_warnings: true,
                    ..insurance.clone()
                },
            )
            .await
            .unwrap();
        uc.add_asset_cost
            .execute(
                tenant,
                AssetRef::vehicle(car),
                AssetCostInput {
                    booked_on: at.date_naive(),
                    category: AssetCategory::Vehicle(VehicleCostCategory::Parking),
                    amount: Decimal::from(12),
                    ..insurance.clone()
                },
            )
            .await
            .unwrap();
        uc.record_charging
            .execute(
                tenant,
                car,
                ChargingInput {
                    charged_at: at,
                    energy: Decimal::from(40),
                    amount: Decimal::from(20),
                    odometer: None,
                    provider: Some("Ladesäule".into()),
                    kind: ChargingKind::Ac,
                    odometer_as_reading: false,
                    confirm_warnings: false,
                },
            )
            .await
            .unwrap();
    }
    car
}

/// `auth`: Cookie der Sitzung, unter `/api/` das Bearer-Token.
async fn get(app: &Router, path: &str, auth: &str) -> (StatusCode, Duration) {
    let req = if path.starts_with("/api/") {
        Request::get(path).header(header::AUTHORIZATION, format!("Bearer {auth}"))
    } else {
        Request::get(path).header(header::COOKIE, auth)
    }
    .body(Body::empty())
    .unwrap();
    let start = Instant::now();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, start.elapsed())
}

fn p95(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[(samples.len() * 95).div_ceil(100) - 1]
}

/// Misst Dashboard, Auswertungsseite, `chart.json` und die Kostenansichten;
/// meldet p95 und scheitert bei Überschreitung des Budgets.
async fn measure(database_url: &str, backend: &str) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.database.url = database_url.to_owned();
    config.storage = StorageConfig::Fs {
        path: dir.path().join("files").to_string_lossy().into_owned(),
    };
    config.auth.dev_login = true;
    config.backup.before_migrate = false;
    let app = crate::wiring::build(&config, false).await.unwrap();

    // eigener Nutzer je Lauf, damit eine geteilte Test-Datenbank genügt
    let name = format!("perf-{}", uuid::Uuid::now_v7().simple());
    let seeded = Instant::now();
    let Seeded {
        meter,
        vehicle,
        grid,
        pv,
        token,
    } = seed(&app.uc, &name).await;
    println!("{backend}: Seed in {:?}", seeded.elapsed());

    let login = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/auth/dev-login?name={name}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();

    let mut failed = Vec::new();
    for (what, path) in [
        ("Dashboard", "/".to_owned()),
        ("Auswertung", format!("/meters/{meter}")),
        ("chart.json", format!("/meters/{meter}/chart.json")),
        ("Kostenansicht", format!("/meters/{meter}/costs")),
        ("Haushaltskosten", "/costs".to_owned()),
        ("Fahrzeug", format!("/vehicles/{vehicle}")),
        ("Kostenbuch", format!("/vehicles/{vehicle}/costs?year=2020")),
        ("Hausanschluss PV", format!("/pv/grid/{grid}")),
        ("PV-Anlage", format!("/pv/{pv}")),
        ("PV-Bilanz", format!("/pv/{pv}/balance.json")),
        ("Sammelerfassung", format!("/pv/grid/{grid}/readings")),
        ("API Messstellen", "/api/v1/meters".to_owned()),
        (
            "API Ablesungen",
            format!("/api/v1/meters/{meter}/readings?limit=1000"),
        ),
        ("API Auswertung", format!("/api/v1/meters/{meter}/analysis")),
        ("API Kosten", format!("/api/v1/meters/{meter}/costs")),
    ] {
        let auth = if path.starts_with("/api/") {
            token.as_str()
        } else {
            cookie.as_str()
        };
        for _ in 0..WARMUP {
            get(&app.router, &path, auth).await;
        }
        let mut samples = Vec::with_capacity(REQUESTS);
        for _ in 0..REQUESTS {
            let (status, elapsed) = get(&app.router, &path, auth).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            samples.push(elapsed);
        }
        let p = p95(samples);
        println!("{backend}: {what} p95 = {:.1} ms", p.as_secs_f64() * 1000.0);
        if p >= BUDGET {
            failed.push(format!("{what} {p:?}"));
        }
    }
    assert!(
        failed.is_empty(),
        "{backend}: Budget {BUDGET:?} überschritten: {failed:?}"
    );
}

// @test SPEC_PERF_BUDGET
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Leistungstest, im Release-Profil ausführen"]
async fn perf_budget_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("perf.db").display());
    measure(&url, "SQLite").await;
}

// @test SPEC_PERF_BUDGET
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Leistungstest, im Release-Profil ausführen"]
async fn perf_budget_postgres() {
    let Ok(url) = std::env::var("ZAEHLER_TEST_DATABASE_URL") else {
        eprintln!("ZAEHLER_TEST_DATABASE_URL nicht gesetzt; PostgreSQL-Messung übersprungen");
        return;
    };
    measure(&url, "PostgreSQL").await;
}
