//! Kostenbuch für Fahrzeuge und Anlagen: Anlagentypen, Kategorien je Typ
//! und wiederkehrende Einträge (SPEC_ASSET_COSTS, ADR_020).

use chrono::{Months, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::period::Period;

/// Art der Anlage, zu der ein Kostenbuch-Eintrag gehört.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Vehicle,
    PvSystem,
}

impl AssetType {
    pub const ALL: [Self; 2] = [Self::Vehicle, Self::PvSystem];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vehicle => "vehicle",
            Self::PvSystem => "pv_system",
        }
    }
}

impl std::str::FromStr for AssetType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|x| x.as_str() == s)
            .ok_or_else(|| format!("unbekannter Anlagentyp: {s}"))
    }
}

/// Kostenkategorien eines Fahrzeugs (REQ_CAR_002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleCostCategory {
    Fuel,
    Charging,
    Insurance,
    Tax,
    Workshop,
    Tires,
    Inspection,
    Care,
    Parking,
    Financing,
    Other,
}

impl VehicleCostCategory {
    pub const ALL: [Self; 11] = [
        Self::Fuel,
        Self::Charging,
        Self::Insurance,
        Self::Tax,
        Self::Workshop,
        Self::Tires,
        Self::Inspection,
        Self::Care,
        Self::Parking,
        Self::Financing,
        Self::Other,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fuel => "fuel",
            Self::Charging => "charging",
            Self::Insurance => "insurance",
            Self::Tax => "tax",
            Self::Workshop => "workshop",
            Self::Tires => "tires",
            Self::Inspection => "inspection",
            Self::Care => "care",
            Self::Parking => "parking",
            Self::Financing => "financing",
            Self::Other => "other",
        }
    }

    /// Variable (fahrleistungsabhängige) Kosten im Gegensatz zu Fixkosten
    /// (SPEC_CAR_COST_PER_KM).
    #[must_use]
    pub fn is_variable(self) -> bool {
        matches!(
            self,
            Self::Fuel | Self::Charging | Self::Workshop | Self::Tires | Self::Care | Self::Parking
        )
    }
}

/// Kostenkategorien einer PV-Anlage (ADR_020).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PvCostCategory {
    Investment,
    Replacement,
    Insurance,
    Maintenance,
    MeterRent,
    Other,
}

impl PvCostCategory {
    pub const ALL: [Self; 6] = [
        Self::Investment,
        Self::Replacement,
        Self::Insurance,
        Self::Maintenance,
        Self::MeterRent,
        Self::Other,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Investment => "investment",
            Self::Replacement => "replacement",
            Self::Insurance => "insurance",
            Self::Maintenance => "maintenance",
            Self::MeterRent => "meter_rent",
            Self::Other => "other",
        }
    }
}

/// Kategorie eines Eintrags; der Anlagentyp bestimmt die zulässigen Werte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetCategory {
    Vehicle(VehicleCostCategory),
    Pv(PvCostCategory),
}

impl AssetCategory {
    #[must_use]
    pub fn asset_type(self) -> AssetType {
        match self {
            Self::Vehicle(_) => AssetType::Vehicle,
            Self::Pv(_) => AssetType::PvSystem,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vehicle(c) => c.as_str(),
            Self::Pv(c) => c.as_str(),
        }
    }

    /// Kategorie `s` des Anlagentyps; `None` bei unbekanntem Wert.
    #[must_use]
    pub fn parse(asset: AssetType, s: &str) -> Option<Self> {
        match asset {
            AssetType::Vehicle => VehicleCostCategory::ALL
                .into_iter()
                .find(|c| c.as_str() == s)
                .map(Self::Vehicle),
            AssetType::PvSystem => PvCostCategory::ALL
                .into_iter()
                .find(|c| c.as_str() == s)
                .map(Self::Pv),
        }
    }

