//! Erwartungsbereich ("liegt der Verbrauch im Rahmen?") und Ampelbewertung.

use chrono::Datelike;
use rust_decimal::{Decimal, MathematicalOps};
use serde::{Deserialize, Serialize};

use crate::{forecast::SeasonProfile, period::Period, series::MonthValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Green,
    Yellow,
    Red,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Band {
    pub expected: Decimal,
    pub lower: Decimal,
    pub upper: Decimal,
}

impl Band {
    /// Band aus Historie: Mittelwert ± `k`·σ der Vergleichswerte (z.B. gleicher
    /// Monat der Vorjahre, saisonbereinigt). Mindestens zwei Werte nötig.
    // @impl SPEC_EXP_BAND
    #[must_use]
    pub fn from_history(values: &[Decimal], k: Decimal) -> Option<Self> {
        if values.len() < 2 {
            return None;
        }
        let n = Decimal::from(values.len());
        let mean = values.iter().sum::<Decimal>() / n;
        let var = values
            .iter()
            .map(|v| (*v - mean) * (*v - mean))
            .sum::<Decimal>()
            / (n - Decimal::ONE);
        let sigma = var.sqrt()?;
        Some(Self {
            expected: mean,
            lower: (mean - k * sigma).max(Decimal::ZERO),
            upper: mean + k * sigma,
        })
    }

    /// Band aus manuellem Ziel mit relativer Toleranz (Fallback ohne Historie).
    // @impl SPEC_EXP_BAND
    #[must_use]
    pub fn from_target(target: Decimal, tolerance_percent: Decimal) -> Self {
        let delta = target * tolerance_percent / Decimal::ONE_HUNDRED;
        Self {
            expected: target,
            lower: (target - delta).max(Decimal::ZERO),
            upper: target + delta,
        }
    }

    /// Grün innerhalb des Bands, gelb bis 1,5-fache Bandbreite darüber/darunter, sonst rot.
    #[must_use]
    pub fn classify(&self, value: Decimal) -> Status {
        if value >= self.lower && value <= self.upper {
            return Status::Green;
        }
        let half_width = (self.upper - self.lower) / Decimal::TWO;
        let slack = half_width / Decimal::TWO;
        if value >= self.lower - slack && value <= self.upper + slack {
            Status::Yellow
        } else {
            Status::Red
        }
    }
}

/// Standardfaktor `k` des Bands aus der Historie (SPEC_EXP_BAND).
pub const HISTORY_K: Decimal = Decimal::from_parts(15, 0, 0, false, 1);
/// Standardtoleranz des Jahresziels in Prozent (SPEC_ANA_TRAFFIC_LIGHT).
pub const DEFAULT_TOLERANCE_PCT: u8 = 20;

/// Grundlage eines Erwartungsbands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BandSource {
    /// Gleicher Kalendermonat der Vorjahre
    History,
    /// Jahresziel, verteilt mit dem Saisonprofil
    Target,
}

/// Erwartung und Ampel eines Monats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthExpectation {
    pub month: Period,
    pub band: Band,
    pub source: BandSource,
    /// `None`, wenn der Monat nicht vollständig gemessen ist
    pub status: Option<Status>,
}

/// Jahresziel mit Toleranz in Prozent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub annual: Decimal,
    pub tolerance_pct: Decimal,
}

/// Erwartung je Monat der Reihe: mit mindestens zwei Werten desselben
/// Kalendermonats aus Vorjahren das Band aus der Historie, sonst mit
/// Jahresziel dessen Anteil nach Saisonprofil, sonst keine Ampel
/// (SPEC_ANA_TRAFFIC_LIGHT).
// @impl SPEC_ANA_TRAFFIC_LIGHT, SPEC_EXP_BAND
#[must_use]
pub fn expectations(
    series: &[MonthValue],
    profile: &SeasonProfile,
    target: Option<Target>,
) -> Vec<Option<MonthExpectation>> {
    series
        .iter()
        .map(|m| {
            let earlier: Vec<Decimal> = series
                .iter()
                .filter(|x| {
                    x.month.start.month() == m.month.start.month()
                        && x.month.start.year() < m.month.start.year()
                })
                .filter_map(|x| x.value)
                .collect();
            let (band, source) = if let Some(b) = Band::from_history(&earlier, HISTORY_K) {
                (b, BandSource::History)
            } else {
                let t = target?;
                let expected = (t.annual * profile.share_of_year(m.month)).round_dp(3);
                (
                    Band::from_target(expected, t.tolerance_pct),
                    BandSource::Target,
                )
            };
            Some(MonthExpectation {
                month: m.month,
                band,
                source,
                status: m.value.map(|v| band.classify(v)),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    // @test SPEC_EXP_BAND
    #[test]
    fn band_and_classification() {
        let b = Band::from_target(dec!(100), dec!(10));
        assert_eq!((b.lower, b.upper), (dec!(90), dec!(110)));
        assert_eq!(b.classify(dec!(105)), Status::Green);
        assert_eq!(b.classify(dec!(114)), Status::Yellow);
        assert_eq!(b.classify(dec!(130)), Status::Red);

        let h = Band::from_history(&[dec!(90), dec!(100), dec!(110)], dec!(1.5)).unwrap();
        assert_eq!(h.expected, dec!(100));
        assert_eq!(h.upper, dec!(115));
        assert!(Band::from_history(&[dec!(1)], dec!(1)).is_none());
    }

    // @test SPEC_ANA_TRAFFIC_LIGHT
    #[test]
    fn traffic_light_from_history_or_target() {
        use chrono::NaiveDate;
        let month = |y: i32, m: u32| Period::month_of(NaiveDate::from_ymd_opt(y, m, 1).unwrap());
        let mv = |y, m, v: Option<Decimal>| MonthValue {
            month: month(y, m),
            value: v,
        };
        let series = [
            mv(2024, 1, Some(dec!(90))),
            mv(2025, 1, Some(dec!(110))),
            mv(2025, 2, Some(dec!(80))),
            mv(2026, 1, Some(dec!(140))),
            mv(2026, 2, Some(dec!(95))),
            mv(2026, 3, None),
        ];
        let gas = SeasonProfile::default_for(crate::meter::MeterKind::Gas);
        let none = expectations(&series, &gas, None);
        assert!(
            none[0].is_none() && none[1].is_none(),
            "ohne Historie und Ziel"
        );
        // Januar 2026: Vorjahre 90 und 110, σ ≈ 14,1: Band 78,8 bis 121,2
        let jan = none[3].expect("Historie");
        assert_eq!(jan.source, BandSource::History);
        assert_eq!(jan.status, Some(Status::Red));
        assert!(none[4].is_none(), "Februar nur ein Vorjahreswert");

        let target = Target {
            annual: dec!(1000),
            tolerance_pct: dec!(20),
        };
        let with = expectations(&series, &gas, Some(target));
        let feb = with[4].expect("Ziel");
        assert_eq!(feb.source, BandSource::Target);
        // Februar: 15 % von 1000 = 150, Band 120 bis 180
        assert_eq!(feb.band.expected, dec!(150));
        assert_eq!(feb.status, Some(Status::Red));
        assert_eq!(with[3].expect("Historie").source, BandSource::History);
        let march = with[5].expect("Ziel");
        assert_eq!(march.status, None, "nicht gemessen");
        assert_eq!(
            Band::from_target(dec!(150), dec!(20)).classify(dec!(115)),
            Status::Yellow
        );
    }
}
