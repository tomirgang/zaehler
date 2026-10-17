//! Kostenansicht je Messstelle und Haushalt, Kostenprognose, Abschläge und
//! Abrechnungsvergleich (SPEC_TAR_COST_VIEW, SPEC_TAR_INSTALLMENT_SETTLEMENT,
//! SPEC_TAR_ACTUAL_SETTLEMENT, SPEC_TAR_SUBMETER_COST).

use chrono::{Datelike, Months, NaiveDate};
use rust_decimal::Decimal;
use zaehler_domain::{
    calendar::Calendar,
    consumption::Interval,
    cost::{CostBreakdown, Estimated, Quantities, cost_in, covered_annual_quantity, quantity_in},
    expectation::{BandSource, DEFAULT_TOLERANCE_PCT, Target},
    forecast::SeasonProfile,
    meter::Direction,
    period::Period,
    series::{monthly, months_between},
    tariff::{self, CostPosition, installment_dues, suggested_installment},
};

use crate::{
    AppError, Role, Tenant,
    ids::{ContractId, MeterId},
    model::{Contract, InstallmentRow, MeterAggregate, Reading, SettlementRow},
    port::UnitOfWork,
    usecase::{
        Ports,
        analysis::{RegisterIntervals, choose_profile, register_intervals},
        use_case,
    },
};

use_case!(
    /// Kostenansicht einer Messstelle (Viewer).
    MeterCosts,
    /// Kosten aller Messstellen des Haushalts im Kalenderjahr (Viewer).
    HouseholdCosts,
);

/// Monate Vorlauf der Prognosegüte vor dem Ende des abgerechneten Zeitraums.
pub const FORECAST_LEAD_MONTHS: u32 = 6;

/// Vertrag mit Positionen im Domain-Modell und den Mengen je Position.
struct Priced {
    contract: Contract,
    positions: Vec<CostPosition>,
    adjustments: Vec<tariff::Adjustment>,
    quantities: Vec<Quantities>,
    /// Alle Register der Richtung (Gesamtmenge, durchschnittlicher Preis)
    total: Quantities,
}

impl Priced {
    fn new(
        contract: Contract,
        per_register: &[RegisterIntervals],
        cal: Calendar,
        profile: &SeasonProfile,
    ) -> Self {
        let direction = contract.data.direction;
        let source = |filter: &dyn Fn(&RegisterIntervals) -> bool| {
            Quantities::new(
                per_register
                    .iter()
                    .filter(|r| r.register.data.direction == direction && filter(r))
                    .map(|r| r.intervals.clone().unwrap_or_default())
                    .collect(),
                cal,
                profile.clone(),
            )
        };
        let quantities = contract
            .data
            .positions
            .iter()
            .map(|p| match p.register_id {
                Some(id) => source(&|r| r.register.id == id),
                None => source(&|_| true),
            })
            .collect();
        Self {
            positions: contract.cost_positions(),
            adjustments: contract.adjustment_values(),
            total: source(&|_| true),
            quantities,
            contract,
        }
    }

    fn terms(&self) -> zaehler_domain::cost::ContractTerms<'_> {
        self.contract.terms(&self.positions, &self.adjustments)
    }

    fn cost(&self, range: Period) -> Option<CostBreakdown> {
        let q: Vec<&Quantities> = self.quantities.iter().collect();
        cost_in(&self.terms(), &q, range)
    }

    fn quantity(&self, range: Period) -> Option<Estimated<Decimal>> {
        quantity_in(&self.terms(), &self.total, range)
    }

    /// Laufender Abrechnungszeitraum am Tag `today` innerhalb der Laufzeit;
    /// nach dem Vertragsende der letzte, vor dem Beginn der erste.
    fn current_period(&self, today: NaiveDate) -> Option<Period> {
        let t = self.terms();
        let v = self.contract.validity();
        let date = today.clamp(v.start, v.end.pred_opt().unwrap_or(v.start));
        t.billing_year(date).and_then(|y| t.clip(y))
    }
}

