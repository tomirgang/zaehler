//! Verträge, Kostenpositionen, Preiskomponenten und Abschläge.
//!
//! Modell (siehe ADR_008): Ein Vertrag gehört zu einer Messstelle und hat einen
//! Abrechnungszeitraum. Er besteht aus Kostenpositionen (z.B. Frischwasser,
//! Schmutzwasser, Zählermiete), jede mit zeitlich gültigen Preiskomponenten.
//! Alle Preise sind brutto in EUR (ADR_009).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::period::Period;

/// Bezugsintervall des Grundpreises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BasePriceInterval {
    Day,
    Month,
    Year,
}

/// Preis pro Einheit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum UnitPricing {
    Flat {
        price: Decimal,
    },
    /// Staffel: Stufen nach kumuliertem Verbrauch im Abrechnungszeitraum.
    Tiered {
        tiers: Vec<Tier>,
    },
    /// Dynamischer Tarif: Monatsdurchschnittspreise werden separat erfasst;
    /// `fallback` dient für die Prognose zukünftiger Monate.
    Dynamic {
        markup: Decimal,
        fallback: Decimal,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier {
    /// Obergrenze (exklusiv) dieser Stufe; `None` = unbegrenzt.
    pub up_to: Option<Decimal>,
    pub price: Decimal,
}

/// Ein Preisstand, gültig ab `valid_from` bis zum nächsten Preisstand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceComponent {
    pub valid_from: NaiveDate,
    pub base_price: Decimal,
    pub base_interval: BasePriceInterval,
    pub unit_pricing: UnitPricing,
}

/// Kostenposition eines Vertrags mit Umrechnungsfaktor Messeinheit →
/// Abrechnungseinheit (z.B. Gas: Brennwert × Zustandszahl, m³ → kWh).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostPosition {
    pub name: String,
    pub conversion_factor: Decimal,
    pub components: Vec<PriceComponent>,
}

/// Einmalige oder zeitraumbezogene Anpassungen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Adjustment {
    /// Bonus/Gutschrift (positiver Betrag reduziert Kosten).
    Bonus {
        amount: Decimal,
        granted_on: NaiveDate,
    },
    /// Preisdeckel: Kontingent `quota` des Verbrauchs zu `capped_price`.
    PriceCap {
        quota: Decimal,
        capped_price: Decimal,
        period: Period,
    },
}

/// Abschlag mit Gültigkeit und Turnus in Monaten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installment {
    pub amount: Decimal,
    pub valid_from: NaiveDate,
    pub every_months: u8,
}

/// Grundpreis für `days` Tage. Monats- und Jahrespreise werden tagesgenau
/// über 365 Tage umgelegt (Monatspreis × 12 / 365).
// @impl SPEC_TAR_BASE_PRORATION
#[must_use]
pub fn base_cost(base_price: Decimal, interval: BasePriceInterval, days: i64) -> Decimal {
    // Erst multiplizieren, dann teilen, um Rundungsartefakte zu vermeiden.
    let days = Decimal::from(days);
    match interval {
        BasePriceInterval::Day => base_price * days,
        BasePriceInterval::Month => base_price * Decimal::from(12) * days / Decimal::from(365),
        BasePriceInterval::Year => base_price * days / Decimal::from(365),
    }
}

/// Arbeitspreis für `amount` Einheiten nach Staffel. `already_consumed` ist der
/// bereits im Abrechnungszeitraum verbrauchte Anteil (für unterjährige Teilrechnung).
// @impl SPEC_TAR_TIERED
#[must_use]
pub fn tiered_cost(tiers: &[Tier], already_consumed: Decimal, amount: Decimal) -> Decimal {
    let mut cost = Decimal::ZERO;
    let mut pos = already_consumed;
    let end = already_consumed + amount;
    let mut lower = Decimal::ZERO;
    for tier in tiers {
        let upper = tier.up_to.unwrap_or(Decimal::MAX);
        let from = pos.max(lower);
        let to = end.min(upper);
        if to > from {
            cost += (to - from) * tier.price;
            pos = to;
        }
        lower = upper;
        if pos >= end {
            break;
        }
    }
    cost
}

/// Summe der fälligen Abschläge im Zeitraum (Fälligkeit jeweils am Tag
/// `valid_from` + k·Turnus). `installments` müssen nach `valid_from` sortiert sein.
// @impl SPEC_TAR_INSTALLMENT_SETTLEMENT
#[must_use]
pub fn installments_in(installments: &[Installment], period: Period) -> Decimal {
    let mut sum = Decimal::ZERO;
    for (i, inst) in installments.iter().enumerate() {
        let valid_until = installments
            .get(i + 1)
            .map_or(period.end, |n| n.valid_from.min(period.end));
        let mut due = inst.valid_from;
        while due < valid_until {
            if due >= period.start {
                sum += inst.amount;
            }
            due = due + chrono::Months::new(u32::from(inst.every_months.max(1)));
        }
    }
    sum
}

/// Erwartetes Ergebnis der Jahresabrechnung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settlement {
    pub projected_cost: Decimal,
    pub installments_paid: Decimal,
    /// Positiv = Nachzahlung, negativ = Erstattung.
    pub balance: Decimal,
    /// Vorgeschlagener neuer monatlicher Abschlag (auf volle Euro aufgerundet).
    pub suggested_monthly_installment: Decimal,
}

// @impl SPEC_TAR_INSTALLMENT_SETTLEMENT
#[must_use]
pub fn settlement(projected_cost: Decimal, installments_paid: Decimal) -> Settlement {
    Settlement {
        projected_cost,
        installments_paid,
        balance: projected_cost - installments_paid,
        suggested_monthly_installment: (projected_cost / Decimal::from(12)).ceil(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    // @test SPEC_TAR_BASE_PRORATION
    #[test]
    fn base_price_prorated_per_day() {
        assert_eq!(
            base_cost(dec!(146), BasePriceInterval::Year, 365),
            dec!(146)
        );
        assert_eq!(
            base_cost(dec!(10), BasePriceInterval::Month, 365),
            dec!(120)
        );
        assert_eq!(base_cost(dec!(0.5), BasePriceInterval::Day, 10), dec!(5));
    }

    // @test SPEC_TAR_TIERED
    #[test]
    fn tiered_spans_tiers() {
        let tiers = [
            Tier {
                up_to: Some(dec!(1000)),
                price: dec!(0.30),
            },
            Tier {
                up_to: None,
                price: dec!(0.40),
            },
        ];
        assert_eq!(tiered_cost(&tiers, dec!(0), dec!(500)), dec!(150));
        assert_eq!(tiered_cost(&tiers, dec!(800), dec!(400)), dec!(140));
    }

    // @test SPEC_TAR_INSTALLMENT_SETTLEMENT
    #[test]
    fn installments_and_settlement() {
        let insts = [
            Installment {
                amount: dec!(80),
                valid_from: d(2026, 1, 15),
                every_months: 1,
            },
            Installment {
                amount: dec!(100),
                valid_from: d(2026, 7, 15),
                every_months: 1,
            },
        ];
        let paid = installments_in(&insts, Period::year(2026));
        assert_eq!(paid, dec!(1080));
        let s = settlement(dec!(1200), paid);
        assert_eq!(s.balance, dec!(120));
        assert_eq!(s.suggested_monthly_installment, dec!(100));
    }
}
