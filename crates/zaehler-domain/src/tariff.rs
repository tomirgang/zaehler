//! Verträge, Kostenpositionen, Preiskomponenten und Abschläge.
//!
//! Modell (siehe ADR_008): Ein Vertrag gehört zu einer Messstelle und hat einen
//! Abrechnungszeitraum. Er besteht aus Kostenpositionen (z.B. Frischwasser,
//! Schmutzwasser, Zählermiete), jede mit zeitlich gültigen Preiskomponenten.
//! Alle Preise sind brutto in EUR (ADR_009).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{meter::UnknownVariant, period::Period};

/// Bezugsintervall des Grundpreises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BasePriceInterval {
    Day,
    Month,
    Year,
}

impl BasePriceInterval {
    pub const ALL: [Self; 3] = [Self::Day, Self::Month, Self::Year];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
}

impl std::str::FromStr for BasePriceInterval {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|i| i.as_str() == s)
            .ok_or_else(|| UnknownVariant(s.to_owned()))
    }
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
    /// `fallback` dient für die Prognose zukünftiger Monate. In v1 nicht
    /// angeboten (REQ_TAR_007).
    Dynamic {
        markup: Decimal,
        fallback: Decimal,
    },
}

impl UnitPricing {
    /// Kosten für `amount` Abrechnungseinheiten, nachdem im
    /// Abrechnungszeitraum bereits `already` Einheiten angefallen sind.
    #[must_use]
    pub fn cost(&self, already: Decimal, amount: Decimal) -> Decimal {
        match self {
            Self::Flat { price } => amount * price,
            Self::Tiered { tiers } => tiered_cost(tiers, already, amount),
            Self::Dynamic { fallback, .. } => amount * fallback,
        }
    }

    /// Preise als Gutschrift (Einspeisevergütung, SPEC_PV_FEED_IN): jeder
    /// Arbeitspreis wird `-|Preis|`, unabhängig davon, ob er positiv oder
    /// negativ erfasst wurde.
    #[must_use]
    pub fn as_credit(&self) -> Self {
        let credit = |p: Decimal| -p.abs();
        match self {
            Self::Flat { price } => Self::Flat {
                price: credit(*price),
            },
            Self::Tiered { tiers } => Self::Tiered {
                tiers: tiers
                    .iter()
                    .map(|t| Tier {
                        up_to: t.up_to,
                        price: credit(t.price),
                    })
                    .collect(),
            },
            Self::Dynamic { markup, fallback } => Self::Dynamic {
                markup: credit(*markup),
                fallback: credit(*fallback),
            },
        }
    }
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
    /// Überschreibt den Umrechnungsfaktor der Position ab `valid_from`
    /// (z.B. jährlich neuer Brennwert × Zustandszahl).
    #[serde(default)]
    pub factor: Option<Decimal>,
}

/// Kostenposition eines Vertrags mit Umrechnungsfaktor Messeinheit →
/// Abrechnungseinheit (z.B. Gas: Brennwert × Zustandszahl, m³ → kWh).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostPosition {
    pub name: String,
    pub conversion_factor: Decimal,
    /// Preisstände, aufsteigend nach `valid_from`
    pub components: Vec<PriceComponent>,
}

impl CostPosition {
    /// Preisstand, der am Tag `date` gilt: der letzte mit `valid_from <= date`.
    #[must_use]
    pub fn component_at(&self, date: NaiveDate) -> Option<(usize, &PriceComponent)> {
        self.components
            .iter()
            .enumerate()
            .rev()
            .find(|(_, c)| c.valid_from <= date)
    }

    /// Umrechnungsfaktor eines Preisstands: eigener Faktor, sonst der der Position.
    #[must_use]
    pub fn factor_of(&self, component: &PriceComponent) -> Decimal {
        component.factor.unwrap_or(self.conversion_factor)
    }
}

/// Art einer Anpassung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjustmentKind {
    /// Einmaliger Bonus (Neukunden-, Sofortbonus)
    Bonus,
    /// Gutschrift
    Credit,
}

impl AdjustmentKind {
    pub const ALL: [Self; 2] = [Self::Bonus, Self::Credit];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bonus => "bonus",
            Self::Credit => "credit",
        }
    }
}

