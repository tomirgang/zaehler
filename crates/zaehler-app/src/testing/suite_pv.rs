//! Port-Tests für PV-Anlagen (SPEC_PV_ASSIGNMENT): Anlegen, Ersetzen der
//! Zuordnung, Tenant-Isolation und Löschen mit dem Haushalt.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use zaehler_domain::pv::{PvKind, PvRole, PvSystemData};

use super::{id, meter, tenant};
use crate::{PortError, ids::PvSystemId, model::PvSystem, port::UnitOfWorkFactory};

fn data(name: &str) -> PvSystemData {
    PvSystemData {
        name: name.into(),
        kind: PvKind::Surplus,
        kwp: Decimal::new(9_840, 3),
        commissioned_on: NaiveDate::from_ymd_opt(2025, 4, 16).unwrap_or_default(),
        azimuth: Some(180),
        tilt: Some(30),
        inverter: Some("WR".into()),
        battery_kwh: Some(Decimal::new(10_240, 3)),
        specific_yield: Decimal::from(950),
        degradation_pct: Decimal::new(5, 1),
        month_shares: Some(zaehler_domain::pv::DEFAULT_MONTH_SHARES),
        alarm_threshold_pct: 50,
    }
}

/// Führt alle Prüfungen zu PV-Anlagen aus.
// @test SPEC_PV_ASSIGNMENT, SPEC_SEC_TENANT_SCOPE
pub(super) async fn pv(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "PV").await;
    let other = tenant(f, "Fremd PV").await;
    let (grid, _, _) = meter(f, &t, "Hausanschluss PV").await;
    let (generation, _, _) = meter(f, &t, "Erzeugung PV").await;
    let (foreign, _, _) = meter(f, &other, "Fremd Erzeugung").await;
    let mut s = PvSystem {
        id: PvSystemId(id()),
        data: data("Dach"),
        meters: vec![(grid, PvRole::Grid), (generation, PvRole::Generation)],
    };

    // fremde Messstelle
    let mut uow = f.begin().await.expect("begin");
    let bad = PvSystem {
        meters: vec![(grid, PvRole::Grid), (foreign, PvRole::Generation)],
        ..s.clone()
    };
    assert!(matches!(
        uow.pv().create(&t, &bad).await,
        Err(PortError::NotFound)
    ));
    drop(uow);

    let mut uow = f.begin().await.expect("begin");
    uow.pv().create(&t, &s).await.expect("create");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert_eq!(uow.pv().get(&t, s.id).await.expect("get"), Some(s.clone()));
    assert_eq!(uow.pv().list(&t).await.expect("list").len(), 1);
    assert!(uow.pv().get(&other, s.id).await.expect("get").is_none());
    assert!(uow.pv().list(&other).await.expect("list").is_empty());
    assert!(uow.pv().delete(&other, s.id).await.is_err());
    assert!(uow.pv().update(&other, &s).await.is_err());
    drop(uow);

    // Daten und Zuordnung ersetzen
    s.data.name = "Dach Süd".into();
    s.data.month_shares = None;
    s.data.battery_kwh = None;
    s.meters = vec![(grid, PvRole::Grid)];
    let mut uow = f.begin().await.expect("begin");
    uow.pv().update(&t, &s).await.expect("update");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert_eq!(uow.pv().get(&t, s.id).await.expect("get"), Some(s.clone()));
    uow.pv().delete(&t, s.id).await.expect("delete");
    assert!(matches!(
        uow.pv().delete(&t, s.id).await,
        Err(PortError::NotFound)
    ));
    uow.commit().await.expect("commit");

    // Löschen mit dem Haushalt
    let mut uow = f.begin().await.expect("begin");
    uow.pv()
        .create(
            &t,
            &PvSystem {
                id: PvSystemId(id()),
                ..s.clone()
            },
        )
        .await
        .expect("create");
    uow.households().delete(&t).await.expect("delete");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert!(uow.pv().list(&t).await.expect("list").is_empty());
}
