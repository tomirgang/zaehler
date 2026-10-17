//! Port-Tests für Fahrzeuge und Kostenbuch (SPEC_CAR_VEHICLES,
//! SPEC_ASSET_COSTS): Anlegen und Ersetzen, Profile und Anschaffung,
//! Ladevorgänge, Tenant-Isolation und Löschen mit dem Haushalt.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use zaehler_domain::{
    assets::{AssetCategory, Recurrence, VehicleCostCategory},
    meter::{MeterKind, MeterPurpose},
    vehicle::{ConsumptionProfile, DriveType, Holding},
};

use super::{id, meter, t0, tenant};
use crate::{
    PortError, Tenant,
    ids::{AssetCostId, ChargingId, DocumentId, MeterId, ProfileId},
    model::{
        AssetCost, AssetRef, ChargingKind, ChargingSession, EntrySource, MeterData, ProfileRow,
        VehicleData,
    },
    port::UnitOfWorkFactory,
};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap_or_default()
}

async fn vehicle_meter(f: &dyn UnitOfWorkFactory, t: &Tenant, name: &str) -> MeterId {
    let m = MeterId(id());
    let mut uow = f.begin().await.expect("begin");
    let data = MeterData {
        name: name.into(),
        kind: MeterKind::Vehicle,
        unit: "km".into(),
        schedule: None,
        purpose: MeterPurpose::General,
        targets: crate::model::MeterTargets::default(),
        max_daily_rate: None,
        billing_start: None,
    };
    uow.meters().create(t, m, &data, t0()).await.expect("meter");
    uow.commit().await.expect("commit");
    m
}

fn vehicle_data(home: Option<MeterId>) -> VehicleData {
    VehicleData {
        plate: "M-ZA 123".into(),
        model: "Kombi".into(),
        drive_type: DriveType::PluginHybrid,
        home_charging_meter_id: home,
        pv_share_pct: Some(Decimal::new(40_500, 3)),
    }
}

fn cost(asset: AssetRef, doc: Option<DocumentId>) -> AssetCost {
    AssetCost {
        id: AssetCostId(id()),
        asset,
        booked_on: day(3, 1),
        category: AssetCategory::Vehicle(VehicleCostCategory::Insurance),
        amount: Decimal::new(48_000, 2),
        quantity: None,
        odometer: Some(Decimal::from(12_345)),
        full_tank: None,
        recurrence: Some(Recurrence {
            every_months: 12,
            until: Some(day(12, 31)),
        }),
        document_id: doc,
        note: Some("Haftpflicht".into()),
    }
}

