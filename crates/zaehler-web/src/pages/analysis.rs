//! Auswertungsbereich der Detailseite und Diagrammdaten
//! (SPEC_ANA_COMPARE_VIEW, SPEC_FC_CURRENT_YEAR, SPEC_ANA_CHARTS,
//! SPEC_ANA_REGISTER_CHARTS).

use axum::{
    Json,
    extract::{Path, State},
    http::header,
    response::{IntoResponse, Response},
};
use chrono::{Datelike, NaiveDate};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde_json::json;
use zaehler_app::{
    AppError,
    ids::MeterId,
    usecase::analysis::{
        Analysis, ChartData, ForecastScope, MeterCharts, RegisterSeries, ScopedForecast,
    },
};
use zaehler_domain::{
    analysis::{MonthComparison, PeriodForecast, YearToDate},
    comparison::Comparison,
    period::{Period, Reference},
    series::MonthValue,
};

use crate::{extract::Ctx, i18n::Lang, i18n::format, state::WebState};

pub struct MonthRow {
    pub label: String,
    pub import: String,
    pub export: String,
    pub registers: Vec<String>,
}

pub struct MonthChoice {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

pub struct CompareRow {
    pub label: String,
    pub reference: String,
    pub abs: String,
    pub rel: String,
    pub incomplete: bool,
}

pub struct YtdView {
    pub label: String,
    pub current: String,
    pub previous: String,
    pub abs: String,
    pub rel: String,
    pub incomplete: bool,
}

pub struct ForecastView {
    pub label: String,
    pub observed: String,
    pub projected: String,
    pub coverage: String,
    pub hint: Option<String>,
    /// Hinweis auf einen geschätzten Zeitraumbeginn
    pub note: Option<String>,
}

pub struct ChartRow {
    pub label: String,
    pub current: String,
    pub current_projected: String,
    pub previous: String,
    pub previous_projected: String,
    pub cumulative: String,
    pub estimate: String,
    pub forecast: String,
}

/// Spalten und Hinweise der Diagramme, die nur bei Schätzungen erscheinen
/// (SPEC_ANA_PARTIAL_MONTHS, SPEC_ANA_CUMULATIVE_ESTIMATE).
pub struct ChartNotes {
    pub has_projected: bool,
    /// Spaltenköpfe der Prognose für laufendes Jahr und Vorjahr
    pub projected_current: String,
    pub projected_previous: String,
    pub has_estimate: bool,
    pub monthly: Option<String>,
    pub cumulative: Option<String>,
}

/// Prognosen und Diagramme eines Registers (SPEC_ANA_REGISTER_CHARTS).
pub struct RegisterView {
    /// Position in `registers` von `chart.json`
    pub index: usize,
    pub title: String,
    pub forecasts: Vec<ForecastView>,
    pub chart_title_monthly: String,
    pub chart_title_cumulative: String,
    pub chart_rows: Vec<ChartRow>,
    pub chart_notes: ChartNotes,
}

/// View-Modell des Auswertungsbereichs.
pub struct AnalysisView {
    pub unit: String,
    pub primary_label: String,
    pub has_export: bool,
    pub register_codes: Vec<String>,
    pub rows: Vec<MonthRow>,
    pub months: Vec<MonthChoice>,
    /// Überschrift der Vergleiche mit dem Bezugsmonat
    pub compare_title: Option<String>,
    pub comparisons: Vec<CompareRow>,
    pub ytd: Option<YtdView>,
    pub forecasts: Vec<ForecastView>,
    pub invalid: Vec<String>,
    pub chart_title_monthly: String,
    pub chart_title_cumulative: String,
    pub chart_year: i32,
    pub chart_previous_year: i32,
    pub chart_rows: Vec<ChartRow>,
    pub chart_notes: ChartNotes,
    /// Quelle der Monatswerte, nur bei mehreren Registern
    pub chart_source: Option<String>,
    /// Nur bei mehreren Registern, ohne das alleinige Quellregister
    pub register_views: Vec<RegisterView>,
}

fn opt(lang: Lang, v: Option<Decimal>, places: u8) -> String {
    v.map(|v| format::decimal(lang, v, places))
        .unwrap_or_default()
}

fn month_key(p: Period) -> String {
    format!("{:04}-{:02}", p.start.year(), p.start.month())
}

/// `2026-03` als Tag im Monat.
#[must_use]
pub fn parse_month(s: &str) -> Option<NaiveDate> {
    let (y, m) = s.trim().split_once('-')?;
    NaiveDate::from_ymd_opt(y.parse().ok()?, m.parse().ok()?, 1)
}

fn reference_label(lang: Lang, r: Reference) -> String {
    match r {
        Reference::PreviousMonth => lang.t("ref-previous_month"),
        Reference::PreviousYear => lang.t("ref-previous_year"),
        Reference::TrailingAverage(n) => lang.ta("ref-trailing", &[("months", n.to_string())]),
    }
}

fn rel(lang: Lang, c: &Comparison) -> String {
    c.rel_diff_percent
        .map(|p| format!("{} %", format::signed(lang, p, 1)))
        .unwrap_or_default()
}

fn compare_row(lang: Lang, m: &MonthComparison, places: u8) -> CompareRow {
    let label = reference_label(lang, m.reference);
    match &m.comparison {
        Some(c) => CompareRow {
            label,
            reference: format::decimal(lang, c.reference, places),
            abs: format::signed(lang, c.abs_diff, places),
            rel: rel(lang, c),
            incomplete: false,
        },
        None => CompareRow {
            label,
            reference: String::new(),
            abs: String::new(),
            rel: String::new(),
            incomplete: true,
        },
    }
}

fn forecast_view(lang: Lang, f: &ScopedForecast, places: u8) -> ForecastView {
    let label = match f.scope {
        ForecastScope::CalendarYear => lang.ta(
            "forecast-calendar_year",
            &[("year", f.period.start.year().to_string())],
        ),
        ForecastScope::BillingYear => lang.ta(
            "forecast-billing_year",
            &[
                ("from", format::date(lang, f.period.start)),
                (
                    "until",
                    format::date(lang, f.period.end.pred_opt().unwrap_or(f.period.end)),
                ),
            ],
        ),
    };
    match &f.forecast {
        PeriodForecast::Projected(p) => ForecastView {
            label,
            observed: format::decimal(lang, p.observed, places),
            projected: format::decimal(lang, p.projected_total, places),
            coverage: format::percent(lang, p.coverage, 0),
            hint: None,
            note: p.estimated_start().then(|| {
                lang.ta(
                    "forecast-estimated_start",
                    &[("from", format::date(lang, p.observed_from))],
                )
            }),
        },
        PeriodForecast::TooShort => ForecastView {
            label,
            observed: String::new(),
            projected: String::new(),
            coverage: String::new(),
            hint: Some(lang.t("forecast-too_short")),
            note: None,
        },
        PeriodForecast::NoReading => ForecastView {
            label,
            observed: String::new(),
            projected: String::new(),
            coverage: String::new(),
            hint: Some(lang.t("forecast-no_reading")),
            note: None,
        },
    }
}

/// Anzeigewerte; `places` sind die Nachkommastellen des eingebauten Geräts.
// @impl SPEC_ANA_COMPARE_VIEW, SPEC_ANA_REGISTER_AGG, SPEC_FC_CURRENT_YEAR, SPEC_ANA_CHARTS
#[must_use]
pub fn view(lang: Lang, a: &Analysis) -> AnalysisView {
    let agg = &a.detail.aggregate;
    let places = agg.active_device().map_or(1, |d| d.data.decimal_places);
    let main: &[MonthValue] = if a.import.is_empty() {
        &a.export
    } else {
        &a.import
    };
    let has_export = !a.import.is_empty() && !a.export.is_empty();
    let multi = a.registers.len() > 1;
    let rows = month_rows(lang, a, main, places, has_export, multi);
    let ytd = a.year_to_date.as_ref().map(|y| ytd_view(lang, y, places));
    let chart = &a.chart;
    AnalysisView {
        unit: agg.meter.data.unit.clone(),
        primary_label: lang.t(&format!("analysis-{}", a.primary.as_str())),
        has_export,
        register_codes: if multi {
            a.registers
                .iter()
                .map(|r| r.register.data.code.clone())
                .collect()
        } else {
            Vec::new()
        },
        rows,
        months: a
            .complete_months
            .iter()
            .rev()
            .map(|m| MonthChoice {
                value: month_key(*m),
                label: format::month(lang, m.start),
                selected: Some(*m) == a.month,
            })
            .collect(),
        compare_title: a.month.map(|m| {
            lang.ta(
                "analysis-compare-title",
                &[("month", format::month(lang, m.start))],
            )
        }),
        comparisons: a
            .comparisons
            .iter()
            .map(|c| compare_row(lang, c, places))
            .collect(),
        ytd,
        forecasts: a
            .forecasts
            .iter()
            .map(|f| forecast_view(lang, f, places))
            .collect(),
        invalid: a
            .registers
            .iter()
            .filter(|r| r.invalid)
            .map(|r| {
                lang.ta(
                    "analysis-register-invalid",
                    &[("code", r.register.data.code.clone())],
                )
            })
            .collect(),
        chart_title_monthly: monthly_title(lang, chart),
        chart_title_cumulative: lang.ta("chart-cumulative", &[("year", chart.year.to_string())]),
        chart_year: chart.year,
        chart_previous_year: chart.year - 1,
        chart_rows: chart_rows(lang, chart, places),
        chart_notes: chart_notes(lang, chart),
        chart_source: if multi { chart_source(lang, a) } else { None },
        register_views: if multi {
            register_views(lang, a, places)
        } else {
            Vec::new()
        },
    }
}

/// Register, aus denen die Werte der Messstelle stammen (Richtung `primary`).
fn source_registers(a: &Analysis) -> Vec<&RegisterSeries> {
    a.registers
        .iter()
        .filter(|r| r.register.data.direction == a.primary)
        .collect()
}

/// Quelle der Monatswerte der Messstelle: ein Register oder die Summe mehrerer.
fn chart_source(lang: Lang, a: &Analysis) -> Option<String> {
    let sources = source_registers(a);
    let direction = lang.t(&format!("direction-{}", a.primary.as_str()));
    match sources.as_slice() {
        [] => None,
        [r] => Some(lang.ta(
            "chart-source-register",
            &[
                ("code", r.register.data.code.clone()),
                ("direction", direction),
            ],
        )),
        all => Some(
            lang.ta(
                "chart-source-sum",
                &[
                    (
                        "codes",
                        all.iter()
                            .map(|r| r.register.data.code.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    ("direction", direction),
                ],
            ),
        ),
    }
}

/// Prognosen und Diagramme je Register (SPEC_ANA_REGISTER_CHARTS). Ein
/// Register, das allein die Werte der Messstelle liefert, entfällt; die
/// Messstelle nennt es als Quelle (`chart_source`).
fn register_views(lang: Lang, a: &Analysis, places: u8) -> Vec<RegisterView> {
    let sources = source_registers(a);
    let redundant = |r: &RegisterSeries| sources.len() == 1 && std::ptr::eq(sources[0], r);
    a.registers
        .iter()
        .enumerate()
        .filter(|(_, r)| !redundant(r))
        .map(|(index, r)| {
            let args = [
                ("code", r.register.data.code.clone()),
                (
                    "direction",
                    lang.t(&format!("direction-{}", r.register.data.direction.as_str())),
                ),
            ];
            let with = |key: &str, extra: (&str, String)| {
                let mut all = args.to_vec();
                all.push(extra);
                lang.ta(key, &all)
            };
            RegisterView {
                index,
                title: lang.ta("analysis-register-title", &args),
                forecasts: r
                    .forecasts
                    .iter()
                    .map(|f| forecast_view(lang, f, places))
                    .collect(),
                chart_title_monthly: with(
                    "chart-register-monthly",
                    ("year", r.chart.year.to_string()),
                ),
                chart_title_cumulative: with(
                    "chart-register-cumulative",
                    ("year", r.chart.year.to_string()),
                ),
                chart_rows: chart_rows(lang, &r.chart, places),
                chart_notes: chart_notes(lang, &r.chart),
            }
        })
        .collect()
}

fn monthly_title(lang: Lang, chart: &ChartData) -> String {
    lang.ta(
        "chart-monthly",
        &[
            ("year", chart.year.to_string()),
            ("previous", (chart.year - 1).to_string()),
        ],
    )
}

/// Hinweise zu geschätzten Werten der Diagramme.
fn chart_notes(lang: Lang, chart: &ChartData) -> ChartNotes {
    let has_projected = chart
        .current_projected
        .iter()
        .chain(&chart.previous_projected)
        .any(Option::is_some);
    let projected = |year: i32| lang.ta("chart-projected-series", &[("year", year.to_string())]);
    ChartNotes {
        has_projected,
        projected_current: projected(chart.year),
        projected_previous: projected(chart.year - 1),
        has_estimate: chart.estimate.iter().any(Option::is_some),
        monthly: has_projected.then(|| lang.t("chart-projected-note")),
        cumulative: chart
            .estimated_until
            .map(|from| lang.ta("chart-estimate-note", &[("from", format::date(lang, from))])),
    }
}

/// Diagrammwerte als Tabelle (Alternative zu den Diagrammen).
fn chart_rows(lang: Lang, chart: &ChartData, places: u8) -> Vec<ChartRow> {
    chart
        .months
        .iter()
        .enumerate()
        .map(|(i, m)| ChartRow {
            label: format::month_short(lang, m.start),
            current: opt(lang, chart.current[i], places),
            current_projected: opt(lang, chart.current_projected[i], places),
            previous: opt(lang, chart.previous[i], places),
            previous_projected: opt(lang, chart.previous_projected[i], places),
            cumulative: opt(lang, chart.cumulative[i], places),
            estimate: opt(lang, chart.estimate[i], places),
            forecast: opt(lang, chart.forecast[i], places),
        })
        .collect()
}

/// Monatstabelle, neuester Monat zuerst; Spalten je Register nur bei mehreren.
fn month_rows(
    lang: Lang,
    a: &Analysis,
    main: &[MonthValue],
    places: u8,
    has_export: bool,
    multi: bool,
) -> Vec<MonthRow> {
    main.iter()
        .enumerate()
        .rev()
        .map(|(i, m)| MonthRow {
            label: format::month(lang, m.month.start),
            import: opt(lang, m.value, places),
            export: if has_export {
                opt(lang, a.export.get(i).and_then(|e| e.value), places)
            } else {
                String::new()
            },
            registers: if multi {
                a.registers
                    .iter()
                    .map(|r| opt(lang, r.months.get(i).and_then(|v| v.value), places))
                    .collect()
            } else {
                Vec::new()
            },
        })
        .collect()
}

fn ytd_view(lang: Lang, y: &YearToDate, places: u8) -> YtdView {
    YtdView {
        label: lang.ta(
            "analysis-ytd",
            &[
                ("until", format::datetime(lang, y.until)),
                ("previous", format::datetime(lang, y.previous_until)),
            ],
        ),
        current: opt(lang, y.current, places),
        previous: opt(lang, y.previous, places),
        abs: y
            .comparison
            .as_ref()
            .map(|c| format::signed(lang, c.abs_diff, places))
            .unwrap_or_default(),
        rel: y
            .comparison
            .as_ref()
            .map(|c| rel(lang, c))
            .unwrap_or_default(),
        incomplete: y.comparison.is_none(),
    }
}

fn numbers(values: &[Option<Decimal>]) -> Vec<Option<f64>> {
    values
        .iter()
        .map(|v| v.and_then(|d| d.round_dp(3).to_f64()))
        .collect()
}

/// JSON für `charts.js`: Messstelle gesamt, bei mehreren Registern dazu
/// `registers` in der Reihenfolge der Register.
// @impl SPEC_ANA_CHARTS, SPEC_ANA_REGISTER_CHARTS
#[must_use]
pub fn chart_json(lang: Lang, c: &MeterCharts) -> serde_json::Value {
    let mut out = series_json(lang, &c.main);
    out["registers"] = c
        .registers
        .iter()
        .map(|r| {
            let mut v = series_json(lang, &r.chart);
            v["code"] = json!(r.register.data.code);
            v
        })
        .collect();
    out
}

/// Beschriftungen in der Sprache des Nutzers.
fn series_json(lang: Lang, c: &ChartData) -> serde_json::Value {
    json!({
        "unit": c.unit,
        "labels": c.months.iter().map(|m| format::month_short(lang, m.start)).collect::<Vec<_>>(),
        "series": {
            "current": { "name": c.year.to_string(), "data": numbers(&c.current) },
            "current_projected": {
                "name": lang.ta("chart-projected-series", &[("year", c.year.to_string())]),
                "data": numbers(&c.current_projected),
            },
            "previous": { "name": (c.year - 1).to_string(), "data": numbers(&c.previous) },
            "previous_projected": {
                "name": lang.ta("chart-projected-series", &[("year", (c.year - 1).to_string())]),
                "data": numbers(&c.previous_projected),
            },
            "cumulative": { "name": lang.t("chart-cumulative-series"), "data": numbers(&c.cumulative) },
            "estimate": { "name": lang.t("chart-estimate-series"), "data": numbers(&c.estimate) },
            "forecast": { "name": lang.t("chart-forecast-series"), "data": numbers(&c.forecast) },
        },
    })
}

/// `GET /meters/{id}/chart.json`
// @impl SPEC_ANA_CHARTS, SPEC_ANA_PARTIAL_MONTHS, SPEC_ANA_CUMULATIVE_ESTIMATE
pub async fn chart(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id: MeterId = id.parse().map_err(|_| ctx.fail(AppError::NotFound))?;
    let data = state
        .uc
        .meter_chart_data
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(chart_json(ctx.page.lang, &data)),
    )
        .into_response())
}
