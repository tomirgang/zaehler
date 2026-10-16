//! Tests: Saisonprofil und Ampel in der Auswertung (SPEC_ANA_TRAFFIC_LIGHT,
//! SPEC_FC_SEASON_PROFILE).

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use zaehler_domain::{
    analysis::PeriodForecast,
    calendar::Calendar,
    expectation::{BandSource, Status},
    meter::{Direction, MeterKind, MeterPurpose},
    reading::ReadingSource,
};

use crate::{
    Tenant,
    ids::MeterId,
    model::{Identity, MeterTargets},
    testing::memory_ports,
    usecase::{
        UseCases,
        analysis::ProfileSource,
        meter::{MeterInput, RegisterInput},
        reading::ReadingsInput,
    },
};

fn berlin(y: i32, m: u32) -> DateTime<Utc> {
    Calendar::new(chrono_tz::Europe::Berlin).start_of(NaiveDate::from_ymd_opt(y, m, 1).unwrap())
}

async fn setup(clock_at: DateTime<Utc>) -> (UseCases, Tenant) {
    let (ports, _, clock) = memory_ports();
    clock.advance(clock_at - ports.clock.now());
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
    (uc, t)
}

async fn gas_meter(
    uc: &UseCases,
    t: &Tenant,
    start: DateTime<Utc>,
    targets: MeterTargets,
) -> MeterId {
    uc.create_meter
        .execute(
            t,
            MeterInput {
                name: format!("Gas {}", start.timestamp()),
                kind: MeterKind::Gas,
                unit: String::new(),
                schedule: None,
                max_daily_rate: None,
                billing_start: None,
                purpose: MeterPurpose::General,
                parent_meter_id: None,
                targets,
                serial_number: None,
                rollover_digits: None,
                decimal_places: 0,
                registers: vec![RegisterInput {
                    id: None,
                    code: "main".into(),
                    direction: Direction::Import,
                    start_value: Some(dec!(0)),
                }],
                start_at: start,
            },
        )
        .await
        .unwrap()
}

/// Ablesungen jeweils am Monatsanfang nach `consumption` (Verbrauch je Monat).
async fn monthly_readings(
    uc: &UseCases,
    t: &Tenant,
    meter: MeterId,
    from: (i32, u32),
    consumption: &[Decimal],
) {
    let register = uc
        .get_meter
        .execute(t, meter)
        .await
        .unwrap()
        .aggregate
        .registers[0]
        .id;
    let mut total = Decimal::ZERO;
    let mut date = NaiveDate::from_ymd_opt(from.0, from.1, 1).unwrap();
    for c in consumption {
        total += c;
        date = date + chrono::Months::new(1);
        uc.record_readings
            .execute(
                t,
                &ReadingsInput {
                    meter_id: meter,
                    at: berlin(date.year(), date.month()),
                    values: vec![(register, total)],
                    note: None,
                },
                true,
                ReadingSource::Manual,
            )
            .await
            .unwrap();
    }
}

/// Monatsverbrauch nach dem Standardprofil Gas bei 1000 im Jahr.
fn gas_year() -> Vec<Decimal> {
    [170, 150, 120, 80, 40, 20, 20, 20, 40, 80, 120, 140]
        .map(Decimal::from)
        .to_vec()
}

// @test SPEC_ANA_TRAFFIC_LIGHT, SPEC_FC_SEASON_PROFILE
#[tokio::test]
async fn gas_forecast_uses_the_default_profile_in_the_first_year() {
    let (uc, t) = setup(berlin(2026, 2) + Duration::days(10)).await;
    let id = gas_meter(&uc, &t, berlin(2026, 1), MeterTargets::default()).await;
    monthly_readings(&uc, &t, id, (2026, 1), &[dec!(170)]).await;
    let a = uc.meter_analysis.execute(&t, id, None).await.unwrap();
    assert_eq!(a.profile_source, ProfileSource::Default);
    let PeriodForecast::Projected(f) = &a.forecasts[0].forecast else {
        panic!("Prognose erwartet");
    };
    // Januar ist 17 % des Jahres
    assert_eq!(f.projected_total.round(), dec!(1000));
    // ohne Historie und ohne Ziel keine Ampel
    assert!(a.expectations.iter().all(Option::is_none));
    assert!(a.month_expectation.is_none());
}