/// Abschläge im Abrechnungszeitraum und erwartete Abrechnung
/// (SPEC_TAR_INSTALLMENT_SETTLEMENT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallmentStatus {
    /// Abschlagsstand, der heute gilt
    pub current: Option<InstallmentRow>,
    /// Fällig bis heute
    pub paid: Decimal,
    /// Noch fällig im Abrechnungszeitraum
    pub open: Decimal,
    /// Prognostizierte Kosten − alle Abschläge; positiv = Nachzahlung
    pub balance: Option<Decimal>,
    /// Abschlag je Turnus, der die prognostizierten Kosten deckt
    pub suggestion: Option<Decimal>,
    /// Turnus des Vorschlags in Monaten
    pub interval: u8,
}

/// Abgerechneter Zeitraum gegen Berechnung und Prognose
/// (SPEC_TAR_ACTUAL_SETTLEMENT).
#[derive(Debug, Clone)]
pub struct SettlementCheck {
    pub contract_id: ContractId,
    pub provider: String,
    pub settlement: SettlementRow,
    /// Berechnet aus allen Ablesungen
    pub calculated: Option<CostBreakdown>,
    /// Abgerechnet − berechnet
    pub difference: Option<Decimal>,
    /// Differenz bezogen auf den abgerechneten Betrag (0..1)
    pub difference_rel: Option<Decimal>,
    /// Stichtag der Prognose (6 Monate vor Ende)
    pub forecast_on: NaiveDate,
    /// Prognose aus den Ablesungen bis zum Stichtag
    pub forecast: Option<Decimal>,
}

/// Kosten eines Vertrags im laufenden Abrechnungszeitraum.
#[derive(Debug, Clone)]
pub struct ContractCost {
    pub contract: Contract,
    pub period: Option<Period>,
    pub breakdown: Option<CostBreakdown>,
    /// Gesamtmenge aller Register der Richtung im Zeitraum
    pub quantity: Option<Estimated<Decimal>>,
    pub installments: Option<InstallmentStatus>,
}

impl ContractCost {
    /// Durchschnittlicher Arbeitspreis je Einheit (SPEC_CAR_ENERGY_COST).
    #[must_use]
    pub fn unit_price(&self) -> Option<Decimal> {
        self.breakdown
            .as_ref()?
            .average_unit_price(self.quantity?.value)
    }
}

/// Summe über alle Verträge einer Richtung in einem Zeitraum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodCost {
    pub period: Period,
    pub measured: Decimal,
    pub estimated: Decimal,
    pub adjustments: Decimal,
    /// Gemessene Tage / Tage mit Vertrag
    pub coverage: Decimal,
    /// Mindestens ein Tag des Zeitraums ohne Vertrag
    pub without_contract: bool,
}

impl PeriodCost {
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.measured + self.estimated - self.adjustments
    }

    #[must_use]
    pub fn has_estimate(&self) -> bool {
        !self.estimated.is_zero() || self.coverage < Decimal::ONE
    }
}

/// Kosten eines Monats für das Diagramm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonthCost {
    pub month: Period,
    /// Bezug gemessen
    pub measured: Decimal,
    /// Bezug geschätzt oder prognostiziert
    pub estimated: Decimal,
    /// Einspeisung (negativ = Gutschrift)
    pub export: Decimal,
}

/// Anteil eines Zwischenzählers an den Kosten der übergeordneten Messstelle
/// (SPEC_TAR_SUBMETER_COST).
#[derive(Debug, Clone)]
pub struct SubmeterShare {
    pub meter_id: MeterId,
    pub name: String,
    pub period: Period,
    pub quantity: Estimated<Decimal>,
    pub unit_price: Decimal,
}

impl SubmeterShare {
    #[must_use]
    pub fn cost(&self) -> Decimal {
        self.quantity.value * self.unit_price
    }
}

