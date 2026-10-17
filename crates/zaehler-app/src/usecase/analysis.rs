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
    expectation::{BandSource, DEFAULT_TOLERANCE_PCT, MonthExpectation, Target, expectations},
    forecast::SeasonProfile,
    meter::Direction,
    period::Period,
    reading::Reading as DomainReading,
    schedule::{self, Due},
    series::{
        MonthValue, Window, covered_until, monthly, monthly_split, monthly_within, months_between,
        subtract, total_in,
    },
};

use crate::{
    AppError, Role, Tenant,
    ids::MeterId,
    model::{Contract, Meter, MeterAggregate, Reading, Register},
    port::UnitOfWork,
    usecase::{
        Ports,
        cost::installment_target,
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

/// Monatswerte, Prognosen und Diagrammdaten eines Registers.
#[derive(Debug, Clone)]
pub struct RegisterSeries {
    pub register: Register,
    pub months: Vec<MonthValue>,
    /// Ablesungen sind inkonsistent; keine Auswertung möglich.
    pub invalid: bool,
    /// Prognosen des Registers (SPEC_ANA_REGISTER_CHARTS)
    pub forecasts: Vec<ScopedForecast>,
    /// Monatswerte und Verlauf des Registers (SPEC_ANA_REGISTER_CHARTS)
    pub chart: ChartData,
}

/// Diagrammdaten eines Registers.
#[derive(Debug, Clone)]
pub struct RegisterChart {
    pub register: Register,
    pub chart: ChartData,
}

/// Ergebnis von [`MeterChartData`]: Messstelle gesamt und, bei mehreren
/// Registern, je Register.
#[derive(Debug, Clone)]
pub struct MeterCharts {
    pub main: ChartData,
    /// Leer bei nur einem Register
    pub registers: Vec<RegisterChart>,
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
    /// Gemessen; bei teilweise abgedeckten Monaten der gemessene Anteil
    pub current: Vec<Option<Decimal>>,
    /// Prognose der fehlenden Zeit teilweise abgedeckter Monate
    pub current_projected: Vec<Option<Decimal>>,
    pub previous: Vec<Option<Decimal>>,
    pub previous_projected: Vec<Option<Decimal>>,
    /// Kumulierter Verbrauch am Monatsende; enthält den geschätzten Beginn
    /// bis `estimated_until`
    pub cumulative: Vec<Option<Decimal>>,
    /// Geschätzter kumulierter Verbrauch vor der ersten Ablesung des Jahres,
    /// angeschlossen an den ersten Messwert
    pub estimate: Vec<Option<Decimal>>,
    /// Beginn des gemessenen Teils, wenn der Jahresbeginn geschätzt ist
    pub estimated_until: Option<NaiveDate>,
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
    /// Zwischenzähler mit ihrem Verbrauch im Zuordnungszeitraum
    /// (SPEC_MTR_SUBMETER)
    pub submeters: Vec<SubmeterSeries>,
    /// Bezug ohne Zwischenzähler je Monat; leer ohne Zwischenzähler
    pub import_net: Vec<MonthValue>,
    /// Bezugsmonat und Vergleiche verwenden den Bezug ohne Zwischenzähler
    pub excluding_submeters: bool,
    /// Saisonprofil der Prognosen mit seiner Herkunft (SPEC_ANA_TRAFFIC_LIGHT)
    pub profile: SeasonProfile,
    pub profile_source: ProfileSource,
    /// Erwartung und Ampel je Monat der Hauptreihe (wie `complete_months`)
    pub expectations: Vec<Option<MonthExpectation>>,
    /// Gesamtverbrauch (Bezug + Eigenverbrauch) je Monat bei einem
    /// Hausanschluss mit PV-Erzeugung; leer ohne Erzeugungszähler (REQ_PV_006)
    pub total_consumption: Vec<MonthValue>,
    /// Bezugsmonat, Vergleiche und Ampel verwenden den Gesamtverbrauch
    pub uses_total: bool,
    /// Ampel des Bezugsmonats
    pub month_expectation: Option<MonthExpectation>,
}

/// Herkunft des Saisonprofils.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileSource {
    /// Aus mindestens 12 aufeinanderfolgenden vollständigen Monaten gelernt
    Learned,
    /// Manuell angepasstes Standardprofil der Messstelle
    Custom,
    /// Standardprofil der Art
    Default,
}

impl ProfileSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Learned => "learned",
            Self::Custom => "custom",
            Self::Default => "default",
        }
    }
}

