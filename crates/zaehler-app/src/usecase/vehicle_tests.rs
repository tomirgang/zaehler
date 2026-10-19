//! Tests: Fahrzeuge, Kostenbuch, Laden und Fahrzeugauswertung
//! (SPEC_CAR_VEHICLES, SPEC_ASSET_COSTS, SPEC_CAR_ENERGY_COST,
//! SPEC_CAR_REPORT).

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use zaehler_domain::{
    assets::{AssetCategory, Recurrence, VehicleCostCategory},
    meter::{Direction, MeterKind, MeterPurpose},
    period::Period,
    vehicle::{ConsumptionProfile, DriveType, Holding},
};

use super::contract_tests::{
    berlin, conflict, contract, day, field, flat, meter_input, position, record, setup,
};
use crate::{
    AppError, Tenant,
    ids::MeterId,
    model::{AssetRef, ChargingKind},
    usecase::{
        UseCases,
        asset_cost::AssetCostInput,
        vehicle::{ChargingInput, NewVehicle, SaveOutcome, VehicleInput},
    },
};

fn input(drive: DriveType, home: Option<MeterId>) -> VehicleInput {
    VehicleInput {
        plate: "M-ZA 1".into(),
        model: "Kombi".into(),
        drive_type: drive,
        home_charging_meter_id: home,
        pv_share_pct: None,
        home_charging_price: None,
    }
}

async fn vehicle(
    uc: &UseCases,
    t: &Tenant,
    name: &str,
    drive: DriveType,
    home: Option<MeterId>,
    odometer: Decimal,
) -> MeterId {
    uc.create_vehicle
        .execute(
            t,
            NewVehicle {
                name: name.into(),
                vehicle: input(drive, home),
                odometer,
                start_at: berlin(2025, 1, 1),
            },
        )
        .await
        .unwrap()
}

fn cost(on: NaiveDate, category: VehicleCostCategory, amount: Decimal) -> AssetCostInput {
    AssetCostInput {
        booked_on: on,
        category: AssetCategory::Vehicle(category),
        amount: Some(amount),
        quantity: None,
        unit_price: None,
        odometer: None,
        full_tank: false,
        recurrence: None,
        note: None,
        odometer_as_reading: false,
        confirm_warnings: false,
    }
}

// @test SPEC_CAR_FUEL_PRICE_UI, SPEC_CAR_FUEL_PRICE
#[tokio::test]
async fn fuel_price_is_preserved_and_fuel_profile_supports_combustion() {
    use zaehler_domain::vehicle::ProfileEnergy;
    let (uc, t, _) = setup().await;
    let car = vehicle(
        &uc,
        &t,
        "Benziner",
        DriveType::Combustion,
        None,
        dec!(20000),
    )
    .await;
    uc.set_consumption_profile
        .execute(
            &t,
            car,
            ConsumptionProfile {
                valid_from: day(2025, 1, 1),
                kwh_per_100km: dec!(6.5),
                charging_loss_pct: Decimal::ZERO,
                electric_share_pct: None,
                energy: ProfileEnergy::Fuel,
            },
        )
        .await
        .unwrap();
    let mut entry = cost(day(2025, 3, 1), VehicleCostCategory::Fuel, dec!(0));
    entry.amount = None;
    entry.quantity = Some(dec!(45.32));
    entry.unit_price = Some(dec!(1.789));
    let id = saved(
        uc.add_asset_cost
            .execute(&t, AssetRef::vehicle(car), entry.clone())
            .await,
    );
    let stored = uc.get_asset_cost.execute(&t, id).await.unwrap();
    assert_eq!(stored.amount, dec!(81.08));
    assert_eq!(stored.unit_price, Some(dec!(1.789)));
    assert_eq!(stored.fuel_unit_price(), Some(dec!(1.789)));
    let legacy = crate::model::AssetCost {
        unit_price: None,
        ..stored
    };
    assert_eq!(legacy.fuel_unit_price(), Some(dec!(1.789)));
    entry.amount = Some(dec!(81.10));
    field(
        uc.update_asset_cost.execute(&t, id, entry).await,
        "unit_price",
        "asset_cost.price_mismatch",
    );
    let mut future = cost(day(2099, 1, 1), VehicleCostCategory::Fuel, dec!(20));
    future.quantity = Some(dec!(10));
    future.unit_price = Some(dec!(2));
    saved(
        uc.add_asset_cost
            .execute(&t, AssetRef::vehicle(car), future)
            .await,
    );
    let report = uc.vehicle_report.execute(&t, car).await.unwrap();
    assert_eq!(report.fuel_price.as_ref().unwrap().points.len(), 1);
    assert_eq!(
        report.fuel_price.unwrap().last,
        Some((day(2025, 3, 1), dec!(1.789)))
    );
}

fn fuel(on: NaiveDate, litres: Decimal, amount: Decimal, odometer: Decimal) -> AssetCostInput {
    AssetCostInput {
        quantity: Some(litres),
        odometer: Some(odometer),
        full_tank: true,
        ..cost(on, VehicleCostCategory::Fuel, amount)
    }
}