/// Ergebnis von [`MeterCosts`].
#[derive(Debug, Clone)]
pub struct MeterCostView {
    pub meter: MeterAggregate,
    pub today: NaiveDate,
    /// Alle Verträge der Messstelle
    pub contracts: Vec<Contract>,
    /// Bezugsvertrag (heute gültig, sonst zuletzt beendet)
    pub import: Option<ContractCost>,
    /// Einspeisevertrag (Gutschrift)
    pub export: Option<ContractCost>,
    /// Laufendes Kalenderjahr, alle Verträge
    pub year_import: Option<PeriodCost>,
    pub year_export: Option<PeriodCost>,
    /// Monate des Kalenderjahres für das Diagramm
    pub months: Vec<MonthCost>,
    /// Anteile der Zwischenzähler im Abrechnungszeitraum des Bezugsvertrags
    pub submeters: Vec<SubmeterShare>,
    /// Kosten als Zwischenzähler über die übergeordnete Messstelle
    pub parent_share: Option<(String, SubmeterShare)>,
    pub settlements: Vec<SettlementCheck>,
}

/// Messstelle mit Verträgen, bereit zur Kostenrechnung.
struct MeterPricing {
    agg: MeterAggregate,
    readings: Vec<Reading>,
    per_register: Vec<RegisterIntervals>,
    profile: SeasonProfile,
    priced: Vec<Priced>,
}

/// Bezugsregister, ohne Bezugsregister Einspeisung (wie die Auswertung).
fn primary(agg: &MeterAggregate) -> Direction {
    if agg
        .registers
        .iter()
        .any(|r| r.data.direction == Direction::Import)
    {
        Direction::Import
    } else {
        Direction::Export
    }
}

/// Saisonprofil aus der Monatsreihe der Hauptrichtung.
fn profile_for(
    agg: &MeterAggregate,
    readings: &[Reading],
    per_register: &[RegisterIntervals],
    cal: Calendar,
    today: NaiveDate,
) -> SeasonProfile {
    let direction = primary(agg);
    let regs: Vec<&[Interval]> = per_register
        .iter()
        .filter(|r| r.register.data.direction == direction)
        .map(|r| r.intervals.as_deref().unwrap_or(&[]))
        .collect();
    let series = match readings.first() {
        Some(first) if !regs.is_empty() => monthly(
            &regs,
            &months_between(cal.local_date(first.read_at), today),
            cal,
        ),
        _ => Vec::new(),
    };
    choose_profile(&agg.meter, &series).0
}

async fn load_pricing(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    id: MeterId,
    cal: Calendar,
    today: NaiveDate,
) -> Result<MeterPricing, AppError> {
    let agg = uow.meters().get(t, id).await?.ok_or(AppError::NotFound)?;
    let readings = uow.readings().list_for_meter(t, id).await?;
    let contracts = uow.contracts().for_meter(t, id).await?;
    Ok(pricing(agg, readings, contracts, cal, today))
}

fn pricing(
    agg: MeterAggregate,
    readings: Vec<Reading>,
    contracts: Vec<Contract>,
    cal: Calendar,
    today: NaiveDate,
) -> MeterPricing {
    let per_register = register_intervals(&agg, &readings);
    let profile = profile_for(&agg, &readings, &per_register, cal, today);
    let priced = contracts
        .into_iter()
        .map(|c| Priced::new(c, &per_register, cal, &profile))
        .collect();
    MeterPricing {
        agg,
        readings,
        per_register,
        profile,
        priced,
    }
}

impl MeterPricing {
    /// Vertrag der Richtung, der heute gilt, sonst der zuletzt begonnene.
    fn current(&self, direction: Direction, today: NaiveDate) -> Option<&Priced> {
        let of: Vec<&Priced> = self
            .priced
            .iter()
            .filter(|p| p.contract.data.direction == direction)
            .collect();
        of.iter()
            .find(|p| p.contract.active_on(today))
            .or_else(|| {
                of.iter()
                    .filter(|p| p.contract.data.starts_on <= today)
                    .max_by_key(|p| p.contract.data.starts_on)
            })
            .copied()
    }

