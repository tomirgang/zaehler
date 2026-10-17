//! Kosten eines Zeitraums und Kostenprognose (SPEC_TAR_COST_CALC,
//! SPEC_TAR_COST_VIEW).
//!
//! Ein Zeitraum wird je Position an den Grenzen der Abrechnungszeiträume, an
//! jedem `valid_from` eines Preisstands und an den Grenzen des gemessenen
//! Abschnitts geteilt. Die Menge eines Teilstücks ist gemessen
//! (SPEC_CONS_INTERPOLATION) oder mit der Rate des gemessenen Anteils am
//! Abrechnungszeitraum und dem Saisonprofil geschätzt (SPEC_FC_PERIOD_FORECAST).
//! Staffeln laufen über die kumulierte Menge ab Beginn des
//! Abrechnungszeitraums; damit summieren sich die Kosten angrenzender
//! Zeiträume zu den Kosten des Gesamtzeitraums.

use chrono::{Datelike, Days, Months, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    calendar::Calendar,
    consumption::Interval,
    forecast::SeasonProfile,
    period::Period,
    series::{covered_from, covered_until},
    tariff::{Adjustment, CostPosition, base_cost},
};

/// Wert mit Kennzeichnung, ob er (teilweise) auf einer Schätzung beruht.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estimated<T> {
    pub value: T,
    pub estimated: bool,
}

/// Mengenquelle: Verbrauchsintervalle der Register einer Position, Zeitzone
/// und Saisonprofil für die Schätzung.
#[derive(Debug, Clone)]
pub struct Quantities {
    registers: Vec<Vec<Interval>>,
    cal: Calendar,
    profile: SeasonProfile,
    measured: Option<Period>,
}

impl Quantities {
    #[must_use]
    pub fn new(registers: Vec<Vec<Interval>>, cal: Calendar, profile: SeasonProfile) -> Self {
        let measured = {
            let refs: Vec<&[Interval]> = registers.iter().map(Vec::as_slice).collect();
            measured_days(&refs, cal)
        };
        Self {
            registers,
            cal,
            profile,
            measured,
        }
    }

    /// Vollständig gemessene Kalendertage `[from, until)`: vom ersten ganzen
    /// Tag nach der ersten gemeinsamen Ablesung bis zum Beginn des Tages der
    /// letzten gemeinsamen Ablesung (wie SPEC_FC_CURRENT_YEAR).
    #[must_use]
    pub fn measured_window(&self) -> Option<Period> {
        self.measured
    }

    fn measured_in(&self, from: NaiveDate, until: NaiveDate) -> Option<Decimal> {
        self.registers
            .iter()
            .map(|r| local_consumption(r, self.cal, from, until))
            .sum()
    }

    /// Menge je Gewichtseinheit des Saisonprofils: aus dem gemessenen Anteil
    /// am Bezugszeitraum `basis`; liegt darin keine Messung, aus den letzten
    /// bis zu 365 gemessenen Tagen.
    fn rate(&self, basis: Period) -> Option<Decimal> {
        let m = self.measured?;
        let inner = Period {
            start: m.start.max(basis.start),
            end: m.end.min(basis.end),
        };
        let window = if inner.start < inner.end {
            inner
        } else {
            Period::new(m.start.max(m.end - Days::new(365)), m.end)
        };
        let weight = self.profile.weight(window);
        if weight.is_zero() {
            return None;
        }
        Some(self.measured_in(window.start, window.end)? / weight)
    }