// @test SPEC_CAR_FUEL_PRICE
#[tokio::test]
async fn fuel_forecast_without_quantities_is_marked_as_estimated() {
    use zaehler_domain::vehicle::ProfileEnergy;
    let (uc, t, clock) = setup().await;
    let car = vehicle(&uc, &t, "Auto", DriveType::Combustion, None, dec!(10000)).await;
    record(&uc, &t, car, berlin(2026, 1, 1), dec!(22000)).await;
    super::contract_tests::set_today(&clock, day(2026, 7, 2));
    record(&uc, &t, car, berlin(2026, 7, 2), dec!(28000)).await;
    uc.set_consumption_profile
        .execute(
            &t,
            car,
            ConsumptionProfile {
                valid_from: day(2025, 1, 1),
                kwh_per_100km: dec!(6.5),
                charging_loss_pct: Decimal::ZERO,
                electric_share_pct: None,
                energy: ProfileEnergy::Fuel,
            },
        )
        .await
        .unwrap();
    saved(
        uc.add_asset_cost
            .execute(
                &t,
                AssetRef::vehicle(car),
                cost(day(2026, 5, 1), VehicleCostCategory::Fuel, dec!(300)),
            )
            .await,
    );
    let report = uc.vehicle_report.execute(&t, car).await.unwrap();
    let price = report.fuel_price.unwrap();
    assert!(price.last.is_none() && price.points.is_empty());
    assert!(price.estimate.unwrap() > Decimal::ZERO);
    let forecast = report.forecast.unwrap();
    let fuel = forecast.fuel_cost.unwrap();
    assert!(fuel.estimated && fuel.value > Decimal::ZERO);
    assert_eq!(
        forecast.expected,
        Some((dec!(300) + fuel.value).round_dp(2))
    );
}

fn charging(at: DateTime<Utc>, kwh: Decimal, amount: Decimal) -> ChargingInput {
    ChargingInput {
        charged_at: at,
        energy: kwh,
        amount,
        odometer: None,
        provider: Some("Ladesäule".into()),
        kind: ChargingKind::Ac,
        odometer_as_reading: false,
        confirm_warnings: false,
    }
}

fn saved<T: std::fmt::Debug>(r: Result<SaveOutcome<T>, AppError>) -> T {
    match r {
        Ok(SaveOutcome::Saved(id)) => id,
        other => panic!("gespeichert erwartet, war {other:?}"),
    }
}

// @test SPEC_CAR_VEHICLES, SPEC_ASSET_COSTS
#[tokio::test]
async fn drive_type_controls_entries_and_locks_after_first_booking() {
    let (uc, t, _) = setup().await;
    let car = vehicle(
        &uc,
        &t,
        "Benziner",
        DriveType::Combustion,
        None,
        dec!(10000),
    )
    .await;
    let detail = uc.get_vehicle.execute(&t, car).await.unwrap();
    assert_eq!(detail.meter.meter.data.kind, MeterKind::Vehicle);
    assert_eq!(detail.meter.meter.data.unit, "km");
    assert_eq!(detail.meter.registers[0].data.code, "km");
    assert_eq!(uc.list_vehicles.execute(&t).await.unwrap().len(), 1);

    // Verbrenner: kein Laden, keine Kategorie Laden, keine Heimlade-Messstelle
    field(
        uc.record_charging
            .execute(&t, car, charging(berlin(2025, 2, 1), dec!(10), dec!(5)))
            .await,
        "drive_type",
        "vehicle.no_charging",
    );
    field(
        uc.add_asset_cost
            .execute(
                &t,
                AssetRef::vehicle(car),
                cost(day(2025, 2, 1), VehicleCostCategory::Charging, dec!(5)),
            )
            .await,
        "category",
        "vehicle.category_not_allowed",
    );
    let (house, _) = house_with_wallbox(&uc, &t).await;
    field(
        uc.update_vehicle
            .execute(&t, car, input(DriveType::Combustion, Some(house)))
            .await,
        "home_charging_meter_id",
        "vehicle.home_charging_invalid",
    );
    // Wechsel vor der ersten Buchung möglich, danach gesperrt
    uc.update_vehicle
        .execute(&t, car, input(DriveType::Electric, None))
        .await
        .unwrap();
    field(
        uc.add_asset_cost
            .execute(
                &t,
                AssetRef::vehicle(car),
                fuel(day(2025, 2, 1), dec!(40), dec!(70), dec!(10500)),
            )
            .await,
        "category",
        "vehicle.category_not_allowed",
    );
    saved(
        uc.add_asset_cost
            .execute(
                &t,
                AssetRef::vehicle(car),
                cost(day(2025, 2, 1), VehicleCostCategory::Tax, dec!(120)),
            )
            .await,
    );
    conflict(
        uc.update_vehicle
            .execute(&t, car, input(DriveType::Combustion, None))
            .await,
        "vehicle.drive_type_locked",
    );
    // gleiche Antriebsart bleibt änderbar
    let mut renamed = input(DriveType::Electric, None);
    renamed.plate = "M-ZA 2".into();
    uc.update_vehicle.execute(&t, car, renamed).await.unwrap();
    // Messstelle ohne Fahrzeugart
    assert!(matches!(
        uc.update_vehicle
            .execute(&t, house, input(DriveType::Electric, None))
            .await,
        Err(AppError::NotFound)
    ));
}

