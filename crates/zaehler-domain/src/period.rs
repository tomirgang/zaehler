//! Zeiträume (Monat, Jahr, Abrechnungszeitraum) und Vergleichszeiträume.

use chrono::{Datelike, Months, NaiveDate};
use serde::{Deserialize, Serialize};

/// Halboffener Zeitraum `[start, end)` in Kalendertagen (Zeitzone des Haushalts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Period {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl Period {
    #[must_use]
    pub fn new(start: NaiveDate, end: NaiveDate) -> Self {
        debug_assert!(start <= end);
        Self { start, end }
    }

    /// Kalendermonat, der `date` enthält.
    ///
    /// # Panics
    /// Nur am Rand des von `chrono` darstellbaren Datumsbereichs.
    #[must_use]
    pub fn month_of(date: NaiveDate) -> Self {
        let start = date.with_day(1).expect("day 1 exists");
        Self::new(start, start + Months::new(1))
    }

    /// Kalenderjahr.
    ///
    /// # Panics
    /// Wenn `year` außerhalb des von `chrono` darstellbaren Bereichs liegt.
    #[must_use]
    pub fn year(year: i32) -> Self {
        Self::new(
            NaiveDate::from_ymd_opt(year, 1, 1).expect("valid"),
            NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("valid"),
        )
    }

    /// Abrechnungsjahr mit beliebigem Starttag, z.B. 01.04.-31.03.
    #[must_use]
    pub fn billing_year(start: NaiveDate) -> Self {
        Self::new(start, start + Months::new(12))
    }

    #[must_use]
    pub fn days(&self) -> i64 {
        (self.end - self.start).num_days()
    }

    /// Gleicher Zeitraum um `n` Monate verschoben (negativ = Vergangenheit).
    #[must_use]
    pub fn shift_months(&self, n: i32) -> Self {
        let shift = |d: NaiveDate| {
            if n >= 0 {
                d + Months::new(n.unsigned_abs())
            } else {
                d - Months::new(n.unsigned_abs())
            }
        };
        Self::new(shift(self.start), shift(self.end))
    }
}

/// Bezugsgröße für Vergleiche.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "months")]
pub enum Reference {
    PreviousMonth,
    PreviousYear,
    /// Durchschnitt der letzten n Monate (3, 6 oder 12) vor dem Monat.
    TrailingAverage(u8),
}

/// Liefert die Vergleichszeiträume für einen Monat. Bei `TrailingAverage`
/// sind es n einzelne Monate, über die gemittelt wird.
// @impl SPEC_CMP_PERIODS
#[must_use]
pub fn reference_periods(month: Period, reference: Reference) -> Vec<Period> {
    match reference {
        Reference::PreviousMonth => vec![month.shift_months(-1)],
        Reference::PreviousYear => vec![month.shift_months(-12)],
        Reference::TrailingAverage(n) => {
            (1..=i32::from(n)).map(|i| month.shift_months(-i)).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    // @test SPEC_CMP_PERIODS
    #[test]
    fn reference_periods_cover_expected_months() {
        let march = Period::month_of(d(2026, 3, 15));
        assert_eq!(
            reference_periods(march, Reference::PreviousMonth),
            vec![Period::month_of(d(2026, 2, 1))]
        );
        assert_eq!(
            reference_periods(march, Reference::PreviousYear),
            vec![Period::month_of(d(2025, 3, 1))]
        );
        let six = reference_periods(march, Reference::TrailingAverage(6));
        assert_eq!(six.len(), 6);
        assert_eq!(six.last().unwrap().start, d(2025, 9, 1));
        assert_eq!(Period::billing_year(d(2026, 4, 1)).end, d(2027, 4, 1));
    }
}
