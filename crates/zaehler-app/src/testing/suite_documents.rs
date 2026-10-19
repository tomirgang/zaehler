//! Port-Tests für Dokumente (SPEC_DOC_UPLOADS): Tenant-Isolation,
//! Statuswechsel nur bei offenen Dokumenten, Housekeeping ohne Verweis.

use chrono::Duration;
use rust_decimal::Decimal;

use super::{id, meter, t0, tenant};
use crate::{
    PortError,
    ids::{BlobKey, ContractId, DocumentId, PositionId, PriceComponentId, SettlementId},
    model::{
        ContractData, Document, DocumentAnalysis, DocumentStatus, DocumentType, InvoiceExtraction,
        MonthDay, Position, PriceComponentRow, SettlementRow,
    },
    port::{MaintenanceRepository, UnitOfWorkFactory},
};

fn document(t: &crate::Tenant, days: i64) -> Document {
    let id = DocumentId(id());
    Document {
        id,
        blob_key: BlobKey::document(t.household_id(), id, true),
        kind: crate::model::DocumentKind::Invoice,
        media_type: DocumentType::Pdf,
        bytes: 1234,
        original_name: "rechnung.pdf".into(),
        created_by: t.user_id(),
        created_at: t0() + Duration::days(days),
        status: DocumentStatus::Uploaded,
        model: None,
        extraction: None,
        receipt: None,
        meter_id: None,
        analysed_at: None,
    }
}

