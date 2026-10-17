//! Tests: Dokumente und Rechnungsimport (SPEC_DOC_UPLOADS, SPEC_AI_INVOICE).

use std::sync::Arc;

use bytes::Bytes;
use chrono::{Duration, NaiveDate, TimeZone, Utc};
use rust_decimal_macros::dec;
use zaehler_domain::{
    meter::{Direction, MeterKind, MeterPurpose},
    reading::ReadingSource,
    tariff::{BasePriceInterval, UnitPricing},
};

use crate::{
    AppError, Tenant,
    ids::MeterId,
    model::{
        DocumentStatus, DocumentType, Identity, InvoiceExtraction, InvoiceReading, MeterTargets,
    },
    port::{Clock, RecognitionError},
    testing::{FixedClock, InMemoryBlobStore, StubExtractor, memory_ports_with_blobs},
    usecase::{
        UseCases,
        contract::{
            ComponentInput, ContractInput, InstallmentInput, PositionInput, SettlementInput,
        },
        document::{ConfirmOutcome, ContractChoice, InvoiceConfirmation},
        housekeeping::Housekeeping,
        meter::{MeterInput, RegisterInput},
        reading::ReadingsInput,
    },
};

const PDF: &[u8] = b"%PDF-1.7 Rechnung";
const JPEG: &[u8] = b"\xFF\xD8\xFFfoto";

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

fn extraction() -> InvoiceExtraction {
    InvoiceExtraction {
        provider: Some("Stadtwerke".into()),
        contract_number: None,
        period_start: Some(day(1, 1)),
        period_end: Some(day(1, 9)),
        readings: vec![InvoiceReading {
            serial_number: Some(" 1emh-0012 ".into()),
            register: Some("main".into()),
            date: day(1, 9),
            value: dec!(80),
        }],
        prices: Vec::new(),
        installment: None,
        total_amount: Some(dec!(30)),
        confidence: dec!(0.9),
    }
}

struct Env {
    uc: UseCases,
    t: Tenant,
    meter: MeterId,
    store: Arc<crate::testing::InMemoryStore>,
    clock: Arc<FixedClock>,
    blobs: Arc<InMemoryBlobStore>,
    ports: crate::usecase::Ports,
}

async fn setup(extractor: Option<Result<InvoiceExtraction, RecognitionError>>) -> Env {
    let (mut ports, store, clock, blobs) = memory_ports_with_blobs();
    ports.extractor =
        extractor.map(|r| Arc::new(StubExtractor(r)) as Arc<dyn crate::port::DocumentExtractor>);
    ports.settings.max_document_bytes = 1024;
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
    let meter = uc
        .create_meter
        .execute(
            &t,
            MeterInput {
                name: "Strom".into(),
                kind: MeterKind::Electricity,
                unit: String::new(),
                schedule: None,
                max_daily_rate: None,
                billing_start: None,
                purpose: MeterPurpose::General,
                targets: MeterTargets::default(),
                parent_meter_id: None,
                serial_number: Some("1EMH0012".into()),
                rollover_digits: None,
                decimal_places: 1,
                registers: vec![RegisterInput {
                    id: None,
                    code: "main".into(),
                    direction: Direction::Import,
                    start_value: Some(dec!(100)),
                }],
                start_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            },
        )
        .await
        .unwrap();
    Env {
        uc,
        t,
        meter,
        store,
        clock,
        blobs,
        ports,
    }
}

fn contract_input() -> ContractInput {
    ContractInput {
        direction: Direction::Import,
        provider: "Stadtwerke".into(),
        contract_number: None,
        starts_on: day(1, 1),
        ends_on: None,
        billing_start: None,
        note: None,
        positions: vec![PositionInput {
            id: None,
            name: "Strom".into(),
            register_id: None,
            factor: dec!(1),
            vat_pct: None,
            components: vec![ComponentInput {
                id: None,
                valid_from: day(1, 1),
                base_price: dec!(120),
                base_interval: BasePriceInterval::Year,
                unit_pricing: UnitPricing::Flat { price: dec!(0.3) },
                factor: None,
            }],
        }],
    }
}

impl Env {
    async fn register(&self) -> crate::ids::RegisterId {
        self.uc
            .get_meter
            .execute(&self.t, self.meter)
            .await
            .unwrap()
            .aggregate
            .registers[0]
            .id
    }

    async fn readings(&self) -> Vec<crate::model::Reading> {
        self.uc
            .get_meter
            .execute(&self.t, self.meter)
            .await
            .unwrap()
            .readings
    }

