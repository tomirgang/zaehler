//! Auswertung einer Messstelle, Diagrammdaten und Übersicht
//! (SPEC_ANA_*, SPEC_FC_CURRENT_YEAR, SPEC_UI_DASHBOARD).

use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, Utc};
use rust_decimal::Decimal;
use zaehler_domain::{
    analysis::{
        MonthComparison, PeriodForecast, YearToDate, compare_month, current_billing_year,
        forecast_period, last_complete, projected_cumulative, value_of, year_to_date,
    },
    calendar::Calendar,
    comparison::{Comparison, compare},
    consumption::{ConsumptionError, Interval, intervals},
    forecast::SeasonProfile,
    meter::Direction,
    period::Period,
    reading::Reading as DomainReading,
    series::{MonthValue, covered_until, monthly, months_between, total_in},
};

use crate::{
    AppError, Role, Tenant,
    ids::MeterId,
    model::{Meter, MeterAggregate, Reading, Register},
    port::UnitOfWork,
    usecase::{
        Ports,
        meter::{MeterDetail, last_of},
        use_case,
    },
};

use_case!(
    /// Auswertung einer Messstelle: Monatswerte, Vergleiche, Prognosen.
    MeterAnalysis,
    /// Diagrammdaten einer Messstelle (laufendes Jahr und Vorjahr).
    MeterChartData,
    /// Karten der Übersicht.
    Dashboard,
);

/// Verbrauchsintervalle eines Registers; ein Fehler bedeutet inkonsistente
/// Ablesungen (z.B. sinkender Stand ohne Überlauf).
#[derive(Debug, Clone)]
pub struct RegisterIntervals {
    pub register: Register,
    pub intervals: Result<Vec<Interval>, ConsumptionError>,
}

/// Intervalle je Register. Ablesungen zum selben Zeitpunkt werden nach der
/// Einbaureihenfolge der Geräte sortiert (Endstand alt vor Startstand neu).
// @impl SPEC_ANA_REGISTER_AGG, SPEC_CONS_DEVICE_SWAP
#[must_use]
pub fn register_intervals(agg: &MeterAggregate, readings: &[Reading]) -> Vec<RegisterIntervals> {
    agg.registers
        .iter()
        .map(|register| {
            let mut rows: Vec<(usize, DomainReading)> = readings
                .iter()
                .filter(|r| r.register_id == register.id)
                .filter_map(|r| {
                    let pos = agg.devices.iter().position(|d| d.id == r.device_id)?;
                    let device = &agg.devices[pos];
                    Some((
                        pos,
                        DomainReading {
                            at: r.read_at,
                            value: r.value,
                            device_id: device.id.0,
                            rollover_digits: device.data.rollover_digits,
                        },
                    ))
                })
                .collect();
            rows.sort_by(|a, b| a.1.at.cmp(&b.1.at).then(a.0.cmp(&b.0)));
            let domain: Vec<DomainReading> = rows.into_iter().map(|(_, r)| r).collect();
            RegisterIntervals {
                register: register.clone(),
                intervals: intervals(&domain),
            }
        })
        .collect()
}

/// Intervalle aller Register einer Richtung; fehlerhafte Register liefern
/// keine Intervalle, damit Summen über sie `None` ergeben.
fn side(registers: &[RegisterIntervals], direction: Direction) -> Vec<&[Interval]> {
    registers
        .iter()
        .filter(|r| r.register.data.direction == direction)
        .map(|r| r.intervals.as_deref().unwrap_or(&[]))
        .collect()
}

/// Hauptrichtung: Bezug, ohne Bezugsregister Einspeisung.
fn primary_direction(agg: &MeterAggregate) -> Direction {
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

/// Monatswerte eines Registers.
#[derive(Debug, Clone)]
pub struct RegisterSeries {
    pub register: Register,
    pub months: Vec<MonthValue>,
    /// Ablesungen sind inkonsistent; keine Auswertung möglich.
    pub invalid: bool,
}

/// Bezugsgröße einer Prognose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForecastScope {
    CalendarYear,
    BillingYear,
}

