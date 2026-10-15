//! Verbrauchsberechnung aus Zählerständen.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::reading::Reading;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConsumptionError {
    #[error("Zählerstand sinkt ohne möglichen Überlauf ({prev} -> {next})")]
    Decreasing { prev: Decimal, next: Decimal },
    #[error("Ablesungen sind nicht chronologisch sortiert")]
    Unordered,
    #[error("Zählerwechsel ohne End-/Startstand zum selben Zeitpunkt")]
    DeviceSwapGap,
}

/// Verbrauch zwischen zwei Ablesungen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub amount: Decimal,
}

/// Differenz zweier Stände desselben Geräts, inkl. Überlauf (`99999 -> 00012`).
// @impl SPEC_CONS_DELTA
pub fn delta(
    prev: Decimal,
    next: Decimal,
    rollover_digits: Option<u8>,
) -> Result<Decimal, ConsumptionError> {
    if next >= prev {
        return Ok(next - prev);
    }
    match rollover_digits {
        Some(digits) => Ok(next + Decimal::from(10_u64.pow(u32::from(digits))) - prev),
        None => Err(ConsumptionError::Decreasing { prev, next }),
    }
}

/// Bildet aus chronologisch sortierten Ablesungen einer Messstelle (ein
/// Register, ggf. mehrere Geräte) die Verbrauchsintervalle.
///
/// Bei einem Gerätewechsel muss der Endstand des alten und der Startstand des
/// neuen Geräts denselben Zeitpunkt haben; das Intervall dazwischen hat dann
/// die Länge 0 und wird übersprungen.
// @impl SPEC_CONS_DELTA, SPEC_CONS_DEVICE_SWAP
pub fn intervals(readings: &[Reading]) -> Result<Vec<Interval>, ConsumptionError> {
    let mut out = Vec::with_capacity(readings.len().saturating_sub(1));
    for pair in readings.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if b.at < a.at {
            return Err(ConsumptionError::Unordered);
        }
        if a.device_id != b.device_id {
            if a.at != b.at {
                return Err(ConsumptionError::DeviceSwapGap);
            }
            continue;
        }
        out.push(Interval {
            start: a.at,
            end: b.at,
            amount: delta(a.value, b.value, b.rollover_digits)?,
        });
    }
    Ok(out)
}

/// Verbrauch im Zeitraum `[from, to)` bei linearer Verteilung innerhalb der
/// Intervalle. `None`, wenn der Zeitraum nicht vollständig durch Ablesungen
/// abgedeckt ist (dann ist eine Prognose nötig).
// @impl SPEC_CONS_INTERPOLATION
#[must_use]
pub fn consumption_in(
    intervals: &[Interval],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Option<Decimal> {
    let first = intervals.first()?;
    let last = intervals.last()?;
    if from < first.start || to > last.end || from > to {
        return None;
    }
    let mut sum = Decimal::ZERO;
    for iv in intervals {
        let len = (iv.end - iv.start).num_seconds();
        if len == 0 {
            continue;
        }
        let overlap_start = iv.start.max(from);
        let overlap_end = iv.end.min(to);
        let overlap = (overlap_end - overlap_start).num_seconds();
        if overlap > 0 {
            sum += iv.amount * Decimal::from(overlap) / Decimal::from(len);
        }
    }
    Some(sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    fn at(month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, month, day, 0, 0, 0).unwrap()
    }

    fn reading(month: u32, day: u32, value: Decimal, device: u128) -> Reading {
        Reading {
            at: at(month, day),
            value,
            device_id: Uuid::from_u128(device),
            rollover_digits: Some(5),
        }
    }

    // @test SPEC_CONS_DELTA
    #[test]
    fn delta_handles_rollover() {
        assert_eq!(delta(dec!(100), dec!(150), None), Ok(dec!(50)));
        assert_eq!(delta(dec!(99990), dec!(10), Some(5)), Ok(dec!(20)));
        assert!(delta(dec!(100), dec!(90), None).is_err());
    }

    // @test SPEC_CONS_DEVICE_SWAP
    #[test]
    fn device_swap_continues_series() {
        let rs = [
            reading(1, 1, dec!(1000), 1),
            reading(1, 11, dec!(1100), 1),
            reading(1, 11, dec!(0), 2),
            reading(1, 21, dec!(50), 2),
        ];
        let ivs = intervals(&rs).unwrap();
        assert_eq!(ivs.iter().map(|i| i.amount).sum::<Decimal>(), dec!(150));

        let gap = [reading(1, 1, dec!(1), 1), reading(1, 2, dec!(0), 2)];
        assert_eq!(intervals(&gap), Err(ConsumptionError::DeviceSwapGap));
    }

    // @test SPEC_CONS_INTERPOLATION
    #[test]
    fn linear_distribution() {
        let rs = [reading(1, 1, dec!(0), 1), reading(1, 11, dec!(100), 1)];
        let ivs = intervals(&rs).unwrap();
        assert_eq!(consumption_in(&ivs, at(1, 1), at(1, 6)), Some(dec!(50)));
        assert_eq!(consumption_in(&ivs, at(1, 1), at(1, 12)), None);
    }
}
