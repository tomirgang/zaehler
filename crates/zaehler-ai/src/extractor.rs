//! Driven-Adapter für den Port `DocumentExtractor`: Rechnungen als PDF
//! (`file`-Part) oder Foto (SPEC_AI_INVOICE, SPEC_AI_EXTRACTION).

use std::str::FromStr;

use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::{Decimal, prelude::FromPrimitive};
use zaehler_app::{
    model::{
        DocumentType, InvoiceExtraction, InvoiceInstallment, InvoicePrice, InvoiceReading,
        ReceiptExtraction,
    },
    port::{DocumentContent, DocumentExtractor, RecognitionError},
};
use zaehler_domain::{assets::VehicleCostCategory, tariff::BasePriceInterval};

use crate::{AiError, Document, InvoiceSuggestion, OpenRouter, ReceiptSuggestion};

fn present(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn invalid(field: &str) -> RecognitionError {
    RecognitionError::Invalid(field.to_owned())
}

fn date(s: &str, field: &str) -> Result<NaiveDate, RecognitionError> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| invalid(field))
}

fn decimal(s: &str, field: &str) -> Result<Decimal, RecognitionError> {
    Decimal::from_str(s.trim()).map_err(|_| invalid(field))
}

fn opt<T>(
    s: Option<String>,
    f: impl Fn(&str) -> Result<T, RecognitionError>,
) -> Result<Option<T>, RecognitionError> {
    present(s).map(|v| f(&v)).transpose()
}

/// Abbildung der Antwort auf den Kerntyp; Datum oder Zahl in falschem
/// Format machen die Antwort ungültig (keine Heuristik).
// @impl SPEC_AI_INVOICE, SPEC_AI_EXTRACTION
pub(crate) fn to_extraction(s: InvoiceSuggestion) -> Result<InvoiceExtraction, RecognitionError> {
    let readings = s
        .readings
        .into_iter()
        .map(|r| {
            Ok(InvoiceReading {
                serial_number: present(r.serial_number),
                register: present(r.register),
                date: date(&r.date, "readings.date")?,
                value: decimal(&r.value, "readings.value")?,
            })
        })
        .collect::<Result<Vec<_>, RecognitionError>>()?;
    let prices = s
        .prices
        .into_iter()
        .filter(|p| !p.position.trim().is_empty())
        .map(|p| {
            Ok(InvoicePrice {
                position: p.position.trim().to_owned(),
                valid_from: opt(p.valid_from, |v| date(v, "prices.valid_from"))?,
                base_price: opt(p.base_price, |v| decimal(v, "prices.base_price"))?,
                base_interval: opt(p.base_interval, |v| {
                    BasePriceInterval::from_str(v).map_err(|_| invalid("prices.base_interval"))
                })?,
                unit_price: opt(p.unit_price, |v| decimal(v, "prices.unit_price"))?,
            })
        })
        .collect::<Result<Vec<_>, RecognitionError>>()?;
    let installment = s
        .installment
        .map(|i| {
            Ok::<_, RecognitionError>(InvoiceInstallment {
                amount: decimal(&i.amount, "installment.amount")?,
                every_months: i.interval_months.clamp(1, 12),
                valid_from: opt(i.valid_from, |v| date(v, "installment.valid_from"))?,
            })
        })
        .transpose()?;
    Ok(InvoiceExtraction {
        provider: present(s.provider),
        contract_number: present(s.contract_number),
        period_start: opt(s.period_start, |v| date(v, "period_start"))?,
        period_end: opt(s.period_end, |v| date(v, "period_end"))?,
        readings,
        prices,
        installment,
        total_amount: opt(s.total_amount, |v| decimal(v, "total_amount"))?,
        confidence: Decimal::from_f64(s.confidence.clamp(0.0, 1.0))
            .ok_or_else(|| invalid("confidence"))?
            .round_dp(3),
    })
}

