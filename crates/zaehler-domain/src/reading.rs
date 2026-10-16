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

impl ReadingSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Photo => "photo",
            Self::CsvImport => "csv_import",
            Self::Invoice => "invoice",
        }
    }
}

impl std::str::FromStr for ReadingSource {
    type Err = crate::meter::UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        [Self::Manual, Self::Photo, Self::CsvImport, Self::Invoice]
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| crate::meter::UnknownVariant(s.to_owned()))
    }
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
    /// Zeitpunkt liegt vor dem Vergleichsstand oder vor dem Einbau des Geräts.
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
    let delta = if candidate.value >= prev.value {
        candidate.value - prev.value
    } else {
        match candidate.rollover_digits {
            // Ein Überlauf ist nur plausibel, wenn er weniger als die Hälfte des
            // Zählbereichs ergibt; sonst ist ein Tippfehler wahrscheinlicher.
            Some(digits) => {
                let range = Decimal::from(10_u64.pow(u32::from(digits)));
                let wrapped = candidate.value + range - prev.value;
                if wrapped > range / Decimal::TWO {
                    return Plausibility::Decreasing;
                }
                wrapped
            }
            None => return Plausibility::Decreasing,
        }
    };
    if let Some(max) = max_daily_rate {
        let seconds = (candidate.at - prev.at).num_seconds();
        if seconds > 0 {
            let days = Decimal::from(seconds) / Decimal::from(86_400);
            if delta / days > max {
                return Plausibility::RateTooHigh;
            }
        }
    }
    Plausibility::Ok
}

/// Prüft einen Stand gegen seine zeitlichen Nachbarn desselben Geräts: zuerst
/// gegen den Vorgänger, bei `Ok` den Nachfolger gegen den Stand. Damit lassen
/// sich auch nachgetragene Zwischenstände prüfen.
// @impl SPEC_RDG_PLAUSIBILITY
#[must_use]
pub fn check_in_sequence(
    previous: Option<&Reading>,
    candidate: &Reading,
    next: Option<&Reading>,
    max_daily_rate: Option<Decimal>,
) -> Plausibility {
    let p = check_plausibility(previous, candidate, max_daily_rate);
    match next {
        Some(next) if p == Plausibility::Ok => {
            check_plausibility(Some(candidate), next, max_daily_rate)
        }
        _ => p,
    }
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

    // @test SPEC_RDG_PLAUSIBILITY
    #[test]
    fn intermediate_reading_is_checked_against_both_neighbours() {
        let (prev, next) = (r(1, dec!(100)), r(10, dec!(200)));
        assert_eq!(
            check_in_sequence(Some(&prev), &r(5, dec!(150)), Some(&next), None),
            Plausibility::Ok
        );
        assert_eq!(
            check_in_sequence(Some(&prev), &r(5, dec!(90)), Some(&next), None),
            Plausibility::Decreasing
        );
        assert_eq!(
            check_in_sequence(Some(&prev), &r(5, dec!(250)), Some(&next), None),
            Plausibility::Decreasing
        );
        assert_eq!(
            check_in_sequence(None, &r(5, dec!(150)), Some(&next), Some(dec!(5))),
            Plausibility::RateTooHigh
        );
    }

    // @test SPEC_RDG_PLAUSIBILITY
    #[test]
    fn rollover_is_plausible_only_near_the_top() {
        let with_digits = |day, value| Reading {
            rollover_digits: Some(5),
            ..r(day, value)
        };
        let prev = with_digits(1, dec!(99_990));
        // Überlauf um 20 Einheiten: plausibel, Rate wird mit dem Überlauf geprüft
        assert_eq!(
            check_plausibility(Some(&prev), &with_digits(2, dec!(10)), Some(dec!(50))),
            Plausibility::Ok
        );
        assert_eq!(
            check_plausibility(Some(&prev), &with_digits(2, dec!(100)), Some(dec!(50))),
            Plausibility::RateTooHigh
        );
        // Rückgang von 1100 auf 1050 wäre ein Überlauf um 99 950: Tippfehler
        let low = with_digits(1, dec!(1100));
        assert_eq!(
            check_plausibility(Some(&low), &with_digits(2, dec!(1050)), None),
            Plausibility::Decreasing
        );
    }
}