// @test SPEC_ANA_TRAFFIC_LIGHT, SPEC_FC_SEASON_PROFILE, SPEC_EXP_BAND
#[tokio::test]
async fn traffic_light_green_yellow_red_from_history() {
    let (uc, t) = setup(berlin(2026, 1) + Duration::days(10)).await;
    let id = gas_meter(&uc, &t, berlin(2023, 1), MeterTargets::default()).await;
    let mut months = Vec::new();
    for (oct, nov, dec_) in [(80, 110, 140), (82, 130, 150), (100, 125, 158)] {
        let mut year = gas_year();
        year[9] = Decimal::from(oct);
        year[10] = Decimal::from(nov);
        year[11] = Decimal::from(dec_);
        months.extend(year);
    }
    monthly_readings(&uc, &t, id, (2023, 1), &months).await;
    let a = uc.meter_analysis.execute(&t, id, None).await.unwrap();
    assert_eq!(
        a.profile_source,
        ProfileSource::Learned,
        "36 vollständige Monate"
    );
    let status_of = |y: i32, m: u32| {
        let i = a
            .import
            .iter()
            .position(|x| x.month.start == NaiveDate::from_ymd_opt(y, m, 1).unwrap())
            .unwrap();
        a.expectations[i].map(|e| (e.source, e.status))
    };
    assert_eq!(
        status_of(2025, 11),
        Some((BandSource::History, Some(Status::Green)))
    );
    assert_eq!(
        status_of(2025, 12),
        Some((BandSource::History, Some(Status::Yellow)))
    );
    assert_eq!(
        status_of(2025, 10),
        Some((BandSource::History, Some(Status::Red)))
    );
    assert_eq!(status_of(2024, 12), None, "nur ein Vorjahreswert");
    // Bezugsmonat ist der letzte vollständige Monat
    assert_eq!(
        a.month_expectation.map(|e| e.status),
        Some(Some(Status::Yellow))
    );
    let cards = uc.dashboard.execute(&t).await.unwrap();
    assert_eq!(
        cards[0].traffic.and_then(|e| e.status),
        Some(Status::Yellow)
    );
}

// @test SPEC_ANA_TRAFFIC_LIGHT
#[tokio::test]
async fn traffic_light_from_annual_target_with_custom_profile() {
    let (uc, t) = setup(berlin(2026, 3) + Duration::days(10)).await;
    let mut weights = [Decimal::ONE; 12];
    weights[0] = dec!(2);
    let targets = MeterTargets {
        season_weights: Some(zaehler_domain::forecast::SeasonProfile {
            monthly_weights: weights,
        }),
        annual_target: Some(dec!(1300)),
        tolerance_pct: Some(10),
    };
    let id = gas_meter(&uc, &t, berlin(2026, 1), targets).await;
    // Januar: 2 von 13 Anteilen = 200 erwartet, Februar 100
    monthly_readings(&uc, &t, id, (2026, 1), &[dec!(205), dec!(140)]).await;
    let a = uc.meter_analysis.execute(&t, id, None).await.unwrap();
    assert_eq!(a.profile_source, ProfileSource::Custom);
    let jan = a.expectations[0].expect("Ziel");
    assert_eq!(jan.source, BandSource::Target);
    assert_eq!(jan.status, Some(Status::Green));
    assert_eq!(a.expectations[1].and_then(|e| e.status), Some(Status::Red));

    // ungültige Vorgaben
    let mut bad = MeterTargets {
        tolerance_pct: Some(0),
        annual_target: Some(dec!(-1)),
        season_weights: Some(zaehler_domain::forecast::SeasonProfile {
            monthly_weights: [Decimal::ZERO; 12],
        }),
    };
    let detail = uc.get_meter.execute(&t, id).await.unwrap();
    let mut input = MeterInput {
        name: detail.aggregate.meter.data.name.clone(),
        kind: MeterKind::Gas,
        unit: detail.aggregate.meter.data.unit.clone(),
        schedule: None,
        max_daily_rate: None,
        billing_start: None,
        purpose: MeterPurpose::General,
        parent_meter_id: None,
        targets: bad.clone(),
        serial_number: None,
        rollover_digits: None,
        decimal_places: 0,
        registers: vec![RegisterInput {
            id: Some(detail.aggregate.registers[0].id),
            code: "main".into(),
            direction: Direction::Import,
            start_value: None,
        }],
        start_at: Utc::now(),
    };
    match uc.update_meter.execute(&t, id, input.clone()).await {
        Err(crate::AppError::Validation(e)) => {
            for f in ["annual_target", "tolerance_pct", "season_weights"] {
                assert!(!e.for_field(f).is_empty(), "{f}");
            }
        }
        other => panic!("{other:?}"),
    }
    bad = MeterTargets::default();
    input.targets = bad;
    uc.update_meter.execute(&t, id, input).await.unwrap();
    let a = uc.meter_analysis.execute(&t, id, None).await.unwrap();
    assert_eq!(a.profile_source, ProfileSource::Default);
    assert!(a.expectations.iter().all(Option::is_none));
}