/// Saisonprofil: gelernt, sonst angepasst, sonst Standard der Art
/// (SPEC_ANA_TRAFFIC_LIGHT, SPEC_FC_SEASON_PROFILE).
// @impl SPEC_ANA_TRAFFIC_LIGHT, SPEC_FC_SEASON_PROFILE
#[must_use]
pub fn choose_profile(meter: &Meter, series: &[MonthValue]) -> (SeasonProfile, ProfileSource) {
    if let Some(learned) = SeasonProfile::learn_series(series) {
        (learned, ProfileSource::Learned)
    } else if let Some(custom) = &meter.data.targets.season_weights {
        (custom.clone(), ProfileSource::Custom)
    } else {
        (
            SeasonProfile::default_for(meter.data.kind),
            ProfileSource::Default,
        )
    }
}

/// Jahresziel mit Toleranz (Standard 20 %), falls hinterlegt.
#[must_use]
pub fn target_of(meter: &Meter) -> Option<Target> {
    let t = &meter.data.targets;
    t.annual_target.map(|annual| Target {
        annual,
        tolerance_pct: Decimal::from(t.tolerance_pct.unwrap_or(DEFAULT_TOLERANCE_PCT)),
        source: BandSource::Target,
    })
}

/// Grundlage der Ampel ohne Historie: Jahresziel, sonst die Menge, die der
/// Abschlag des heute gültigen Bezugsvertrags deckt (SPEC_ANA_TRAFFIC_LIGHT).
// @impl SPEC_ANA_TRAFFIC_LIGHT
#[must_use]
pub fn target_for(
    agg: &MeterAggregate,
    contracts: &[Contract],
    today: NaiveDate,
) -> Option<Target> {
    target_of(&agg.meter).or_else(|| installment_target(agg, contracts, today))
}

/// Monatsreihe der Register von der ersten Ablesung bis heute.
fn full_series(
    detail: &MeterDetail,
    regs: &[&[Interval]],
    cal: Calendar,
    today: NaiveDate,
) -> Vec<MonthValue> {
    let months = detail
        .readings
        .first()
        .map(|first| months_between(cal.local_date(first.read_at), today))
        .unwrap_or_default();
    if regs.is_empty() {
        Vec::new()
    } else {
        monthly(regs, &months, cal)
    }
}

/// Monatsverbrauch eines Zwischenzählers, solange er der Messstelle
/// zugeordnet war.
#[derive(Debug, Clone)]
pub struct SubmeterSeries {
    pub meter: Meter,
    pub months: Vec<MonthValue>,
}