    /// Alle Kategorien des Anlagentyps.
    #[must_use]
    pub fn all(asset: AssetType) -> Vec<Self> {
        match asset {
            AssetType::Vehicle => VehicleCostCategory::ALL.map(Self::Vehicle).to_vec(),
            AssetType::PvSystem => PvCostCategory::ALL.map(Self::Pv).to_vec(),
        }
    }
}

/// Wiederholung eines Eintrags alle `every_months` Monate, optional bis
/// einschließlich `until`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recurrence {
    pub every_months: u8,
    pub until: Option<NaiveDate>,
}

/// Buchungstage eines Eintrags im Zeitraum: `booked_on + k · Turnus`
/// (Monatsende bei kürzeren Monaten), bis `until` einschließlich. Grundlage
/// aller Summen des Kostenbuchs (SPEC_ASSET_COSTS).
// @impl SPEC_ASSET_COSTS
#[must_use]
pub fn occurrences(
    booked_on: NaiveDate,
    recurrence: Option<Recurrence>,
    period: Period,
) -> Vec<NaiveDate> {
    let inside = |d: NaiveDate| d >= period.start && d < period.end;
    let Some(r) = recurrence.filter(|r| r.every_months > 0) else {
        return inside(booked_on).then_some(booked_on).into_iter().collect();
    };
    let mut out = Vec::new();
    for k in 0_u32.. {
        // jeweils vom Buchungstag aus, damit der 31. nicht dauerhaft auf
        // den 28. rutscht
        let Some(d) = booked_on.checked_add_months(Months::new(k * u32::from(r.every_months)))
        else {
            break;
        };
        if d >= period.end || r.until.is_some_and(|u| d > u) {
            break;
        }
        if inside(d) {
            out.push(d);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    // @test SPEC_ASSET_COSTS
    #[test]
    fn yearly_insurance_appears_every_year_until_the_end() {
        let r = Some(Recurrence {
            every_months: 12,
            until: Some(d(2028, 12, 31)),
        });
        for y in 2026..=2028 {
            assert_eq!(occurrences(d(2026, 3, 1), r, Period::year(y)), [d(y, 3, 1)]);
        }
        assert!(occurrences(d(2026, 3, 1), r, Period::year(2029)).is_empty());
        assert!(occurrences(d(2026, 3, 1), r, Period::year(2025)).is_empty());
        // ohne Ende
        let open = Some(Recurrence {
            every_months: 12,
            until: None,
        });
        assert_eq!(
            occurrences(d(2026, 3, 1), open, Period::year(2040)).len(),
            1
        );
    }

    // @test SPEC_ASSET_COSTS
    #[test]
    fn monthly_entries_keep_the_day_and_clamp_to_month_end() {
        let r = Some(Recurrence {
            every_months: 1,
            until: None,
        });
        let days = occurrences(d(2026, 1, 31), r, Period::new(d(2026, 1, 1), d(2026, 5, 1)));
        assert_eq!(
            days,
            [
                d(2026, 1, 31),
                d(2026, 2, 28),
                d(2026, 3, 31),
                d(2026, 4, 30)
            ]
        );
        // einmalig
        assert_eq!(
            occurrences(d(2026, 2, 1), None, Period::year(2026)),
            [d(2026, 2, 1)]
        );
        assert!(occurrences(d(2026, 2, 1), None, Period::year(2027)).is_empty());
    }

    #[test]
    fn categories_depend_on_the_asset_type() {
        assert_eq!(
            AssetCategory::parse(AssetType::Vehicle, "fuel"),
            Some(AssetCategory::Vehicle(VehicleCostCategory::Fuel))
        );
        assert_eq!(AssetCategory::parse(AssetType::PvSystem, "fuel"), None);
        assert_eq!(
            AssetCategory::parse(AssetType::PvSystem, "meter_rent").map(AssetCategory::as_str),
            Some("meter_rent")
        );
        assert_eq!(AssetCategory::all(AssetType::Vehicle).len(), 11);
        assert_eq!("pv_system".parse::<AssetType>(), Ok(AssetType::PvSystem));
    }
}
