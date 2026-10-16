//! Hochrechnung des Verbrauchs auf einen Zeitraum (Kalender- oder Abrechnungsjahr).

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{meter::MeterKind, period::Period, series::MonthValue};

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

    /// Standardprofil einer Art (SPEC_ANA_TRAFFIC_LIGHT): Gas und Wärme mit
    /// Heizperiode, alle anderen gleichverteilt.
    // @impl SPEC_ANA_TRAFFIC_LIGHT
    #[must_use]
    pub fn default_for(kind: MeterKind) -> Self {
        match kind {
            MeterKind::Gas | MeterKind::Heat => Self {
                monthly_weights: [17, 15, 12, 8, 4, 2, 2, 2, 4, 8, 12, 14].map(Decimal::from),
            },
            _ => Self::uniform(),
        }
    }

    /// Gelerntes Profil, sobald die Reihe mindestens 12 aufeinanderfolgende
    /// vollständige Monate enthält; gelernt wird aus allen vollständigen
    /// Monaten (SPEC_FC_SEASON_PROFILE).
    // @impl SPEC_FC_SEASON_PROFILE
    #[must_use]
    pub fn learn_series(series: &[MonthValue]) -> Option<Self> {
        let mut run = 0;
        let mut longest = 0;
        for m in series {
            run = if m.value.is_some() { run + 1 } else { 0 };
            longest = longest.max(run);
        }
        if longest < 12 {
            return None;
        }
        let samples: Vec<(u32, Decimal)> = series
            .iter()
            .filter_map(|m| m.value.map(|v| (m.month.start.month(), v)))
            .collect();
        Some(Self::learn(&samples))
    }

    /// Anteil des Zeitraums am Gewicht seines Kalenderjahres.
    #[must_use]
    pub fn share_of_year(&self, period: Period) -> Decimal {
        let total = self.weight(Period::year(period.start.year()));
        if total.is_zero() {
            Decimal::ZERO
        } else {
            self.weight(period) / total
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
    /// Beginn des gemessenen Teils; liegt er nach `period.start`, ist der
    /// Verbrauch davor geschätzt.
    pub observed_from: NaiveDate,
    /// Tatsächlich gemessener Verbrauch im abgedeckten Teil.
    pub observed: Decimal,
    /// Hochgerechneter Gesamtverbrauch des Zeitraums.
    pub projected_total: Decimal,
    /// Anteil (0..1) des Zeitraums, der durch Messwerte abgedeckt ist (gewichtet).
    pub coverage: Decimal,
}

impl Forecast {
    /// Der Verbrauch vor `observed_from` ist geschätzt.
    #[must_use]
    pub fn estimated_start(&self) -> bool {
        self.observed_from > self.period.start
    }
}

/// Rechnet den in `observed` (Teil von `period`) gemessenen Verbrauch
/// `value` anhand des Saisonprofils auf den Gesamtzeitraum hoch; fehlende
/// Teile vor und nach der Messung werden mit derselben Rate geschätzt.
// @impl SPEC_FC_PERIOD_FORECAST
#[must_use]
pub fn project(
    period: Period,
    observed: Period,
    value: Decimal,
    profile: &SeasonProfile,
) -> Option<Forecast> {
    let from = observed.start.clamp(period.start, period.end);
    let until = observed.end.clamp(from, period.end);
    let covered = profile.weight(Period::new(from, until));
    let total = profile.weight(period);
    if covered.is_zero() || total.is_zero() {
        return None;
    }
    Some(Forecast {
        period,
        observed_from: from,
        observed: value,
        projected_total: (value * total / covered).round_dp(3),
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
        let year = Period::year(2026);
        let uniform = SeasonProfile::uniform();
        let f = project(year, Period::new(d(1, 1), d(7, 2)), dec!(1820), &uniform).unwrap();
        // 182 von 365 Tagen gemessen
        assert_eq!(f.projected_total, dec!(3650));
        assert!(!f.estimated_start());

        // gemessen erst ab 01.04.: Beginn mit derselben Rate geschätzt
        let f = project(year, Period::new(d(4, 1), d(7, 1)), dec!(910), &uniform).unwrap();
        assert_eq!(f.projected_total, dec!(3650));
        assert_eq!(f.coverage, dec!(0.2493));
        assert!(f.estimated_start());
    }

    // @test SPEC_FC_SEASON_PROFILE, SPEC_ANA_TRAFFIC_LIGHT
    #[test]
    fn defaults_and_learning_from_twelve_consecutive_months() {
        let gas = SeasonProfile::default_for(MeterKind::Gas);
        let total: Decimal = gas.monthly_weights.iter().sum();
        assert_eq!(total, dec!(100));
        assert_eq!(
            gas.share_of_year(Period::month_of(d(1, 1))).round_dp(2),
            dec!(0.17)
        );
        assert_eq!(
            SeasonProfile::default_for(MeterKind::Electricity),
            SeasonProfile::uniform()
        );

        let month = |m: u32, y: i32| Period::month_of(NaiveDate::from_ymd_opt(y, m, 1).unwrap());
        let mut series: Vec<MonthValue> = (1..=11)
            .map(|m| MonthValue {
                month: month(m, 2025),
                value: Some(if m <= 3 { dec!(300) } else { dec!(100) }),
            })
            .collect();
        assert_eq!(SeasonProfile::learn_series(&series), None, "11 Monate");
        series.push(MonthValue {
            month: month(12, 2025),
            value: None,
        });
        series.push(MonthValue {
            month: month(1, 2026),
            value: Some(dec!(300)),
        });
        assert_eq!(SeasonProfile::learn_series(&series), None, "Lücke");
        series[11].value = Some(dec!(250));
        let learned = SeasonProfile::learn_series(&series).expect("12 Monate");
        assert!(learned.monthly_weights[0] > learned.monthly_weights[6]);
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
        let f = project(
            Period::year(2026),
            Period::new(d(1, 1), d(4, 1)),
            dec!(600),
            &p,
        )
        .unwrap();
        // Q1 = 600 von 1500 Gesamtjahresgewicht
        assert_eq!(f.projected_total.round(), dec!(1500));
    }
}