#[derive(Debug, Clone)]
pub struct ScopedForecast {
    pub scope: ForecastScope,
    pub period: Period,
    pub forecast: PeriodForecast,
}

/// Diagrammdaten: Monatswerte des laufenden Jahres und des Vorjahres,
/// kumulierter Verlauf mit Prognose bis Jahresende.
#[derive(Debug, Clone)]
pub struct ChartData {
    pub unit: String,
    pub year: i32,
    /// Monate des laufenden Jahres
    pub months: Vec<Period>,
    pub current: Vec<Option<Decimal>>,
    pub previous: Vec<Option<Decimal>>,
    /// Kumulierter Verbrauch am Monatsende
    pub cumulative: Vec<Option<Decimal>>,
    /// Kumulierte Prognose am Monatsende, beginnend am letzten Messwert
    pub forecast: Vec<Option<Decimal>>,
}

/// Ergebnis von [`MeterAnalysis`].
#[derive(Debug, Clone)]
pub struct Analysis {
    pub detail: MeterDetail,
    pub primary: Direction,
    /// Summe der Bezugsregister je Monat (leer ohne Bezugsregister)
    pub import: Vec<MonthValue>,
    /// Summe der Einspeiseregister je Monat (leer ohne Einspeiseregister)
    pub export: Vec<MonthValue>,
    pub registers: Vec<RegisterSeries>,
    /// Bezugsmonat der Vergleiche
    pub month: Option<Period>,
    /// Wählbare Bezugsmonate (vollständig abgedeckt)
    pub complete_months: Vec<Period>,
    pub comparisons: Vec<MonthComparison>,
    pub year_to_date: Option<YearToDate>,
    pub forecasts: Vec<ScopedForecast>,
    pub chart: ChartData,
}

/// Geladene Daten einer Messstelle mit Zeitzone des Haushalts.
struct Loaded {
    detail: MeterDetail,
    cal: Calendar,
    today: NaiveDate,
}

async fn load(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    id: MeterId,
) -> Result<Loaded, AppError> {
    let household = uow.households().get(t).await?;
    let aggregate = uow.meters().get(t, id).await?.ok_or(AppError::NotFound)?;
    let readings = uow.readings().list_for_meter(t, id).await?;
    let cal = Calendar::new(household.data.timezone);
    Ok(Loaded {
        detail: MeterDetail {
            aggregate,
            readings,
        },
        today: cal.local_date(p.clock.now()),
        cal,
    })
}

/// Prognosen für Kalenderjahr und, falls hinterlegt, Abrechnungsjahr.
// @impl SPEC_FC_CURRENT_YEAR
fn forecasts(
    meter: &Meter,
    regs: &[&[Interval]],
    cal: Calendar,
    today: NaiveDate,
) -> Vec<ScopedForecast> {
    let profile = SeasonProfile::uniform();
    let mut out = vec![{
        let period = Period::year(today.year());
        ScopedForecast {
            scope: ForecastScope::CalendarYear,
            period,
            forecast: forecast_period(regs, cal, period, &profile),
        }
    }];
    if let Some(md) = meter.data.billing_start
        && let Some(period) = current_billing_year(today, u32::from(md.month), u32::from(md.day))
    {
        out.push(ScopedForecast {
            scope: ForecastScope::BillingYear,
            period,
            forecast: forecast_period(regs, cal, period, &profile),
        });
    }
    out
}