/// Zwischenzähler der Messstelle `parent` mit ihrem Bezug je Monat
/// innerhalb der Einbauzeit der zugeordneten Geräte.
// @impl SPEC_MTR_SUBMETER
async fn load_submeters(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    parent: MeterId,
    months: &[Period],
    cal: Calendar,
) -> Result<Vec<SubmeterSeries>, AppError> {
    let mut out = Vec::new();
    for id in uow.meters().submeters_of(t, parent).await? {
        let Some(agg) = uow.meters().get(t, id).await? else {
            continue;
        };
        let windows: Vec<Window> = agg
            .devices
            .iter()
            .filter(|d| d.data.parent_meter_id == Some(parent))
            .map(|d| Window {
                from: d.data.installed_at,
                until: d.removed_at,
            })
            .collect();
        let readings = uow.readings().list_for_meter(t, id).await?;
        let per_register = register_intervals(&agg, &readings);
        let regs = side(&per_register, Direction::Import);
        out.push(SubmeterSeries {
            months: monthly_within(&regs, &windows, months, cal),
            meter: agg.meter,
        });
    }
    Ok(out)
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
    profile: &SeasonProfile,
) -> Vec<ScopedForecast> {
    let mut out = vec![{
        let period = Period::year(today.year());
        ScopedForecast {
            scope: ForecastScope::CalendarYear,
            period,
            forecast: forecast_period(regs, cal, period, profile),
        }
    }];
    if let Some(md) = meter.data.billing_start
        && let Some(period) = current_billing_year(today, u32::from(md.month), u32::from(md.day))
    {
        out.push(ScopedForecast {
            scope: ForecastScope::BillingYear,
            period,
            forecast: forecast_period(regs, cal, period, profile),
        });
    }
    out
}

/// Monatswerte und kumulierter Verlauf des laufenden Jahres und des Vorjahres.
// @impl SPEC_ANA_CHARTS, SPEC_ANA_PARTIAL_MONTHS, SPEC_ANA_CUMULATIVE_ESTIMATE
fn chart_data(
    unit: &str,
    regs: &[&[Interval]],
    cal: Calendar,
    today: NaiveDate,
    profile: &SeasonProfile,
) -> ChartData {
    let year = today.year();
    let period = Period::year(year);
    let months = months_between(period.start, period.start + Months::new(11));
    let previous_months: Vec<Period> = months.iter().map(|m| m.shift_months(-12)).collect();
    let year_start = cal.start_of(period.start);
    let projected = match forecast_period(regs, cal, period, profile) {
        PeriodForecast::Projected(f) => Some(f),
        _ => None,
    };
    // ohne Ablesung am Jahresbeginn: Verbrauch bis `from` geschätzt
    let estimated = projected.as_ref().filter(|f| f.estimated_start()).map(|f| {
        (
            f.observed_from,
            projected_cumulative(f, profile, f.observed_from),
        )
    });
    let cumulative: Vec<Option<Decimal>> = months
        .iter()
        .map(|m| {
            total_in(regs, year_start, cal.start_of(m.end)).or_else(|| {
                let (from, base) = estimated.filter(|(from, _)| m.end >= *from)?;
                total_in(regs, cal.start_of(from), cal.start_of(m.end)).map(|v| base + v)
            })
        })
        .collect();
    let mut forecast = vec![None; months.len()];
    let mut estimate = vec![None; months.len()];
    if let Some(f) = &projected
        && let Some(until) = covered_until(regs).map(|t| cal.local_date(t))
    {
        let last_actual = cumulative.iter().rposition(Option::is_some);
        for (i, m) in months.iter().enumerate() {
            if m.end > until {
                forecast[i] = Some(projected_cumulative(f, profile, m.end));
            } else if Some(i) == last_actual {
                forecast[i] = cumulative[i];
            }
            if f.estimated_start() && m.end <= f.observed_from {
                estimate[i] = Some(projected_cumulative(f, profile, m.end));
            }
        }
        // Schätzung an den ersten Messwert bzw. die Prognose anschließen
        if let Some(last) = estimate.iter().rposition(Option::is_some)
            && cumulative[last].is_none()
            && let Some(next) = estimate.get_mut(last + 1)
        {
            *next = cumulative[last + 1].or(forecast[last + 1]);
        }
    }
    let current = monthly_split(regs, &months, cal);
    let previous = monthly_split(regs, &previous_months, cal);
    ChartData {
        unit: unit.to_owned(),
        year,
        current: current.iter().map(|m| m.measured).collect(),
        current_projected: current.iter().map(|m| m.projected).collect(),
        previous: previous.iter().map(|m| m.measured).collect(),
        previous_projected: previous.iter().map(|m| m.projected).collect(),
        months,
        cumulative,
        estimate,
        estimated_until: estimated.map(|(from, _)| from),
        forecast,
    }
}

