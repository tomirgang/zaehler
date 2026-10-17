//! AI-gestützte Extraktion strukturierter Daten aus Fotos und PDFs.
//!
//! Die AI liefert ausschließlich **Vorschläge** (ADR_005); Zuordnung zum
//! Zähler, Plausibilisierung und Bestätigung erfolgen in der Anwendung.

mod extractor;
mod openrouter;
mod recognizer;
mod schema;

pub use openrouter::{OpenRouter, OpenRouterConfig};
pub use schema::{InvoiceSuggestion, MeterReadingSuggestion, ReceiptSuggestion};

use thiserror::Error;

/// Eingangsdokument für die Erkennung.
#[derive(Debug, Clone)]
pub enum Document {
    Image { mime: String, bytes: Vec<u8> },
    Pdf { filename: String, bytes: Vec<u8> },
}

#[derive(Debug, Error)]
pub enum AiError {
    #[error("HTTP-Fehler: {0}")]
    Http(#[from] reqwest::Error),
    #[error("OpenRouter meldet Fehler {status}: {body}")]
    Api { status: u16, body: String },
    #[error("Antwort ist kein gültiges JSON gemäß Schema: {0}")]
    InvalidResponse(String),
}