    /// Menge im Zeitraum `p`: im gemessenen Abschnitt gemessen, außerhalb mit
    /// der Rate des Bezugszeitraums `basis` geschätzt. `None` ohne Messung.
    // @impl SPEC_TAR_COST_CALC, SPEC_FC_PERIOD_FORECAST
    #[must_use]
    pub fn amount(&self, p: Period, basis: Period) -> Option<Estimated<Decimal>> {
        let m = self.measured?;
        if p.start >= p.end {
            return Some(Estimated {
                value: Decimal::ZERO,
                estimated: false,
            });
        }
        let (a, b) = (p.start.max(m.start), p.end.min(m.end));
        let mut value = Decimal::ZERO;
        if a < b {
            value += self.measured_in(a, b)?;
        }
        let outside: Decimal = [
            Period {
                start: p.start,
                end: p.end.min(m.start),
            },
            Period {
                start: p.start.max(m.end),
                end: p.end,
            },
        ]
        .iter()
        .filter(|x| x.start < x.end)
        .map(|x| self.profile.weight(*x))
        .sum();
        let estimated = a >= b || p.start < m.start || p.end > m.end;
        if estimated {
            value += self.rate(basis)? * outside;
        }
        Some(Estimated { value, estimated })
    }
}

/// Verbrauch der Tage `[from, until)` bei linearer Verteilung auf der
/// Ortszeit des Haushalts: jeder Kalendertag zählt gleich lang, auch an den
/// Tagen der Zeitumstellung (tagesgenaue Abgrenzung wie in Abrechnungen).
/// `None`, wenn die Intervalle die Tage nicht abdecken.
// @impl SPEC_TAR_COST_CALC, SPEC_CONS_INTERPOLATION
fn local_consumption(
    intervals: &[Interval],
    cal: Calendar,
    from: NaiveDate,
    until: NaiveDate,
) -> Option<Decimal> {
    let (first, last) = (intervals.first()?, intervals.last()?);
    let (from, until) = (from.and_hms_opt(0, 0, 0)?, until.and_hms_opt(0, 0, 0)?);
    if from < cal.local(first.start) || until > cal.local(last.end) || from > until {
        return None;
    }
    let mut sum = Decimal::ZERO;
    for iv in intervals {
        let (s, e) = (cal.local(iv.start), cal.local(iv.end));
        let len = (e - s).num_seconds();
        if len <= 0 {
            // Intervall innerhalb der zurückgestellten Stunde
            if s >= from && s < until && iv.end > iv.start {
                sum += iv.amount;
            }
            continue;
        }
        let overlap = (e.min(until) - s.max(from)).num_seconds();
        if overlap > 0 {
            sum += iv.amount * Decimal::from(overlap) / Decimal::from(len);
        }
    }
    Some(sum)
}

fn measured_days(registers: &[&[Interval]], cal: Calendar) -> Option<Period> {
    if registers.is_empty() {
        return None;
    }
    let first = covered_from(registers)?;
    let last = covered_until(registers)?;
    let first_day = cal.local_date(first);
    let from = if cal.start_of(first_day) == first {
        first_day
    } else {
        first_day + Days::new(1)
    };
    let until = cal.local_date(last);
    (from < until).then(|| Period::new(from, until))
}

/// Laufzeit, Abrechnungszeitraum, Positionen und Anpassungen eines Vertrags.
#[derive(Debug, Clone, Copy)]
pub struct ContractTerms<'a> {
    pub starts_on: NaiveDate,
    /// Letzter Tag (einschließlich); `None` = unbefristet
    pub ends_on: Option<NaiveDate>,
    /// Beginn des Abrechnungszeitraums (Monat, Tag), Dauer 12 Monate
    pub billing_start: (u32, u32),
    pub positions: &'a [CostPosition],
    pub adjustments: &'a [Adjustment],
}

