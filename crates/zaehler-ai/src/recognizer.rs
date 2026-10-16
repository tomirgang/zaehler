//! Driven-Adapter für den Port `MeterRecognizer` (ADR_005, ADR_022).

use std::str::FromStr;

use async_trait::async_trait;
use rust_decimal::{Decimal, prelude::FromPrimitive};
use zaehler_app::{
    model::Recognition,
    port::{MeterRecognizer, NormalizedJpeg, RecognitionError},
};
use zaehler_domain::meter::MeterKind;

use crate::{AiError, Document, MeterReadingSuggestion, OpenRouter};

/// Leere Zeichenketten gelten als nicht erkannt.
fn present(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// Abbildung der Antwort auf den Kerntyp; ein Wert, der keine Dezimalzahl
/// ist, macht die Antwort ungültig.
// @impl SPEC_AI_EXTRACTION
pub(crate) fn to_recognition(s: MeterReadingSuggestion) -> Result<Recognition, RecognitionError> {
    let value = present(s.value)
        .map(|v| Decimal::from_str(&v))
        .transpose()
        .map_err(|e| RecognitionError::Invalid(format!("value: {e}")))?;
    let confidence = Decimal::from_f64(s.confidence.clamp(0.0, 1.0))
        .ok_or_else(|| RecognitionError::Invalid("confidence".into()))?
        .round_dp(3);
    Ok(Recognition {
        value,
        serial_number: present(s.serial_number),
        register: present(s.register),
        unit: present(s.unit),
        meter_kind: MeterKind::from_str(&s.meter_kind).ok(),
        confidence,
    })
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
impl MeterRecognizer for OpenRouter {
    fn model(&self) -> &str {
        self.model_name()
    }

    /// Sendet nur das normalisierte JPEG und den festen Prompt (REQ_AI_006).
    // @impl SPEC_AI_EXTRACTION, SPEC_AI_FLOW
    async fn recognize(&self, jpeg: &NormalizedJpeg) -> Result<Recognition, RecognitionError> {
        let doc = Document::Image {
            mime: "image/jpeg".into(),
            bytes: jpeg.bytes.to_vec(),
        };
        let suggestion = self.read_meter(&doc).await.map_err(to_error)?;
        to_recognition(suggestion)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, header, method, path},
    };

    use super::*;
    use crate::OpenRouterConfig;

    fn jpeg() -> NormalizedJpeg {
        NormalizedJpeg {
            bytes: Bytes::from_static(b"\xFF\xD8\xFFtest"),
            width: 1,
            height: 1,
            taken_at: None,
        }
    }

    fn client(server: &MockServer, timeout_secs: u64) -> OpenRouter {
        OpenRouter::new(OpenRouterConfig {
            api_key: "test-key".into(),
            base_url: server.uri(),
            model: "test/vision".into(),
            timeout_secs,
        })
        .unwrap()
    }

    fn completion(content: &str) -> serde_json::Value {
        json!({ "choices": [{ "message": { "content": content } }] })
    }

    // @test SPEC_AI_EXTRACTION, SPEC_AI_FLOW
    #[tokio::test]
    async fn successful_recognition() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_string_contains("\"model\":\"test/vision\""))
            .and(body_string_contains("data:image/jpeg;base64,"))
            .respond_with(ResponseTemplate::new(200).set_body_json(completion(
                r#"{"value":"12345.678","serial_number":" 1EMH0012345678 ","register":"1.8.0","unit":"kWh","meter_kind":"electricity","confidence":0.93,"notes":null}"#,
            )))
            .expect(1)
            .mount(&server)
            .await;
        let r = client(&server, 5).recognize(&jpeg()).await.unwrap();
        assert_eq!(r.value, Some(Decimal::new(12_345_678, 3)));
        assert_eq!(r.serial_number.as_deref(), Some("1EMH0012345678"));
        assert_eq!(r.register.as_deref(), Some("1.8.0"));
        assert_eq!(r.meter_kind, Some(MeterKind::Electricity));
        assert_eq!(r.confidence, Decimal::new(93, 2));
        assert_eq!(client(&server, 5).model(), "test/vision");
    }

    // @test SPEC_AI_EXTRACTION
    #[tokio::test]
    async fn unreadable_fields_stay_empty() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(completion(
                r#"{"value":null,"serial_number":"","register":null,"unit":null,"meter_kind":"unknown","confidence":0.1,"notes":"unscharf"}"#,
            )))
            .mount(&server)
            .await;
        let r = client(&server, 5).recognize(&jpeg()).await.unwrap();
        assert_eq!(r.value, None);
        assert_eq!(r.serial_number, None);
        assert_eq!(r.meter_kind, None);
    }

    // @test SPEC_AI_EXTRACTION
    #[tokio::test]
    async fn invalid_response_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(completion("Der Stand ist 12345")),
            )
            .mount(&server)
            .await;
        let e = client(&server, 5).recognize(&jpeg()).await.unwrap_err();
        assert!(matches!(e, RecognitionError::Invalid(_)), "{e:?}");
        assert_eq!(e.outcome(), "invalid");

        // Wert ohne Dezimalformat trotz Schema
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(completion(
                r#"{"value":"12,5","serial_number":null,"register":null,"unit":null,"meter_kind":"gas","confidence":0.5,"notes":null}"#,
            )))
            .mount(&server)
            .await;
        assert!(matches!(
            client(&server, 5).recognize(&jpeg()).await,
            Err(RecognitionError::Invalid(_))
        ));
    }

    // @test SPEC_AI_EXTRACTION
    #[tokio::test]
    async fn http_500_is_an_error_without_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("intern: geheim"))
            .mount(&server)
            .await;
        let e = client(&server, 5).recognize(&jpeg()).await.unwrap_err();
        assert_eq!(e, RecognitionError::Failed("HTTP 500".into()));
        assert_eq!(e.outcome(), "error");
    }

    // @test SPEC_AI_EXTRACTION
    #[tokio::test]
    async fn timeout_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(completion("{}"))
                    .set_delay(Duration::from_secs(3)),
            )
            .mount(&server)
            .await;
        let e = client(&server, 1).recognize(&jpeg()).await.unwrap_err();
        assert_eq!(e, RecognitionError::Timeout);
        assert_eq!(e.outcome(), "timeout");
    }
}