// @test SPEC_ASSET_COSTS
#[tokio::test]
async fn recurring_insurance_appears_every_year_until_its_end() {
    let (uc, t, _) = setup().await;
    let car = vehicle(&uc, &t, "Auto", DriveType::Combustion, None, dec!(10000)).await;
    let mut insurance = cost(day(2025, 3, 1), VehicleCostCategory::Insurance, dec!(480));
    insurance.recurrence = Some(Recurrence {
        every_months: 12,
        until: Some(day(2027, 12, 31)),
    });
    let id = saved(
        uc.add_asset_cost
            .execute(&t, AssetRef::vehicle(car), insurance.clone())
            .await,
    );
    for (year, n) in [(2025, 1), (2026, 1), (2027, 1), (2028, 0)] {
        let book = uc
            .list_asset_costs
            .execute(&t, AssetRef::vehicle(car), year)
            .await
            .unwrap();
        assert_eq!(book.entries.len(), n, "{year}");
        assert_eq!(book.total, dec!(480) * Decimal::from(n));
    }
    // monatlich: 12 Buchungen im Jahr
    let mut monthly = insurance.clone();
    monthly.recurrence = Some(Recurrence {
        every_months: 1,
        until: None,
    });
    uc.update_asset_cost.execute(&t, id, monthly).await.unwrap();
    let book = uc
        .list_asset_costs
        .execute(&t, AssetRef::vehicle(car), 2026)
        .await
        .unwrap();
    assert_eq!(book.total, dec!(5760));
    assert_eq!(
        book.totals,
        [(
            AssetCategory::Vehicle(VehicleCostCategory::Insurance),
            dec!(5760)
        )]
    );
    // ungültiger Turnus und Ende vor der Buchung
    let mut bad = insurance;
    bad.recurrence = Some(Recurrence {
        every_months: 0,
        until: Some(day(2024, 1, 1)),
    });
    let r = uc
        .add_asset_cost
        .execute(&t, AssetRef::vehicle(car), bad.clone())
        .await;
    field(r, "recurrence_months", "asset.recurrence_invalid");
    field(
        uc.add_asset_cost
            .execute(&t, AssetRef::vehicle(car), bad)
            .await,
        "recurrence_until",
        "asset.until_before_booking",
    );
    let asset = uc.delete_asset_cost.execute(&t, id).await.unwrap();
    assert_eq!(asset, AssetRef::vehicle(car));
}

// @test SPEC_ASSET_COSTS, SPEC_RDG_PLAUSIBILITY
#[tokio::test]
async fn odometer_is_taken_over_as_reading_after_confirmation() {
    let (uc, t, _) = setup().await;
    let car = vehicle(&uc, &t, "Auto", DriveType::Combustion, None, dec!(10000)).await;
    let mut fill = fuel(day(2025, 6, 1), dec!(40), dec!(70), dec!(9000));
    fill.odometer_as_reading = true;
    // Rückgang: Warnung, nichts gespeichert
    let out = uc
        .add_asset_cost
        .execute(&t, AssetRef::vehicle(car), fill.clone())
        .await
        .unwrap();
    assert!(matches!(out, SaveOutcome::NeedsConfirmation(_)));
    let book = |y| uc.list_asset_costs.execute(&t, AssetRef::vehicle(car), y);
    assert!(book(2025).await.unwrap().entries.is_empty());
    // plausibler Stand
    fill.odometer = Some(dec!(12000));
    saved(
        uc.add_asset_cost
            .execute(&t, AssetRef::vehicle(car), fill)
            .await,
    );
    let readings = uc.get_meter.execute(&t, car).await.unwrap().readings;
    assert_eq!(readings.len(), 2);
    assert_eq!(readings[1].value, dec!(12000));
    assert_eq!(book(2025).await.unwrap().entries.len(), 1);
}

