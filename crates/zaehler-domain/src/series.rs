//! Monatsserien und Aggregation über Register.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    calendar::Calendar,
    consumption::{Interval, consumption_in},
    period::Period,
};

/// Verbrauch eines Kalendermonats; `None`, wenn der Monat nicht vollständig
/// durch Ablesungen abgedeckt ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthValue {
    pub month: Period,
    pub value: Option<Decimal>,
}

/// Kalendermonate vom Monat von `first` bis einschließlich des Monats von `last`.
#[must_use]
pub fn months_between(first: NaiveDate, last: NaiveDate) -> Vec<Period> {
    let end = Period::month_of(last);
    let mut month = Period::month_of(first);
    let mut out = Vec::new();
    while month.start <= end.start {
        out.push(month);
        month = month.shift_months(1);
    }
    out
}

/// Verbrauch mehrerer Register im Zeitraum `[from, to)` als Summe; `None`
/// ohne Register oder sobald ein Register den Zeitraum nicht abdeckt.
// @impl SPEC_ANA_REGISTER_AGG
#[must_use]
pub fn total_in(
    registers: &[&[Interval]],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Option<Decimal> {
    if registers.is_empty() {
        return None;
    }
    registers
        .iter()
        .map(|intervals| consumption_in(intervals, from, to))
        .sum()
}

/// Ende des Zeitraums, den alle Register abdecken (letzte gemeinsame Ablesung).
#[must_use]
pub fn covered_until(registers: &[&[Interval]]) -> Option<DateTime<Utc>> {
    registers
        .iter()
        .map(|intervals| intervals.last().map(|i| i.end))
        .min()
        .flatten()
}

/// Monatswerte der Register: Beginn und Ende jedes Monats in der Zeitzone des
/// Haushalts, Verbrauch über [`total_in`]. Gerundet wird erst bei der Anzeige.
// @impl SPEC_ANA_MONTHLY_SERIES, SPEC_ANA_REGISTER_AGG
#[must_use]
pub fn monthly(registers: &[&[Interval]], months: &[Period], cal: Calendar) -> Vec<MonthValue> {
    months
        .iter()
        .map(|m| MonthValue {
            month: *m,
            value: total_in(registers, cal.start_of(m.start), cal.start_of(m.end)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{consumption::intervals, reading::Reading};
    use chrono::{Duration, TimeZone};
    use proptest::prelude::*;
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    fn berlin() -> Calendar {
        Calendar::new(chrono_tz::Europe::Berlin)
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn reading(at: DateTime<Utc>, value: Decimal) -> Reading {
        Reading {
            at,
            value,
            device_id: Uuid::nil(),
            rollover_digits: None,
        }
    }

    // @test SPEC_ANA_MONTHLY_SERIES
    #[test]
    fn months_are_local_and_incomplete_months_are_none() {
        let cal = berlin();
        // Ablesungen am lokalen Monatsbeginn (UTC+1 im Winter)
        let rs = [
            reading(cal.start_of(d(2026, 1, 1)), dec!(100)),
            reading(cal.start_of(d(2026, 2, 1)), dec!(410)),
            // 42 Tage, davon 28 im Februar
            reading(cal.start_of(d(2026, 3, 15)), dec!(830)),
        ];
        let ivs = intervals(&rs).unwrap();
        let months = months_between(d(2025, 12, 5), d(2026, 3, 20));
        assert_eq!(months.len(), 4);
        let series = monthly(&[&ivs], &months, cal);
        assert_eq!(series[0].value, None, "Dezember nicht abgedeckt");
        assert_eq!(series[1].value, Some(dec!(310)));
        assert_eq!(series[2].value, Some(dec!(280)), "Februar anteilig");
        assert_eq!(series[3].value, None, "März nur zur Hälfte abgedeckt");
    }

    // @test SPEC_ANA_REGISTER_AGG
    #[test]
    fn register_totals_need_every_register() {
        let cal = berlin();
        let ht = intervals(&[
            reading(cal.start_of(d(2026, 1, 1)), dec!(0)),
            reading(cal.start_of(d(2026, 3, 1)), dec!(200)),
        ])
        .unwrap();
        let nt = intervals(&[
            reading(cal.start_of(d(2026, 1, 1)), dec!(0)),
            reading(cal.start_of(d(2026, 2, 1)), dec!(50)),
        ])
        .unwrap();
        let months = months_between(d(2026, 1, 1), d(2026, 2, 1));
        let series = monthly(&[&ht, &nt], &months, cal);
        assert!(series[0].value.is_some());
        assert_eq!(series[1].value, None, "NT deckt Februar nicht ab");
        assert_eq!(monthly(&[], &months, cal)[0].value, None);
        assert_eq!(
            covered_until(&[&ht, &nt]),
            Some(cal.start_of(d(2026, 2, 1)))
        );
        assert_eq!(covered_until(&[&ht, &[]]), None);
    }

    proptest! {
        /// Die Summe der Monatswerte eines vollständig abgedeckten Zeitraums
        /// entspricht dem Verbrauch des Zeitraums.
        // @test SPEC_ANA_MONTHLY_SERIES
        #[test]
        fn month_values_sum_to_period_consumption(
            start_hour in 0_i64..(24 * 365),
            steps in prop::collection::vec((1_i64..(24 * 75), 0_i64..500_000), 2..60),
        ) {
            let cal = berlin();
            let mut at = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap() + Duration::hours(start_hour);
            let mut value = Decimal::ZERO;
            let mut rs = vec![reading(at, value)];
            for (hours, milli) in steps {
                at += Duration::hours(hours);
                value += Decimal::new(milli, 3);
                rs.push(reading(at, value));
            }
            let ivs = intervals(&rs).unwrap();
            let first = cal.local_date(rs[0].at);
            let last = cal.local_date(at);
            let series = monthly(&[&ivs], &months_between(first, last), cal);
            let complete: Vec<&MonthValue> = series.iter().filter(|m| m.value.is_some()).collect();
            prop_assume!(!complete.is_empty());
            // abgedeckte Monate liegen lückenlos hintereinander
            for pair in complete.windows(2) {
                prop_assert_eq!(pair[0].month.end, pair[1].month.start);
            }
            let from = cal.start_of(complete[0].month.start);
            let to = cal.start_of(complete[complete.len() - 1].month.end);
            let sum: Decimal = complete.iter().filter_map(|m| m.value).sum();
            let whole = consumption_in(&ivs, from, to).unwrap();
            prop_assert!((sum - whole).abs() < dec!(0.000000001), "{} != {}", sum, whole);
            prop_assert!(complete.iter().all(|m| m.value.unwrap() >= Decimal::ZERO));
        }
    }
}
