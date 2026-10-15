//! Zähler (Messstellen), physische Zählergeräte und Register.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Art der Messstelle. Bestimmt Standard-Einheit, Saisonprofil und verfügbare
/// Auswertungen (z.B. Kosten/km nur bei [`MeterKind::Vehicle`]).
// @impl SPEC_MTR_KINDS
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeterKind {
    Electricity,
    Gas,
    Water,
    Heat,
    Vehicle,
    Custom,
}

impl MeterKind {
    /// Standard-Einheit, die beim Anlegen vorgeschlagen wird.
    #[must_use]
    pub fn default_unit(self) -> &'static str {
        match self {
            Self::Electricity | Self::Heat => "kWh",
            Self::Gas | Self::Water => "m³",
            Self::Vehicle => "km",
            Self::Custom => "",
        }
    }
}

/// Richtung eines Registers: Bezug oder Einspeisung (z.B. PV).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Import,
    Export,
}

/// Logische Messstelle, z.B. "Hausstrom"; überdauert Zählerwechsel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meter {
    pub id: Uuid,
    pub household_id: Uuid,
    pub name: String,
    pub kind: MeterKind,
    pub unit: String,
}

/// Physisches Gerät an einer Messstelle; ein Zählerwechsel beendet ein Gerät
/// und startet ein neues.
// @impl SPEC_CONS_DEVICE_SWAP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeterDevice {
    pub id: Uuid,
    pub meter_id: Uuid,
    pub serial_number: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub removed_at: Option<DateTime<Utc>>,
    /// Anzahl Vorkommastellen; gesetzt bedeutet Überlauf auf 0 ist möglich.
    pub rollover_digits: Option<u8>,
}

/// Register eines Zählers, z.B. `HT`, `NT` oder `Einspeisung`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Register {
    pub id: Uuid,
    pub meter_id: Uuid,
    pub code: String,
    pub direction: Direction,
}