impl std::str::FromStr for AdjustmentKind {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| UnknownVariant(s.to_owned()))
    }
}

/// Bonus oder Gutschrift; ein positiver Betrag senkt die Kosten des
/// Zeitraums, in dem `booked_on` liegt (REQ_TAR_005).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adjustment {
    pub kind: AdjustmentKind,
    pub booked_on: NaiveDate,
    pub amount: Decimal,
}

/// Abschlag mit Gültigkeit und Turnus in Monaten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installment {
    pub amount: Decimal,
    pub valid_from: NaiveDate,
    pub every_months: u8,
}

/// Staffelgrenzen sind aufsteigend und positiv, nur die letzte Stufe ist
/// unbegrenzt (SPEC_TAR_TIERED).
// @impl SPEC_TAR_TIERED
#[must_use]
pub fn tiers_valid(tiers: &[Tier]) -> bool {
    let Some((last, rest)) = tiers.split_last() else {
        return false;
    };
    let mut lower = Decimal::ZERO;
    for tier in rest {
        match tier.up_to {
            Some(up) if up > lower => lower = up,
            _ => return false,
        }
    }
    last.up_to.is_none()
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

/// Fälligkeitstage der Abschläge im Zeitraum: je Abschlagsstand
/// `valid_from + k·Turnus` bis zum nächsten Stand. `installments` müssen
/// nach `valid_from` sortiert sein.
// @impl SPEC_TAR_INSTALLMENT_SETTLEMENT
#[must_use]
pub fn installment_dues(installments: &[Installment], period: Period) -> Vec<(NaiveDate, Decimal)> {
    let mut out = Vec::new();
    for (i, inst) in installments.iter().enumerate() {
        let valid_until = installments
            .get(i + 1)
            .map_or(period.end, |n| n.valid_from.min(period.end));
        let mut due = inst.valid_from;
        while due < valid_until {
            if due >= period.start {
                out.push((due, inst.amount));
            }
            due = due + chrono::Months::new(u32::from(inst.every_months.max(1)));
        }
    }
    out
}

/// Summe der fälligen Abschläge im Zeitraum (siehe [`installment_dues`]).
// @impl SPEC_TAR_INSTALLMENT_SETTLEMENT
#[must_use]
pub fn installments_in(installments: &[Installment], period: Period) -> Decimal {
    installment_dues(installments, period)
        .iter()
        .map(|(_, a)| *a)
        .sum()
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
        suggested_monthly_installment: suggested_installment(projected_cost, 1),
    }
}

/// Vorschlag für einen Abschlag mit Turnus `every_months`, der die
/// prognostizierten Jahreskosten deckt: Jahreskosten · Turnus / 12, auf volle
/// Euro aufgerundet; nie negativ.
// @impl SPEC_TAR_INSTALLMENT_SETTLEMENT
#[must_use]
pub fn suggested_installment(projected_annual: Decimal, every_months: u8) -> Decimal {
    (projected_annual * Decimal::from(every_months.max(1)) / Decimal::from(12))
        .ceil()
        .max(Decimal::ZERO)
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
        assert!(tiers_valid(&tiers));
        assert!(!tiers_valid(&tiers[..1]), "letzte Stufe begrenzt");
        assert!(!tiers_valid(&[]));
        let descending = [
            Tier {
                up_to: Some(dec!(1000)),
                price: dec!(0.30),
            },
            Tier {
                up_to: Some(dec!(500)),
                price: dec!(0.35),
            },
            Tier {
                up_to: None,
                price: dec!(0.40),
            },
        ];
        assert!(!tiers_valid(&descending));
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
        assert_eq!(suggested_installment(dec!(1200.01), 1), dec!(101));
        assert_eq!(suggested_installment(dec!(1200), 3), dec!(300));
        assert_eq!(suggested_installment(dec!(-50), 1), dec!(0));
        let quarterly = [Installment {
            amount: dec!(240),
            valid_from: d(2026, 2, 1),
            every_months: 3,
        }];
        let dues = installment_dues(&quarterly, Period::year(2026));
        assert_eq!(
            dues.iter().map(|(d, _)| *d).collect::<Vec<_>>(),
            [d(2026, 2, 1), d(2026, 5, 1), d(2026, 8, 1), d(2026, 11, 1)]
        );
    }
}
