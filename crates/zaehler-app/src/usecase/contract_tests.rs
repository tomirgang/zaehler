//! Tests: Verträge, Kosten, Abschläge und Abrechnung (SPEC_TAR_CONTRACTS,
//! SPEC_TAR_COST_VIEW, SPEC_TAR_INSTALLMENT_SETTLEMENT,
//! SPEC_TAR_ACTUAL_SETTLEMENT, SPEC_TAR_SUBMETER_COST).

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use zaehler_domain::{
    calendar::Calendar,
    expectation::{BandSource, Status},
    meter::{Direction, MeterKind, MeterPurpose},
    reading::ReadingSource,
    tariff::{AdjustmentKind, BasePriceInterval, Tier, UnitPricing},
};

use crate::{
    AppError, Tenant,
    ids::{MeterId, RegisterId},
    model::{ContractEntry, Identity},
    testing::{FixedClock, memory_ports},
    usecase::{
        UseCases,
        contract::{
            AdjustmentInput, ComponentInput, ContractInput, InstallmentInput, PositionInput,
            SettlementInput,
        },
        meter::{MeterInput, RegisterInput, SwapInput},
        reading::{ReadingsInput, RecordOutcome},
    },
};

pub(super) fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

pub(super) fn berlin(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    Calendar::new(chrono_tz::Europe::Berlin).start_of(day(y, m, d))
}

