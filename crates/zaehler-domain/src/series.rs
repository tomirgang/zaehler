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

/// Beginn des Zeitraums, den alle Register abdecken (erste gemeinsame Ablesung).
#[must_use]
pub fn covered_from(registers: &[&[Interval]]) -> Option<DateTime<Utc>> {
    registers
        .iter()
        .map(|intervals| intervals.first().map(|i| i.start))
        .max()
        .flatten()
}

/// Monatswert für Diagramme: vollständig gemessen, oder bei einem nur
/// teilweise abgedeckten Monat gemessener Anteil und Prognose der fehlenden
/// Zeit mit der Rate des gemessenen Anteils.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthSplit {
    pub month: Period,
    pub measured: Option<Decimal>,
    /// Nur bei teilweise abgedeckten Monaten
    pub projected: Option<Decimal>,
}

/// Monatswerte wie [`monthly`]; ein Monat, den die Ablesungen nur teilweise
/// abdecken (erster oder letzter Monat), erhält den gemessenen Anteil und
/// die Prognose für die fehlende Zeit: `gemessen · fehlend / abgedeckt`
/// (sekundengenau in der Zeitzone des Haushalts).
// @impl SPEC_ANA_PARTIAL_MONTHS
#[must_use]
pub fn monthly_split(
    registers: &[&[Interval]],
    months: &[Period],
    cal: Calendar,
) -> Vec<MonthSplit> {
    let covered = covered_from(registers).zip(covered_until(registers));
    months
        .iter()
        .map(|m| {
            let (start, end) = (cal.start_of(m.start), cal.start_of(m.end));
            let empty = MonthSplit {
                month: *m,
                measured: None,
                projected: None,
            };
            if let Some(value) = total_in(registers, start, end) {
                return MonthSplit {
                    measured: Some(value),
                    ..empty
                };
            }
            let Some((from, until)) = covered else {
                return empty;
            };
            let (a, b) = (start.max(from), end.min(until));
            let covered_secs = (b - a).num_seconds();
            let Some(measured) = total_in(registers, a, b).filter(|_| covered_secs > 0) else {
                return empty;
            };
            let missing = (end - start).num_seconds() - covered_secs;
            MonthSplit {
                measured: Some(measured),
                projected: Some(
                    (measured * Decimal::from(missing) / Decimal::from(covered_secs)).round_dp(3),
                ),
                ..empty
            }
        })
        .collect()
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

/// Zeitraum `[from, until)`, in dem ein Zwischenzähler einer Messstelle
/// zugeordnet ist (Einbauzeit des Geräts); `until = None` bis heute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub from: DateTime<Utc>,
    pub until: Option<DateTime<Utc>>,
}

/// Monatsverbrauch der Register innerhalb der Zeitfenster: außerhalb 0,
/// `None`, wenn ein Fenster im Monat nicht durch Ablesungen abgedeckt ist.
// @impl SPEC_MTR_SUBMETER
#[must_use]
pub fn monthly_within(
    registers: &[&[Interval]],
    windows: &[Window],
    months: &[Period],
    cal: Calendar,
) -> Vec<MonthValue> {
    months
        .iter()
        .map(|m| {
            let (start, end) = (cal.start_of(m.start), cal.start_of(m.end));
            let value = windows
                .iter()
                .filter_map(|w| {
                    let from = start.max(w.from);
                    let to = w.until.map_or(end, |u| end.min(u));
                    (from < to).then_some((from, to))
                })
                .map(|(from, to)| total_in(registers, from, to))
                .sum::<Option<Decimal>>();
            MonthValue { month: *m, value }
        })
        .collect()
}

