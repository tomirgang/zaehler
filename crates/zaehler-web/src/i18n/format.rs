//! Formatierung von Zahlen und Zeitpunkten je Sprache (SPEC_I18N).

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use rust_decimal::Decimal;

use super::Lang;

/// Dezimalzahl mit fester Anzahl Nachkommastellen, ohne Tausendertrennzeichen.
// @impl SPEC_I18N
#[must_use]
pub fn decimal(lang: Lang, value: Decimal, places: u8) -> String {
    let rounded = value.round_dp(u32::from(places));
    // kein „-0,0“ nach dem Runden
    let rounded = if rounded.is_zero() {
        Decimal::ZERO
    } else {
        rounded
    };
    let s = format!("{:.*}", usize::from(places), rounded);
    match lang {
        Lang::De => s.replace('.', ","),
        Lang::En => s,
    }
}

/// Dezimalzahl mit den vorhandenen Nachkommastellen (für Formularwerte).
#[must_use]
pub fn decimal_plain(lang: Lang, value: Decimal) -> String {
    let s = value.normalize().to_string();
    match lang {
        Lang::De => s.replace('.', ","),
        Lang::En => s,
    }
}

/// Dezimalzahl mit Vorzeichen (`+12,5`, `-3,0`).
#[must_use]
pub fn signed(lang: Lang, value: Decimal, places: u8) -> String {
    let s = decimal(lang, value, places);
    if value.round_dp(u32::from(places)) > Decimal::ZERO {
        format!("+{s}")
    } else {
        s
    }
}

/// Anteil 0..1 in Prozent, z.B. `33 %`.
#[must_use]
pub fn percent(lang: Lang, share: Decimal, places: u8) -> String {
    format!("{} %", decimal(lang, share * Decimal::ONE_HUNDRED, places))
}

/// Monat mit Jahr, z.B. „März 2026“.
#[must_use]
pub fn month(lang: Lang, d: NaiveDate) -> String {
    format!("{} {}", lang.t(&format!("month-{}", d.month())), d.year())
}

/// Kurzer Monatsname für Diagrammachsen.
#[must_use]
pub fn month_short(lang: Lang, d: NaiveDate) -> String {
    lang.t(&format!("month-short-{}", d.month()))
}

#[must_use]
pub fn date(lang: Lang, d: NaiveDate) -> String {
    match lang {
        Lang::De => d.format("%d.%m.%Y").to_string(),
        Lang::En => d.format("%Y-%m-%d").to_string(),
    }
}

#[must_use]
pub fn datetime(lang: Lang, local: NaiveDateTime) -> String {
    match lang {
        Lang::De => local.format("%d.%m.%Y %H:%M").to_string(),
        Lang::En => local.format("%Y-%m-%d %H:%M").to_string(),
    }
}

/// UTC-Zeitpunkt in der Zeitzone des Haushalts.
#[must_use]
pub fn datetime_in(lang: Lang, at: DateTime<Utc>, tz: Tz) -> String {
    datetime(lang, at.with_timezone(&tz).naive_local())
}

/// Wert für `<input type="datetime-local">` (sprachunabhängig).
#[must_use]
pub fn datetime_input(local: NaiveDateTime) -> String {
    local.format("%Y-%m-%dT%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    // @test SPEC_I18N
    #[test]
    fn formats_per_language() {
        let v = Decimal::new(12_345_678, 3);
        assert_eq!(decimal(Lang::De, v, 1), "12345,7");
        assert_eq!(decimal(Lang::En, v, 3), "12345.678");
        assert_eq!(decimal(Lang::De, Decimal::new(5, 0), 2), "5,00");
        assert_eq!(decimal_plain(Lang::De, Decimal::new(1500, 3)), "1,5");
        let d = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        assert_eq!(date(Lang::De, d), "15.01.2026");
        assert_eq!(date(Lang::En, d), "2026-01-15");
        assert_eq!(month(Lang::De, d), "Januar 2026");
        assert_eq!(month(Lang::En, d), "January 2026");
        assert_eq!(month_short(Lang::De, d), "Jan");
        assert_eq!(signed(Lang::De, Decimal::new(125, 1), 1), "+12,5");
        assert_eq!(signed(Lang::De, Decimal::new(-3, 0), 1), "-3,0");
        assert_eq!(signed(Lang::De, Decimal::new(-1, 3), 1), "0,0");
        assert_eq!(percent(Lang::De, Decimal::new(3288, 4), 0), "33 %");
    }
}
