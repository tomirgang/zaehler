//! Vergleich eines Werts (Verbrauch oder Kosten) mit einer Referenz.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comparison {
    pub current: Decimal,
    pub reference: Decimal,
    pub abs_diff: Decimal,
    /// Relative Abweichung in Prozent; `None` bei Referenz 0.
    pub rel_diff_percent: Option<Decimal>,
}

/// Vergleicht `current` mit dem Mittelwert der `references`.
// @impl SPEC_CMP_DIFF
#[must_use]
pub fn compare(current: Decimal, references: &[Decimal]) -> Option<Comparison> {
    if references.is_empty() {
        return None;
    }
    let reference = references.iter().sum::<Decimal>() / Decimal::from(references.len());
    let abs_diff = current - reference;
    let rel_diff_percent =
        (!reference.is_zero()).then(|| (abs_diff / reference * Decimal::ONE_HUNDRED).round_dp(1));
    Some(Comparison {
        current,
        reference,
        abs_diff,
        rel_diff_percent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    // @test SPEC_CMP_DIFF
    #[test]
    fn compares_against_average() {
        let c = compare(dec!(120), &[dec!(90), dec!(110)]).unwrap();
        assert_eq!(c.reference, dec!(100));
        assert_eq!(c.abs_diff, dec!(20));
        assert_eq!(c.rel_diff_percent, Some(dec!(20.0)));
        assert!(compare(dec!(1), &[]).is_none());
    }
}
