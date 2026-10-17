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

/// Jahres-/Schlussrechnung eines Versorgers (SPEC_AI_INVOICE).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct InvoiceSuggestion {
    pub provider: Option<String>,
    pub contract_number: Option<String>,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub readings: Vec<InvoiceReading>,
    pub prices: Vec<InvoicePrice>,
    pub installment: Option<InvoiceInstallment>,
    /// Rechnungsbetrag (Kosten des Zeitraums, brutto)
    pub total_amount: Option<String>,
    pub confidence: f64,
}

/// Neuer Abschlag laut Rechnung.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct InvoiceInstallment {
    pub amount: String,
    pub interval_months: u8,
    pub valid_from: Option<String>,
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
    pub(crate) const PROMPT: &'static str = "Extrahiere aus der Versorgerrechnung (Strom, Gas, Wasser, Wärme): \
        Anbieter, Vertragsnummer, Abrechnungszeitraum (erster und letzter Tag, ISO 8601), \
        alle Zählerstände mit Zählernummer, Register, Datum (ISO 8601) und Stand, jeweils Anfangs- und Endstand, \
        alle Preispositionen brutto mit Grundpreis, Intervall des Grundpreises (day, month oder year), \
        Arbeitspreis je Einheit und Gültigkeitsbeginn, den künftigen Abschlag mit Betrag, Turnus in Monaten und Beginn, \
        sowie den Rechnungsbetrag (Bruttokosten des Zeitraums vor Abzug der Abschläge). \
        Beträge, Preise und Stände als Dezimalstring mit Punkt. Unlesbares als null. Erfinde keine Werte.";

    pub(crate) fn schema() -> Value {
        let s = json!({"type": ["string", "null"]});
        let date = json!({"type": ["string", "null"], "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$"});
        let decimal = json!({"type": ["string", "null"], "pattern": "^-?[0-9]+(\\.[0-9]+)?$"});
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["provider", "contract_number", "period_start", "period_end", "readings", "prices", "installment", "total_amount", "confidence"],
            "properties": {
                "provider": s, "contract_number": s,
                "period_start": date, "period_end": date,
                "total_amount": decimal,
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
                "readings": { "type": "array", "items": {
                    "type": "object", "additionalProperties": false,
                    "required": ["serial_number", "register", "date", "value"],
                    "properties": {
                        "serial_number": s, "register": s,
                        "date": {"type": "string", "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$"},
                        "value": {"type": "string", "pattern": "^[0-9]+(\\.[0-9]+)?$"}
                    }
                }},
                "prices": { "type": "array", "items": {
                    "type": "object", "additionalProperties": false,
                    "required": ["position", "valid_from", "base_price", "base_interval", "unit_price"],
                    "properties": {
                        "position": {"type": "string"}, "valid_from": date,
                        "base_price": decimal, "unit_price": decimal,
                        "base_interval": {"type": ["string", "null"], "enum": ["day", "month", "year", null]}
                    }
                }},
                "installment": { "type": ["object", "null"], "additionalProperties": false,
                    "required": ["amount", "interval_months", "valid_from"],
                    "properties": {
                        "amount": {"type": "string", "pattern": "^[0-9]+(\\.[0-9]+)?$"},
                        "interval_months": {"type": "integer", "minimum": 1, "maximum": 12},
                        "valid_from": date
                    }
                }
            }
        })
    }
}