    /// Summe aller Verträge der Richtung im Zeitraum; `None`, wenn kein
    /// Vertrag den Zeitraum berührt oder eine Menge fehlt.
    fn period_cost(&self, direction: Direction, range: Period) -> Option<PeriodCost> {
        let mut out = PeriodCost {
            period: range,
            measured: Decimal::ZERO,
            estimated: Decimal::ZERO,
            adjustments: Decimal::ZERO,
            coverage: Decimal::ZERO,
            without_contract: false,
        };
        let (mut days, mut measured_days, mut contract_days) = (0_i64, 0_i64, 0_i64);
        let mut any = false;
        for p in self
            .priced
            .iter()
            .filter(|p| p.contract.data.direction == direction)
        {
            let Some(clipped) = p.terms().clip(range) else {
                continue;
            };
            any = true;
            contract_days += clipped.days();
            let b = p.cost(range)?;
            out.measured += b.measured();
            out.estimated += b.estimated();
            out.adjustments += b.adjustments;
            for s in &b.segments {
                days += s.period.days();
                if !s.estimated {
                    measured_days += s.period.days();
                }
            }
        }
        if !any {
            return None;
        }
        out.without_contract = contract_days < range.days();
        if days > 0 {
            out.coverage = (Decimal::from(measured_days) / Decimal::from(days)).round_dp(4);
        }
        Some(out)
    }

    fn months(&self, year: i32) -> Vec<MonthCost> {
        let y = Period::year(year);
        months_between(y.start, y.end.pred_opt().unwrap_or(y.start))
            .into_iter()
            .map(|month| {
                let import = self.period_cost(Direction::Import, month);
                let export = self.period_cost(Direction::Export, month);
                MonthCost {
                    month,
                    measured: import.map_or(Decimal::ZERO, |c| c.measured - c.adjustments),
                    estimated: import.map_or(Decimal::ZERO, |c| c.estimated),
                    export: export.map_or(Decimal::ZERO, |c| c.total()),
                }
            })
            .collect()
    }

    /// Menge der Messstelle als Zwischenzähler von `parent` im Zeitraum.
    fn submeter_quantity(
        &self,
        parent: MeterId,
        cal: Calendar,
        range: Period,
    ) -> Option<Estimated<Decimal>> {
        let q = Quantities::new(
            self.per_register
                .iter()
                .filter(|r| r.register.data.direction == Direction::Import)
                .map(|r| r.intervals.clone().unwrap_or_default())
                .collect(),
            cal,
            self.profile.clone(),
        );
        let mut out = Estimated {
            value: Decimal::ZERO,
            estimated: false,
        };
        for d in self
            .agg
            .devices
            .iter()
            .filter(|d| d.data.parent_meter_id == Some(parent))
        {
            let from = cal.local_date(d.data.installed_at).max(range.start);
            let until = d
                .removed_at
                .map_or(range.end, |r| cal.local_date(r).min(range.end));
            if from < until {
                let a = q.amount(Period::new(from, until), range)?;
                out.value += a.value;
                out.estimated |= a.estimated;
            }
        }
        Some(out)
    }
}

fn contract_cost(p: &Priced, today: NaiveDate) -> ContractCost {
    let period = p.current_period(today);
    let breakdown = period.and_then(|r| p.cost(r));
    let quantity = period.and_then(|r| p.quantity(r));
    let installments = period.map(|r| installment_status(p, r, today, breakdown.as_ref()));
    ContractCost {
        contract: p.contract.clone(),
        period,
        breakdown,
        quantity,
        installments,
    }
}