/// Hausstrom mit Vertrag (Grundpreis 120 €/Jahr, 0,30 €/kWh) und
/// Wallbox-Zwischenzähler.
async fn house_with_wallbox(uc: &UseCases, t: &Tenant) -> (MeterId, MeterId) {
    let house = uc
        .create_meter
        .execute(
            t,
            meter_input(
                "Haus",
                MeterKind::Electricity,
                &[("1.8.0", Direction::Import)],
                berlin(2025, 1, 1),
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
    wb.parent_meter_id = Some(house);
    let wallbox = uc.create_meter.execute(t, wb).await.unwrap();
    uc.create_contract
        .execute(
            t,
            house,
            contract(
                day(2025, 1, 1),
                vec![position(
                    "Strom",
                    None,
                    vec![flat(day(2025, 1, 1), dec!(120), dec!(0.30))],
                )],
            ),
        )
        .await
        .unwrap();
    (house, wallbox)
}

// @test SPEC_CAR_ENERGY_COST, SPEC_CAR_REPORT, SPEC_CAR_COST_PER_KM
#[tokio::test]
async fn electric_vehicle_with_wallbox_meter_and_external_charging() {
    let (uc, t, _) = setup().await;
    let (house, wallbox) = house_with_wallbox(&uc, &t).await;
    let car = vehicle(
        &uc,
        &t,
        "Stromer",
        DriveType::Electric,
        Some(wallbox),
        dec!(10000),
    )
    .await;
    record(&uc, &t, house, berlin(2026, 1, 1), dec!(3000)).await;
    record(&uc, &t, wallbox, berlin(2026, 1, 1), dec!(900)).await;
    record(&uc, &t, car, berlin(2026, 1, 1), dec!(16000)).await;
    // kostenloses und bezahltes externes Laden
    saved(
        uc.record_charging
            .execute(&t, car, charging(berlin(2025, 2, 10), dec!(100), dec!(50)))
            .await,
    );
    saved(
        uc.record_charging
            .execute(&t, car, charging(berlin(2025, 5, 10), dec!(20), dec!(0)))
            .await,
    );

    let r = uc.vehicle_report.execute(&t, car).await.unwrap();
    let home = r.home.as_ref().unwrap();
    assert!(home.wallbox);
    assert_eq!(home.price_meter.as_deref(), Some("Haus"));
    let y = r.years.iter().find(|y| y.year == Some(2025)).unwrap();
    assert_eq!(y.period, Period::year(2025));
    assert_eq!(y.km.unwrap().value, dec!(6000));
    let e = y.energy.as_ref().unwrap();
    assert_eq!((e.external.kwh, e.external.cost), (dec!(120), dec!(50)));
    // 900 kWh × 0,30 €/kWh, ohne Grundpreis des Hausstroms; gemessen
    let home_cost = e.home_cost.unwrap();
    assert_eq!(home_cost.value, dec!(270));
    assert!(!home_cost.estimated && !y.has_estimate());
    assert_eq!(e.kwh_per_100km.unwrap().value, dec!(17));
    assert!(!e.kwh_per_100km.unwrap().estimated);
    assert_eq!(y.categories, [(VehicleCostCategory::Charging, dec!(320))]);
    // 320 € / 6000 km
    assert_eq!(y.cost.variable_per_km, Some(dec!(0.0533)));

    // Hausstrom-Auswertung ohne Wallbox-Anteil: 3000 − 900 kWh
    let costs = uc.meter_costs.execute(&t, house).await.unwrap();
    assert_eq!(costs.submeters.len(), 1);
    assert_eq!(costs.submeters[0].quantity.value, dec!(900));
}

// @test SPEC_CAR_ENERGY_COST, SPEC_CAR_REPORT
#[tokio::test]
async fn estimate_without_submeter_is_marked() {
    let (uc, t, _) = setup().await;
    let (house, _) = house_with_wallbox(&uc, &t).await;
    record(&uc, &t, house, berlin(2026, 1, 1), dec!(3000)).await;
    let car = vehicle(
        &uc,
        &t,
        "Stromer",
        DriveType::Electric,
        Some(house),
        dec!(20000),
    )
    .await;
    record(&uc, &t, car, berlin(2026, 1, 1), dec!(25000)).await;
    uc.set_consumption_profile
        .execute(
            &t,
            car,
            ConsumptionProfile {
                valid_from: day(2025, 1, 1),
                kwh_per_100km: dec!(18),
                charging_loss_pct: dec!(10),
                electric_share_pct: Some(dec!(50)),
                energy: zaehler_domain::vehicle::ProfileEnergy::Electric,
            },
        )
        .await
        .unwrap();
    saved(
        uc.record_charging
            .execute(&t, car, charging(berlin(2025, 2, 10), dec!(100), dec!(50)))
            .await,
    );
    let r = uc.vehicle_report.execute(&t, car).await.unwrap();
    assert!(!r.home.as_ref().unwrap().wallbox);
    assert_eq!(
        r.vehicle.profiles[0].profile.electric_share_pct, None,
        "nur Plug-in-Hybrid"
    );
    let y = r.years.iter().find(|y| y.year == Some(2025)).unwrap();
    let e = y.energy.as_ref().unwrap();
    // 5000 km × 18 kWh/100 km × 1,1 = 990 kWh, davon 100 extern
    assert_eq!(e.home_kwh.unwrap().value, dec!(890));
    assert!(e.home_kwh.unwrap().estimated && y.has_estimate());
    assert_eq!(e.home_cost.unwrap().value, dec!(267));
    assert!(e.kwh_per_100km.unwrap().estimated, "hinterlegter Verbrauch");
}

// @test SPEC_CAR_FUEL_CONSUMPTION, SPEC_CAR_COST_PER_KM, SPEC_CAR_DEPRECIATION, SPEC_CAR_REPORT
#[tokio::test]
async fn combustion_report_with_full_tank_method_and_sale() {
    let (uc, t, _) = setup().await;
    let car = vehicle(
        &uc,
        &t,
        "Benziner",
        DriveType::Combustion,
        None,
        dec!(10000),
    )
    .await;
    record(&uc, &t, car, berlin(2026, 1, 1), dec!(20000)).await;
    for (on, litres, amount, km) in [
        (day(2025, 3, 1), dec!(40), dec!(70), dec!(10000)),
        (day(2025, 3, 20), dec!(45), dec!(80), dec!(10750)),
    ] {
        saved(
            uc.add_asset_cost
                .execute(&t, AssetRef::vehicle(car), fuel(on, litres, amount, km))
                .await,
        );
    }
    saved(
        uc.add_asset_cost
            .execute(
                &t,
                AssetRef::vehicle(car),
                cost(day(2025, 4, 1), VehicleCostCategory::Insurance, dec!(600)),
            )
            .await,
    );
    // Verkauf am 10.01.2026 für 22 000 € (Restwert 10 000 € ersetzt)
    uc.set_acquisition
        .execute(
            &t,
            car,
            Some(Holding {
                purchased_on: day(2025, 1, 1),
                price: dec!(30000),
                mode: zaehler_domain::vehicle::DepreciationMode::Time,
                first_registration: None,
                residual_value: Some(dec!(10000)),
                planned_end: Some(day(2030, 1, 1)),
                odometer_at_purchase: None,
                lifetime_km: None,
                end_value: None,
                sold_on: Some(day(2026, 1, 10)),
                sale_price: Some(dec!(22000)),
            }),
        )
        .await
        .unwrap();
    let r = uc.vehicle_report.execute(&t, car).await.unwrap();
    assert!(r.forecast.is_none(), "verkauft");
    assert_eq!(r.fuel_segments.len(), 1);
    assert_eq!(r.fuel_segments[0].per_100km, dec!(6));
    let y = r.years.iter().find(|y| y.year == Some(2025)).unwrap();
    assert_eq!(y.fuel_per_100km, Some(dec!(6)));
    assert!(y.energy.is_none());
    // 8000 € Wertverlust bis zum Verkauf nach 374 Tagen, nach der
    // Restwertkurve verteilt: 365 Tage in 2025
    let curve = |days: i64| {
        Decimal::ONE - zaehler_domain::vehicle::residual_ratio(Decimal::from(days) / dec!(365.25))
    };
    let dep = (dec!(8000) * curve(365) / curve(374)).round_dp(2);
    assert_eq!(dep, dec!(7879.17));
    assert_eq!(y.cost.depreciation, dep);
    // variabel 150 € / 10 000 km, Vollkosten (150 + 600 + Wertverlust) / km
    assert_eq!(y.cost.variable_per_km, Some(dec!(0.015)));
    assert_eq!(
        y.cost.total_per_km,
        Some(((dec!(750) + dep) / dec!(10000)).round_dp(4))
    );
    let h = r.holding.as_ref().unwrap();
    assert_eq!(h.period, Period::new(day(2025, 1, 1), day(2026, 1, 10)));
    assert_eq!(h.cost.depreciation, dec!(8000));
}

// @test SPEC_CAR_REPORT, SPEC_FC_PERIOD_FORECAST
#[tokio::test]
async fn forecast_of_mileage_and_annual_costs() {
    let (uc, t, clock) = setup().await;
    let car = vehicle(&uc, &t, "Auto", DriveType::Combustion, None, dec!(10000)).await;
    record(&uc, &t, car, berlin(2026, 1, 1), dec!(22000)).await;
    super::contract_tests::set_today(&clock, day(2026, 7, 2));
    record(&uc, &t, car, berlin(2026, 7, 2), dec!(28000)).await;
    let mut insurance = cost(day(2025, 3, 1), VehicleCostCategory::Insurance, dec!(600));
    insurance.recurrence = Some(Recurrence {
        every_months: 12,
        until: None,
    });
    saved(
        uc.add_asset_cost
            .execute(&t, AssetRef::vehicle(car), insurance)
            .await,
    );
    saved(
        uc.add_asset_cost
            .execute(
                &t,
                AssetRef::vehicle(car),
                fuel(day(2026, 2, 1), dec!(50), dec!(1200), dec!(23000)),
            )
            .await,
    );
    let r = uc.vehicle_report.execute(&t, car).await.unwrap();
    let f = r.forecast.unwrap();
    assert_eq!(f.year, 2026);
    // 6000 km im ersten Halbjahr: rund 12 000 km im Jahr
    assert!(f.km.estimated);
    assert!(
        (f.km.value - dec!(12000)).abs() < dec!(100),
        "{}",
        f.km.value
    );
    assert_eq!(f.fixed, dec!(600));
    let v = f.variable_per_km.unwrap();
    assert!(v > Decimal::ZERO);
    assert_eq!(f.expected, Some((dec!(600) + f.km.value * v).round_dp(2)));
}

/// Fahrzeug mit erster Ablesung am `start` (Startstand `odometer`).
async fn vehicle_from(
    uc: &UseCases,
    t: &Tenant,
    start: DateTime<Utc>,
    odometer: Decimal,
) -> MeterId {
    uc.create_vehicle
        .execute(
            t,
            NewVehicle {
                name: "Fiat 500e".into(),
                vehicle: input(DriveType::Electric, None),
                odometer,
                start_at: start,
            },
        )
        .await
        .unwrap()
}

/// Die Fahrleistung des laufenden Jahres ist dieselbe Hochrechnung wie in
/// der Auswertung der Messstelle, auch wenn erst im Jahr gemessen wird.
// @test SPEC_CAR_REPORT, SPEC_FC_CURRENT_YEAR
#[tokio::test]
async fn vehicle_forecast_matches_the_meter_analysis() {
    let (uc, t, clock) = setup().await;
    let car = vehicle_from(&uc, &t, berlin(2026, 4, 27), dec!(1000)).await;
    super::contract_tests::set_today(&clock, day(2026, 10, 19));
    record(&uc, &t, car, berlin(2026, 10, 19), dec!(6258)).await;

    let f = uc
        .vehicle_report
        .execute(&t, car)
        .await
        .unwrap()
        .forecast
        .unwrap();
    let analysis = uc.meter_analysis.execute(&t, car, None).await.unwrap();
    let year = analysis
        .forecasts
        .iter()
        .find(|s| s.scope == super::analysis::ForecastScope::CalendarYear)
        .and_then(|s| s.forecast.projected())
        .unwrap();
    assert!(year.estimated_start(), "Januar bis April geschätzt");
    assert_eq!(f.km.value, year.projected_total.round_dp(0));
    // nur ab der ersten Ablesung wären es rund 8600 km
    assert!(f.km.value > dec!(9500), "{}", f.km.value);
}

/// Laufleistungsmodus mit Kauf lange vor der ersten Ablesung: die km bis
/// zur ersten Ablesung werden gleichmäßig auf die Haltedauer verteilt.
// @test SPEC_CAR_MILEAGE_DEPRECIATION, SPEC_CAR_REPORT
#[tokio::test]
async fn mileage_depreciation_spreads_km_before_the_first_reading() {
    let (uc, t, clock) = setup().await;
    let car = vehicle_from(&uc, &t, berlin(2026, 4, 27), dec!(150000)).await;
    super::contract_tests::set_today(&clock, day(2026, 10, 19));
    record(&uc, &t, car, berlin(2026, 10, 19), dec!(155258)).await;
    uc.set_acquisition
        .execute(
            &t,
            car,
            Some(Holding {
                purchased_on: day(2016, 1, 1),
                price: dec!(25000),
                mode: zaehler_domain::vehicle::DepreciationMode::Mileage,
                first_registration: Some(day(2016, 1, 1)),
                residual_value: None,
                planned_end: None,
                odometer_at_purchase: Some(dec!(15000)),
                lifetime_km: None,
                end_value: None,
                sold_on: None,
                sale_price: None,
            }),
        )
        .await
        .unwrap();
    let r = uc.vehicle_report.execute(&t, car).await.unwrap();
    let year = |y: i32| {
        r.years
            .iter()
            .find(|p| p.year == Some(y))
            .map_or(Decimal::ZERO, |p| p.cost.depreciation)
    };
    let total = r.holding.as_ref().unwrap().cost.depreciation;
    for y in 2016..2026 {
        assert!(year(y) > Decimal::ZERO, "Wertverlust {y}");
    }
    assert!(year(2016) > year(2020), "größter Verlust im ersten Jahr");
    assert!(
        year(2026) * dec!(5) < total,
        "2026: {} von {total}",
        year(2026)
    );
    let sum: Decimal = (2016..=2026).map(year).sum();
    assert!((sum - total).abs() <= dec!(0.1), "{sum} ≠ {total}");
    // km vor der ersten Ablesung gleichmäßig verteilt, damit auch Kosten
    // pro km: 135.000 km über 3769 Tage, 2020 hat 366 Tage
    let y2020 = r.years.iter().find(|p| p.year == Some(2020)).unwrap();
    let km = y2020.km.unwrap();
    assert!(km.estimated);
    assert_eq!(km.value.round_dp(0), dec!(13110));
    assert!(y2020.cost.total_per_km.is_some(), "Kosten pro km 2020");
    let km_total: Decimal = r.years.iter().filter_map(|p| p.km.map(|k| k.value)).sum();
    // 15.000 → 155.258 km, dazu der laufende Tag nach der letzten Ablesung
    // (rund 30 km/Tag)
    assert!((km_total - dec!(140258)).abs() < dec!(50), "{km_total}");
}

/// Elektroauto lädt am Hausstrom ohne Vertrag: kWh aus den gefahrenen km
/// und dem Profil abzüglich Fremdladen, Kosten mit dem Preis am Fahrzeug.
// @test SPEC_CAR_ENERGY_COST
#[tokio::test]
async fn home_charging_without_contract_uses_the_vehicle_price() {
    use zaehler_domain::vehicle::ProfileEnergy;
    let (uc, t, clock) = setup().await;
    let house = uc
        .create_meter
        .execute(
            &t,
            meter_input(
                "Strom",
                MeterKind::Electricity,
                &[("1.8.0", Direction::Import)],
                berlin(2025, 1, 1),
            ),
        )
        .await
        .unwrap();
    let car = uc
        .create_vehicle
        .execute(
            &t,
            NewVehicle {
                name: "Fiat 500e".into(),
                vehicle: VehicleInput {
                    home_charging_price: Some(dec!(0.30)),
                    ..input(DriveType::Electric, Some(house))
                },
                odometer: dec!(10000),
                start_at: berlin(2025, 1, 1),
            },
        )
        .await
        .unwrap();
    uc.set_consumption_profile
        .execute(
            &t,
            car,
            ConsumptionProfile {
                valid_from: day(2025, 1, 1),
                kwh_per_100km: dec!(16),
                charging_loss_pct: dec!(20),
                electric_share_pct: None,
                energy: ProfileEnergy::Electric,
            },
        )
        .await
        .unwrap();
    saved(
        uc.record_charging
            .execute(&t, car, charging(berlin(2025, 3, 1), dec!(100), dec!(60)))
            .await,
    );
    super::contract_tests::set_today(&clock, day(2025, 7, 2));
    record(&uc, &t, car, berlin(2025, 7, 2), dec!(16000)).await;

    let r = uc.vehicle_report.execute(&t, car).await.unwrap();
    let y = r.years.iter().find(|y| y.year == Some(2025)).unwrap();
    let km = y.km.unwrap().value;
    assert!(km >= dec!(6000), "{km}");
    let e = y.energy.as_ref().unwrap();
    let kwh = (km * dec!(0.16) * dec!(1.2) - dec!(100)).round_dp(3);
    assert_eq!(e.home_kwh.unwrap().value, kwh, "Fremdladen abgezogen");
    let cost = e.home_cost.unwrap();
    assert_eq!(cost.value, (kwh * dec!(0.30)).round_dp(2));
    let charging_total = y
        .categories
        .iter()
        .find(|(c, _)| *c == VehicleCostCategory::Charging)
        .map(|(_, v)| *v)
        .unwrap();
    assert_eq!(charging_total, cost.value + dec!(60));

    // mit Vertrag und Verbrauch gilt weiter der Arbeitspreis des Vertrags
    record(&uc, &t, house, berlin(2025, 7, 2), dec!(2500)).await;
    uc.create_contract
        .execute(
            &t,
            house,
            contract(
                day(2025, 1, 1),
                vec![position(
                    "Energie",
                    None,
                    vec![flat(day(2025, 1, 1), dec!(0), dec!(0.40))],
                )],
            ),
        )
        .await
        .unwrap();
    let r = uc.vehicle_report.execute(&t, car).await.unwrap();
    let y = r.years.iter().find(|y| y.year == Some(2025)).unwrap();
    let e = y.energy.as_ref().unwrap();
    assert_eq!(
        e.home_cost.unwrap().value,
        (e.home_kwh.unwrap().value * dec!(0.40)).round_dp(2)
    );
}

/// Haushaltsarchiv mit Fahrzeug: neue IDs, Verweise auf Heimlade-Messstelle
/// und Kostenbuch-Anlage zeigen auf die neuen Messstellen.
// @test SPEC_BACKUP_HOUSEHOLD, SPEC_CAR_VEHICLES, SPEC_ASSET_COSTS
#[tokio::test]
async fn household_archive_keeps_vehicles() {
    use std::io::Cursor;
    let (uc, t, _) = setup().await;
    let (_, wallbox) = house_with_wallbox(&uc, &t).await;
    let car = vehicle(
        &uc,
        &t,
        "Stromer",
        DriveType::Electric,
        Some(wallbox),
        dec!(10000),
    )
    .await;
    saved(
        uc.record_charging
            .execute(&t, car, charging(berlin(2025, 2, 10), dec!(10), dec!(5)))
            .await,
    );
    saved(
        uc.add_asset_cost
            .execute(
                &t,
                AssetRef::vehicle(car),
                cost(day(2025, 2, 1), VehicleCostCategory::Tax, dec!(120)),
            )
            .await,
    );
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
    let [v] = uc
        .list_vehicles
        .execute(&copy)
        .await
        .unwrap()
        .try_into()
        .unwrap();
    let imported = v.vehicle.unwrap();
    assert_ne!(imported.meter_id, car);
    let new_wallbox = imported.data.home_charging_meter_id.unwrap();
    assert_ne!(new_wallbox, wallbox);
    assert_eq!(
        uc.get_meter
            .execute(&copy, new_wallbox)
            .await
            .unwrap()
            .aggregate
            .meter
            .data
            .name,
        "Wallbox"
    );
    let detail = uc
        .get_vehicle
        .execute(&copy, imported.meter_id)
        .await
        .unwrap();
    assert_eq!(detail.charging.len(), 1);
    let book = uc
        .list_asset_costs
        .execute(&copy, AssetRef::vehicle(imported.meter_id), 2025)
        .await
        .unwrap();
    assert_eq!(book.total, dec!(120));
}

/// Beleg als Ladevorgang und als Kostenbuch-Eintrag übernehmen; der Beleg
/// bleibt über das Housekeeping hinaus erhalten (SPEC_AI_RECEIPT).
// @test SPEC_AI_RECEIPT, SPEC_DOC_UPLOADS
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn receipts_become_charging_sessions_and_cost_entries() {
    use std::sync::Arc;

    use bytes::Bytes;

    use crate::{
        model::{DocumentKind, DocumentStatus, EntrySource, Identity, ReceiptExtraction},
        port::RecognitionError,
        testing::{StubExtractor, memory_ports},
        usecase::{document::ReceiptEntry, housekeeping::Housekeeping},
    };

    let receipt = ReceiptExtraction {
        date: Some(day(2026, 1, 10)),
        total_amount: Some(dec!(18.99)),
        category: Some(VehicleCostCategory::Charging),
        quantity: Some(dec!(32.125)),
        unit_price: None,
        unit: Some("kWh".into()),
        vendor: Some("Ladepark".into()),
        odometer: None,
        charging_kind: Some("dc".into()),
        confidence: dec!(0.9),
    };
    let (mut ports, store, clock) = memory_ports();
    ports.extractor = Some(Arc::new(
        StubExtractor(Err(RecognitionError::Timeout)).with_receipt(Ok(receipt.clone())),
    ));
    let uc = UseCases::new(&ports);
    let user = uc
        .sign_in
        .execute(&Identity {
            issuer: "dev".into(),
            subject: "anna".into(),
            email: None,
            display_name: "Anna".into(),
            locale: "de".into(),
        })
        .await
        .unwrap()
        .user
        .id;
    let t = uc.resolve_tenant.execute(user, None).await.unwrap().tenant;
    let car = vehicle(&uc, &t, "Stromer", DriveType::Electric, None, dec!(10000)).await;
    let upload = || async {
        let id = uc
            .upload_document
            .execute(
                &t,
                Bytes::from_static(b"%PDF-1.7 Beleg"),
                "beleg.pdf",
                DocumentKind::Receipt,
            )
            .await
            .unwrap();
        uc.analyse_document.execute(&t, id).await.unwrap();
        id
    };
    let first = upload().await;
    let doc = uc.get_document.execute(&t, first).await.unwrap();
    assert_eq!(doc.kind, DocumentKind::Receipt);
    assert_eq!(doc.receipt, Some(receipt));
    assert!(doc.extraction.is_none());
    // Rechnungen sind keine Belege
    let invoice = uc
        .upload_document
        .execute(
            &t,
            Bytes::from_static(b"%PDF-1.7 R"),
            "r.pdf",
            DocumentKind::Invoice,
        )
        .await
        .unwrap();
    assert!(matches!(
        uc.confirm_receipt
            .execute(
                &t,
                invoice,
                car,
                ReceiptEntry::Charging(charging(berlin(2026, 1, 10), dec!(32), dec!(19)))
            )
            .await,
        Err(AppError::NotFound)
    ));

    uc.confirm_receipt
        .execute(
            &t,
            first,
            car,
            ReceiptEntry::Charging(charging(berlin(2026, 1, 10), dec!(32.125), dec!(18.99))),
        )
        .await
        .unwrap();
    let detail = uc.get_vehicle.execute(&t, car).await.unwrap();
    assert_eq!(detail.charging[0].source, EntrySource::Receipt);
    assert_eq!(detail.charging[0].document_id, Some(first));
    assert_eq!(
        uc.get_document.execute(&t, first).await.unwrap().status,
        DocumentStatus::Confirmed
    );

    let second = upload().await;
    uc.confirm_receipt
        .execute(
            &t,
            second,
            car,
            ReceiptEntry::Cost(cost(
                day(2026, 1, 12),
                VehicleCostCategory::Workshop,
                dec!(240),
            )),
        )
        .await
        .unwrap();
    let book = uc
        .list_asset_costs
        .execute(&t, AssetRef::vehicle(car), 2026)
        .await
        .unwrap();
    assert_eq!(book.entries[0].0.document_id, Some(second));

    // Belege mit Verweis überstehen das Housekeeping, verworfene nicht
    let third = upload().await;
    uc.discard_document.execute(&t, third).await.unwrap();
    clock.advance(chrono::Duration::hours(25));
    let hk = Housekeeping::new(
        store.clone(),
        ports.blobs.clone(),
        clock.clone(),
        chrono::Duration::hours(24),
    );
    let report = hk.execute().await.unwrap();
    assert_eq!(report.documents, 2, "verworfener Beleg und offene Rechnung");
    assert!(uc.get_document.execute(&t, first).await.is_ok());
    assert!(uc.get_document.execute(&t, second).await.is_ok());
}
