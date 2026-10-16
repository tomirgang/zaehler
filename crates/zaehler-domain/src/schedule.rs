//! Ableseplan und Fälligkeit (SPEC_RDG_SCHEDULE). Alle Tage sind
//! Kalendertage in der Zeitzone des Haushalts.

use chrono::{Datelike, Days, Months, NaiveDate};
use serde::{Deserialize, Serialize};

/// Tolerierter Abstand einer Ablesung vor dem Stichtag eines monatlichen
/// Plans: sie zählt für diesen Stichtag.
pub const MONTHLY_EARLY_DAYS: u64 = 3;

/// Ableseplan einer Messstelle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ReadingSchedule {
    /// Fällig `n` Tage nach der letzten Ablesung (1 bis 3650)
    EveryDays(u16),
    /// Fällig monatlich am Tag `d` (1 bis 28)
    MonthlyOnDay(u8),
}

impl ReadingSchedule {
    /// Bezeichner der Art für Speicherung und Formulare.
    #[must_use]
    pub fn kind(self) -> &'static str {
        match self {
            Self::EveryDays(_) => "every_days",
            Self::MonthlyOnDay(_) => "monthly",
        }
    }

    /// Zahl der Art (Tage bzw. Tag im Monat).
    #[must_use]
    pub fn value(self) -> u16 {
        match self {
            Self::EveryDays(n) => n,
            Self::MonthlyOnDay(d) => u16::from(d),
        }
    }

    /// Plan aus Art und Zahl; `None` bei unbekannter Art oder ungültiger Zahl.
    #[must_use]
    pub fn from_parts(kind: &str, value: u16) -> Option<Self> {
        let plan = match kind {
            "every_days" => Self::EveryDays(value),
            "monthly" => Self::MonthlyOnDay(u8::try_from(value).ok()?),
            _ => return None,
        };
        plan.is_valid().then_some(plan)
    }

    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::EveryDays(n) => (1..=3650).contains(&n),
            Self::MonthlyOnDay(d) => (1..=28).contains(&d),
        }
    }

    /// Nächster Stichtag nach der letzten Ablesung am Tag `last`.
    // @impl SPEC_RDG_SCHEDULE
    #[must_use]
    pub fn next_due(self, last: NaiveDate) -> NaiveDate {
        match self {
            Self::EveryDays(n) => last + Days::new(u64::from(n)),
            Self::MonthlyOnDay(d) => {
                // erster Stichtag am oder nach dem Tag der Ablesung
                let mut deadline = last.with_day(u32::from(d)).unwrap_or(last);
                if deadline < last {
                    deadline = deadline + Months::new(1);
                }
                // eine Ablesung bis zu drei Tage vor dem Stichtag zählt für ihn
                if deadline - Days::new(MONTHLY_EARLY_DAYS) <= last {
                    deadline + Months::new(1)
                } else {
                    deadline
                }
            }
        }
    }
}

/// Fälligkeit einer Messstelle an einem Tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "date")]
pub enum Due {
    /// Nächster Stichtag liegt in der Zukunft
    Upcoming(NaiveDate),
    /// Heute fällig, oder ohne Ablesung sofort fällig
    Due(NaiveDate),
    /// Fällig seit dem Stichtag (ab dem Folgetag überfällig)
    Overdue(NaiveDate),
}

impl Due {
    #[must_use]
    pub fn date(self) -> NaiveDate {
        match self {
            Self::Upcoming(d) | Self::Due(d) | Self::Overdue(d) => d,
        }
    }

    /// Fällig oder überfällig.
    #[must_use]
    pub fn needs_reading(self) -> bool {
        !matches!(self, Self::Upcoming(_))
    }
}

/// Fälligkeit am Tag `today` nach dem Plan; ohne Ablesung ist die Messstelle
/// sofort fällig.
// @impl SPEC_RDG_SCHEDULE
#[must_use]
pub fn due(schedule: ReadingSchedule, last: Option<NaiveDate>, today: NaiveDate) -> Due {
    let Some(last) = last else {
        return Due::Due(today);
    };
    let date = schedule.next_due(last);
    match today.cmp(&date) {
        std::cmp::Ordering::Less => Due::Upcoming(date),
        std::cmp::Ordering::Equal => Due::Due(date),
        std::cmp::Ordering::Greater => Due::Overdue(date),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    // @test SPEC_RDG_SCHEDULE
    #[test]
    fn every_n_days() {
        let s = ReadingSchedule::EveryDays(30);
        assert_eq!(s.next_due(d(1, 15)), d(2, 14));
        assert_eq!(due(s, Some(d(1, 15)), d(2, 13)), Due::Upcoming(d(2, 14)));
        assert_eq!(due(s, Some(d(1, 15)), d(2, 14)), Due::Due(d(2, 14)));
        assert_eq!(due(s, Some(d(1, 15)), d(2, 15)), Due::Overdue(d(2, 14)));
        // ohne Ablesung sofort fällig
        assert_eq!(due(s, None, d(3, 1)), Due::Due(d(3, 1)));
    }

    // @test SPEC_RDG_SCHEDULE
    #[test]
    fn monthly_on_day_with_early_readings_and_month_ends() {
        let s = ReadingSchedule::MonthlyOnDay(1);
        // Ablesung am Stichtag: nächster Stichtag im Folgemonat
        assert_eq!(s.next_due(d(3, 1)), d(4, 1));
        // bis zu drei Tage davor zählt für den Stichtag
        assert_eq!(s.next_due(d(2, 26)), d(4, 1));
        assert_eq!(s.next_due(d(2, 25)), d(3, 1));
        // nach dem Stichtag: nächster Monat
        assert_eq!(s.next_due(d(3, 2)), d(4, 1));
        // Jahreswechsel: drei Tage vor dem 01.01. zählt für diesen
        assert_eq!(
            s.next_due(d(12, 29)),
            NaiveDate::from_ymd_opt(2027, 2, 1).unwrap()
        );
        assert_eq!(
            s.next_due(d(12, 28)),
            NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        );

        let s = ReadingSchedule::MonthlyOnDay(28);
        // Februar hat den 28.
        assert_eq!(s.next_due(d(2, 10)), d(2, 28));
        assert_eq!(s.next_due(d(2, 26)), d(3, 28));
        assert_eq!(s.next_due(d(1, 31)), d(2, 28));
        // überfällig ab dem Folgetag
        assert_eq!(due(s, Some(d(1, 31)), d(3, 1)), Due::Overdue(d(2, 28)));
        assert!(due(s, Some(d(1, 31)), d(2, 28)).needs_reading());
        assert!(!due(s, Some(d(1, 31)), d(2, 27)).needs_reading());
    }

    // @test SPEC_RDG_SCHEDULE
    #[test]
    fn parts_roundtrip_and_validation() {
        for s in [
            ReadingSchedule::EveryDays(7),
            ReadingSchedule::MonthlyOnDay(15),
        ] {
            assert_eq!(ReadingSchedule::from_parts(s.kind(), s.value()), Some(s));
        }
        assert_eq!(ReadingSchedule::from_parts("monthly", 29), None);
        assert_eq!(ReadingSchedule::from_parts("monthly", 0), None);
        assert_eq!(ReadingSchedule::from_parts("every_days", 0), None);
        assert_eq!(ReadingSchedule::from_parts("weekly", 1), None);
    }
}
