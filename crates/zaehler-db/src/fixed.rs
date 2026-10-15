//! Festkomma-Abbildung von Dezimalwerten auf `i64` (ADR_009).
//!
//! - Mengen (Zählerstände, Verbräuche): Skala 10⁻³ ("milli")
//! - Geldbeträge und Preise: Skala 10⁻⁶ ("micro", z.B. 0,324571 €/kWh)
//!
//! So verhalten sich PostgreSQL und SQLite identisch (keine REAL-Rundung),
//! und Sortierung/Aggregation in SQL bleibt möglich.

use rust_decimal::Decimal;
use thiserror::Error;

pub const QUANTITY_SCALE: u32 = 3;
pub const MONEY_SCALE: u32 = 6;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FixedError {
    #[error("Wert {0} hat mehr als {1} Nachkommastellen")]
    TooPrecise(Decimal, u32),
    #[error("Wert {0} ist zu groß")]
    Overflow(Decimal),
}

// @impl SPEC_STORE_FIXED_POINT
fn to_fixed(value: Decimal, scale: u32) -> Result<i64, FixedError> {
    let mut v = value.normalize();
    if v.scale() > scale {
        return Err(FixedError::TooPrecise(value, scale));
    }
    v.rescale(scale);
    i64::try_from(v.mantissa()).map_err(|_| FixedError::Overflow(value))
}

fn from_fixed(raw: i64, scale: u32) -> Decimal {
    Decimal::new(raw, scale)
}

pub fn quantity_to_db(value: Decimal) -> Result<i64, FixedError> {
    to_fixed(value, QUANTITY_SCALE)
}

#[must_use]
pub fn quantity_from_db(raw: i64) -> Decimal {
    from_fixed(raw, QUANTITY_SCALE)
}

pub fn money_to_db(value: Decimal) -> Result<i64, FixedError> {
    to_fixed(value, MONEY_SCALE)
}

#[must_use]
pub fn money_from_db(raw: i64) -> Decimal {
    from_fixed(raw, MONEY_SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    // @test SPEC_STORE_FIXED_POINT
    #[test]
    fn roundtrip_and_precision_guard() {
        assert_eq!(quantity_to_db(dec!(12345.678)), Ok(12_345_678));
        assert_eq!(quantity_from_db(12_345_678), dec!(12345.678));
        assert_eq!(money_to_db(dec!(0.324571)), Ok(324_571));
        assert!(matches!(
            quantity_to_db(dec!(1.0001)),
            Err(FixedError::TooPrecise(..))
        ));
    }
}
