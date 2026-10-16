//! Vergleichsansicht und Prognose laufender Zeiträume.

use chrono::{DateTime, Datelike, Months, NaiveDate, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    calendar::Calendar,
    comparison::{Comparison, compare},
    consumption::Interval,
    forecast::{Forecast, SeasonProfile, project},
    period::{Period, Reference, reference_periods},
    series::{MonthValue, covered_until, total_in},
};

/// Vergleiche eines Monats in der Reihenfolge der Anzeige.
pub const REFERENCES: [Reference; 5] = [
    Reference::PreviousMonth,
    Reference::PreviousYear,
    Reference::TrailingAverage(3),
    Reference::TrailingAverage(6),
    Reference::TrailingAverage(12),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthComparison {
    pub reference: Reference,
    /// `None`, wenn der Monat oder ein Referenzmonat nicht vollständig
    /// abgedeckt ist („Daten unvollständig“).
    pub comparison: Option<Comparison>,
}

/// Wert eines Monats aus der Serie.
#[must_use]
pub fn value_of(series: &[MonthValue], month: Period) -> Option<Decimal> {
    series
        .iter()
        .find(|m| m.month == month)
        .and_then(|m| m.value)
}

/// Letzter vollständig abgedeckter Monat.
#[must_use]
pub fn last_complete(series: &[MonthValue]) -> Option<Period> {
    series
        .iter()
        .rev()
        .find(|m| m.value.is_some())
        .map(|m| m.month)
}

/// Vergleicht einen Monat mit Vormonat, Vorjahresmonat und den
/// Durchschnitten der letzten 3, 6 und 12 Monate.
// @impl SPEC_ANA_COMPARE_VIEW
#[must_use]
pub fn compare_month(series: &[MonthValue], month: Period) -> Vec<MonthComparison> {
    let current = value_of(series, month);
    REFERENCES
        .iter()
        .map(|&reference| {
            let references: Option<Vec<Decimal>> = reference_periods(month, reference)
                .into_iter()
                .map(|p| value_of(series, p))
                .collect();
            MonthComparison {
                reference,
                comparison: current.zip(references).and_then(|(c, r)| compare(c, &r)),
            }
        })
        .collect()
}

/// Laufendes Jahr bis zu einem Zeitpunkt gegen denselben Zeitraum des Vorjahres.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YearToDate {
    /// Lokaler Endzeitpunkt im laufenden Jahr
    pub until: NaiveDateTime,
    /// Lokaler Endzeitpunkt im Vorjahr (gleicher Tag und Monat, 29.02. wird 28.02.)
    pub previous_until: NaiveDateTime,
    pub current: Option<Decimal>,
    pub previous: Option<Decimal>,
    pub comparison: Option<Comparison>,
}

/// Verbrauch vom 01.01. bis `until` und im selben Zeitraum des Vorjahres.
// @impl SPEC_ANA_COMPARE_VIEW
#[must_use]
pub fn year_to_date(
    registers: &[&[Interval]],
    cal: Calendar,
    until: DateTime<Utc>,
) -> Option<YearToDate> {
    let local = cal.local(until);
    let previous_until = local.checked_sub_months(Months::new(12))?;
    let start = NaiveDate::from_ymd_opt(local.year(), 1, 1)?;
    let previous_start = NaiveDate::from_ymd_opt(local.year() - 1, 1, 1)?;
    let current = total_in(registers, cal.start_of(start), until);
    let previous = total_in(
        registers,
        cal.start_of(previous_start),
        cal.to_utc(previous_until),
    );
    Some(YearToDate {
        until: local,
        previous_until,
        current,
        previous,
        comparison: current.zip(previous).and_then(|(c, p)| compare(c, &[p])),
    })
}

/// Ergebnis der Prognose eines laufenden Zeitraums.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeriodForecast {
    Projected(Forecast),
    /// Keine Ablesung vor dem Zeitraumbeginn (Startwert fehlt)
    MissingStart,
    /// Keine Ablesung nach dem Zeitraumbeginn
    NoReading,
}