/// Gezahlte und offene Abschläge, Saldo und Vorschlag
/// (SPEC_TAR_INSTALLMENT_SETTLEMENT).
// @impl SPEC_TAR_INSTALLMENT_SETTLEMENT, SPEC_TAR_COST_VIEW
fn installment_status(
    p: &Priced,
    period: Period,
    today: NaiveDate,
    breakdown: Option<&CostBreakdown>,
) -> InstallmentStatus {
    let dues = installment_dues(&p.contract.installment_values(), period);
    let paid: Decimal = dues
        .iter()
        .filter(|(d, _)| *d <= today)
        .map(|(_, a)| *a)
        .sum();
    let open: Decimal = dues
        .iter()
        .filter(|(d, _)| *d > today)
        .map(|(_, a)| *a)
        .sum();
    let current = p.contract.installment_on(today).cloned();
    let interval = current
        .as_ref()
        .or(p.contract.installments.last())
        .map_or(1, |i| i.installment.every_months);
    let projected = breakdown.map(CostBreakdown::total);
    // Vorschlag auf ein volles Jahr bezogen, auch bei kürzerem Zeitraum
    let annual = projected.map(|c| c * Decimal::from(365) / Decimal::from(period.days().max(1)));
    InstallmentStatus {
        current,
        paid,
        open,
        balance: projected.map(|c| c - paid - open),
        suggestion: annual.map(|a| suggested_installment(a, interval)),
        interval,
    }
}

/// Berechnung und Prognose zum Stichtag je erfasster Abrechnung
/// (SPEC_TAR_ACTUAL_SETTLEMENT).
// @impl SPEC_TAR_ACTUAL_SETTLEMENT
fn settlement_checks(m: &MeterPricing, cal: Calendar) -> Vec<SettlementCheck> {
    let mut out = Vec::new();
    for p in &m.priced {
        for s in &p.contract.settlements {
            let period = s.period();
            let calculated = p.cost(period);
            let difference = calculated.as_ref().map(|c| s.amount - c.total());
            let forecast_on = period
                .end
                .checked_sub_months(Months::new(FORECAST_LEAD_MONTHS))
                .unwrap_or(period.start);
            let cutoff = cal.start_of(forecast_on);
            let earlier: Vec<Reading> = m
                .readings
                .iter()
                .filter(|r| r.read_at <= cutoff)
                .cloned()
                .collect();
            let truncated = pricing(
                m.agg.clone(),
                earlier,
                vec![p.contract.clone()],
                cal,
                forecast_on,
            );
            let forecast = truncated
                .priced
                .first()
                .and_then(|x| x.cost(period))
                .map(|b| b.total());
            out.push(SettlementCheck {
                contract_id: p.contract.id,
                provider: p.contract.data.provider.clone(),
                difference_rel: difference
                    .filter(|_| !s.amount.is_zero())
                    .map(|d| (d / s.amount).round_dp(4)),
                difference,
                calculated,
                forecast_on,
                forecast,
                settlement: s.clone(),
            });
        }
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.settlement.period_start));
    out
}

/// Zielmenge der Ampel aus dem Abschlag: Jahresbetrag der Abschläge,
/// umgerechnet mit den heute gültigen Preisständen des Bezugsvertrags
/// (SPEC_ANA_TRAFFIC_LIGHT). Positionen auf einem Register zählen mit dem
/// Anteil 1 / Anzahl der Bezugsregister.
// @impl SPEC_ANA_TRAFFIC_LIGHT
#[must_use]
pub fn installment_target(
    meter: &MeterAggregate,
    contracts: &[Contract],
    today: NaiveDate,
) -> Option<Target> {
    let c = contracts
        .iter()
        .find(|c| c.data.direction == Direction::Import && c.active_on(today))?;
    let inst = &c.installment_on(today)?.installment;
    let budget = inst.amount * Decimal::from(12) / Decimal::from(inst.every_months.max(1));
    let registers = meter
        .registers
        .iter()
        .filter(|r| r.data.direction == Direction::Import)
        .count()
        .max(1);
    let positions = c.cost_positions();
    let shares: Vec<(&CostPosition, Decimal)> = positions
        .iter()
        .zip(&c.data.positions)
        .map(|(p, row)| {
            let share = if row.register_id.is_some() {
                Decimal::ONE / Decimal::from(registers)
            } else {
                Decimal::ONE
            };
            (p, share)
        })
        .collect();
    let annual = covered_annual_quantity(&shares, today, budget)?;
    Some(Target {
        annual,
        tolerance_pct: Decimal::from(
            meter
                .meter
                .data
                .targets
                .tolerance_pct
                .unwrap_or(DEFAULT_TOLERANCE_PCT),
        ),
        source: BandSource::Installment,
    })
}