impl MeterAnalysis {
    /// `month`: gewünschter Bezugsmonat (ein Tag darin); ohne Angabe oder
    /// wenn nicht vollständig abgedeckt der letzte vollständige Monat.
    // @impl SPEC_ANA_MONTHLY_SERIES, SPEC_ANA_REGISTER_AGG, SPEC_ANA_COMPARE_VIEW, SPEC_FC_CURRENT_YEAR
    #[tracing::instrument(name = "MeterAnalysis", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        id: MeterId,
        month: Option<NaiveDate>,
    ) -> Result<Analysis, AppError> {
        self.execute_with(t, id, month, false).await
    }

    /// Wie [`Self::execute`]; `exclude_submeters` bezieht Bezugsmonat und
    /// Vergleiche auf den Bezug ohne Zwischenzähler (SPEC_MTR_SUBMETER).
    // @impl SPEC_ANA_MONTHLY_SERIES, SPEC_MTR_SUBMETER
    #[tracing::instrument(name = "MeterAnalysis", skip_all)]
    pub async fn execute_with(
        &self,
        t: &Tenant,
        id: MeterId,
        month: Option<NaiveDate>,
        exclude_submeters: bool,
    ) -> Result<Analysis, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let Loaded { detail, cal, today } = load(&self.p, uow.as_mut(), t, id).await?;
        let months = detail
            .readings
            .first()
            .map(|first| months_between(cal.local_date(first.read_at), today))
            .unwrap_or_default();
        let submeters = load_submeters(uow.as_mut(), t, id, &months, cal).await?;
        let total_consumption = super::pv::consumption_series(uow.as_mut(), t, id, cal, &months)
            .await?
            .unwrap_or_default();
        let contracts = uow.contracts().for_meter(t, id).await?;
        drop(uow);
        let agg = &detail.aggregate;
        let per_register = register_intervals(agg, &detail.readings);
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
        let import_net = if submeters.is_empty() || imports.is_empty() {
            Vec::new()
        } else {
            let parts: Vec<Vec<MonthValue>> = submeters.iter().map(|s| s.months.clone()).collect();
            subtract(&import, &parts)
        };
        let excluding_submeters =
            exclude_submeters && !import_net.is_empty() && primary == Direction::Import;
        // Hausanschluss mit PV: Gesamtverbrauch statt Bezug (REQ_PV_006)
        let uses_total =
            !total_consumption.is_empty() && primary == Direction::Import && !excluding_submeters;
        let (main, main_regs) = match primary {
            Direction::Import if uses_total => (&total_consumption, &imports),
            Direction::Import if excluding_submeters => (&import_net, &imports),
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
        let unit = &agg.meter.data.unit;
        let (profile, profile_source) = choose_profile(&agg.meter, main);
        let expectations = expectations(main, &profile, target_for(agg, &contracts, today));
        let registers = per_register
            .iter()
            .map(|r| {
                let own: &[&[Interval]] = &[r.intervals.as_deref().unwrap_or(&[])];
                RegisterSeries {
                    register: r.register.clone(),
                    months: monthly(own, &months, cal),
                    invalid: r.intervals.is_err(),
                    forecasts: forecasts(&agg.meter, own, cal, today, &profile),
                    chart: chart_data(unit, own, cal, today, &profile),
                }
            })
            .collect();
        Ok(Analysis {
            comparisons: selected.map(|m| compare_month(main, m)).unwrap_or_default(),
            year_to_date: covered_until(main_regs).and_then(|u| year_to_date(main_regs, cal, u)),
            forecasts: forecasts(&agg.meter, main_regs, cal, today, &profile),
            chart: chart_data(unit, main_regs, cal, today, &profile),
            month_expectation: selected.and_then(|m| {
                main.iter()
                    .position(|x| x.month == m)
                    .and_then(|i| expectations.get(i).copied().flatten())
            }),
            expectations,
            profile,
            profile_source,
            month: selected,
            complete_months,
            registers,
            primary,
            import,
            export,
            detail,
            submeters,
            import_net,
            excluding_submeters,
            uses_total,
            total_consumption: total_consumption.clone(),
        })
    }
}