impl PeriodForecast {
    #[must_use]
    pub fn projected(&self) -> Option<&Forecast> {
        match self {
            Self::Projected(f) => Some(f),
            _ => None,
        }
    }
}

/// Prognose für einen laufenden Zeitraum: gemessen bis zum Beginn des Tages
/// der letzten gemeinsamen Ablesung aller Register, hochgerechnet mit dem
/// Saisonprofil.
// @impl SPEC_FC_CURRENT_YEAR
#[must_use]
pub fn forecast_period(
    registers: &[&[Interval]],
    cal: Calendar,
    period: Period,
    profile: &SeasonProfile,
) -> PeriodForecast {
    let Some(last) = covered_until(registers) else {
        return PeriodForecast::NoReading;
    };
    let until = cal.local_date(last).min(period.end);
    if until <= period.start {
        return PeriodForecast::NoReading;
    }
    let Some(observed) = total_in(registers, cal.start_of(period.start), cal.start_of(until))
    else {
        return PeriodForecast::MissingStart;
    };
    project(period, until, observed, profile)
        .map_or(PeriodForecast::NoReading, PeriodForecast::Projected)
}

/// Laufendes Abrechnungsjahr mit Beginn am Tag `day` des Monats `month`.
// @impl SPEC_FC_CURRENT_YEAR
#[must_use]
pub fn current_billing_year(today: NaiveDate, month: u32, day: u32) -> Option<Period> {
    let this_year = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    let start = if this_year <= today {
        this_year
    } else {
        NaiveDate::from_ymd_opt(today.year() - 1, month, day)?
    };
    Some(Period::billing_year(start))
}