    async fn confirmation(&self, value: rust_decimal::Decimal) -> InvoiceConfirmation {
        InvoiceConfirmation {
            meter_id: self.meter,
            readings: vec![ReadingsInput {
                meter_id: self.meter,
                at: Utc.with_ymd_and_hms(2026, 1, 9, 23, 0, 0).unwrap(),
                values: vec![(self.register().await, value)],
                note: None,
            }],
            confirm_warnings: false,
            contract: ContractChoice::Create(contract_input()),
            installment: Some(InstallmentInput {
                valid_from: day(2, 1),
                amount: dec!(50),
                every_months: 1,
            }),
            settlement: Some(SettlementInput {
                period_start: day(1, 1),
                period_end: day(1, 9),
                amount: dec!(30),
            }),
        }
    }
}

// @test SPEC_DOC_UPLOADS, SPEC_SEC_UPLOADS
#[tokio::test]
async fn upload_accepts_pdf_and_photos_only() {
    let e = setup(None).await;
    let field = |r: Result<_, AppError>| match r {
        Err(AppError::Validation(f)) => f.0[0].code,
        other => panic!("Validierungsfehler erwartet: {other:?}"),
    };
    assert_eq!(
        field(e.uc.upload_document.execute(&e.t, Bytes::new(), "x").await),
        "document.required"
    );
    assert_eq!(
        field(
            e.uc.upload_document
                .execute(&e.t, Bytes::from_static(b"<html>"), "rechnung.pdf")
                .await
        ),
        "document.type_invalid"
    );
    assert_eq!(
        field(
            e.uc.upload_document
                .execute(&e.t, Bytes::from(vec![b'%'; 2048]), "gross.pdf")
                .await
        ),
        "document.too_large"
    );

    let pdf =
        e.uc.upload_document
            .execute(&e.t, Bytes::from_static(PDF), "../Ordner/Rechnung 2026.pdf")
            .await
            .unwrap();
    let doc = e.uc.get_document.execute(&e.t, pdf).await.unwrap();
    assert_eq!(doc.media_type, DocumentType::Pdf);
    assert_eq!(doc.original_name, "Rechnung 2026.pdf");
    assert_eq!(doc.status, DocumentStatus::Uploaded);
    let (kind, bytes, _) = e.uc.get_document_file.execute(&e.t, pdf).await.unwrap();
    assert_eq!(
        (kind, bytes.as_ref()),
        (DocumentType::Pdf, PDF),
        "unverändert"
    );

    let photo =
        e.uc.upload_document
            .execute(&e.t, Bytes::from_static(JPEG), "foto.jpg")
            .await
            .unwrap();
    let doc = e.uc.get_document.execute(&e.t, photo).await.unwrap();
    assert_eq!(doc.media_type, DocumentType::Jpeg);
    // die Art folgt dem Inhalt, nicht der Endung
    let renamed =
        e.uc.upload_document
            .execute(&e.t, Bytes::from_static(PDF), "scan.jpg")
            .await
            .unwrap();
    let doc = e.uc.get_document.execute(&e.t, renamed).await.unwrap();
    assert_eq!(doc.media_type, DocumentType::Pdf);
    assert_eq!(e.uc.list_documents.execute(&e.t).await.unwrap().len(), 3);
    assert_eq!(e.blobs.keys().len(), 3);
}

// @test SPEC_AI_INVOICE, SPEC_AI_EXTRACTION
#[tokio::test]
async fn analysis_matches_meter_or_leaves_document_for_manual_entry() {
    let e = setup(Some(Ok(extraction()))).await;
    let id =
        e.uc.upload_document
            .execute(&e.t, Bytes::from_static(PDF), "r.pdf")
            .await
            .unwrap();
    assert_eq!(
        e.uc.analyse_document.execute(&e.t, id).await.unwrap(),
        DocumentStatus::Analysed
    );
    let doc = e.uc.get_document.execute(&e.t, id).await.unwrap();
    assert_eq!(doc.meter_id, Some(e.meter), "Zählernummer normalisiert");
    assert_eq!(doc.model.as_deref(), Some("stub"));
    assert_eq!(doc.extraction, Some(extraction()));

    // Fehler der Erkennung und fehlende AI-Konfiguration
    for e in [
        setup(Some(Err(RecognitionError::Timeout))).await,
        setup(None).await,
    ] {
        let id =
            e.uc.upload_document
                .execute(&e.t, Bytes::from_static(PDF), "r.pdf")
                .await
                .unwrap();
        assert_eq!(
            e.uc.analyse_document.execute(&e.t, id).await.unwrap(),
            DocumentStatus::Failed
        );
        let doc = e.uc.get_document.execute(&e.t, id).await.unwrap();
        assert!(doc.extraction.is_none() && doc.status.is_open());
    }
}

