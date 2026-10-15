//! Formatierung von Zahlen und Zeitpunkten je Sprache (SPEC_I18N).

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use rust_decimal::Decimal;

use super::Lang;

/// Dezimalzahl mit fester Anzahl Nachkommastellen, ohne Tausendertrennzeichen.
// @impl SPEC_I18N
#[must_use]
pub fn decimal(lang: Lang, value: Decimal, places: u8) -> String {
    let s = format!(
        "{:.*}",
        usize::from(places),
        value.round_dp(u32::from(places))
    );
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
    }
}