impl ContractTerms<'_> {
    /// Ende der Laufzeit (ausschließlich).
    #[must_use]
    pub fn end(&self) -> Option<NaiveDate> {
        self.ends_on.map(|d| d + Days::new(1))
    }

    /// Laufzeit geschnitten mit `p`; `None`, wenn leer.
    #[must_use]
    pub fn clip(&self, p: Period) -> Option<Period> {
        let start = p.start.max(self.starts_on);
        let end = self.end().map_or(p.end, |e| e.min(p.end));
        (start < end).then(|| Period::new(start, end))
    }

    /// Abrechnungszeitraum, der `date` enthält (12 Monate ab dem Starttag).
    #[must_use]
    pub fn billing_year(&self, date: NaiveDate) -> Option<Period> {
        let (month, day) = self.billing_start;
        let this = NaiveDate::from_ymd_opt(date.year(), month, day)?;
        let start = if this <= date {
            this
        } else {
            NaiveDate::from_ymd_opt(date.year() - 1, month, day)?
        };
        Some(Period::billing_year(start))
    }

    /// Abrechnungszeiträume (geschnitten mit der Laufzeit), die `range`
    /// überschneiden, aufsteigend.
    #[must_use]
    pub fn billing_periods(&self, range: Period) -> Vec<Period> {
        let Some(clipped) = self.clip(range) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut year = self.billing_year(clipped.start);
        while let Some(y) = year {
            if y.start >= clipped.end {
                break;
            }
            if let Some(p) = self.clip(y) {
                out.push(p);
            }
            year = y
                .start
                .checked_add_months(Months::new(12))
                .map(Period::billing_year);
        }
        out
    }
}

/// Kosten eines Teilstücks: eine Position mit einem Preisstand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentCost {
    /// Index der Position im Vertrag
    pub position: usize,
    /// Index des Preisstands in der Position
    pub component: usize,
    pub period: Period,
    /// Menge in der Einheit der Messstelle
    pub quantity: Decimal,
    /// Umrechnungsfaktor des Preisstands
    pub factor: Decimal,
    /// Anteiliger Grundpreis
    pub base: Decimal,
    /// Arbeitspreis für `quantity · factor`
    pub work: Decimal,
    /// Menge geschätzt (vor der ersten oder nach der letzten Ablesung)
    pub estimated: bool,
}

impl SegmentCost {
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.base + self.work
    }
}

/// Kosten eines Zeitraums, aufgeteilt nach Position und Preisstand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub range: Period,
    pub segments: Vec<SegmentCost>,
    /// Summe der Boni und Gutschriften mit Buchungstag im Zeitraum
    pub adjustments: Decimal,
}

impl CostBreakdown {
    fn sum(&self, f: impl Fn(&SegmentCost) -> bool) -> Decimal {
        self.segments
            .iter()
            .filter(|s| f(s))
            .map(SegmentCost::total)
            .sum()
    }

    /// Kosten der gemessenen Teilstücke.
    #[must_use]
    pub fn measured(&self) -> Decimal {
        self.sum(|s| !s.estimated)
    }

    /// Kosten der geschätzten und prognostizierten Teilstücke.
    #[must_use]
    pub fn estimated(&self) -> Decimal {
        self.sum(|s| s.estimated)
    }

    /// Kosten ohne Anpassungen.
    #[must_use]
    pub fn gross(&self) -> Decimal {
        self.sum(|_| true)
    }

    /// Kosten abzüglich Boni und Gutschriften.
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.gross() - self.adjustments
    }

    /// Summe der Arbeitspreise.
    #[must_use]
    pub fn work(&self) -> Decimal {
        self.segments.iter().map(|s| s.work).sum()
    }

    /// Summe der Grundpreise.
    #[must_use]
    pub fn base(&self) -> Decimal {
        self.segments.iter().map(|s| s.base).sum()
    }

    /// Kosten einer Position.
    #[must_use]
    pub fn position_total(&self, position: usize) -> Decimal {
        self.sum(|s| s.position == position)
    }

    /// Mindestens ein Teilstück beruht auf einer Schätzung.
    #[must_use]
    pub fn has_estimate(&self) -> bool {
        self.segments.iter().any(|s| s.estimated)
    }

    /// Anteil (0..1) der gemessenen Tage an allen Tagen der Teilstücke.
    #[must_use]
    pub fn coverage(&self) -> Decimal {
        let days = |f: &dyn Fn(&SegmentCost) -> bool| -> i64 {
            self.segments
                .iter()
                .filter(|s| f(s))
                .map(|s| s.period.days())
                .sum()
        };
        let all = days(&|_| true);
        if all == 0 {
            return Decimal::ZERO;
        }
        (Decimal::from(days(&|s| !s.estimated)) / Decimal::from(all)).round_dp(4)
    }

    /// Durchschnittlicher Arbeitspreis je Einheit der Messstelle bei der
    /// Gesamtmenge `quantity` (ohne Grundpreis und Anpassungen,
    /// SPEC_CAR_ENERGY_COST).
    #[must_use]
    pub fn average_unit_price(&self, quantity: Decimal) -> Option<Decimal> {
        (!quantity.is_zero()).then(|| self.work() / quantity)
    }
}

