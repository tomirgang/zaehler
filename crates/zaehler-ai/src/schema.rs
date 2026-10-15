//! Antwortstrukturen und zugehörige JSON-Schemas (Structured Outputs).
//!
//! Dezimalwerte werden als String übertragen, um Rundungsfehler durch
//! Fließkommazahlen zu vermeiden.

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MeterReadingSuggestion {
    /// Abgelesener Stand als Dezimalstring mit Punkt, z.B. `"12345.678"`.
    pub value: Option<String>,
    /// Zählernummer laut Typenschild; dient der automatischen Zuordnung.
    pub serial_number: Option<String>,
    /// Erkanntes Register (z.B. `1.8.1`, `HT`, `NT`), falls angezeigt.
    pub register: Option<String>,
    pub unit: Option<String>,
    /// `electricity`, `gas`, `water`, `heat`, `vehicle` oder `unknown`.
    pub meter_kind: String,
    /// Selbsteinschätzung 0..1.
    pub confidence: f64,
    pub notes: Option<String>,
}

impl MeterReadingSuggestion {
    pub(crate) const PROMPT: &'static str = "Du liest Verbrauchszähler (Strom, Gas, Wasser, Wärme) \
        oder Kilometerzähler von Fahrzeugen ab. Gib den Zählerstand exakt so an, wie er angezeigt wird. \
        Nachkommastellen (oft rot umrandet oder kleiner) gehören mit Dezimalpunkt dazu. \
        Lies die Zählernummer vom Typenschild, falls sichtbar. Wenn etwas nicht lesbar ist, gib null an \
        und senke die confidence. Erfinde keine Werte.";

    pub(crate) fn schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["value", "serial_number", "register", "unit", "meter_kind", "confidence", "notes"],
            "properties": {
                "value": { "type": ["string", "null"], "pattern": "^[0-9]+(\\.[0-9]+)?$" },
                "serial_number": { "type": ["string", "null"] },
                "register": { "type": ["string", "null"] },
                "unit": { "type": ["string", "null"] },
                "meter_kind": { "type": "string", "enum": ["electricity", "gas", "water", "heat", "vehicle", "unknown"] },
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
                "notes": { "type": ["string", "null"] }
            }
        })
    }
}

/// Tankquittung / Werkstattrechnung (Fahrzeugkosten).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReceiptSuggestion {
    pub date: Option<String>,
    pub total_amount: Option<String>,
    /// Kategorie gemäß `zaehler_domain::car::CostCategory` (`snake_case`).
    pub category: String,
    pub quantity: Option<String>,
    pub quantity_unit: Option<String>,
    pub vendor: Option<String>,
    pub confidence: f64,
}

impl ReceiptSuggestion {
    pub(crate) const PROMPT: &'static str = "Extrahiere aus dem Beleg Datum (ISO 8601), Bruttobetrag in EUR, \
        Kostenkategorie, bei Tank-/Ladebelegen die Menge (Liter bzw. kWh) und den Händler. \
        Unlesbares als null. Erfinde keine Werte.";

    pub(crate) fn schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["date", "total_amount", "category", "quantity", "quantity_unit", "vendor", "confidence"],
            "properties": {
                "date": { "type": ["string", "null"] },
                "total_amount": { "type": ["string", "null"] },
                "category": { "type": "string", "enum": ["fuel", "charging", "insurance", "tax", "workshop", "tires", "inspection", "care", "parking", "financing", "other"] },
                "quantity": { "type": ["string", "null"] },
                "quantity_unit": { "type": ["string", "null"] },
                "vendor": { "type": ["string", "null"] },
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
            }
        })
    }
}

/// Jahres-/Schlussrechnung eines Versorgers.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct InvoiceSuggestion {
    pub provider: Option<String>,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub readings: Vec<InvoiceReading>,
    pub prices: Vec<InvoicePrice>,
    pub installments_paid: Option<String>,
    pub new_installment: Option<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct InvoiceReading {
    pub serial_number: Option<String>,
    pub register: Option<String>,
    pub date: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct InvoicePrice {
    pub position: String,
    pub valid_from: Option<String>,
    pub base_price: Option<String>,
    /// `day`, `month` oder `year`.
    pub base_interval: Option<String>,
    pub unit_price: Option<String>,
}

impl InvoiceSuggestion {
    pub(crate) const PROMPT: &'static str = "Extrahiere aus der Versorgerrechnung: Anbieter, Abrechnungszeitraum, \
        alle Zählerstände (Zählernummer, Register, Datum ISO 8601, Stand), alle Preispositionen brutto \
        (Grundpreis mit Intervall, Arbeitspreis, Gültigkeit), Summe gezahlter Abschläge und den neuen Abschlag. \
        Beträge als Dezimalstring mit Punkt. Erfinde keine Werte.";

    pub(crate) fn schema() -> Value {
        let s = json!({"type": ["string", "null"]});
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["provider", "period_start", "period_end", "readings", "prices", "installments_paid", "new_installment", "confidence"],
            "properties": {
                "provider": s, "period_start": s, "period_end": s,
                "installments_paid": s, "new_installment": s,
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
                "readings": { "type": "array", "items": {
                    "type": "object", "additionalProperties": false,
                    "required": ["serial_number", "register", "date", "value"],
                    "properties": { "serial_number": s, "register": s, "date": {"type": "string"}, "value": {"type": "string"} }
                }},
                "prices": { "type": "array", "items": {
                    "type": "object", "additionalProperties": false,
                    "required": ["position", "valid_from", "base_price", "base_interval", "unit_price"],
                    "properties": { "position": {"type": "string"}, "valid_from": s, "base_price": s, "base_interval": s, "unit_price": s }
                }}
            }
        })
    }
}
