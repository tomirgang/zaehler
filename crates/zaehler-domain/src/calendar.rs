//! Kalendertage in der Zeitzone des Haushalts (ADR_019).

use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

/// Abbildung zwischen lokalen Kalendertagen und UTC-Zeitpunkten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Calendar {
    tz: Tz,
}

impl Calendar {
    #[must_use]
    pub fn new(tz: Tz) -> Self {
        Self { tz }
    }

    #[must_use]
    pub fn tz(&self) -> Tz {
        self.tz
    }

    /// UTC-Zeitpunkt des lokalen Tagesbeginns. Existiert Mitternacht wegen
    /// einer Zeitumstellung nicht, gilt der früheste gültige Zeitpunkt des Tages.
    // @impl SPEC_TZ
    #[must_use]
    pub fn start_of(&self, date: NaiveDate) -> DateTime<Utc> {
        let mut local = date.and_time(NaiveTime::MIN);
        loop {
            match self.tz.from_local_datetime(&local) {
                LocalResult::Single(t) => return t.with_timezone(&Utc),
                LocalResult::Ambiguous(earliest, _) => return earliest.with_timezone(&Utc),
                LocalResult::None => local += chrono::Duration::minutes(15),
            }
        }
    }

    /// Lokaler Zeitpunkt (Formulareingabe) nach UTC. Nicht existierende lokale
    /// Zeitpunkte (Zeitumstellung) werden auf den nächsten gültigen verschoben,
    /// mehrdeutige auf den früheren.
    // @impl SPEC_TZ
    #[must_use]
    pub fn to_utc(&self, local: NaiveDateTime) -> DateTime<Utc> {
        let mut candidate = local;
        loop {
            match self.tz.from_local_datetime(&candidate) {
                LocalResult::Single(t) => return t.with_timezone(&Utc),
                LocalResult::Ambiguous(earliest, _) => return earliest.with_timezone(&Utc),
                LocalResult::None => candidate += chrono::Duration::minutes(15),
            }
        }
    }

    /// Lokales Datum eines UTC-Zeitpunkts.
    #[must_use]
    pub fn local_date(&self, at: DateTime<Utc>) -> NaiveDate {
        at.with_timezone(&self.tz).date_naive()
    }

    /// Lokaler Zeitpunkt eines UTC-Zeitpunkts.
    #[must_use]
    pub fn local(&self, at: DateTime<Utc>) -> NaiveDateTime {
        at.with_timezone(&self.tz).naive_local()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    // @test SPEC_TZ
    #[test]
    fn day_start_in_berlin_including_dst_changes() {
        let cal = Calendar::new(chrono_tz::Europe::Berlin);
        // Winterzeit UTC+1
        assert_eq!(
            cal.start_of(d(2026, 1, 1)).to_rfc3339(),
            "2025-12-31T23:00:00+00:00"
        );
        // Sommerzeit UTC+2
        assert_eq!(
            cal.start_of(d(2026, 7, 1)).to_rfc3339(),
            "2026-06-30T22:00:00+00:00"
        );
        // Tag der Umstellung auf Sommerzeit: Mitternacht existiert
        assert_eq!(
            cal.start_of(d(2026, 3, 29)).to_rfc3339(),
            "2026-03-28T23:00:00+00:00"
        );
        // nicht existierender lokaler Zeitpunkt 02:30 am 29.03.2026
        let gap = d(2026, 3, 29).and_hms_opt(2, 30, 0).unwrap();
        assert_eq!(cal.to_utc(gap).to_rfc3339(), "2026-03-29T01:00:00+00:00");
        // mehrdeutiger Zeitpunkt 02:30 am 25.10.2026: früherer (Sommerzeit)
        let amb = d(2026, 10, 25).and_hms_opt(2, 30, 0).unwrap();
        assert_eq!(cal.to_utc(amb).to_rfc3339(), "2026-10-25T00:30:00+00:00");
    }

    // @test SPEC_TZ
    #[test]
    fn day_start_without_local_midnight() {
        // America/Santiago: Umstellung um Mitternacht (00:00 existiert nicht am 2026-09-06)
        let cal = Calendar::new(chrono_tz::America::Santiago);
        let start = cal.start_of(d(2026, 9, 6));
        assert_eq!(
            cal.local(start).time(),
            NaiveTime::from_hms_opt(1, 0, 0).unwrap()
        );
    }
}