/// Kosten des Vertrags im Zeitraum `range` (SPEC_TAR_COST_CALC):
/// Σ Positionen [ Σ Preisstände (Grundpreis anteilig + Menge · Faktor ·
/// Arbeitspreis) ] − Boni. `quantities[i]` liefert die Mengen der Position
/// `i`. `None`, wenn eine benötigte Menge weder gemessen noch schätzbar ist.
///
/// # Panics
/// Wenn `quantities` weniger Einträge als Positionen hat.
// @impl SPEC_TAR_COST_CALC, SPEC_TAR_TIERED, SPEC_TAR_BASE_PRORATION
#[must_use]
pub fn cost_in(
    terms: &ContractTerms,
    quantities: &[&Quantities],
    range: Period,
) -> Option<CostBreakdown> {
    assert!(quantities.len() >= terms.positions.len());
    let mut segments = Vec::new();
    for bp in terms.billing_periods(range) {
        for (pi, position) in terms.positions.iter().enumerate() {
            let q = quantities[pi];
            let mut bounds = vec![bp.start, bp.end];
            let mut add = |d: NaiveDate| {
                if d > bp.start && d < bp.end {
                    bounds.push(d);
                }
            };
            add(range.start);
            add(range.end);
            if let Some(m) = q.measured_window() {
                add(m.start);
                add(m.end);
            }
            for c in &position.components {
                add(c.valid_from);
            }
            bounds.sort_unstable();
            bounds.dedup();
            let mut cumulative = Decimal::ZERO;
            for w in bounds.windows(2) {
                let part = Period::new(w[0], w[1]);
                if part.start >= range.end {
                    break;
                }
                let Some((ci, component)) = position.component_at(part.start) else {
                    continue;
                };
                let amount = q.amount(part, bp)?;
                let factor = position.factor_of(component);
                let billed = amount.value * factor;
                let work = component.unit_pricing.cost(cumulative, billed);
                cumulative += billed;
                if part.start >= range.start {
                    segments.push(SegmentCost {
                        position: pi,
                        component: ci,
                        period: part,
                        quantity: amount.value,
                        factor,
                        base: base_cost(component.base_price, component.base_interval, part.days()),
                        work,
                        estimated: amount.estimated,
                    });
                }
            }
        }
    }
    let adjustments = terms.clip(range).map_or(Decimal::ZERO, |p| {
        terms
            .adjustments
            .iter()
            .filter(|a| a.booked_on >= p.start && a.booked_on < p.end)
            .map(|a| a.amount)
            .sum()
    });
    Some(CostBreakdown {
        range,
        segments,
        adjustments,
    })
}

/// Menge im Zeitraum innerhalb der Laufzeit, geschätzt je
/// Abrechnungszeitraum wie in [`cost_in`].
#[must_use]
pub fn quantity_in(
    terms: &ContractTerms,
    q: &Quantities,
    range: Period,
) -> Option<Estimated<Decimal>> {
    let mut out = Estimated {
        value: Decimal::ZERO,
        estimated: false,
    };
    for bp in terms.billing_periods(range) {
        let part = Period::new(bp.start.max(range.start), bp.end.min(range.end));
        let a = q.amount(part, bp)?;
        out.value += a.value;
        out.estimated |= a.estimated;
    }
    Some(out)
}