/// Monatswerte abzüglich der Teile (z.B. Zwischenzähler); `None`, sobald
/// der Hauptwert oder ein Teil im Monat fehlt.
// @impl SPEC_MTR_SUBMETER
#[must_use]
pub fn subtract(main: &[MonthValue], parts: &[Vec<MonthValue>]) -> Vec<MonthValue> {
    main.iter()
        .map(|m| {
            let parts: Option<Decimal> = parts
                .iter()
                .map(|p| {
                    p.iter()
                        .find(|x| x.month == m.month)
                        .map_or(Some(Decimal::ZERO), |x| x.value)
                })
                .sum();
            MonthValue {
                month: m.month,
                value: m.value.zip(parts).map(|(v, p)| v - p),
            }
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

    // @test SPEC_MTR_SUBMETER
    #[test]
    fn submeters_are_subtracted_only_while_assigned() {
        let cal = berlin();
        let at = |m: u32| cal.start_of(d(2026, m, 1));
        let house = intervals(&[
            reading(at(1), dec!(0)),
            reading(at(2), dec!(500)),
            reading(at(3), dec!(900)),
            reading(at(4), dec!(1300)),
        ])
        .unwrap();
        // Wärmepumpe: Januar und Februar Zwischenzähler, ab März eigener Zähler
        let pump = intervals(&[
            reading(at(1), dec!(0)),
            reading(at(2), dec!(200)),
            reading(at(3), dec!(350)),
            reading(at(4), dec!(450)),
        ])
        .unwrap();
        let months = months_between(d(2026, 1, 1), d(2026, 4, 1));
        let windows = [Window {
            from: at(1),
            until: Some(at(3)),
        }];
        let part = monthly_within(&[&pump], &windows, &months, cal);
        assert_eq!(
            part.iter().map(|m| m.value).collect::<Vec<_>>(),
            [
                Some(dec!(200)),
                Some(dec!(150)),
                Some(dec!(0)),
                Some(dec!(0))
            ]
        );
        let main = monthly(&[&house], &months, cal);
        let net = subtract(&main, &[part]);
        assert_eq!(
            net.iter().map(|m| m.value).collect::<Vec<_>>(),
            [Some(dec!(300)), Some(dec!(250)), Some(dec!(400)), None]
        );
        // fehlt der Zwischenzähler im Monat, fehlt der Nettowert
        let open = [Window {
            from: at(1),
            until: None,
        }];
        let part = monthly_within(&[&pump], &open, &months, cal);
        assert_eq!(part[3].value, None);
        assert_eq!(subtract(&main, &[part])[2].value, Some(dec!(300)));
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

    // @test SPEC_ANA_PARTIAL_MONTHS
    #[test]
    fn partial_months_are_measured_and_projected() {
        let cal = berlin();
        // 10 Einheiten je Tag vom 21.04. bis 21.09.
        let rs = [
            reading(cal.start_of(d(2026, 4, 21)), dec!(0)),
            reading(cal.start_of(d(2026, 9, 21)), dec!(1530)),
        ];
        let ivs = intervals(&rs).unwrap();
        let months = months_between(d(2026, 3, 1), d(2026, 10, 1));
        let s = monthly_split(&[&ivs], &months, cal);
        assert_eq!((s[0].measured, s[0].projected), (None, None), "März");
        assert_eq!(s[1].measured, Some(dec!(100)), "April: 10 Tage gemessen");
        assert_eq!(s[1].projected, Some(dec!(200)), "20 Tage prognostiziert");
        assert_eq!((s[2].measured, s[2].projected), (Some(dec!(310)), None));
        assert_eq!(s[6].measured, Some(dec!(200)), "September: 20 Tage");
        assert_eq!(s[6].projected, Some(dec!(100)));
        assert_eq!((s[7].measured, s[7].projected), (None, None), "Oktober");
        // vollständige Monate wie `monthly`
        let plain = monthly(&[&ivs], &months, cal);
        for (split, value) in s.iter().zip(&plain) {
            if value.value.is_some() {
                assert_eq!(split.measured, value.value);
            }
        }
        assert_eq!(covered_from(&[&ivs]), Some(cal.start_of(d(2026, 4, 21))));
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