impl MeterChartData {
    // @impl SPEC_ANA_CHARTS, SPEC_ANA_REGISTER_CHARTS
    #[tracing::instrument(name = "MeterChartData", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: MeterId) -> Result<MeterCharts, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let Loaded { detail, cal, today } = load(&self.p, uow.as_mut(), t, id).await?;
        drop(uow);
        let agg = &detail.aggregate;
        let unit = &agg.meter.data.unit;
        let per_register = register_intervals(agg, &detail.readings);
        let regs = side(&per_register, primary_direction(agg));
        let (profile, _) = choose_profile(&agg.meter, &full_series(&detail, &regs, cal, today));
        let registers = if per_register.len() > 1 {
            per_register
                .iter()
                .map(|r| RegisterChart {
                    register: r.register.clone(),
                    chart: chart_data(
                        unit,
                        &[r.intervals.as_deref().unwrap_or(&[])],
                        cal,
                        today,
                        &profile,
                    ),
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(MeterCharts {
            main: chart_data(unit, &regs, cal, today, &profile),
            registers,
        })
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
    /// Fälligkeit nach dem Ableseplan (SPEC_RDG_SCHEDULE)
    pub due: Option<Due>,
    /// Ampel des letzten vollständigen Monats (SPEC_ANA_TRAFFIC_LIGHT)
    pub traffic: Option<MonthExpectation>,
}

impl DashboardCard {
    #[must_use]
    pub fn overdue(&self) -> bool {
        matches!(self.due, Some(Due::Overdue(_)))
    }
}

// @impl SPEC_UI_DASHBOARD
fn card(
    detail: &MeterDetail,
    contracts: &[Contract],
    cal: Calendar,
    now: DateTime<Utc>,
    total: Option<Vec<MonthValue>>,
) -> DashboardCard {
    let agg = &detail.aggregate;
    let today = cal.local_date(now);
    let last_reading_at = detail.readings.iter().map(|r| r.read_at).max();
    let due = agg
        .meter
        .data
        .schedule
        .filter(|_| !agg.meter.archived)
        .map(|s| schedule::due(s, last_reading_at.map(|t| cal.local_date(t)), today));
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
    // ganze Reihe: Saisonprofil und Ampel brauchen die Vorjahre
    // Hausanschluss mit PV: Ampel und Vergleich auf den Gesamtverbrauch
    // (REQ_PV_006)
    let series = total.unwrap_or_else(|| full_series(detail, &regs, cal, today));
    let (profile, _) = choose_profile(&agg.meter, &series);
    let traffic = last_complete(&series).and_then(|m| {
        let i = series.iter().position(|x| x.month == m)?;
        expectations(&series, &profile, target_for(agg, contracts, today))
            .get(i)
            .copied()
            .flatten()
    });
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
        forecast: forecast_period(&regs, cal, Period::year(today.year()), &profile),
        due,
        traffic,
    }
}

impl Dashboard {
    /// Aktive Messstellen; überfällige zuerst, dann fällige, dann nach Name.
    // @impl SPEC_UI_DASHBOARD, SPEC_RDG_SCHEDULE
    #[tracing::instrument(name = "Dashboard", skip_all)]
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
            let contracts = uow.contracts().for_meter(t, id).await?;
            let detail = MeterDetail {
                aggregate,
                readings,
            };
            let months = detail
                .readings
                .first()
                .map(|f| months_between(cal.local_date(f.read_at), cal.local_date(now)))
                .unwrap_or_default();
            let total = super::pv::consumption_series(uow.as_mut(), t, id, cal, &months).await?;
            cards.push(card(&detail, &contracts, cal, now, total));
        }
        let urgency = |c: &DashboardCard| match c.due {
            Some(Due::Overdue(_)) => 0,
            Some(Due::Due(_)) => 1,
            _ => 2,
        };
        cards.sort_by(|a, b| {
            urgency(a).cmp(&urgency(b)).then_with(|| {
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

/// Ergebnis der Alarmprüfung einer Ablesung (SPEC_NTF_ALARMS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmFinding {
    pub from: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub consumption: Decimal,
    /// Erwartung des Zeitraums, aus den Monatsbändern anteilig berechnet
    pub band: zaehler_domain::expectation::Band,
}

/// Bewertet den Verbrauch seit der vorherigen Ablesung desselben Registers
/// gegen die saisonbereinigte Erwartung des Zeitraums: je Monat das Band aus
/// `expectations`, anteilig nach dem Gewicht des Saisonprofils. Rot ergibt
/// einen Befund; Intervalle unter 24 h und Monate ohne Band werden nicht
/// bewertet.
// @impl SPEC_NTF_ALARMS, SPEC_ANA_TRAFFIC_LIGHT
#[must_use]
pub fn check_alarm(
    detail: &MeterDetail,
    contracts: &[Contract],
    cal: Calendar,
    today: NaiveDate,
    reading: crate::ids::ReadingId,
) -> Option<AlarmFinding> {
    let agg = &detail.aggregate;
    let r = detail.readings.iter().find(|x| x.id == reading)?;
    let primary = primary_direction(agg);
    if agg.register(r.register_id)?.data.direction != primary {
        return None;
    }
    let from = detail
        .readings
        .iter()
        .filter(|x| {
            x.register_id == r.register_id && x.device_id == r.device_id && x.read_at < r.read_at
        })
        .map(|x| x.read_at)
        .max()?;
    if r.read_at - from < Duration::hours(24) {
        return None;
    }
    let per_register = register_intervals(agg, &detail.readings);
    let regs = side(&per_register, primary);
    let consumption = total_in(&regs, from, r.read_at)?;
    let series = full_series(detail, &regs, cal, today);
    let (profile, _) = choose_profile(&agg.meter, &series);
    let bands = expectations(&series, &profile, target_for(agg, contracts, today));
    let window = Period::new(cal.local_date(from), cal.local_date(r.read_at));
    let (mut expected, mut lower, mut upper) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for (m, e) in series.iter().zip(&bands) {
        let part = Period::new(window.start.max(m.month.start), window.end.min(m.month.end));
        if part.start >= part.end {
            continue;
        }
        let e = (*e)?;
        let month_weight = profile.weight(m.month);
        if month_weight.is_zero() {
            continue;
        }
        let share = profile.weight(part) / month_weight;
        expected += e.band.expected * share;
        lower += e.band.lower * share;
        upper += e.band.upper * share;
    }
    let band = zaehler_domain::expectation::Band {
        expected: expected.round_dp(3),
        lower: lower.round_dp(3),
        upper: upper.round_dp(3),
    };
    (band.classify(consumption) == zaehler_domain::expectation::Status::Red).then_some(
        AlarmFinding {
            from,
            until: r.read_at,
            consumption,
            band,
        },
    )
}

/// Messstelle mit Ablesungen und Kalender für Jobs (Job-Tenant).
pub(crate) async fn load_for_job(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    id: MeterId,
) -> Result<(MeterDetail, Vec<Contract>, Calendar), AppError> {
    let household = uow.households().get(t).await?;
    let aggregate = uow.meters().get(t, id).await?.ok_or(AppError::NotFound)?;
    let readings = uow.readings().list_for_meter(t, id).await?;
    let contracts = uow.contracts().for_meter(t, id).await?;
    Ok((
        MeterDetail {
            aggregate,
            readings,
        },
        contracts,
        Calendar::new(household.data.timezone),
    ))
}
