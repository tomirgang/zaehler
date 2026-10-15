//! Minimaler OpenRouter-Client (OpenAI-kompatible Chat-Completions-API).

use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{AiError, Document, InvoiceSuggestion, MeterReadingSuggestion, ReceiptSuggestion};

#[derive(Debug, Clone, Deserialize)]
pub struct OpenRouterConfig {
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Vision-fähiges Modell, z.B. `google/gemini-2.5-flash`.
    pub model: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_base_url() -> String {
    "https://openrouter.ai/api/v1".into()
}

const fn default_timeout_secs() -> u64 {
    60
}

#[derive(Debug, Clone)]
pub struct OpenRouter {
    http: reqwest::Client,
    cfg: OpenRouterConfig,
}

impl OpenRouter {
    pub fn new(cfg: OpenRouterConfig) -> Result<Self, AiError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .user_agent(concat!("zaehler/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { http, cfg })
    }

    // @impl SPEC_AI_EXTRACTION
    pub async fn read_meter(&self, doc: &Document) -> Result<MeterReadingSuggestion, AiError> {
        self.extract(
            MeterReadingSuggestion::PROMPT,
            "meter_reading",
            MeterReadingSuggestion::schema(),
            doc,
        )
        .await
    }

    // @impl SPEC_AI_EXTRACTION
    pub async fn read_receipt(&self, doc: &Document) -> Result<ReceiptSuggestion, AiError> {
        self.extract(
            ReceiptSuggestion::PROMPT,
            "receipt",
            ReceiptSuggestion::schema(),
            doc,
        )
        .await
    }

    // @impl SPEC_AI_EXTRACTION
    pub async fn read_invoice(&self, doc: &Document) -> Result<InvoiceSuggestion, AiError> {
        self.extract(
            InvoiceSuggestion::PROMPT,
            "invoice",
            InvoiceSuggestion::schema(),
            doc,
        )
        .await
    }

    async fn extract<T: DeserializeOwned>(
        &self,
        prompt: &str,
        name: &str,
        schema: Value,
        doc: &Document,
    ) -> Result<T, AiError> {
        let body = request_body(&self.cfg.model, prompt, name, &schema, doc);
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.cfg.base_url))
            .bearer_auth(&self.cfg.api_key)
            .header("X-Title", "zaehler")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AiError::Api {
                status: status.as_u16(),
                body,
            });
        }
        let completion: Value = resp.json().await?;
        tracing::debug!(model = %self.cfg.model, kind = name, "ai extraction finished");
        parse_completion(&completion)
    }
}

fn request_body(model: &str, prompt: &str, name: &str, schema: &Value, doc: &Document) -> Value {
    let attachment = match doc {
        Document::Image { mime, bytes } => json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{mime};base64,{}", STANDARD.encode(bytes)) }
        }),
        Document::Pdf { filename, bytes } => json!({
            "type": "file",
            "file": { "filename": filename, "file_data": format!("data:application/pdf;base64,{}", STANDARD.encode(bytes)) }
        }),
    };
    json!({
        "model": model,
        "temperature": 0,
        "messages": [{
            "role": "user",
            "content": [ { "type": "text", "text": prompt }, attachment ]
        }],
        "response_format": {
            "type": "json_schema",
            "json_schema": { "name": name, "strict": true, "schema": schema }
        }
    })
}

fn parse_completion<T: DeserializeOwned>(completion: &Value) -> Result<T, AiError> {
    let content = completion
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| AiError::InvalidResponse("choices[0].message.content fehlt".into()))?;
    serde_json::from_str(content).map_err(|e| AiError::InvalidResponse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // @test SPEC_AI_EXTRACTION
    #[test]
    fn parses_structured_meter_reading() {
        let completion = json!({
            "choices": [{ "message": { "content": r#"{"value":"12345.678","serial_number":"1EMH0012345678","register":"1.8.0","unit":"kWh","meter_kind":"electricity","confidence":0.93,"notes":null}"# } }]
        });
        let s: MeterReadingSuggestion = parse_completion(&completion).unwrap();
        assert_eq!(s.value.as_deref(), Some("12345.678"));
        assert_eq!(s.meter_kind, "electricity");

        let body = request_body(
            "m",
            "p",
            "meter_reading",
            &MeterReadingSuggestion::schema(),
            &Document::Image {
                mime: "image/jpeg".into(),
                bytes: vec![1, 2, 3],
            },
        );
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert!(
            body["messages"][0]["content"][1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
    }
}