/// Abbildung eines Belegs; unbekannte Kategorie oder falsches Format
/// machen die Antwort ungültig.
// @impl SPEC_AI_RECEIPT, SPEC_AI_EXTRACTION
pub(crate) fn to_receipt(s: ReceiptSuggestion) -> Result<ReceiptExtraction, RecognitionError> {
    let category = VehicleCostCategory::ALL
        .into_iter()
        .find(|c| c.as_str() == s.category.trim())
        .ok_or_else(|| invalid("category"))?;
    let unit = present(s.quantity_unit).map(|u| {
        if u.eq_ignore_ascii_case("kwh") {
            "kWh".to_owned()
        } else if u.eq_ignore_ascii_case("l") || u.to_lowercase().starts_with("liter") {
            "l".to_owned()
        } else {
            u
        }
    });
    let charging_kind = present(s.charging_kind)
        .map(|k| k.to_lowercase())
        .filter(|k| k == "ac" || k == "dc");
    Ok(ReceiptExtraction {
        date: opt(s.date, |v| date(v, "date"))?,
        total_amount: opt(s.total_amount, |v| decimal(v, "total_amount"))?,
        category: Some(category),
        quantity: opt(s.quantity, |v| decimal(v, "quantity"))?,
        unit_price: opt(s.unit_price, |v| decimal(v, "unit_price"))?,
        unit,
        vendor: present(s.vendor),
        odometer: opt(s.odometer, |v| decimal(v, "odometer"))?,
        charging_kind,
        confidence: Decimal::from_f64(s.confidence.clamp(0.0, 1.0))
            .ok_or_else(|| invalid("confidence"))?
            .round_dp(3),
    })
}

/// PDF als `file`-Part, Foto als Data-URL.
fn document_of(doc: &DocumentContent) -> Document {
    match doc.media_type {
        DocumentType::Pdf => Document::Pdf {
            filename: "dokument.pdf".into(),
            bytes: doc.bytes.to_vec(),
        },
        DocumentType::Jpeg => Document::Image {
            mime: "image/jpeg".into(),
            bytes: doc.bytes.to_vec(),
        },
    }
}

fn to_error(e: AiError) -> RecognitionError {
    match e {
        AiError::Http(e) if e.is_timeout() => RecognitionError::Timeout,
        AiError::Http(e) => RecognitionError::Failed(e.to_string()),
        // Antworttext nicht übernehmen: er gelangt sonst in Logs
        AiError::Api { status, .. } => RecognitionError::Failed(format!("HTTP {status}")),
        AiError::InvalidResponse(m) => RecognitionError::Invalid(m),
    }
}

#[async_trait]
impl DocumentExtractor for OpenRouter {
    fn model(&self) -> &str {
        self.document_model_name()
    }