/// Führt alle Prüfungen zu Fahrzeugen und Kostenbuch aus.
// @test SPEC_CAR_VEHICLES, SPEC_ASSET_COSTS, SPEC_SEC_TENANT_SCOPE
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
pub(super) async fn vehicles(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "Fahrzeuge").await;
    let other = tenant(f, "Fremd F").await;
    let car = vehicle_meter(f, &t, "Auto").await;
    let (house, _, _) = meter(f, &t, "Hausstrom Fz").await;
    let (foreign_house, _, _) = meter(f, &other, "Strom Fremd Fz").await;
    let foreign_car = vehicle_meter(f, &other, "Auto Fremd").await;

    // keine Fahrzeug-Messstelle, fremde Heimlade-Messstelle, fremdes Fahrzeug
    for (target, home) in [
        (house, None),
        (car, Some(foreign_house)),
        (foreign_car, None),
    ] {
        let mut uow = f.begin().await.expect("begin");
        assert!(
            uow.vehicles()
                .save(&t, target, &vehicle_data(home))
                .await
                .is_err()
        );
    }

    let profile = ProfileRow {
        id: ProfileId(id()),
        profile: ConsumptionProfile {
            valid_from: day(1, 1),
            kwh_per_100km: Decimal::new(18_500, 3),
            charging_loss_pct: Decimal::from(10),
            electric_share_pct: Some(Decimal::from(60)),
        },
    };
    let holding = Holding {
        purchased_on: day(1, 1),
        price: Decimal::new(3_000_000, 2),
        residual_value: Decimal::from(10_000),
        planned_end: NaiveDate::from_ymd_opt(2031, 1, 1).unwrap_or_default(),
        sold_on: None,
        sale_price: None,
    };
    let session = ChargingSession {
        id: ChargingId(id()),
        vehicle: car,
        charged_at: t0(),
        energy: Decimal::new(42_250, 3),
        amount: Decimal::new(2_450, 2),
        odometer: Some(Decimal::from(12_400)),
        provider: Some("Ladesäule".into()),
        kind: ChargingKind::Dc,
        source: EntrySource::Receipt,
        document_id: Some(DocumentId(id())),
    };
    let entry = cost(AssetRef::vehicle(car), Some(DocumentId(id())));
    let mut uow = f.begin().await.expect("begin");
    uow.vehicles()
        .save(&t, car, &vehicle_data(Some(house)))
        .await
        .expect("save");
    uow.vehicles()
        .add_profile(&t, car, &profile)
        .await
        .expect("profile");
    uow.vehicles()
        .set_holding(&t, car, Some(&holding))
        .await
        .expect("holding");
    uow.vehicles()
        .add_charging(&t, &session)
        .await
        .expect("charging");
    uow.asset_costs().create(&t, &entry).await.expect("cost");
    uow.commit().await.expect("commit");

    let mut uow = f.begin().await.expect("begin");
    let v = uow.vehicles().get(&t, car).await.expect("get").expect("da");
    assert_eq!(v.data, vehicle_data(Some(house)));
    assert_eq!(v.profiles, std::slice::from_ref(&profile));
    assert_eq!(v.holding.as_ref(), Some(&holding));
    assert_eq!(
        uow.vehicles().charging(&t, car).await.expect("charging"),
        std::slice::from_ref(&session)
    );
    assert_eq!(
        uow.asset_costs()
            .list(&t, AssetRef::vehicle(car))
            .await
            .expect("list"),
        std::slice::from_ref(&entry)
    );
    assert_eq!(uow.vehicles().list(&t).await.expect("list").len(), 1);
    // gleicher Gültigkeitsbeginn
    let same_day = ProfileRow {
        id: ProfileId(id()),
        ..profile.clone()
    };
    assert!(matches!(
        uow.vehicles().add_profile(&t, car, &same_day).await,
        Err(PortError::Conflict(_))
    ));
    drop(uow);

    // fremder Tenant sieht und ändert nichts
    let mut uow = f.begin().await.expect("begin");
    assert!(
        uow.vehicles()
            .get(&other, car)
            .await
            .expect("get")
            .is_none()
    );
    assert!(uow.vehicles().list(&other).await.expect("list").is_empty());
    assert!(
        uow.vehicles()
            .charging(&other, car)
            .await
            .expect("charging")
            .is_empty()
    );
    assert!(
        uow.asset_costs()
            .get(&other, entry.id)
            .await
            .expect("get")
            .is_none()
    );
    assert!(
        uow.asset_costs()
            .list(&other, AssetRef::vehicle(car))
            .await
            .expect("list")
            .is_empty()
    );
    assert!(uow.asset_costs().delete(&other, entry.id).await.is_err());
    assert!(uow.asset_costs().update(&other, &entry).await.is_err());
    assert!(
        uow.vehicles()
            .delete_charging(&other, car, session.id)
            .await
            .is_err()
    );
    assert!(uow.vehicles().set_holding(&other, car, None).await.is_err());
    assert!(
        uow.vehicles()
            .delete_profile(&other, car, profile.id)
            .await
            .is_err()
    );
    drop(uow);

    // Ändern: Stammdaten ersetzen, Anschaffung entfernen, Eintrag ändern
    let mut changed = cost(AssetRef::vehicle(house), None);
    changed.id = entry.id;
    changed.amount = Decimal::new(50_000, 2);
    changed.recurrence = None;
    let mut uow = f.begin().await.expect("begin");
    let data = VehicleData {
        drive_type: DriveType::Electric,
        home_charging_meter_id: None,
        pv_share_pct: None,
        ..vehicle_data(None)
    };
    uow.vehicles().save(&t, car, &data).await.expect("save");
    uow.vehicles()
        .set_holding(&t, car, None)
        .await
        .expect("holding");
    uow.vehicles()
        .delete_profile(&t, car, profile.id)
        .await
        .expect("profile");
    uow.asset_costs()
        .update(&t, &changed)
        .await
        .expect("update");
    uow.vehicles()
        .delete_charging(&t, car, session.id)
        .await
        .expect("charging");
    uow.commit().await.expect("commit");

    let mut uow = f.begin().await.expect("begin");
    let v = uow.vehicles().get(&t, car).await.expect("get").expect("da");
    assert_eq!(v.data, data);
    assert!(v.profiles.is_empty() && v.holding.is_none());
    let stored = uow
        .asset_costs()
        .get(&t, entry.id)
        .await
        .expect("get")
        .expect("da");
    assert_eq!(stored.asset, AssetRef::vehicle(car), "Anlage bleibt");
    assert_eq!(stored.amount, changed.amount);
    assert_eq!(stored.recurrence, None);
    assert!(
        uow.vehicles()
            .charging(&t, car)
            .await
            .expect("charging")
            .is_empty()
    );
    uow.asset_costs()
        .delete(&t, entry.id)
        .await
        .expect("delete");
    assert!(matches!(
        uow.asset_costs().delete(&t, entry.id).await,
        Err(PortError::NotFound)
    ));
    uow.asset_costs()
        .create(&t, &cost(AssetRef::vehicle(car), None))
        .await
        .expect("cost");
    uow.commit().await.expect("commit");

    // Löschen mit dem Haushalt
    let mut uow = f.begin().await.expect("begin");
    uow.households().delete(&t).await.expect("delete");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert!(uow.vehicles().list(&t).await.expect("list").is_empty());
    assert!(
        uow.asset_costs()
            .list(&t, AssetRef::vehicle(car))
            .await
            .expect("list")
            .is_empty()
    );
}
