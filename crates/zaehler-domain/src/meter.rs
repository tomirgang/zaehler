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
    pub const ALL: [Self; 6] = [
        Self::Electricity,
        Self::Gas,
        Self::Water,
        Self::Heat,
        Self::Vehicle,
        Self::Custom,
    ];

    /// Bezeichner für Speicherung und Formulare (`snake_case`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Electricity => "electricity",
            Self::Gas => "gas",
            Self::Water => "water",
            Self::Heat => "heat",
            Self::Vehicle => "vehicle",
            Self::Custom => "custom",
        }
    }

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

impl std::str::FromStr for MeterKind {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| UnknownVariant(s.to_owned()))
    }
}

/// Richtung eines Registers: Bezug oder Einspeisung (z.B. PV).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Import,
    Export,
}

impl Direction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Export => "export",
        }
    }
}

impl std::str::FromStr for Direction {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "import" => Ok(Self::Import),
            "export" => Ok(Self::Export),
            other => Err(UnknownVariant(other.to_owned())),
        }
    }
}

/// Unbekannter Bezeichner beim Parsen einer Aufzählung.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unbekannter Wert: {0}")]
pub struct UnknownVariant(pub String);

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

#[cfg(test)]
mod tests {
    use super::*;

    // @test SPEC_MTR_KINDS
    #[test]
    fn kinds_roundtrip_and_default_units() {
        for kind in MeterKind::ALL {
            assert_eq!(kind.as_str().parse::<MeterKind>(), Ok(kind));
        }
        assert!("strom".parse::<MeterKind>().is_err());
        assert_eq!(MeterKind::Electricity.default_unit(), "kWh");
        assert_eq!(MeterKind::Water.default_unit(), "m³");
        assert_eq!(MeterKind::Vehicle.default_unit(), "km");
        assert_eq!("export".parse::<Direction>(), Ok(Direction::Export));
    }
}
