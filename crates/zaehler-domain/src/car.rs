//! Fahrzeugkosten: Kostenbuch, Wertverlust und Kosten pro Kilometer.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::period::Period;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostCategory {
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

impl CostCategory {
    /// Variable (fahrleistungsabhängige) Kosten im Gegensatz zu Fixkosten.
    #[must_use]
    pub fn is_variable(self) -> bool {
        matches!(
            self,
            Self::Fuel | Self::Charging | Self::Workshop | Self::Tires | Self::Care | Self::Parking
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostEntry {
    pub date: NaiveDate,
    pub category: CostCategory,
    pub amount: Decimal,
    /// Menge bei Tanken/Laden (l bzw. kWh) für Verbrauchsberechnung.
    pub quantity: Option<Decimal>,
}

/// Anschaffung für die Wertverlust-Berechnung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acquisition {
    pub date: NaiveDate,
    pub price: Decimal,
    /// Geschätzter Restwert am Ende der geplanten Haltedauer bzw. Verkaufserlös.
    pub residual_value: Decimal,
    pub planned_end: NaiveDate,
}

/// Linearer Wertverlust im Zeitraum (tagesgenau).
// @impl SPEC_CAR_DEPRECIATION
#[must_use]
pub fn depreciation_in(acq: &Acquisition, period: Period) -> Decimal {
    let total_days = (acq.planned_end - acq.date).num_days();
    if total_days <= 0 {
        return Decimal::ZERO;
    }
    let start = period.start.max(acq.date);
    let end = period.end.min(acq.planned_end);
    let days = (end - start).num_days().max(0);
    (acq.price - acq.residual_value) * Decimal::from(days) / Decimal::from(total_days)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostPerKm {
    pub km: Decimal,
    pub variable: Decimal,
    pub fixed: Decimal,
    pub depreciation: Decimal,
    /// Variable Kosten pro km.
    pub variable_per_km: Option<Decimal>,
    /// Vollkosten (variabel + fix + Wertverlust) pro km.
    pub total_per_km: Option<Decimal>,
}

/// Kosten pro km für einen Zeitraum (Jahr oder gesamte Haltedauer).
// @impl SPEC_CAR_COST_PER_KM
#[must_use]
pub fn cost_per_km(
    entries: &[CostEntry],
    acquisition: Option<&Acquisition>,
    period: Period,
    km: Decimal,
) -> CostPerKm {
    let in_period = entries
        .iter()
        .filter(|e| e.date >= period.start && e.date < period.end);
    let (mut variable, mut fixed) = (Decimal::ZERO, Decimal::ZERO);
    for e in in_period {
        if e.category.is_variable() {
            variable += e.amount;
        } else {
            fixed += e.amount;
        }
    }
    let depreciation = acquisition.map_or(Decimal::ZERO, |a| depreciation_in(a, period));
    let per_km = |v: Decimal| (!km.is_zero()).then(|| (v / km).round_dp(4));
    CostPerKm {
        km,
        variable,
        fixed,
        depreciation,
        variable_per_km: per_km(variable),
        total_per_km: per_km(variable + fixed + depreciation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    // @test SPEC_CAR_COST_PER_KM, SPEC_CAR_DEPRECIATION
    #[test]
    fn full_cost_per_km() {
        let entries = [
            CostEntry {
                date: d(2026, 2, 1),
                category: CostCategory::Fuel,
                amount: dec!(1000),
                quantity: Some(dec!(550)),
            },
            CostEntry {
                date: d(2026, 3, 1),
                category: CostCategory::Insurance,
                amount: dec!(500),
                quantity: None,
            },
        ];
        let acq = Acquisition {
            date: d(2026, 1, 1),
            price: dec!(30000),
            residual_value: dec!(10000),
            planned_end: d(2031, 1, 1),
        };
        let r = cost_per_km(&entries, Some(&acq), Period::year(2026), dec!(10000));
        assert_eq!(r.variable_per_km, Some(dec!(0.1)));
        // 20000 € Wertverlust über 1826 Tage, davon 365 in 2026
        assert_eq!(r.depreciation.round_dp(2), dec!(3997.81));
        assert_eq!(r.total_per_km, Some(dec!(0.5498)));
    }
}
