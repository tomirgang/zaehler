//! Port-Tests für Verträge (SPEC_TAR_CONTRACTS): vollständiges Laden,
//! Ersetzen der Positionen, Eindeutigkeit der Stände, Tenant-Isolation und
//! Löschen mit dem Haushalt.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use zaehler_domain::{
    meter::Direction,
    tariff::{
        Adjustment, AdjustmentKind, BasePriceInterval, Installment, PriceComponent, Tier,
        UnitPricing,
    },
};

use super::{id, meter, tenant};
use crate::{
    PortError,
    ids::{
        AdjustmentId, ContractId, InstallmentId, PositionId, PriceComponentId, RegisterId,
        SettlementId,
    },
    model::{
        AdjustmentRow, ContractData, ContractEntry, InstallmentRow, MonthDay, Position,
        PriceComponentRow, SettlementRow,
    },
    port::UnitOfWorkFactory,
};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap_or_default()
}

fn component(from: NaiveDate, pricing: UnitPricing) -> PriceComponentRow {
    PriceComponentRow {
        id: PriceComponentId(id()),
        component: PriceComponent {
            valid_from: from,
            base_price: Decimal::new(1295, 2),
            base_interval: BasePriceInterval::Month,
            unit_pricing: pricing,
            factor: None,
        },
    }
}

fn data(register: Option<RegisterId>) -> ContractData {
    ContractData {
        direction: Direction::Import,
        provider: "Stadtwerke".into(),
        contract_number: Some("V-1".into()),
        starts_on: day(1, 1),
        ends_on: Some(day(12, 31)),
        billing_start: MonthDay { month: 4, day: 1 },
        note: None,
        positions: vec![
            Position {
                id: PositionId(id()),
                name: "Arbeitspreis".into(),
                register_id: register,
                factor: Decimal::ONE,
                vat_pct: Some(Decimal::new(19, 0)),
                components: vec![
                    component(
                        day(1, 1),
                        UnitPricing::Tiered {
                            tiers: vec![
                                Tier {
                                    up_to: Some(Decimal::from(1000)),
                                    price: Decimal::new(30, 2),
                                },
                                Tier {
                                    up_to: None,
                                    price: Decimal::new(32, 2),
                                },
                            ],
                        },
                    ),
                    PriceComponentRow {
                        component: PriceComponent {
                            factor: Some(Decimal::new(10_025, 4)),
                            ..component(
                                day(7, 1),
                                UnitPricing::Flat {
                                    price: Decimal::new(3149, 4),
                                },
                            )
                            .component
                        },
                        ..component(
                            day(7, 1),
                            UnitPricing::Flat {
                                price: Decimal::ZERO,
                            },
                        )
                    },
                ],
            },
            Position {
                id: PositionId(id()),
                name: "Messstellenbetrieb".into(),
                register_id: None,
                factor: Decimal::ONE,
                vat_pct: None,
                components: vec![component(
                    day(1, 1),
                    UnitPricing::Flat {
                        price: Decimal::ZERO,
                    },
                )],
            },
        ],
    }
}