async fn calendar_today(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
) -> Result<(Calendar, NaiveDate), AppError> {
    let cal = Calendar::new(uow.households().get(t).await?.data.timezone);
    Ok((cal, cal.local_date(p.clock.now())))
}

/// Übergeordnete Messstelle des Geräts, das heute eingebaut ist, sonst
/// des zuletzt ausgebauten.
fn parent_of(agg: &MeterAggregate) -> Option<MeterId> {
    agg.active_device()
        .and_then(|d| d.data.parent_meter_id)
        .or_else(|| {
            agg.devices
                .iter()
                .rev()
                .find_map(|d| d.data.parent_meter_id)
        })
}

impl MeterCosts {
    // @impl SPEC_TAR_COST_VIEW, SPEC_TAR_SUBMETER_COST
    #[tracing::instrument(name = "MeterCosts", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: MeterId) -> Result<MeterCostView, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let (cal, today) = calendar_today(&self.p, uow.as_mut(), t).await?;
        let m = load_pricing(uow.as_mut(), t, id, cal, today).await?;
        let import = m
            .current(Direction::Import, today)
            .map(|p| contract_cost(p, today));
        let export = m
            .current(Direction::Export, today)
            .map(|p| contract_cost(p, today));

        // Anteile der Zwischenzähler am Bezugsvertrag
        let mut submeters = Vec::new();
        if let Some(ic) = &import
            && let (Some(period), Some(price)) = (ic.period, ic.unit_price())
        {
            for sid in uow.meters().submeters_of(t, id).await? {
                let s = load_pricing(uow.as_mut(), t, sid, cal, today).await?;
                if let Some(quantity) = s.submeter_quantity(id, cal, period) {
                    submeters.push(SubmeterShare {
                        meter_id: sid,
                        name: s.agg.meter.data.name.clone(),
                        period,
                        quantity,
                        unit_price: price,
                    });
                }
            }
        }

        // eigene Kosten als Zwischenzähler ohne eigenen Bezugsvertrag
        let mut parent_share = None;
        if import.as_ref().is_none_or(|c| !c.contract.active_on(today))
            && let Some(parent) = parent_of(&m.agg)
        {
            let pm = load_pricing(uow.as_mut(), t, parent, cal, today).await?;
            if let Some(pc) = pm
                .current(Direction::Import, today)
                .map(|p| contract_cost(p, today))
                && let (Some(period), Some(price)) = (pc.period, pc.unit_price())
                && let Some(quantity) = m.submeter_quantity(parent, cal, period)
            {
                parent_share = Some((
                    pm.agg.meter.data.name.clone(),
                    SubmeterShare {
                        meter_id: parent,
                        name: pm.agg.meter.data.name.clone(),
                        period,
                        quantity,
                        unit_price: price,
                    },
                ));
            }
        }
        drop(uow);
        let year = Period::year(today.year());
        Ok(MeterCostView {
            year_import: m.period_cost(Direction::Import, year),
            year_export: m.period_cost(Direction::Export, year),
            months: m.months(today.year()),
            settlements: settlement_checks(&m, cal),
            contracts: m.priced.iter().map(|p| p.contract.clone()).collect(),
            meter: m.agg,
            today,
            import,
            export,
            submeters,
            parent_share,
        })
    }
}

/// Zeile der Haushaltsübersicht.
#[derive(Debug, Clone)]
pub struct HouseholdCostRow {
    pub meter_id: MeterId,
    pub name: String,
    pub unit: String,
    pub import: Option<PeriodCost>,
    pub export: Option<PeriodCost>,
    /// Kosten als Zwischenzähler; in den Kosten der übergeordneten
    /// Messstelle enthalten und nicht Teil der Summe
    pub included_in: Option<(String, Estimated<Decimal>)>,
}

/// Ergebnis von [`HouseholdCosts`].
#[derive(Debug, Clone)]
pub struct HouseholdCostView {
    pub year: i32,
    pub today: NaiveDate,
    pub rows: Vec<HouseholdCostRow>,
    pub months: Vec<MonthCost>,
}