/// Kumulierter Prognosewert vom Zeitraumbeginn bis zum Beginn von `date`.
// @impl SPEC_ANA_CHARTS
#[must_use]
pub fn projected_cumulative(f: &Forecast, profile: &SeasonProfile, date: NaiveDate) -> Decimal {
    let total = profile.weight(f.period);
    if total.is_zero() {
        return Decimal::ZERO;
    }
    let until = date.clamp(f.period.start, f.period.end);
    (f.projected_total * profile.weight(Period::new(f.period.start, until)) / total).round_dp(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{consumption::intervals, reading::Reading, series::months_between};
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    fn berlin() -> Calendar {
        Calendar::new(chrono_tz::Europe::Berlin)
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn ivs(cal: Calendar, points: &[(NaiveDate, Decimal)]) -> Vec<Interval> {
        let rs: Vec<Reading> = points
            .iter()
            .map(|(date, value)| Reading {
                at: cal.start_of(*date),
                value: *value,
                device_id: Uuid::nil(),
                rollover_digits: None,
            })
            .collect();
        intervals(&rs).unwrap()
    }

    // @test SPEC_ANA_COMPARE_VIEW
    #[test]
    fn month_comparisons_need_complete_reference_months() {
        let months = months_between(d(2025, 1, 1), d(2026, 3, 1));
        let mut series: Vec<MonthValue> = months
            .iter()
            .enumerate()
            .map(|(i, m)| MonthValue {
                month: *m,
                value: Some(Decimal::from(100 + i64::try_from(i).unwrap())),
            })
            .collect();
        let march = Period::month_of(d(2026, 3, 1));
        assert_eq!(last_complete(&series), Some(march));
        let c = compare_month(&series, march);
        assert_eq!(c.len(), 5);
        let prev_month = c[0].comparison.as_ref().unwrap();
        assert_eq!(
            (prev_month.current, prev_month.reference),
            (dec!(114), dec!(113))
        );
        let prev_year = c[1].comparison.as_ref().unwrap();
        assert_eq!(prev_year.reference, dec!(102));
        assert_eq!(c[2].comparison.as_ref().unwrap().reference, dec!(112));

        // ein unvollständiger Monat im Jahresfenster
        series[3].value = None;
        let c = compare_month(&series, march);
        assert!(c[0].comparison.is_some());
        assert!(c[3].comparison.is_some(), "6 Monate ohne April 2025");
        assert!(c[4].comparison.is_none(), "Daten unvollständig");
        assert!(
            compare_month(&series, Period::month_of(d(2025, 4, 1)))
                .iter()
                .all(|m| m.comparison.is_none())
        );
    }

    // @test SPEC_ANA_COMPARE_VIEW
    #[test]
    fn year_to_date_uses_the_28th_of_february_in_the_previous_year() {
        let cal = berlin();
        // 1 Einheit je Tag über 425 Tage
        let iv = ivs(cal, &[(d(2027, 1, 1), dec!(0)), (d(2028, 3, 1), dec!(425))]);
        let until = cal.to_utc(d(2028, 2, 29).and_hms_opt(12, 0, 0).unwrap());
        let ytd = year_to_date(&[&iv], cal, until).unwrap();
        assert_eq!(
            ytd.previous_until,
            d(2027, 2, 28).and_hms_opt(12, 0, 0).unwrap()
        );
        assert_eq!(ytd.current, Some(dec!(59.5)));
        assert_eq!(ytd.previous, Some(dec!(58.5)));
        assert_eq!(ytd.comparison.unwrap().abs_diff, dec!(1.0));

        let short = ivs(cal, &[(d(2028, 1, 1), dec!(0)), (d(2028, 3, 1), dec!(60))]);
        let ytd = year_to_date(&[&short], cal, until).unwrap();
        assert!(ytd.current.is_some() && ytd.previous.is_none() && ytd.comparison.is_none());
    }

    // @test SPEC_FC_CURRENT_YEAR
    #[test]
    fn forecast_for_calendar_and_billing_year() {
        let cal = berlin();
        let uniform = SeasonProfile::uniform();
        let year = Period::year(2026);
        let iv = ivs(
            cal,
            &[(d(2026, 1, 1), dec!(0)), (d(2026, 7, 2), dec!(1820))],
        );
        let PeriodForecast::Projected(f) = forecast_period(&[&iv], cal, year, &uniform) else {
            panic!("Prognose erwartet");
        };
        assert_eq!((f.observed, f.projected_total), (dec!(1820), dec!(3650)));
        assert_eq!(
            projected_cumulative(&f, &uniform, d(2026, 7, 2)),
            dec!(1820)
        );
        assert_eq!(
            projected_cumulative(&f, &uniform, d(2027, 1, 1)),
            dec!(3650)
        );

        let late = ivs(
            cal,
            &[(d(2026, 2, 1), dec!(0)), (d(2026, 7, 2), dec!(1000))],
        );
        assert_eq!(
            forecast_period(&[&late], cal, year, &uniform),
            PeriodForecast::MissingStart
        );
        let old = ivs(
            cal,
            &[(d(2025, 1, 1), dec!(0)), (d(2025, 12, 1), dec!(1000))],
        );
        assert_eq!(
            forecast_period(&[&old], cal, year, &uniform),
            PeriodForecast::NoReading
        );

        let billing = current_billing_year(d(2026, 3, 10), 4, 1).unwrap();
        assert_eq!(billing, Period::new(d(2025, 4, 1), d(2026, 4, 1)));
        assert_eq!(
            current_billing_year(d(2026, 4, 1), 4, 1).unwrap().start,
            d(2026, 4, 1)
        );
        let iv = ivs(
            cal,
            &[(d(2025, 3, 1), dec!(0)), (d(2025, 10, 1), dec!(2140))],
        );
        let f = forecast_period(&[&iv], cal, billing, &uniform);
        // 183 von 365 Tagen gemessen (01.04. bis 30.09.)
        assert_eq!(f.projected().unwrap().coverage, dec!(0.5014));
    }
}