/// Monatswerte und kumulierter Verlauf des laufenden Jahres und des Vorjahres.
// @impl SPEC_ANA_CHARTS
fn chart_data(unit: &str, regs: &[&[Interval]], cal: Calendar, today: NaiveDate) -> ChartData {
    let year = today.year();
    let period = Period::year(year);
    let months = months_between(period.start, period.start + Months::new(11));
    let previous_months: Vec<Period> = months.iter().map(|m| m.shift_months(-12)).collect();
    let values = |series: Vec<MonthValue>| series.into_iter().map(|m| m.value).collect();
    let year_start = cal.start_of(period.start);
    let cumulative: Vec<Option<Decimal>> = months
        .iter()
        .map(|m| total_in(regs, year_start, cal.start_of(m.end)))
        .collect();
    let profile = SeasonProfile::uniform();
    let mut forecast = vec![None; months.len()];
    if let PeriodForecast::Projected(f) = forecast_period(regs, cal, period, &profile)
        && let Some(until) = covered_until(regs).map(|t| cal.local_date(t))
    {
        let last_actual = cumulative.iter().rposition(Option::is_some);
        for (i, m) in months.iter().enumerate() {
            if m.end > until {
                forecast[i] = Some(projected_cumulative(&f, &profile, m.end));
            } else if Some(i) == last_actual {
                forecast[i] = cumulative[i];
            }
        }
    }
    ChartData {
        unit: unit.to_owned(),
        year,
        current: values(monthly(regs, &months, cal)),
        previous: values(monthly(regs, &previous_months, cal)),
        months,
        cumulative,
        forecast,
    }
}

impl MeterAnalysis {
    /// `month`: gewünschter Bezugsmonat (ein Tag darin); ohne Angabe oder
    /// wenn nicht vollständig abgedeckt der letzte vollständige Monat.
    // @impl SPEC_ANA_MONTHLY_SERIES, SPEC_ANA_REGISTER_AGG, SPEC_ANA_COMPARE_VIEW, SPEC_FC_CURRENT_YEAR
    pub async fn execute(
        &self,
        t: &Tenant,
        id: MeterId,
        month: Option<NaiveDate>,
    ) -> Result<Analysis, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let Loaded { detail, cal, today } = load(&self.p, uow.as_mut(), t, id).await?;
        drop(uow);
        let agg = &detail.aggregate;
        let per_register = register_intervals(agg, &detail.readings);
        let months = detail
            .readings
            .first()
            .map(|first| months_between(cal.local_date(first.read_at), today))
            .unwrap_or_default();
        let imports = side(&per_register, Direction::Import);
        let exports = side(&per_register, Direction::Export);
        let series_of = |regs: &[&[Interval]]| {
            if regs.is_empty() {
                Vec::new()
            } else {
                monthly(regs, &months, cal)
            }
        };
        let import = series_of(&imports);
        let export = series_of(&exports);
        let primary = primary_direction(agg);
        let (main, main_regs) = match primary {
            Direction::Import => (&import, &imports),
            Direction::Export => (&export, &exports),
        };
        let complete_months: Vec<Period> = main
            .iter()
            .filter(|m| m.value.is_some())
            .map(|m| m.month)
            .collect();
        let selected = month
            .map(Period::month_of)
            .filter(|m| complete_months.contains(m))
            .or_else(|| last_complete(main));
        let registers = per_register
            .iter()
            .map(|r| RegisterSeries {
                register: r.register.clone(),
                months: monthly(&[r.intervals.as_deref().unwrap_or(&[])], &months, cal),
                invalid: r.intervals.is_err(),
            })
            .collect();
        Ok(Analysis {
            comparisons: selected.map(|m| compare_month(main, m)).unwrap_or_default(),
            year_to_date: covered_until(main_regs).and_then(|u| year_to_date(main_regs, cal, u)),
            forecasts: forecasts(&agg.meter, main_regs, cal, today),
            chart: chart_data(&agg.meter.data.unit, main_regs, cal, today),
            month: selected,
            complete_months,
            registers,
            primary,
            import,
            export,
            detail,
        })
    }
}