// @test SPEC_AI_INVOICE, SPEC_RDG_PLAUSIBILITY
#[tokio::test]
async fn confirmation_saves_everything_in_one_unit_of_work() {
    let e = setup(Some(Ok(extraction()))).await;
    let id =
        e.uc.upload_document
            .execute(&e.t, Bytes::from_static(PDF), "r.pdf")
            .await
            .unwrap();
    e.uc.analyse_document.execute(&e.t, id).await.unwrap();

    // ungültige Abrechnung: auch Ablesung und Vertrag bleiben ungespeichert
    let mut bad = e.confirmation(dec!(180)).await;
    bad.settlement = Some(SettlementInput {
        period_start: day(1, 9),
        period_end: day(1, 1),
        amount: dec!(30),
    });
    assert!(matches!(
        e.uc.confirm_invoice.execute(&e.t, id, bad).await,
        Err(AppError::Validation(_))
    ));
    assert_eq!(e.readings().await.len(), 1, "nur Startstand");
    assert!(
        e.uc.meter_contracts
            .execute(&e.t, e.meter)
            .await
            .unwrap()
            .is_empty()
    );

    // Warnung (Rückgang): erst nach Bestätigung gespeichert
    let warn = e.confirmation(dec!(50)).await;
    let outcome =
        e.uc.confirm_invoice
            .execute(&e.t, id, warn.clone())
            .await
            .unwrap();
    assert!(matches!(outcome, ConfirmOutcome::NeedsConfirmation(ref w) if w.len() == 1));
    assert_eq!(e.readings().await.len(), 1);

    // Abschlag oder Abrechnung ohne Vertrag
    let mut no_contract = e.confirmation(dec!(180)).await;
    no_contract.contract = ContractChoice::None;
    assert!(matches!(
        e.uc.confirm_invoice.execute(&e.t, id, no_contract).await,
        Err(AppError::Validation(_))
    ));

    let ok = e.confirmation(dec!(180)).await;
    let ConfirmOutcome::Saved {
        contract: Some(contract),
    } = e.uc.confirm_invoice.execute(&e.t, id, ok).await.unwrap()
    else {
        panic!("gespeichert erwartet");
    };
    let readings = e.readings().await;
    assert_eq!(readings.len(), 2);
    assert_eq!(readings[1].source, ReadingSource::Invoice);
    let c =
        e.uc.get_contract
            .execute(&e.t, contract)
            .await
            .unwrap()
            .contract;
    assert_eq!(c.installments.len(), 1);
    assert_eq!(c.settlements[0].document_id, Some(id.0), "Beleg verknüpft");
    let doc = e.uc.get_document.execute(&e.t, id).await.unwrap();
    assert_eq!(doc.status, DocumentStatus::Confirmed);
    assert!(e.uc.list_documents.execute(&e.t).await.unwrap().is_empty());
    // kein zweites Mal
    assert!(matches!(
        e.uc.confirm_invoice
            .execute(&e.t, id, e.confirmation(dec!(90)).await)
            .await,
        Err(AppError::NotFound)
    ));
}

// @test SPEC_DOC_UPLOADS, SPEC_OPS_HOUSEKEEPING
#[tokio::test]
async fn housekeeping_keeps_only_documents_of_settlements() {
    let e = setup(Some(Ok(extraction()))).await;
    let upload = || async {
        e.uc.upload_document
            .execute(&e.t, Bytes::from_static(PDF), "r.pdf")
            .await
            .unwrap()
    };
    let receipt = upload().await;
    let open = upload().await;
    let discarded = upload().await;
    e.uc.discard_document
        .execute(&e.t, discarded)
        .await
        .unwrap();
    e.uc.confirm_invoice
        .execute(&e.t, receipt, e.confirmation(dec!(180)).await)
        .await
        .unwrap();
    let hk = Housekeeping::new(
        e.store.clone(),
        e.ports.blobs.clone(),
        e.clock.clone(),
        Duration::hours(24),
    );
    assert_eq!(hk.execute().await.unwrap().documents, 0, "noch jung");
    e.clock.advance(Duration::hours(25));
    assert_eq!(hk.execute().await.unwrap().documents, 2);
    assert!(e.uc.get_document.execute(&e.t, receipt).await.is_ok());
    for id in [open, discarded] {
        assert!(matches!(
            e.uc.get_document.execute(&e.t, id).await,
            Err(AppError::NotFound)
        ));
    }
    assert_eq!(e.blobs.keys().len(), 1);
    assert!(e.clock.now() > Utc.with_ymd_and_hms(2026, 1, 16, 0, 0, 0).unwrap());
}