impl HouseholdCostView {
    /// Summe Bezug (ohne Zwischenzähler-Anteile).
    #[must_use]
    pub fn import_total(&self) -> Decimal {
        self.rows
            .iter()
            .filter_map(|r| r.import.map(|c| c.total()))
            .sum()
    }

    /// Summe Einspeisung (negativ = Gutschrift).
    #[must_use]
    pub fn export_total(&self) -> Decimal {
        self.rows
            .iter()
            .filter_map(|r| r.export.map(|c| c.total()))
            .sum()
    }

    #[must_use]
    pub fn has_estimate(&self) -> bool {
        self.rows
            .iter()
            .flat_map(|r| [r.import, r.export])
            .flatten()
            .any(|c| c.has_estimate())
    }
}

impl HouseholdCosts {
    /// Kalenderjahr `year` (ohne Angabe das laufende).
    // @impl SPEC_TAR_COST_VIEW, SPEC_TAR_SUBMETER_COST
    #[tracing::instrument(name = "HouseholdCosts", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        year: Option<i32>,
    ) -> Result<HouseholdCostView, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let (cal, today) = calendar_today(&self.p, uow.as_mut(), t).await?;
        let year = year.unwrap_or_else(|| today.year());
        let range = Period::year(year);
        let mut contracts = uow.contracts().list(t).await?;
        let mut pricings = Vec::new();
        for summary in uow.meters().list(t).await? {
            let id = summary.meter.id;
            let Some(agg) = uow.meters().get(t, id).await? else {
                continue;
            };
            let own: Vec<Contract> = contracts.extract_if(.., |c| c.meter_id == id).collect();
            let has_parent = agg.devices.iter().any(|d| d.data.parent_meter_id.is_some());
            if own.is_empty() && !has_parent {
                continue;
            }
            let readings = uow.readings().list_for_meter(t, id).await?;
            pricings.push(pricing(agg, readings, own, cal, today));
        }
        drop(uow);
        let mut rows = Vec::new();
        let mut months: Vec<MonthCost> = Vec::new();
        for m in &pricings {
            let import = m.period_cost(Direction::Import, range);
            let export = m.period_cost(Direction::Export, range);
            let mut included_in = None;
            if import.is_none()
                && let Some(parent) = parent_of(&m.agg)
                && let Some(pm) = pricings.iter().find(|x| x.agg.meter.id == parent)
                && let Some(cost) = pm.period_cost(Direction::Import, range)
            {
                let q = pm
                    .priced
                    .iter()
                    .filter(|p| p.contract.data.direction == Direction::Import);
                let (mut work, mut total) = (Decimal::ZERO, Decimal::ZERO);
                for p in q {
                    if let (Some(b), Some(qty)) = (p.cost(range), p.quantity(range)) {
                        work += b.work();
                        total += qty.value;
                    }
                }
                if let (false, Some(qty)) =
                    (total.is_zero(), m.submeter_quantity(parent, cal, range))
                {
                    included_in = Some((
                        pm.agg.meter.data.name.clone(),
                        Estimated {
                            value: qty.value * work / total,
                            estimated: qty.estimated || cost.has_estimate(),
                        },
                    ));
                }
            }
            if import.is_none() && export.is_none() && included_in.is_none() {
                continue;
            }
            let own_months = m.months(year);
            if months.is_empty() {
                months = own_months;
            } else {
                for (a, b) in months.iter_mut().zip(own_months) {
                    a.measured += b.measured;
                    a.estimated += b.estimated;
                    a.export += b.export;
                }
            }
            rows.push(HouseholdCostRow {
                meter_id: m.agg.meter.id,
                name: m.agg.meter.data.name.clone(),
                unit: m.agg.meter.data.unit.clone(),
                import,
                export,
                included_in,
            });
        }
        rows.sort_by_key(|r| r.name.to_lowercase());
        Ok(HouseholdCostView {
            year,
            today,
            rows,
            months,
        })
    }
}
