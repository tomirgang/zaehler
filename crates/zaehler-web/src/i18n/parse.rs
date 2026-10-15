//! Parsen lokalisierter Eingaben (SPEC_I18N).

use std::str::FromStr;

use chrono::NaiveDateTime;
use rust_decimal::Decimal;

/// Eingabe entspricht nicht dem erwarteten Format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalid;

/// Dezimalzahl mit Komma oder Punkt als Dezimaltrenner. Tausendertrennzeichen
/// (mehr als ein Trenner) werden abgelehnt. Leere Eingabe ergibt `Ok(None)`.
// @impl SPEC_I18N
pub fn decimal(input: &str) -> Result<Option<Decimal>, Invalid> {
    let s: String = input
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if s.is_empty() {
        return Ok(None);
    }
    let separators = s.chars().filter(|c| *c == ',' || *c == '.').count();
    if separators > 1 || s.starts_with(['+']) {
        return Err(Invalid);
    }
    let normalized = s.replace(',', ".");
    if !normalized
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
    {
        return Err(Invalid);
    }
    Decimal::from_str(&normalized)
        .map(Some)
        .map_err(|_| Invalid)
}

/// Ganze Zahl; leere Eingabe ergibt `Ok(None)`.
pub fn integer<T: FromStr>(input: &str) -> Result<Option<T>, Invalid> {
    let s = input.trim();
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<T>().map(Some).map_err(|_| Invalid)
}

/// Wert eines `<input type="datetime-local">` (`2026-01-15T14:30`, optional mit Sekunden).
pub fn datetime_local(input: &str) -> Result<NaiveDateTime, Invalid> {
    let s = input.trim();
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|_| Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    // @test SPEC_I18N
    #[test]
    fn decimal_accepts_comma_and_point_only_once() {
        assert_eq!(decimal("12,5"), Ok(Some(Decimal::new(125, 1))));
        assert_eq!(decimal(" 12.5 "), Ok(Some(Decimal::new(125, 1))));
        assert_eq!(decimal("1 234"), Ok(Some(Decimal::new(1234, 0))));
        assert_eq!(decimal(""), Ok(None));
        assert_eq!(decimal("1.234,5"), Err(Invalid));
        assert_eq!(decimal("12a"), Err(Invalid));
        assert_eq!(decimal("1e5"), Err(Invalid));
        assert!(datetime_local("2026-01-15T14:30").is_ok());
        assert!(datetime_local("15.01.2026").is_err());
        assert_eq!(integer::<u8>("3"), Ok(Some(3)));
        assert_eq!(integer::<u8>("300"), Err(Invalid));
    }
}
