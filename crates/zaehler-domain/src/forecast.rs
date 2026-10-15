//! Hochrechnung des Verbrauchs auf einen Zeitraum (Kalender- oder Abrechnungsjahr).

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::period::Period;

/// Saisonprofil: relativer Anteil jedes Kalendermonats am Jahresverbrauch.
/// Innerhalb eines Monats wird gleichmäßig auf die Tage verteilt.
// @impl SPEC_FC_SEASON_PROFILE
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonProfile {
    /// Gewichte Januar..Dezember; müssen nicht auf 1 normiert sein.
    pub monthly_weights: [Decimal; 12],
}

impl SeasonProfile {
    /// Gleichverteilung pro Tag (Strom ohne Heizung, Wasser, km).
    #[must_use]
    pub fn uniform() -> Self {
        let days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        Self {
            monthly_weights: days.map(Decimal::from),
        }
    }

    /// Lernt das Profil aus Monatsverbräuchen mehrerer Jahre: `samples` sind
    /// `(Monat 1..=12, Verbrauch)`. Monate ohne Daten fallen auf Gleichverteilung zurück.
    #[must_use]
    pub fn learn(samples: &[(u32, Decimal)]) -> Self {
        let uniform = Self::uniform();
        let total_uniform: Decimal = uniform.monthly_weights.iter().sum();
        let mut sums = [Decimal::ZERO; 12];
        let mut counts = [0_u32; 12];
        for (month, value) in samples {
            if let Some(idx) = month.checked_sub(1).map(|m| m as usize).filter(|i| *i < 12) {
                sums[idx] += *value;
                counts[idx] += 1;
            }
        }
        if counts.contains(&0) {
            return uniform;
        }
        let avgs: [Decimal; 12] = std::array::from_fn(|i| sums[i] / Decimal::from(counts[i]));
        let total: Decimal = avgs.iter().sum();
        if total.is_zero() {
            return uniform;
        }
        Self {
            monthly_weights: avgs.map(|a| a / total * total_uniform),
        }
    }

    fn day_weight(&self, date: NaiveDate) -> Decimal {
        let idx = date.month0() as usize;
        let days_in_month = Period::month_of(date).days();
        self.monthly_weights[idx] / Decimal::from(days_in_month)
    }

    /// Summe der Tagesgewichte im Zeitraum.
    #[must_use]
    pub fn weight(&self, period: Period) -> Decimal {
        period
            .start
            .iter_days()
            .take_while(|d| *d < period.end)
            .map(|d| self.day_weight(d))
            .sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forecast {
    pub period: Period,
    /// Tatsächlich gemessener Verbrauch im abgedeckten Teil.
    pub observed: Decimal,
    /// Hochgerechneter Gesamtverbrauch des Zeitraums.
    pub projected_total: Decimal,
    /// Anteil (0..1) des Zeitraums, der durch Messwerte abgedeckt ist (gewichtet).
    pub coverage: Decimal,
}

/// Rechnet den in `[period.start, observed_until)` gemessenen Verbrauch
/// anhand des Saisonprofils auf den Gesamtzeitraum hoch.
// @impl SPEC_FC_PERIOD_FORECAST
#[must_use]
pub fn project(
    period: Period,
    observed_until: NaiveDate,
    observed: Decimal,
    profile: &SeasonProfile,
) -> Option<Forecast> {
    let until = observed_until.clamp(period.start, period.end);
    let covered = profile.weight(Period::new(period.start, until));
    let total = profile.weight(period);
    if covered.is_zero() || total.is_zero() {
        return None;
    }
    Some(Forecast {
        period,
        observed,
        projected_total: (observed * total / covered).round_dp(3),
        coverage: (covered / total).round_dp(4),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    // @test SPEC_FC_PERIOD_FORECAST
    #[test]
    fn uniform_projection_scales_by_days() {
        let f = project(
            Period::year(2026),
            d(7, 2),
            dec!(1820),
            &SeasonProfile::uniform(),
        )
        .unwrap();
        // 182 von 365 Tagen gemessen
        assert_eq!(f.projected_total, dec!(3650));
    }

    // @test SPEC_FC_SEASON_PROFILE
    #[test]
    fn learned_profile_weights_winter_higher() {
        let samples: Vec<(u32, Decimal)> = (1..=12)
            .map(|m| {
                (
                    m,
                    if m <= 3 || m >= 10 {
                        dec!(200)
                    } else {
                        dec!(50)
                    },
                )
            })
            .collect();
        let p = SeasonProfile::learn(&samples);
        assert!(p.weight(Period::month_of(d(1, 1))) > p.weight(Period::month_of(d(7, 1))));
        let f = project(Period::year(2026), d(4, 1), dec!(600), &p).unwrap();
        // Q1 = 600 von 1500 Gesamtjahresgewicht
        assert_eq!(f.projected_total.round(), dec!(1500));
    }
}