/// Führt alle Prüfungen zu Verträgen aus.
// @test SPEC_TAR_CONTRACTS, SPEC_SEC_TENANT_SCOPE
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
pub(super) async fn contracts(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "Verträge").await;
    let other = tenant(f, "Fremd V").await;
    let (m, _, r) = meter(f, &t, "Strom V").await;
    let (foreign_meter, _, foreign_register) = meter(f, &other, "Strom F").await;
    let c = ContractId(id());
    let mut d = data(Some(r));

    // Register einer fremden Messstelle wird abgelehnt
    let mut uow = f.begin().await.expect("begin");
    assert!(!uow.contracts().any(&t).await.expect("any"));
    assert!(
        uow.contracts()
            .create(&t, c, m, &data(Some(foreign_register)))
            .await
            .is_err()
    );
    drop(uow);
    // fremde Messstelle
    let mut uow = f.begin().await.expect("begin");
    assert!(
        uow.contracts()
            .create(&t, c, foreign_meter, &d)
            .await
            .is_err()
    );
    drop(uow);

    let mut uow = f.begin().await.expect("begin");
    uow.contracts().create(&t, c, m, &d).await.expect("create");
    let inst = InstallmentRow {
        id: InstallmentId(id()),
        installment: Installment {
            amount: Decimal::new(85, 0),
            valid_from: day(1, 15),
            every_months: 1,
        },
    };
    uow.contracts()
        .add_installment(&t, c, &inst)
        .await
        .expect("installment");
    let adj = AdjustmentRow {
        id: AdjustmentId(id()),
        adjustment: Adjustment {
            kind: AdjustmentKind::Bonus,
            booked_on: day(3, 1),
            amount: Decimal::new(50, 0),
        },
        note: Some("Bonus".into()),
    };
    uow.contracts()
        .add_adjustment(&t, c, &adj)
        .await
        .expect("adjustment");
    let set = SettlementRow {
        id: SettlementId(id()),
        period_start: day(1, 1),
        period_end: day(3, 31),
        amount: Decimal::new(31_245, 2),
        document_id: None,
    };
    uow.contracts()
        .add_settlement(&t, c, &set)
        .await
        .expect("settlement");
    uow.commit().await.expect("commit");

    let mut uow = f.begin().await.expect("begin");
    let loaded = uow.contracts().get(&t, c).await.expect("get").expect("da");
    assert_eq!(loaded.data, d, "vollständig und in Reihenfolge");
    assert_eq!(loaded.meter_id, m);
    assert_eq!(loaded.installments, std::slice::from_ref(&inst));
    assert_eq!(loaded.adjustments, std::slice::from_ref(&adj));
    assert_eq!(loaded.settlements, std::slice::from_ref(&set));
    assert_eq!(uow.contracts().list(&t).await.expect("list").len(), 1);
    assert!(uow.contracts().any(&t).await.expect("any"));
    assert!(!uow.contracts().any(&other).await.expect("any"));
    assert_eq!(
        uow.contracts().for_meter(&t, m).await.expect("list").len(),
        1
    );
    // fremder Tenant sieht nichts
    assert!(uow.contracts().get(&other, c).await.expect("get").is_none());
    assert!(uow.contracts().list(&other).await.expect("list").is_empty());
    assert!(
        uow.contracts()
            .for_meter(&other, m)
            .await
            .expect("list")
            .is_empty()
    );
    drop(uow);

    // gleicher Stand doppelt ist ein Konflikt
    let mut uow = f.begin().await.expect("begin");
    let again = InstallmentRow {
        id: InstallmentId(id()),
        ..inst.clone()
    };
    assert!(matches!(
        uow.contracts().add_installment(&t, c, &again).await,
        Err(PortError::Conflict(_))
    ));
    drop(uow);
    let mut uow = f.begin().await.expect("begin");
    let position = d.positions[0].id;
    let dup = component(
        day(7, 1),
        UnitPricing::Flat {
            price: Decimal::ONE,
        },
    );
    assert!(matches!(
        uow.contracts().add_component(&t, position, &dup).await,
        Err(PortError::Conflict(_))
    ));
    drop(uow);

    // schreibende Zugriffe über fremden Tenant
    for attempt in 0..6 {
        let mut uow = f.begin().await.expect("begin");
        let repo = uow.contracts();
        let res = match attempt {
            0 => repo.update(&other, c, &d).await,
            1 => repo.delete(&other, c).await,
            2 => {
                let x = component(
                    day(9, 1),
                    UnitPricing::Flat {
                        price: Decimal::ONE,
                    },
                );
                repo.add_component(&other, position, &x).await
            }
            3 => {
                let x = InstallmentRow {
                    id: InstallmentId(id()),
                    ..inst.clone()
                };
                repo.add_installment(&other, c, &x).await
            }
            4 => {
                repo.delete_entry(&other, c, ContractEntry::Adjustment(adj.id))
                    .await
            }
            _ => {
                let x = SettlementRow {
                    id: SettlementId(id()),
                    ..set.clone()
                };
                repo.add_settlement(&other, c, &x).await
            }
        };
        assert!(
            matches!(res, Err(PortError::NotFound)),
            "fremder Zugriff {attempt}"
        );
    }

    // neuer Preisstand, Ersetzen der Positionen, Löschen eines Eintrags
    let mut uow = f.begin().await.expect("begin");
    let september = component(
        day(9, 1),
        UnitPricing::Flat {
            price: Decimal::ONE,
        },
    );
    uow.contracts()
        .add_component(&t, position, &september)
        .await
        .expect("component");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    let loaded = uow.contracts().get(&t, c).await.expect("get").expect("da");
    assert_eq!(loaded.data.positions[0].components.len(), 3);
    assert_eq!(loaded.data.positions[0].components[2], september);
    d.positions.remove(1);
    d.provider = "Neu".into();
    d.ends_on = None;
    uow.contracts().update(&t, c, &d).await.expect("update");
    uow.contracts()
        .delete_entry(&t, c, ContractEntry::Adjustment(adj.id))
        .await
        .expect("delete entry");
    assert!(matches!(
        uow.contracts()
            .delete_entry(&t, c, ContractEntry::Adjustment(adj.id))
            .await,
        Err(PortError::NotFound)
    ));
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    let loaded = uow.contracts().get(&t, c).await.expect("get").expect("da");
    assert_eq!(loaded.data, d);
    assert!(loaded.adjustments.is_empty());
    assert_eq!(loaded.installments.len(), 1, "Einträge bleiben beim Ändern");

    // Löschen des Vertrags und des Haushalts
    uow.contracts().delete(&t, c).await.expect("delete");
    assert!(uow.contracts().get(&t, c).await.expect("get").is_none());
    let c2 = ContractId(id());
    uow.contracts()
        .create(&t, c2, m, &data(None))
        .await
        .expect("create");
    uow.households().delete(&t).await.expect("delete household");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert!(uow.contracts().list(&t).await.expect("list").is_empty());
    assert!(uow.contracts().get(&t, c2).await.expect("get").is_none());
}