impl MeterChartData {
    // @impl SPEC_ANA_CHARTS
    pub async fn execute(&self, t: &Tenant, id: MeterId) -> Result<ChartData, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let Loaded { detail, cal, today } = load(&self.p, uow.as_mut(), t, id).await?;
        drop(uow);
        let agg = &detail.aggregate;
        let per_register = register_intervals(agg, &detail.readings);
        let regs = side(&per_register, primary_direction(agg));
        Ok(chart_data(&agg.meter.data.unit, &regs, cal, today))
    }
}

/// Letzter Stand eines Registers auf dem eingebauten Gerät.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastValue {
    pub register: String,
    pub value: Decimal,
    pub decimal_places: u8,
}

/// Karte einer Messstelle in der Übersicht.
#[derive(Debug, Clone)]
pub struct DashboardCard {
    pub meter: Meter,
    pub last_reading_at: Option<DateTime<Utc>>,
    pub last_values: Vec<LastValue>,
    /// Letzter vollständig abgedeckter Monat
    pub last_month: Option<MonthValue>,
    /// Vergleich des letzten vollständigen Monats mit dem Vormonat
    pub previous_month: Option<Comparison>,
    /// Prognose des laufenden Kalenderjahres
    pub forecast: PeriodForecast,
    pub due_at: Option<DateTime<Utc>>,
    pub overdue: bool,
}

// @impl SPEC_UI_DASHBOARD
fn card(detail: &MeterDetail, cal: Calendar, now: DateTime<Utc>) -> DashboardCard {
    let agg = &detail.aggregate;
    let today = cal.local_date(now);
    let last_reading_at = detail.readings.iter().map(|r| r.read_at).max();
    let due_at = last_reading_at
        .zip(agg.meter.data.reading_interval_days)
        .map(|(last, days)| last + Duration::days(i64::from(days)));
    let last_values = agg
        .active_device()
        .map(|device| {
            agg.registers
                .iter()
                .filter_map(|r| {
                    last_of(&detail.readings, r.id, device.id).map(|x| LastValue {
                        register: r.data.code.clone(),
                        value: x.value,
                        decimal_places: device.data.decimal_places,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let per_register = register_intervals(agg, &detail.readings);
    let regs = side(&per_register, primary_direction(agg));
    // 14 Monate genügen für den letzten vollständigen Monat und seinen Vormonat
    let months = months_between(today - Months::new(13), today);
    let series = monthly(&regs, &months, cal);
    let last_month = last_complete(&series).map(|m| MonthValue {
        month: m,
        value: value_of(&series, m),
    });
    let previous_month = last_month.and_then(|lm| {
        let prev = value_of(&series, lm.month.shift_months(-1))?;
        compare(lm.value?, &[prev])
    });
    DashboardCard {
        meter: agg.meter.clone(),
        last_reading_at,
        last_values,
        last_month,
        previous_month,
        forecast: forecast_period(
            &regs,
            cal,
            Period::year(today.year()),
            &SeasonProfile::uniform(),
        ),
        overdue: due_at.is_some_and(|d| d < now),
        due_at,
    }
}

impl Dashboard {
    /// Aktive Messstellen; überfällige zuerst, dann nach Name.
    // @impl SPEC_UI_DASHBOARD
    pub async fn execute(&self, t: &Tenant) -> Result<Vec<DashboardCard>, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let cal = Calendar::new(uow.households().get(t).await?.data.timezone);
        let now = self.p.clock.now();
        let mut cards = Vec::new();
        for summary in uow.meters().list(t).await? {
            if summary.meter.archived {
                continue;
            }
            let id = summary.meter.id;
            let Some(aggregate) = uow.meters().get(t, id).await? else {
                continue;
            };
            let readings = uow.readings().list_for_meter(t, id).await?;
            let detail = MeterDetail {
                aggregate,
                readings,
            };
            cards.push(card(&detail, cal, now));
        }
        cards.sort_by(|a, b| {
            b.overdue.cmp(&a.overdue).then_with(|| {
                a.meter
                    .data
                    .name
                    .to_lowercase()
                    .cmp(&b.meter.data.name.to_lowercase())
            })
        });
        Ok(cards)
    }
}