/// Führt alle Prüfungen zu Dokumenten aus.
///
/// # Panics
///
/// Wenn der Adapter eine Prüfung nicht erfüllt.
// @test SPEC_DOC_UPLOADS, SPEC_SEC_TENANT_SCOPE
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
pub async fn documents(f: &dyn UnitOfWorkFactory, m: &dyn MaintenanceRepository) {
    let t = tenant(f, "Dokumente").await;
    let other = tenant(f, "Fremd D").await;
    let (meter_id, _, _) = meter(f, &t, "Strom D").await;
    let (foreign_meter, _, _) = meter(f, &other, "Strom DF").await;
    let a = document(&t, 0);
    let b = document(&t, 1);
    let mut uow = f.begin().await.expect("begin");
    uow.documents().create(&t, &a).await.expect("create");
    uow.documents().create(&t, &b).await.expect("create");
    uow.commit().await.expect("commit");

    let mut uow = f.begin().await.expect("begin");
    assert_eq!(
        uow.documents().get(&t, a.id).await.expect("get"),
        Some(a.clone())
    );
    assert!(
        uow.documents()
            .get(&other, a.id)
            .await
            .expect("get")
            .is_none()
    );
    let open = uow.documents().list_open(&t).await.expect("list");
    assert_eq!(open.iter().map(|d| d.id).collect::<Vec<_>>(), [b.id, a.id]);
    assert!(
        uow.documents()
            .list_open(&other)
            .await
            .expect("list")
            .is_empty()
    );
    drop(uow);

    let extraction = InvoiceExtraction {
        provider: Some("Stadtwerke".into()),
        contract_number: None,
        period_start: None,
        period_end: None,
        readings: Vec::new(),
        prices: Vec::new(),
        installment: None,
        total_amount: Some(Decimal::new(31_245, 2)),
        confidence: Decimal::new(8, 1),
    };
    let analysis = DocumentAnalysis {
        model: Some("stub".into()),
        extraction: Some(extraction),
        receipt: None,
        meter_id: Some(meter_id),
        status: DocumentStatus::Analysed,
        at: t0() + Duration::hours(1),
    };
    // fremder Tenant und fremde Messstelle
    let mut uow = f.begin().await.expect("begin");
    assert!(matches!(
        uow.documents()
            .record_analysis(&other, a.id, &analysis)
            .await,
        Err(PortError::NotFound)
    ));
    drop(uow);
    let mut uow = f.begin().await.expect("begin");
    let foreign = DocumentAnalysis {
        meter_id: Some(foreign_meter),
        ..analysis.clone()
    };
    assert!(
        uow.documents()
            .record_analysis(&t, a.id, &foreign)
            .await
            .is_err()
    );
    drop(uow);
    let mut uow = f.begin().await.expect("begin");
    assert!(matches!(
        uow.documents()
            .finish(&other, a.id, DocumentStatus::Discarded, t0())
            .await,
        Err(PortError::NotFound)
    ));
    drop(uow);

    let mut uow = f.begin().await.expect("begin");
    uow.documents()
        .record_analysis(&t, a.id, &analysis)
        .await
        .expect("analysis");
    uow.documents()
        .finish(&t, b.id, DocumentStatus::Discarded, t0())
        .await
        .expect("finish");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    let got = uow
        .documents()
        .get(&t, a.id)
        .await
        .expect("get")
        .expect("da");
    assert_eq!(got.status, DocumentStatus::Analysed);
    assert_eq!(got.extraction, analysis.extraction);
    assert_eq!(got.meter_id, Some(meter_id));
    assert_eq!(got.analysed_at, Some(analysis.at));
    // abgeschlossene Dokumente sind nicht mehr offen
    assert_eq!(
        uow.documents()
            .list_open(&t)
            .await
            .expect("list")
            .iter()
            .map(|d| d.id)
            .collect::<Vec<_>>(),
        [a.id]
    );
    assert!(matches!(
        uow.documents()
            .finish(&t, b.id, DocumentStatus::Confirmed, t0())
            .await,
        Err(PortError::NotFound)
    ));
    drop(uow);

    // a bestätigt und an einer Abrechnung, c verworfen ohne Verweis
    let c = document(&t, 2);
    let mut uow = f.begin().await.expect("begin");
    uow.documents().create(&t, &c).await.expect("create");
    uow.documents()
        .finish(&t, a.id, DocumentStatus::Confirmed, t0())
        .await
        .expect("finish");
    let contract = ContractId(id());
    let data = ContractData {
        direction: zaehler_domain::meter::Direction::Import,
        provider: "Stadtwerke".into(),
        contract_number: None,
        starts_on: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap_or_default(),
        ends_on: None,
        billing_start: MonthDay { month: 1, day: 1 },
        note: None,
        positions: vec![Position {
            id: PositionId(id()),
            name: "Strom".into(),
            register_id: None,
            factor: Decimal::ONE,
            vat_pct: None,
            components: vec![PriceComponentRow {
                id: PriceComponentId(id()),
                component: zaehler_domain::tariff::PriceComponent {
                    valid_from: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap_or_default(),
                    base_price: Decimal::ZERO,
                    base_interval: zaehler_domain::tariff::BasePriceInterval::Year,
                    unit_pricing: zaehler_domain::tariff::UnitPricing::Flat {
                        price: Decimal::ONE,
                    },
                    factor: None,
                },
            }],
        }],
    };
    uow.contracts()
        .create(&t, contract, meter_id, &data)
        .await
        .expect("contract");
    let settlement = SettlementRow {
        id: SettlementId(id()),
        period_start: data.starts_on,
        period_end: data.starts_on,
        amount: Decimal::ONE,
        document_id: Some(a.id.0),
    };
    uow.contracts()
        .add_settlement(&t, contract, &settlement)
        .await
        .expect("settlement");
    uow.commit().await.expect("commit");

    // Housekeeping: vor dem Stichtag und ohne Verweis
    let expired = m
        .expire_documents(t0() + Duration::days(2))
        .await
        .expect("expire");
    assert_eq!(
        expired,
        std::slice::from_ref(&b.blob_key),
        "a hat einen Verweis, c ist neuer"
    );
    assert!(
        m.expire_documents(t0() + Duration::days(2))
            .await
            .expect("expire")
            .is_empty(),
        "idempotent"
    );
    let mut uow = f.begin().await.expect("begin");
    assert!(uow.documents().get(&t, b.id).await.expect("get").is_none());
    assert!(uow.documents().get(&t, a.id).await.expect("get").is_some());
    // ohne Abrechnung verfällt auch das bestätigte Dokument
    uow.contracts()
        .delete_entry(
            &t,
            contract,
            crate::model::ContractEntry::Settlement(settlement.id),
        )
        .await
        .expect("delete");
    uow.commit().await.expect("commit");
    let mut keys = m
        .expire_documents(t0() + Duration::days(3))
        .await
        .expect("expire");
    keys.sort();
    let mut expected = vec![a.blob_key.clone(), c.blob_key.clone()];
    expected.sort();
    assert_eq!(keys, expected);

    // Löschen des Haushalts entfernt die Dokumente
    let d = document(&t, 10);
    let mut uow = f.begin().await.expect("begin");
    uow.documents().create(&t, &d).await.expect("create");
    uow.households().delete(&t).await.expect("delete");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert!(uow.documents().get(&t, d.id).await.expect("get").is_none());

    // Beleg: Art und Vorschlag bleiben erhalten (SPEC_AI_RECEIPT)
    let t = super::tenant(f, "Belege").await;
    let r = Document {
        kind: crate::model::DocumentKind::Receipt,
        ..document(&t, 11)
    };
    let receipt = crate::model::ReceiptExtraction {
        date: chrono::NaiveDate::from_ymd_opt(2026, 3, 14),
        total_amount: Some(Decimal::new(1899, 2)),
        category: Some(zaehler_domain::assets::VehicleCostCategory::Charging),
        quantity: Some(Decimal::new(32_125, 3)),
        unit_price: None,
        unit: Some("kWh".into()),
        vendor: Some("Ladepark".into()),
        odometer: Some(Decimal::from(12_345)),
        charging_kind: Some("dc".into()),
        confidence: Decimal::new(9, 1),
    };
    let mut uow = f.begin().await.expect("begin");
    uow.documents().create(&t, &r).await.expect("create");
    uow.documents()
        .record_analysis(
            &t,
            r.id,
            &DocumentAnalysis {
                model: Some("stub".into()),
                extraction: None,
                receipt: Some(receipt.clone()),
                meter_id: None,
                status: DocumentStatus::Analysed,
                at: t0(),
            },
        )
        .await
        .expect("analysis");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    let loaded = uow
        .documents()
        .get(&t, r.id)
        .await
        .expect("get")
        .expect("da");
    assert_eq!(loaded.kind, crate::model::DocumentKind::Receipt);
    assert_eq!(loaded.receipt, Some(receipt));
    assert!(loaded.extraction.is_none());
}
