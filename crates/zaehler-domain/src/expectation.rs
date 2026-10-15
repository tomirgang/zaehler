//! Erwartungsbereich ("liegt der Verbrauch im Rahmen?") und Ampelbewertung.

use rust_decimal::{Decimal, MathematicalOps};
use serde::{Deserialize, Serialize};

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
}