    /// Sendet nur das Dokument und den festen Prompt (REQ_AI_006); PDFs als
    /// `file`-Part, Fotos als Data-URL.
    // @impl SPEC_AI_INVOICE, SPEC_AI_EXTRACTION, SPEC_OPS_OTEL
    #[tracing::instrument(
        name = "ai.invoice",
        skip_all,
        fields(model = %self.document_model_name(), outcome = tracing::field::Empty)
    )]
    async fn invoice(&self, doc: &DocumentContent) -> Result<InvoiceExtraction, RecognitionError> {
        let document = document_of(doc);
        let result = match self.read_invoice(&document).await {
            Ok(s) => to_extraction(s),
            Err(e) => Err(to_error(e)),
        };
        let outcome = result
            .as_ref()
            .map_or_else(RecognitionError::outcome, |_| "ok");
        tracing::Span::current().record("outcome", outcome);
        result
    }

    /// Wie `invoice` mit dem Schema für Belege (SPEC_AI_RECEIPT).
    // @impl SPEC_AI_RECEIPT, SPEC_OPS_OTEL
    #[tracing::instrument(
        name = "ai.receipt",
        skip_all,
        fields(model = %self.document_model_name(), outcome = tracing::field::Empty)
    )]
    async fn receipt(&self, doc: &DocumentContent) -> Result<ReceiptExtraction, RecognitionError> {
        let result = match self.read_receipt(&document_of(doc)).await {
            Ok(s) => to_receipt(s),
            Err(e) => Err(to_error(e)),
        };
        let outcome = result
            .as_ref()
            .map_or_else(RecognitionError::outcome, |_| "ok");
        tracing::Span::current().record("outcome", outcome);
        result
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };

    use super::*;
    use crate::OpenRouterConfig;

    fn client(server: &MockServer, timeout_secs: u64) -> OpenRouter {
        OpenRouter::new(OpenRouterConfig {
            api_key: "test-key".into(),
            base_url: server.uri(),
            model: "test/vision".into(),
            document_model: Some("test/documents".into()),
            timeout_secs,
        })
        .unwrap()
    }

    fn pdf() -> DocumentContent {
        DocumentContent {
            media_type: DocumentType::Pdf,
            bytes: Bytes::from_static(b"%PDF-1.7 test"),
        }
    }

    fn completion(content: &str) -> serde_json::Value {
        json!({ "choices": [{ "message": { "content": content } }] })
    }

    const INVOICE: &str = r#"{
        "provider": "Stadtwerke Süd",
        "contract_number": " V-4711 ",
        "period_start": "2025-01-01",
        "period_end": "2025-12-31",
        "readings": [
            {"serial_number": "1EMH0012", "register": "1.8.0", "date": "2025-01-01", "value": "10000.0"},
            {"serial_number": "1EMH0012", "register": "1.8.0", "date": "2025-12-31", "value": "13650.0"}
        ],
        "prices": [
            {"position": "Arbeitspreis", "valid_from": "2025-01-01", "base_price": null, "base_interval": null, "unit_price": "0.3012"},
            {"position": "Grundpreis", "valid_from": "2025-01-01", "base_price": "143.40", "base_interval": "year", "unit_price": null}
        ],
        "installment": {"amount": "105", "interval_months": 1, "valid_from": "2026-02-15"},
        "total_amount": "1242.78",
        "confidence": 0.82
    }"#;

    // @test SPEC_AI_INVOICE, SPEC_AI_EXTRACTION
    #[tokio::test]
    async fn invoice_from_pdf_with_document_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_string_contains("\"model\":\"test/documents\""))
            .and(body_string_contains("data:application/pdf;base64,"))
            .and(body_string_contains("\"type\":\"file\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(completion(INVOICE)))
            .expect(1)
            .mount(&server)
            .await;
        let c = client(&server, 5);
        assert_eq!(DocumentExtractor::model(&c), "test/documents");
        let x = c.invoice(&pdf()).await.unwrap();
        assert_eq!(x.provider.as_deref(), Some("Stadtwerke Süd"));
        assert_eq!(x.contract_number.as_deref(), Some("V-4711"));
        assert_eq!(x.period_end, NaiveDate::from_ymd_opt(2025, 12, 31));
        assert_eq!(x.readings.len(), 2);
        assert_eq!(x.readings[1].value, Decimal::new(136_500, 1));
        assert_eq!(x.prices[1].base_interval, Some(BasePriceInterval::Year));
        assert_eq!(x.prices[0].unit_price, Some(Decimal::new(3012, 4)));
        let inst = x.installment.unwrap();
        assert_eq!((inst.amount, inst.every_months), (Decimal::from(105), 1));
        assert_eq!(x.total_amount, Some(Decimal::new(124_278, 2)));
        assert_eq!(x.confidence, Decimal::new(82, 2));
    }

    // @test SPEC_AI_INVOICE
    #[tokio::test]
    async fn photo_is_sent_as_image() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("data:image/jpeg;base64,"))
            .respond_with(ResponseTemplate::new(200).set_body_json(completion(INVOICE)))
            .expect(1)
            .mount(&server)
            .await;
        let doc = DocumentContent {
            media_type: DocumentType::Jpeg,
            bytes: Bytes::from_static(b"\xFF\xD8\xFFtest"),
        };
        assert!(client(&server, 5).invoice(&doc).await.is_ok());
    }

    // @test SPEC_AI_INVOICE, SPEC_AI_EXTRACTION
    #[tokio::test]
    async fn invalid_json_and_formats_are_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(completion("kein JSON")))
            .mount(&server)
            .await;
        let e = client(&server, 5).invoice(&pdf()).await.unwrap_err();
        assert_eq!(e.outcome(), "invalid");

        let server = MockServer::start().await;
        let wrong_date = INVOICE.replace("\"2025-12-31\",", "\"31.12.2025\",");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(completion(&wrong_date)))
            .mount(&server)
            .await;
        assert!(matches!(
            client(&server, 5).invoice(&pdf()).await,
            Err(RecognitionError::Invalid(_))
        ));
    }

    // @test SPEC_AI_INVOICE
    #[tokio::test]
    async fn timeout_and_http_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(completion(INVOICE))
                    .set_delay(Duration::from_secs(3)),
            )
            .mount(&server)
            .await;
        assert_eq!(
            client(&server, 1).invoice(&pdf()).await.unwrap_err(),
            RecognitionError::Timeout
        );

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(502).set_body_string("geheim"))
            .mount(&server)
            .await;
        assert_eq!(
            client(&server, 5).invoice(&pdf()).await.unwrap_err(),
            RecognitionError::Failed("HTTP 502".into())
        );
    }

    const RECEIPT: &str = r#"{
        "date": "2026-03-14",
        "total_amount": "18.99",
        "category": "charging",
        "quantity": "32.125",
        "quantity_unit": "KWH",
        "vendor": " Ladepark Süd ",
        "odometer": "12345",
        "charging_kind": "DC",
        "confidence": 0.91
    }"#;

    // @test SPEC_AI_RECEIPT, SPEC_CAR_FUEL_PRICE_UI
    #[test]
    fn fuel_receipt_preserves_price_per_litre() {
        let suggestion: ReceiptSuggestion = serde_json::from_value(serde_json::json!({
            "date": "2026-03-14", "total_amount": "81.08",
            "category": "fuel", "quantity": "45.32", "unit_price": "1.789",
            "quantity_unit": "l", "vendor": "Tankstelle", "confidence": 0.95
        }))
        .unwrap();
        let extracted = to_receipt(suggestion).unwrap();
        assert_eq!(extracted.unit_price, Some(Decimal::new(1789, 3)));
        assert_eq!(extracted.quantity, Some(Decimal::new(4532, 2)));
        assert!(!extracted.is_charging());
    }

    // @test SPEC_AI_RECEIPT, SPEC_AI_EXTRACTION
    #[tokio::test]
    async fn receipt_from_photo_with_document_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_string_contains("\"model\":\"test/documents\""))
            .and(body_string_contains("\"name\":\"receipt\""))
            .and(body_string_contains("data:image/jpeg;base64,"))
            .respond_with(ResponseTemplate::new(200).set_body_json(completion(RECEIPT)))
            .expect(1)
            .mount(&server)
            .await;
        let doc = DocumentContent {
            media_type: DocumentType::Jpeg,
            bytes: Bytes::from_static(b"\xFF\xD8\xFFtest"),
        };
        let x = client(&server, 5).receipt(&doc).await.unwrap();
        assert_eq!(x.date, NaiveDate::from_ymd_opt(2026, 3, 14));
        assert_eq!(x.total_amount, Some(Decimal::new(1899, 2)));
        assert_eq!(x.category, Some(VehicleCostCategory::Charging));
        assert_eq!(x.unit.as_deref(), Some("kWh"));
        assert!(x.is_charging());
        assert_eq!(x.vendor.as_deref(), Some("Ladepark Süd"));
        assert_eq!(x.odometer, Some(Decimal::from(12345)));
        assert_eq!(x.charging_kind.as_deref(), Some("dc"));
    }

    // @test SPEC_AI_RECEIPT
    #[tokio::test]
    async fn receipt_errors() {
        let server = MockServer::start().await;
        let wrong = RECEIPT.replace("\"charging\"", "\"snacks\"");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(completion(&wrong)))
            .mount(&server)
            .await;
        assert!(matches!(
            client(&server, 5).receipt(&pdf()).await,
            Err(RecognitionError::Invalid(_))
        ));
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(completion(RECEIPT))
                    .set_delay(Duration::from_secs(3)),
            )
            .mount(&server)
            .await;
        assert_eq!(
            client(&server, 1).receipt(&pdf()).await.unwrap_err(),
            RecognitionError::Timeout
        );
    }
}
