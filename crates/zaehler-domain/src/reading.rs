//! Zählerstände und deren Plausibilisierung.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Herkunft eines Zählerstands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingSource {
    Manual,
    Photo,
    CsvImport,
    Invoice,
}

/// Ein Zählerstand eines Registers auf einem bestimmten Gerät.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub at: DateTime<Utc>,
    pub value: Decimal,
    pub device_id: Uuid,
    /// Vorkommastellen des Geräts, falls ein Überlauf möglich ist.
    pub rollover_digits: Option<u8>,
}

/// Ergebnis der Plausibilitätsprüfung eines neuen Zählerstands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Plausibility {
    Ok,
    /// Stand ist kleiner als der vorherige (Tippfehler, Überlauf oder Zählerwechsel).
    Decreasing,
    /// Tagesverbrauch liegt über dem konfigurierten Maximum.
    RateTooHigh,
    /// Zeitpunkt liegt vor der letzten Ablesung.
    OutOfOrder,
}

/// Prüft einen neuen Stand gegen den letzten bekannten Stand desselben Geräts.
///
/// Verwendet für Formular, Foto-Erkennung (AI-Vorschlag) und Import; das
/// Ergebnis ist ein Hinweis, die Entscheidung trifft immer der Nutzer.
// @impl SPEC_RDG_PLAUSIBILITY
#[must_use]
pub fn check_plausibility(
    previous: Option<&Reading>,
    candidate: &Reading,
    max_daily_rate: Option<Decimal>,
) -> Plausibility {
    let Some(prev) = previous.filter(|p| p.device_id == candidate.device_id) else {
        return Plausibility::Ok;
    };
    if candidate.at < prev.at {
        return Plausibility::OutOfOrder;
    }
    if candidate.value < prev.value && candidate.rollover_digits.is_none() {
        return Plausibility::Decreasing;
    }
    if let Some(max) = max_daily_rate {
        let seconds = (candidate.at - prev.at).num_seconds();
        if seconds > 0 && candidate.value >= prev.value {
            let days = Decimal::from(seconds) / Decimal::from(86_400);
            if (candidate.value - prev.value) / days > max {
                return Plausibility::RateTooHigh;
            }
        }
    }
    Plausibility::Ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    fn r(day: u32, value: Decimal) -> Reading {
        Reading {
            at: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap(),
            value,
            device_id: Uuid::nil(),
            rollover_digits: None,
        }
    }

    // @test SPEC_RDG_PLAUSIBILITY
    #[test]
    fn detects_decreasing_and_rate() {
        let prev = r(1, dec!(100));
        assert_eq!(
            check_plausibility(Some(&prev), &r(2, dec!(99)), None),
            Plausibility::Decreasing
        );
        assert_eq!(
            check_plausibility(Some(&prev), &r(2, dec!(200)), Some(dec!(50))),
            Plausibility::RateTooHigh
        );
        assert_eq!(
            check_plausibility(Some(&prev), &r(3, dec!(150)), Some(dec!(50))),
            Plausibility::Ok
        );
        assert_eq!(
            check_plausibility(None, &r(3, dec!(1)), None),
            Plausibility::Ok
        );
    }
}