pub(super) async fn setup() -> (UseCases, Tenant, std::sync::Arc<FixedClock>) {
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

/// Uhr auf 12:00 UTC des Tages stellen (nur vorwärts).
pub(super) fn set_today(clock: &FixedClock, d: NaiveDate) {
    let target = Utc
        .from_utc_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
        .timestamp();
    let now = crate::port::Clock::now(clock).timestamp();
    clock.advance(Duration::seconds(target - now));
}

pub(super) fn meter_input(
    name: &str,
    kind: MeterKind,
    registers: &[(&str, Direction)],
    start: DateTime<Utc>,
) -> MeterInput {
    MeterInput {
        name: name.into(),
        kind,
        unit: String::new(),
        schedule: None,
        max_daily_rate: None,
        billing_start: None,
        purpose: MeterPurpose::General,
        targets: crate::model::MeterTargets::default(),
        parent_meter_id: None,
        serial_number: None,
        rollover_digits: None,
        decimal_places: 1,
        registers: registers
            .iter()
            .map(|(code, direction)| RegisterInput {
                id: None,
                code: (*code).into(),
                direction: *direction,
                start_value: Some(dec!(0)),
            })
            .collect(),
        start_at: start,
    }
}

pub(super) async fn registers(uc: &UseCases, t: &Tenant, meter: MeterId) -> Vec<RegisterId> {
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

async fn register(uc: &UseCases, t: &Tenant, meter: MeterId, code: &str) -> RegisterId {
    uc.get_meter
        .execute(t, meter)
        .await
        .unwrap()
        .aggregate
        .registers
        .iter()
        .find(|r| r.data.code == code)
        .unwrap()
        .id
}

pub(super) async fn record(
    uc: &UseCases,
    t: &Tenant,
    meter: MeterId,
    at: DateTime<Utc>,
    value: Decimal,
) {
    let register = registers(uc, t, meter).await[0];
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

pub(super) fn flat(from: NaiveDate, base: Decimal, price: Decimal) -> ComponentInput {
    ComponentInput {
        id: None,
        valid_from: from,
        base_price: base,
        base_interval: BasePriceInterval::Year,
        unit_pricing: UnitPricing::Flat { price },
        factor: None,
    }
}

pub(super) fn position(
    name: &str,
    register: Option<RegisterId>,
    components: Vec<ComponentInput>,
) -> PositionInput {
    PositionInput {
        id: None,
        name: name.into(),
        register_id: register,
        factor: Decimal::ONE,
        vat_pct: None,
        components,
    }
}

pub(super) fn contract(start: NaiveDate, positions: Vec<PositionInput>) -> ContractInput {
    ContractInput {
        direction: Direction::Import,
        provider: "Stadtwerke".into(),
        contract_number: None,
        starts_on: start,
        ends_on: None,
        billing_start: None,
        note: None,
        positions,
    }
}

pub(super) fn conflict<T: std::fmt::Debug>(r: Result<T, AppError>, code: &str) {
    match r {
        Err(AppError::Conflict(c)) if c == code => {}
        other => panic!("{code} erwartet, war {other:?}"),
    }
}

pub(super) fn field<T: std::fmt::Debug>(r: Result<T, AppError>, name: &str, code: &str) {
    match r {
        Err(AppError::Validation(e)) => assert!(
            e.for_field(name).iter().any(|x| x.code == code),
            "{name}/{code} fehlt in {e:?}"
        ),
        other => panic!("{name}/{code} erwartet, war {other:?}"),
    }
}

/// Strom HT/NT, Gas mit Brennwert und Wasser mit drei Positionen über die
/// Vorlagen; Überlappung, Folgevertrag und paralleler Einspeisevertrag.
// @test SPEC_TAR_CONTRACTS
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein Ablauf je Vorlage
async fn contracts_from_templates_and_overlap_rules() {
    let (uc, t, _) = setup().await;
    let start = berlin(2025, 12, 1);
    let strom = uc
        .create_meter
        .execute(
            &t,
            meter_input(
                "Strom",
                MeterKind::Electricity,
                &[
                    ("HT", Direction::Import),
                    ("NT", Direction::Import),
                    ("Einspeisung", Direction::Export),
                ],
                start,
            ),
        )
        .await
        .unwrap();
    let (_, templates) = uc.contract_templates.execute(&t, strom).await.unwrap();
    let codes: Vec<&str> = templates.iter().map(|x| x.code).collect();
    assert_eq!(codes, ["electricity", "electricity_ht_nt", "feed_in"]);
    let ht_nt = &templates[1];
    let ht = register(&uc, &t, strom, "HT").await;
    let nt = register(&uc, &t, strom, "NT").await;
    let export = register(&uc, &t, strom, "Einspeisung").await;
    assert_eq!(
        ht_nt
            .positions
            .iter()
            .map(|p| p.register_id)
            .collect::<Vec<_>>(),
        [Some(ht), Some(nt)]
    );
    let from = day(2026, 1, 1);
    let input = contract(
        from,
        ht_nt
            .positions
            .iter()
            .map(|p| {
                position(
                    p.name,
                    p.register_id,
                    vec![flat(
                        from,
                        if p.base { dec!(150) } else { dec!(0) },
                        dec!(0.30),
                    )],
                )
            })
            .collect(),
    );
    let c = uc
        .create_contract
        .execute(&t, strom, input.clone())
        .await
        .unwrap();
    let detail = uc.get_contract.execute(&t, c).await.unwrap();
    assert_eq!(detail.contract.data.positions.len(), 2);
    assert_eq!(
        (
            detail.contract.data.billing_start.month,
            detail.contract.data.billing_start.day
        ),
        (1, 1),
        "Abrechnungsbeginn aus dem Vertragsbeginn"
    );

    // Überlappung je Richtung
    conflict(
        uc.create_contract
            .execute(
                &t,
                strom,
                contract(day(2026, 6, 1), input.positions.clone()),
            )
            .await,
        "contract.overlap",
    );
    // Anbieterwechsel: alter Vertrag endet, Folgevertrag beginnt am Tag danach
    let mut ended = input.clone();
    ended.ends_on = Some(day(2026, 5, 31));
    uc.update_contract.execute(&t, c, ended).await.unwrap();
    let mut follow = contract(
        day(2026, 6, 1),
        vec![position(
            "Strom",
            None,
            vec![flat(day(2026, 6, 1), dec!(120), dec!(0.28))],
        )],
    );
    follow.provider = "Neuer Anbieter".into();
    uc.create_contract.execute(&t, strom, follow).await.unwrap();
    // Einspeisevertrag läuft parallel zum Bezug
    let mut feed = contract(
        from,
        vec![position(
            "Einspeisevergütung",
            None,
            vec![flat(from, dec!(0), dec!(-0.0786))],
        )],
    );
    feed.direction = Direction::Export;
    uc.create_contract.execute(&t, strom, feed).await.unwrap();
    assert_eq!(
        uc.meter_contracts.execute(&t, strom).await.unwrap().len(),
        3
    );

    // Gas: Faktor Brennwert × Zustandszahl, neuer Faktor mit dem Preisstand
    let gas = uc
        .create_meter
        .execute(
            &t,
            meter_input("Gas", MeterKind::Gas, &[("main", Direction::Import)], start),
        )
        .await
        .unwrap();
    let (_, gas_templates) = uc.contract_templates.execute(&t, gas).await.unwrap();
    assert_eq!(
        gas_templates[0].positions[0].factor, None,
        "Faktor ist einzugeben"
    );
    let mut gas_pos = position("Gas", None, vec![flat(from, dec!(180), dec!(0.12))]);
    gas_pos.factor = dec!(10.25);
    let mut october = flat(day(2026, 10, 1), dec!(180), dec!(0.11));
    october.factor = Some(dec!(10.3));
    gas_pos.components.push(october);
    uc.create_contract
        .execute(&t, gas, contract(from, vec![gas_pos]))
        .await
        .unwrap();

    // Wasser mit drei Positionen
    let water = uc
        .create_meter
        .execute(
            &t,
            meter_input(
                "Wasser",
                MeterKind::Water,
                &[("main", Direction::Import)],
                start,
            ),
        )
        .await
        .unwrap();
    let (_, water_templates) = uc.contract_templates.execute(&t, water).await.unwrap();
    assert_eq!(water_templates[0].positions.len(), 3);
    let positions = water_templates[0]
        .positions
        .iter()
        .map(|p| position(p.name, None, vec![flat(from, dec!(24), dec!(2.08))]))
        .collect();
    uc.create_contract
        .execute(&t, water, contract(from, positions))
        .await
        .unwrap();

    // Prüfungen der Eingaben
    let mut bad = contract(
        from,
        vec![position(
            "x",
            Some(export),
            vec![flat(from, dec!(0), dec!(1))],
        )],
    );
    bad.provider = "Prüf".into();
    field(
        uc.create_contract.execute(&t, gas, bad.clone()).await,
        "pos0_register",
        "contract.register_invalid",
    );
    bad.positions[0].register_id = None;
    bad.positions[0].components[0].valid_from = day(2026, 2, 1);
    field(
        uc.create_contract.execute(&t, gas, bad.clone()).await,
        "pos0_c0_valid_from",
        "contract.component_after_start",
    );
    bad.positions[0].components[0].valid_from = from;
    bad.positions[0].components[0].unit_pricing = UnitPricing::Tiered {
        tiers: vec![Tier {
            up_to: Some(dec!(100)),
            price: dec!(0.3),
        }],
    };
    field(
        uc.create_contract.execute(&t, gas, bad.clone()).await,
        "pos0_c0_tiers",
        "contract.tiers_invalid",
    );
    bad.positions[0].components[0].unit_pricing = UnitPricing::Flat {
        price: dec!(0.1234567),
    };
    field(
        uc.create_contract.execute(&t, gas, bad.clone()).await,
        "pos0_c0_price",
        "contract.too_precise",
    );
    let car = uc
        .create_meter
        .execute(
            &t,
            meter_input(
                "Auto",
                MeterKind::Vehicle,
                &[("km", Direction::Import)],
                start,
            ),
        )
        .await
        .unwrap();
    field(
        uc.create_contract.execute(&t, car, input).await,
        "direction",
        "contract.meter_kind_invalid",
    );
}

/// Zwischenzähler haben keinen eigenen Vertrag, solange sie zugeordnet sind.
// @test SPEC_MTR_SUBMETER, SPEC_TAR_CONTRACTS
#[tokio::test]
async fn submeters_have_no_contract_while_assigned() {
    let (uc, t, clock) = setup().await;
    let start = berlin(2026, 1, 1);
    let house = uc
        .create_meter
        .execute(
            &t,
            meter_input(
                "Hausstrom",
                MeterKind::Electricity,
                &[("main", Direction::Import)],
                start,
            ),
        )
        .await
        .unwrap();
    let mut wp = meter_input(
        "Wärmepumpe",
        MeterKind::Electricity,
        &[("main", Direction::Import)],
        start,
    );
    wp.purpose = MeterPurpose::HeatPump;
    wp.parent_meter_id = Some(house);
    let wp = uc.create_meter.execute(&t, wp).await.unwrap();
    let wp_contract = contract(
        day(2026, 1, 1),
        vec![position(
            "WP-Strom",
            None,
            vec![flat(day(2026, 1, 1), dec!(60), dec!(0.25))],
        )],
    );
    conflict(
        uc.create_contract
            .execute(&t, wp, wp_contract.clone())
            .await,
        "contract.submeter",
    );

    // Wechsel auf einen eigenen Zähler: ab dem Wechsel ist ein Vertrag möglich
    set_today(&clock, day(2026, 6, 2));
    let reg = registers(&uc, &t, wp).await[0];
    uc.swap_device
        .execute(
            &t,
            wp,
            SwapInput {
                at: berlin(2026, 6, 1),
                end_values: vec![(reg, dec!(500))],
                serial_number: None,
                rollover_digits: None,
                decimal_places: 1,
                start_values: vec![(reg, dec!(0))],
                parent_meter_id: None,
            },
        )
        .await
        .unwrap();
    conflict(
        uc.create_contract
            .execute(&t, wp, wp_contract.clone())
            .await,
        "contract.submeter",
    );
    let own = contract(
        day(2026, 6, 1),
        vec![position(
            "WP-Strom",
            None,
            vec![flat(day(2026, 6, 1), dec!(60), dec!(0.25))],
        )],
    );
    uc.create_contract.execute(&t, wp, own).await.unwrap();
    // eine Zuordnung, die den Vertrag überschneidet, wird abgelehnt
    let mut again = meter_input(
        "Wärmepumpe",
        MeterKind::Electricity,
        &[("main", Direction::Import)],
        start,
    );
    again.purpose = MeterPurpose::HeatPump;
    again.parent_meter_id = Some(house);
    again.registers[0].id = Some(reg);
    conflict(
        uc.update_meter.execute(&t, wp, again).await,
        "contract.submeter",
    );
}

/// Kosten im laufenden Abrechnungszeitraum: gemessen und prognostiziert,
/// Abschlagsänderung mitten im Zeitraum, Saldo, Vorschlag, erfasste
/// Abrechnung mit Differenz und Prognose zum Stichtag, Ampel mit
/// Abschlagsbasis.
// @test SPEC_TAR_COST_VIEW, SPEC_TAR_INSTALLMENT_SETTLEMENT, SPEC_TAR_ACTUAL_SETTLEMENT, SPEC_ANA_TRAFFIC_LIGHT
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn costs_installments_and_settlement() {
    let (uc, t, clock) = setup().await;
    let strom = uc
        .create_meter
        .execute(
            &t,
            meter_input(
                "Strom",
                MeterKind::Electricity,
                &[("main", Direction::Import)],
                berlin(2025, 1, 1),
            ),
        )
        .await
        .unwrap();
    set_today(&clock, day(2026, 3, 10));
    // 10 kWh je Tag
    record(&uc, &t, strom, berlin(2025, 7, 1), dec!(1810)).await;
    record(&uc, &t, strom, berlin(2026, 1, 1), dec!(3650)).await;
    record(&uc, &t, strom, berlin(2026, 3, 1), dec!(4240)).await;

    let c = uc
        .create_contract
        .execute(
            &t,
            strom,
            contract(
                day(2025, 1, 1),
                vec![position(
                    "Strom",
                    None,
                    vec![
                        flat(day(2025, 1, 1), dec!(120), dec!(0.30)),
                        flat(day(2026, 1, 1), dec!(150), dec!(0.35)),
                    ],
                )],
            ),
        )
        .await
        .unwrap();
    let inst = |from: NaiveDate, amount: Decimal| InstallmentInput {
        valid_from: from,
        amount,
        every_months: 1,
    };
    uc.set_installment
        .execute(&t, c, inst(day(2025, 1, 15), dec!(100)))
        .await
        .unwrap();
    uc.set_installment
        .execute(&t, c, inst(day(2025, 7, 15), dec!(110)))
        .await
        .unwrap();
    conflict(
        uc.set_installment
            .execute(&t, c, inst(day(2025, 7, 15), dec!(111)))
            .await,
        "contract.installment_exists",
    );
    field(
        uc.set_installment
            .execute(&t, c, inst(day(2024, 7, 15), dec!(111)))
            .await,
        "installment_valid_from",
        "contract.date_outside",
    );

    // Ampel ohne Historie und Ziel: Abschlagsbasis (1320 € - 150 €) / 0,35
    let card = uc.dashboard.execute(&t).await.unwrap().remove(0);
    let traffic = card.traffic.expect("Ampel aus dem Abschlag");
    assert_eq!(traffic.source, BandSource::Installment);
    assert_eq!(
        traffic.month,
        zaehler_domain::period::Period::month_of(day(2026, 2, 1))
    );
    assert_eq!(traffic.status, Some(Status::Green));

    // Abschlagsänderung mitten im laufenden Abrechnungszeitraum
    uc.set_installment
        .execute(&t, c, inst(day(2026, 5, 15), dec!(120)))
        .await
        .unwrap();
    let view = uc.meter_costs.execute(&t, strom).await.unwrap();
    let ic = view.import.as_ref().expect("Bezugsvertrag");
    assert_eq!(ic.period, Some(zaehler_domain::period::Period::year(2026)));
    let b = ic.breakdown.as_ref().expect("Kosten");
    assert_eq!(b.measured(), dec!(206.5) + dec!(150) * dec!(59) / dec!(365));
    assert!(b.has_estimate());
    // 3650 kWh × 0,35 + 150; das aus 14 Monaten gelernte Saisonprofil weicht
    // durch die sekundengenauen Monatswerte (Zeitumstellung) minimal ab
    let total = b.total();
    assert!((total - dec!(1427.5)).abs() < dec!(0.5), "{total}");
    assert!((ic.quantity.unwrap().value - dec!(3650)).abs() < dec!(1));
    assert_eq!(ic.unit_price().map(|p| p.round_dp(6)), Some(dec!(0.35)));
    let st = ic.installments.as_ref().unwrap();
    assert_eq!(st.paid, dec!(220), "15.01. und 15.02.");
    assert_eq!(
        st.open,
        dec!(220) + dec!(960),
        "März, April zu 110, Mai bis Dezember zu 120"
    );
    assert_eq!(st.balance, Some(total - dec!(1400)));
    assert_eq!(
        st.suggestion,
        Some(dec!(119)),
        "rund 1427 € / 12 aufgerundet"
    );
    assert_eq!(st.current.as_ref().unwrap().installment.amount, dec!(110));
    let year = view.year_import.unwrap();
    assert_eq!(year.total(), total);
    let months: Decimal = view.months.iter().map(|m| m.measured + m.estimated).sum();
    assert!((months - year.total()).abs() < dec!(0.000001));

    // Bonus senkt die Kosten des Zeitraums
    uc.add_adjustment
        .execute(
            &t,
            c,
            AdjustmentInput {
                kind: AdjustmentKind::Bonus,
                booked_on: day(2026, 2, 1),
                amount: dec!(50),
                note: Some("Treuebonus".into()),
            },
        )
        .await
        .unwrap();
    // tatsächliche Abrechnung 2025
    uc.record_settlement
        .execute(
            &t,
            c,
            SettlementInput {
                period_start: day(2025, 1, 1),
                period_end: day(2025, 12, 31),
                amount: dec!(1270),
            },
        )
        .await
        .unwrap();
    let view = uc.meter_costs.execute(&t, strom).await.unwrap();
    assert_eq!(
        view.import
            .as_ref()
            .unwrap()
            .breakdown
            .as_ref()
            .unwrap()
            .total(),
        total - dec!(50)
    );
    let check = &view.settlements[0];
    let calculated = check.calculated.as_ref().unwrap();
    assert_eq!(calculated.total(), dec!(1215));
    assert!(!calculated.has_estimate());
    assert_eq!(check.difference, Some(dec!(55)));
    assert_eq!(check.difference_rel, Some(dec!(0.0433)));
    assert_eq!(check.forecast_on, day(2025, 7, 1));
    assert_eq!(
        check.forecast,
        Some(dec!(1215)),
        "Rate aus dem ersten Halbjahr"
    );

    // Einträge löschen
    let id = view.import.unwrap().contract.adjustments[0].id;
    uc.delete_contract_entry
        .execute(&t, c, ContractEntry::Adjustment(id))
        .await
        .unwrap();
    assert!(matches!(
        uc.delete_contract_entry
            .execute(&t, c, ContractEntry::Adjustment(id))
            .await,
        Err(AppError::NotFound)
    ));
}

/// Haushaltsarchiv: Verträge mit allen Einträgen erhalten neue IDs und
/// verweisen auf die neuen Register.
// @test SPEC_BACKUP_HOUSEHOLD, SPEC_TAR_CONTRACTS
#[tokio::test]
async fn household_archive_keeps_contracts() {
    use std::io::Cursor;
    let (uc, t, _) = setup().await;
    let strom = uc
        .create_meter
        .execute(
            &t,
            meter_input(
                "Strom",
                MeterKind::Electricity,
                &[("HT", Direction::Import), ("NT", Direction::Import)],
                berlin(2025, 12, 1),
            ),
        )
        .await
        .unwrap();
    let ht = register(&uc, &t, strom, "HT").await;
    let from = day(2026, 1, 1);
    let mut tiered = flat(from, dec!(150), dec!(0));
    tiered.unit_pricing = UnitPricing::Tiered {
        tiers: vec![
            Tier {
                up_to: Some(dec!(1000)),
                price: dec!(0.3),
            },
            Tier {
                up_to: None,
                price: dec!(0.32),
            },
        ],
    };
    let c = uc
        .create_contract
        .execute(
            &t,
            strom,
            contract(from, vec![position("HT", Some(ht), vec![tiered])]),
        )
        .await
        .unwrap();
    uc.set_installment
        .execute(
            &t,
            c,
            InstallmentInput {
                valid_from: from,
                amount: dec!(80),
                every_months: 2,
            },
        )
        .await
        .unwrap();
    let (archive, _) = uc
        .export_household
        .execute(&t, Cursor::new(Vec::new()))
        .await
        .unwrap();
    let (copy, _) = uc
        .import_household
        .execute(&t, Cursor::new(archive.into_inner()))
        .await
        .unwrap();
    let copy = uc
        .resolve_tenant
        .execute(t.user_id(), Some(copy))
        .await
        .unwrap()
        .tenant;
    let meter = uc.list_meters.execute(&copy).await.unwrap()[0].meter.id;
    let contracts = uc.meter_contracts.execute(&copy, meter).await.unwrap();
    let original = uc.get_contract.execute(&t, c).await.unwrap().contract;
    let [imported] = contracts.as_slice() else {
        panic!("ein Vertrag erwartet");
    };
    assert_ne!(imported.id, original.id);
    assert_eq!(
        imported.data.positions[0].register_id,
        Some(register(&uc, &copy, meter, "HT").await)
    );
    assert_eq!(
        imported.data.positions[0].components[0].component,
        original.data.positions[0].components[0].component
    );
    assert_eq!(
        imported.installments[0].installment,
        original.installments[0].installment
    );
}

/// Wärmepumpe als Zwischenzähler: Kosten über den Arbeitspreis der
/// übergeordneten Messstelle ohne Grundpreis; die Haushaltssumme zählt sie
/// nicht doppelt.
// @test SPEC_TAR_SUBMETER_COST, SPEC_TAR_COST_VIEW
#[tokio::test]
async fn heat_pump_submeter_costs_without_base_price() {
    let (uc, t, clock) = setup().await;
    let start = berlin(2026, 1, 1);
    let house = uc
        .create_meter
        .execute(
            &t,
            meter_input(
                "Hausstrom",
                MeterKind::Electricity,
                &[("main", Direction::Import)],
                start,
            ),
        )
        .await
        .unwrap();
    let mut wp = meter_input(
        "Wärmepumpe",
        MeterKind::Electricity,
        &[("main", Direction::Import)],
        start,
    );
    wp.purpose = MeterPurpose::HeatPump;
    wp.parent_meter_id = Some(house);
    let wp = uc.create_meter.execute(&t, wp).await.unwrap();
    set_today(&clock, day(2026, 3, 10));
    record(&uc, &t, house, berlin(2026, 3, 1), dec!(1180)).await;
    record(&uc, &t, wp, berlin(2026, 3, 1), dec!(295)).await;
    uc.create_contract
        .execute(
            &t,
            house,
            contract(
                day(2026, 1, 1),
                vec![position(
                    "Strom",
                    None,
                    vec![flat(day(2026, 1, 1), dec!(120), dec!(0.30))],
                )],
            ),
        )
        .await
        .unwrap();

    let parent = uc.meter_costs.execute(&t, house).await.unwrap();
    assert_eq!(
        parent
            .import
            .as_ref()
            .unwrap()
            .breakdown
            .as_ref()
            .unwrap()
            .total(),
        dec!(2310)
    );
    let share = &parent.submeters[0];
    assert_eq!(share.meter_id, wp);
    assert_eq!(share.quantity.value, dec!(1825));
    assert!(share.quantity.estimated);
    assert_eq!(share.cost(), dec!(547.5));

    let own = uc.meter_costs.execute(&t, wp).await.unwrap();
    assert!(own.import.is_none());
    let (parent_name, share) = own.parent_share.expect("über die übergeordnete Messstelle");
    assert_eq!(parent_name, "Hausstrom");
    assert_eq!(share.cost(), dec!(547.5), "ohne Grundpreis");

    let household = uc.household_costs.execute(&t, None).await.unwrap();
    assert_eq!(household.year, 2026);
    assert_eq!(household.rows.len(), 2);
    assert_eq!(household.import_total(), dec!(2310));
    let wp_row = household.rows.iter().find(|r| r.meter_id == wp).unwrap();
    let (name, value) = wp_row.included_in.as_ref().unwrap();
    assert_eq!((name.as_str(), value.value), ("Hausstrom", dec!(547.5)));
    assert!(household.has_estimate());
}
