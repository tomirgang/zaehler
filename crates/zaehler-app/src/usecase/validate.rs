//! Gemeinsame Eingabeprüfungen.

use rust_decimal::Decimal;

use crate::FieldErrors;

/// Prüft einen Namen auf 1 bis `max` Zeichen nach `trim`.
pub fn name(errors: &mut FieldErrors, field: &str, value: &str, max: usize, code: &'static str) {
    let len = value.trim().chars().count();
    if len == 0 || len > max {
        errors.add_with(field, code, vec![("max", max.to_string())]);
    }
}

/// Prüft einen Zählerstand: nicht negativ, höchstens `places` Nachkommastellen,
/// kleiner als 10^`digits` (falls Vorkommastellen gesetzt).
pub fn meter_value(
    errors: &mut FieldErrors,
    field: &str,
    value: Decimal,
    places: u8,
    digits: Option<u8>,
) {
    if value.is_sign_negative() && !value.is_zero() {
        errors.add(field, "reading.negative");
        return;
    }
    if value.normalize().scale() > u32::from(places) {
        errors.add_with(
            field,
            "reading.too_many_decimals",
            vec![("places", places.to_string())],
        );
    }
    if let Some(d) = digits
        && value >= Decimal::from(10_u64.pow(u32::from(d)))
    {
        errors.add_with(
            field,
            "reading.exceeds_digits",
            vec![("digits", d.to_string())],
        );
    }
}

/// Zählernummer vergleichbar machen: Großbuchstaben, ohne Leerzeichen und Bindestriche.
#[must_use]
pub fn normalize_serial(serial: &str) -> String {
    serial
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn meter_value_rules() {
        let mut e = FieldErrors::default();
        meter_value(&mut e, "v", dec!(12.34), 2, Some(5));
        assert!(e.is_empty());
        meter_value(&mut e, "v", dec!(12.345), 2, None);
        meter_value(&mut e, "v", dec!(100000), 0, Some(5));
        meter_value(&mut e, "v", dec!(-1), 0, None);
        let codes: Vec<_> = e.0.iter().map(|f| f.code).collect();
        assert_eq!(
            codes,
            [
                "reading.too_many_decimals",
                "reading.exceeds_digits",
                "reading.negative"
            ]
        );
        // nachgestellte Nullen zählen nicht als Nachkommastellen
        let mut e = FieldErrors::default();
        meter_value(&mut e, "v", dec!(1.500), 1, None);
        assert!(e.is_empty());
    }

    #[test]
    fn serial_normalization() {
        assert_eq!(normalize_serial(" 1 emh-0012 "), "1EMH0012");
    }
}