/// Jahresmenge, deren Kosten nach den am Tag `date` gültigen Preisständen
/// dem Jahresbetrag `budget` entsprechen (Ampel mit Abschlagsbasis,
/// SPEC_ANA_TRAFFIC_LIGHT). `positions` enthält je Position ihren Anteil an
/// der Gesamtmenge (1 für Positionen über alle Register). `None`, wenn der
/// Betrag die Grundpreise nicht übersteigt oder die Arbeitspreise ihn nicht
/// erreichen.
// @impl SPEC_ANA_TRAFFIC_LIGHT
#[must_use]
pub fn covered_annual_quantity(
    positions: &[(&CostPosition, Decimal)],
    date: NaiveDate,
    budget: Decimal,
) -> Option<Decimal> {
    let cost = |q: Decimal| -> Decimal {
        positions
            .iter()
            .filter_map(|(p, share)| {
                let (_, c) = p.component_at(date)?;
                Some(
                    base_cost(c.base_price, c.base_interval, 365)
                        + c.unit_pricing
                            .cost(Decimal::ZERO, q * share * p.factor_of(c)),
                )
            })
            .sum()
    };
    if cost(Decimal::ZERO) >= budget {
        return None;
    }
    let mut hi = Decimal::ONE;
    let mut steps = 0;
    while cost(hi) < budget {
        hi *= Decimal::TWO;
        steps += 1;
        if steps > 60 {
            return None;
        }
    }
    let mut lo = Decimal::ZERO;
    for _ in 0..80 {
        let mid = (lo + hi) / Decimal::TWO;
        if cost(mid) < budget {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(hi.round_dp(3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        consumption::intervals,
        reading::Reading,
        tariff::{AdjustmentKind, BasePriceInterval, PriceComponent, Tier, UnitPricing},
    };
    use proptest::prelude::*;
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    fn berlin() -> Calendar {
        Calendar::new(chrono_tz::Europe::Berlin)
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn quantities(points: &[(NaiveDate, Decimal)]) -> Quantities {
        let cal = berlin();
        let rs: Vec<Reading> = points
            .iter()
            .map(|(date, value)| Reading {
                at: cal.start_of(*date),
                value: *value,
                device_id: Uuid::nil(),
                rollover_digits: None,
            })
            .collect();
        Quantities::new(vec![intervals(&rs).unwrap()], cal, SeasonProfile::uniform())
    }

    fn component(
        from: NaiveDate,
        base: Decimal,
        interval: BasePriceInterval,
        price: Decimal,
    ) -> PriceComponent {
        PriceComponent {
            valid_from: from,
            base_price: base,
            base_interval: interval,
            unit_pricing: UnitPricing::Flat { price },
            factor: None,
        }
    }

    fn position(name: &str, factor: Decimal, components: Vec<PriceComponent>) -> CostPosition {
        CostPosition {
            name: name.into(),
            conversion_factor: factor,
            components,
        }
    }

    fn terms<'a>(
        start: NaiveDate,
        billing: (u32, u32),
        positions: &'a [CostPosition],
        adjustments: &'a [Adjustment],
    ) -> ContractTerms<'a> {
        ContractTerms {
            starts_on: start,
            ends_on: None,
            billing_start: billing,
            positions,
            adjustments,
        }
    }

    /// Rechnungsbetrag einer Position auf Cent gerundet (kaufmännisch).
    fn cents(b: &CostBreakdown, position: usize) -> Decimal {
        b.position_total(position)
            .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
    }

    /// Abweichung vom Rechnungsbetrag höchstens 1 Cent je Position.
    fn assert_bill(b: &CostBreakdown, position: usize, bill: Decimal) {
        let diff = (cents(b, position) - bill).abs();
        assert!(
            diff <= dec!(0.01),
            "Position {position}: {} statt {bill}",
            cents(b, position)
        );
    }

    /// Golden-Test Strom mit Preiswechsel zum 01.07.: Arbeitspreis und
    /// Grundpreis tagesgenau je Preisstand, 10 kWh je Tag.
    // @test SPEC_TAR_COST_CALC, SPEC_TAR_BASE_PRORATION
    #[test]
    fn golden_electricity_with_price_change() {
        let q = quantities(&[(d(2025, 1, 1), dec!(10000)), (d(2026, 1, 1), dec!(13650))]);
        let positions = [position(
            "Strom",
            Decimal::ONE,
            vec![
                component(
                    d(2025, 1, 1),
                    dec!(120),
                    BasePriceInterval::Year,
                    dec!(0.30),
                ),
                component(
                    d(2025, 7, 1),
                    dec!(150),
                    BasePriceInterval::Year,
                    dec!(0.35),
                ),
            ],
        )];
        let t = terms(d(2025, 1, 1), (1, 1), &positions, &[]);
        let b = cost_in(&t, &[&q], Period::year(2025)).unwrap();
        assert_eq!(b.segments.len(), 2);
        let h1 = &b.segments[0];
        assert_eq!((h1.period.days(), h1.quantity), (181, dec!(1810)));
        assert_eq!(h1.work, dec!(543));
        assert_eq!(h1.base.round_dp(2), dec!(59.51));
        let h2 = &b.segments[1];
        assert_eq!((h2.period.days(), h2.quantity), (184, dec!(1840)));
        assert_eq!(h2.work, dec!(644));
        assert_eq!(h2.base.round_dp(2), dec!(75.62));
        // Rechnung mit gerundeten Zeilen: 543,00 + 59,51 + 644,00 + 75,62
        assert_bill(&b, 0, dec!(1322.13));
        assert_eq!(cents(&b, 0), dec!(1322.12));
        assert!(!b.has_estimate());
        assert_eq!(b.coverage(), Decimal::ONE);
    }

    /// Golden-Test Gas: m³ mit Brennwert × Zustandszahl in kWh, neuer
    /// Faktor ab dem Preisstand vom 01.10.
    // @test SPEC_TAR_COST_CALC
    #[test]
    fn golden_gas_with_calorific_value() {
        // 1460 m³ in 365 Tagen: 4 m³ je Tag
        let q = quantities(&[(d(2025, 4, 1), dec!(5000)), (d(2026, 4, 1), dec!(6460))]);
        let mut october = component(
            d(2025, 10, 1),
            dec!(15),
            BasePriceInterval::Month,
            dec!(0.11),
        );
        october.factor = Some(dec!(10.30));
        let positions = [position(
            "Gas",
            dec!(10.25),
            vec![
                component(
                    d(2025, 4, 1),
                    dec!(15),
                    BasePriceInterval::Month,
                    dec!(0.12),
                ),
                october,
            ],
        )];
        let t = terms(d(2025, 4, 1), (4, 1), &positions, &[]);
        let b = cost_in(&t, &[&q], Period::new(d(2025, 4, 1), d(2026, 4, 1))).unwrap();
        // 183 Tage × 4 m³ × 10,25 × 0,12 und 182 Tage × 4 m³ × 10,30 × 0,11
        assert_eq!(b.segments[0].work, dec!(900.36));
        assert_eq!(b.segments[1].work, dec!(824.824));
        assert_eq!(b.base().round_dp(2), dec!(180.00));
        // Rechnung: 900,36 + 90,25 + 824,82 + 89,75
        assert_bill(&b, 0, dec!(1905.18));
    }

    /// Golden-Test Wasser: Frischwasser, Schmutzwasser und Zählermiete auf
    /// derselben Menge.
    // @test SPEC_TAR_COST_CALC
    #[test]
    fn golden_water_with_three_positions() {
        let q = quantities(&[(d(2025, 1, 1), dec!(100)), (d(2026, 1, 1), dec!(186.5))]);
        let positions = [
            position(
                "Frischwasser",
                Decimal::ONE,
                vec![component(
                    d(2024, 1, 1),
                    dec!(60),
                    BasePriceInterval::Year,
                    dec!(2.08),
                )],
            ),
            position(
                "Schmutzwasser",
                Decimal::ONE,
                vec![component(
                    d(2024, 1, 1),
                    dec!(0),
                    BasePriceInterval::Year,
                    dec!(2.71),
                )],
            ),
            position(
                "Zählermiete",
                Decimal::ONE,
                vec![component(
                    d(2024, 1, 1),
                    dec!(2),
                    BasePriceInterval::Month,
                    dec!(0),
                )],
            ),
        ];
        let t = terms(d(2024, 1, 1), (1, 1), &positions, &[]);
        let b = cost_in(&t, &[&q, &q, &q], Period::year(2025)).unwrap();
        assert_bill(&b, 0, dec!(239.92)); // 86,5 m³ × 2,08 + 60
        assert_bill(&b, 1, dec!(234.42)); // 86,5 m³ × 2,71
        assert_bill(&b, 2, dec!(24.00));
        assert_eq!(b.total(), dec!(498.335));
    }

    /// Staffel über die kumulierte Menge ab Beginn des Abrechnungszeitraums
    /// und Bonus im Zeitraum.
    // @test SPEC_TAR_TIERED, SPEC_TAR_COST_CALC
    #[test]
    fn tiers_accumulate_over_the_billing_period_and_bonus_reduces() {
        let q = quantities(&[(d(2025, 1, 1), dec!(0)), (d(2026, 1, 1), dec!(3650))]);
        let positions = [position(
            "Strom",
            Decimal::ONE,
            vec![PriceComponent {
                valid_from: d(2025, 1, 1),
                base_price: dec!(0),
                base_interval: BasePriceInterval::Year,
                unit_pricing: UnitPricing::Tiered {
                    tiers: vec![
                        Tier {
                            up_to: Some(dec!(2000)),
                            price: dec!(0.30),
                        },
                        Tier {
                            up_to: None,
                            price: dec!(0.40),
                        },
                    ],
                },
                factor: None,
            }],
        )];
        let bonus = [Adjustment {
            kind: AdjustmentKind::Bonus,
            booked_on: d(2025, 3, 1),
            amount: dec!(50),
        }];
        let t = terms(d(2025, 1, 1), (1, 1), &positions, &bonus);
        let year = cost_in(&t, &[&q], Period::year(2025)).unwrap();
        assert_eq!(year.work(), dec!(1260), "2000 × 0,30 + 1650 × 0,40");
        assert_eq!(year.total(), dec!(1210));
        // Dezember allein: kumuliert bereits über der Grenze
        let december = cost_in(&t, &[&q], Period::month_of(d(2025, 12, 1))).unwrap();
        assert_eq!(december.work(), dec!(124));
        assert_eq!(december.adjustments, dec!(0));
    }

    /// Prognose der restlichen Tage mit der Rate des gemessenen Anteils;
    /// Kennzeichnung und Abdeckungsgrad.
    // @test SPEC_TAR_COST_VIEW, SPEC_FC_PERIOD_FORECAST
    #[test]
    fn forecast_marks_projected_segments() {
        // gemessen 01.01. bis 01.04. (90 Tage, 10 je Tag)
        let q = quantities(&[(d(2026, 1, 1), dec!(0)), (d(2026, 4, 1), dec!(900))]);
        let positions = [position(
            "Strom",
            Decimal::ONE,
            vec![component(
                d(2026, 1, 1),
                dec!(365),
                BasePriceInterval::Year,
                dec!(0.5),
            )],
        )];
        let t = terms(d(2026, 1, 1), (1, 1), &positions, &[]);
        let b = cost_in(&t, &[&q], Period::year(2026)).unwrap();
        assert_eq!(b.measured(), dec!(450) + dec!(90));
        assert_eq!(b.estimated(), dec!(1375) + dec!(275));
        assert_eq!(b.total(), dec!(2190));
        assert_eq!(b.coverage(), dec!(0.2466));
        let total = quantity_in(&t, &q, Period::year(2026)).unwrap();
        assert_eq!(total.value, dec!(3650));
        assert!(total.estimated);
        assert_eq!(b.average_unit_price(total.value), Some(dec!(0.5)));
        // ohne Messung im Abrechnungszeitraum: Rate der letzten Messung
        let next = cost_in(&t, &[&q], Period::year(2027)).unwrap();
        assert_eq!(next.work(), dec!(1825));
        // ohne Ablesungen keine Kosten
        let none = quantities(&[(d(2026, 1, 1), dec!(0))]);
        assert!(cost_in(&t, &[&none], Period::year(2026)).is_none());
    }

    /// Laufzeit und Abrechnungszeiträume mit abweichendem Starttag.
    #[test]
    fn billing_periods_are_clipped_to_the_term() {
        let positions = [];
        let t = ContractTerms {
            starts_on: d(2025, 3, 15),
            ends_on: Some(d(2026, 6, 30)),
            billing_start: (4, 1),
            positions: &positions,
            adjustments: &[],
        };
        assert_eq!(
            t.billing_periods(Period::new(d(2025, 1, 1), d(2027, 1, 1))),
            [
                Period::new(d(2025, 3, 15), d(2025, 4, 1)),
                Period::new(d(2025, 4, 1), d(2026, 4, 1)),
                Period::new(d(2026, 4, 1), d(2026, 7, 1)),
            ]
        );
        assert!(t.billing_periods(Period::year(2024)).is_empty());
    }

    // @test SPEC_ANA_TRAFFIC_LIGHT
    #[test]
    fn covered_quantity_inverts_the_annual_cost() {
        let strom = position(
            "Strom",
            Decimal::ONE,
            vec![component(
                d(2026, 1, 1),
                dec!(120),
                BasePriceInterval::Year,
                dec!(0.30),
            )],
        );
        // 12 × 100 € - 120 € Grundpreis = 1080 € / 0,30 = 3600 kWh
        let q = covered_annual_quantity(&[(&strom, Decimal::ONE)], d(2026, 5, 1), dec!(1200));
        assert_eq!(q, Some(dec!(3600)));
        assert_eq!(
            covered_annual_quantity(&[(&strom, Decimal::ONE)], d(2026, 5, 1), dec!(100)),
            None
        );
        assert_eq!(
            covered_annual_quantity(&[(&strom, Decimal::ONE)], d(2025, 5, 1), dec!(1200)),
            None,
            "kein Preisstand"
        );
    }

    proptest! {
        /// Kosten zweier angrenzender Zeiträume = Kosten des Gesamtzeitraums,
        /// auch mit Staffel, Preiswechsel und Prognose.
        // @test SPEC_TAR_COST_CALC
        #[test]
        fn adjacent_periods_sum_to_the_whole(
            split in 1_u64..729,
            length in 1_u64..729,
            daily in prop::collection::vec(0_i64..40_000, 2..30),
        ) {
            let start = d(2025, 2, 10);
            let mut points = vec![(start, Decimal::ZERO)];
            let mut value = Decimal::ZERO;
            for (i, milli) in daily.iter().enumerate() {
                value += Decimal::new(*milli, 3) * Decimal::from(20);
                points.push((start + Days::new(20 * (u64::try_from(i).unwrap() + 1)), value));
            }
            let q = quantities(&points);
            let positions = [position(
                "Strom",
                dec!(1.5),
                vec![
                    PriceComponent {
                        valid_from: d(2025, 1, 1),
                        base_price: dec!(9.95),
                        base_interval: BasePriceInterval::Month,
                        unit_pricing: UnitPricing::Tiered {
                            tiers: vec![
                                Tier { up_to: Some(dec!(500)), price: dec!(0.25) },
                                Tier { up_to: None, price: dec!(0.31) },
                            ],
                        },
                        factor: None,
                    },
                    component(d(2025, 8, 15), dec!(140), BasePriceInterval::Year, dec!(0.33)),
                ],
            )];
            let t = terms(d(2025, 1, 1), (4, 1), &positions, &[]);
            let from = d(2025, 1, 1);
            let mid = from + Days::new(split);
            let end = mid + Days::new(length);
            let whole = cost_in(&t, &[&q], Period::new(from, end)).unwrap();
            let a = cost_in(&t, &[&q], Period::new(from, mid)).unwrap();
            let b = cost_in(&t, &[&q], Period::new(mid, end)).unwrap();
            let diff = (a.total() + b.total() - whole.total()).abs();
            prop_assert!(diff < dec!(0.000001), "{} + {} != {}", a.total(), b.total(), whole.total());
        }
    }
}
