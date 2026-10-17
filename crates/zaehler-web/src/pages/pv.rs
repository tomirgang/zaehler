//! Photovoltaik: Anlagen mit Zuordnung, Anlagenbericht mit Erwartung und
//! Wirtschaftlichkeit, Hausanschluss mit Kennzahlen, Sammelerfassung und
//! Diagrammdaten (SPEC_PV_ASSIGNMENT, SPEC_PV_BULK_CAPTURE, SPEC_PV_KPI,
//! SPEC_PV_ECONOMICS, SPEC_PV_CHARTS).

use std::collections::HashMap;

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use zaehler_app::{
    AppError, FieldErrors,
    ids::{DraftId, MeterId, PvSystemId, RegisterId},
    model::PvSystem,
    usecase::pv::{BulkInput, BulkOutcome, GridPeriod, PvReport, PvSystemInput},
};
use zaehler_domain::{
    calendar::Calendar,
    expectation::Status,
    meter::MeterKind,
    pv::{
        DEFAULT_ALARM_THRESHOLD_PCT, DEFAULT_DEGRADATION_PCT, DEFAULT_SPECIFIC_YIELD, Kpi, PvKind,
        PvRole, PvSystemData,
    },
};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format, parse},
    pages::{
        FormData,
        contracts::{date_input, decimal, money, plain, required_date},
        costs::json_response,
        field_errors,
        readings::hint,
        render, render_status, see_other,
        vehicles::required_decimal,
    },
    state::WebState,
};

fn parse_system(id: &str) -> Result<PvSystemId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

fn parse_meter(id: &str) -> Result<MeterId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

fn kwh(lang: Lang, v: Option<Decimal>) -> String {
    v.map(|v| format!("{} kWh", format::decimal(lang, v, 0)))
        .unwrap_or_default()
}

fn pct(lang: Lang, v: Option<Decimal>) -> String {
    v.map(|v| format::percent(lang, v, 0)).unwrap_or_default()
}

fn status_key(s: Option<Status>) -> &'static str {
    match s {
        Some(Status::Green) => "green",
        Some(Status::Yellow) => "yellow",
        Some(Status::Red) => "red",
        None => "",
    }
}

// --- Liste ---------------------------------------------------------------

pub struct SystemRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub kwp: String,
    pub grid_id: String,
    pub grid: String,
}

#[derive(Template)]
#[template(path = "pages/pv/list.html")]
struct ListPage {
    ctx: PageCtx,
    systems: Vec<SystemRow>,
}

async fn meter_names(state: &WebState, ctx: &Ctx) -> Result<HashMap<MeterId, String>, Response> {
    Ok(state
        .uc
        .list_meters
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?
        .into_iter()
        .map(|m| (m.meter.id, m.meter.data.name))
        .collect())
}

// @impl SPEC_PV_ASSIGNMENT
pub async fn list(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    let lang = ctx.page.lang;
    let names = meter_names(&state, &ctx).await?;
    let systems = state
        .uc
        .list_pv_systems
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?
        .into_iter()
        .map(|s| {
            let grid = s.grid();
            SystemRow {
                id: s.id.to_string(),
                kind: lang.t(&format!("pv-kind-{}", s.data.kind.as_str())),
                kwp: format!("{} kWp", plain(lang, s.data.kwp)),
                grid_id: grid.map(|g| g.to_string()).unwrap_or_default(),
                grid: grid
                    .and_then(|g| names.get(&g).cloned())
                    .unwrap_or_default(),
                name: s.data.name,
            }
        })
        .collect();
    Ok(render(&ListPage {
        ctx: ctx.page.clone(),
        systems,
    }))
}

// --- Formular ------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SystemForm {
    pub name: String,
    pub kind: String,
    pub kwp: String,
    pub commissioned_on: String,
    pub azimuth: String,
    pub tilt: String,
    pub inverter: String,
    pub battery_kwh: String,
    pub specific_yield: String,
    pub degradation_pct: String,
    pub alarm_threshold_pct: String,
    pub shares: Vec<String>,
    pub grid: String,
    pub generation: Vec<String>,
    pub battery: Vec<String>,
}

pub struct MeterChoice {
    pub id: String,
    pub name: String,
}

#[derive(Template)]
#[template(path = "pages/pv/form.html")]
struct FormPage {
    ctx: PageCtx,
    system_id: Option<String>,
    kinds: Vec<(String, String)>,
    meters: Vec<MeterChoice>,
    months: Vec<String>,
    form: SystemForm,
    errors: FieldErrors,
    conflict: Option<String>,
}

impl FormPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn has_errs(&self, field: &str) -> bool {
        !self.errors.for_field(field).is_empty()
    }

    fn is_generation(&self, id: &str) -> bool {
        self.form.generation.iter().any(|g| g == id)
    }

    fn is_battery(&self, id: &str) -> bool {
        self.form.battery.iter().any(|g| g == id)
    }

    #[allow(clippy::trivially_copy_pass_by_ref)] // Aufruf aus dem Template mit Referenz
    fn share(&self, i: &usize) -> String {
        self.form.shares.get(*i).cloned().unwrap_or_default()
    }
}

async fn form_page(
    state: &WebState,
    ctx: &Ctx,
    system_id: Option<PvSystemId>,
    form: SystemForm,
    errors: FieldErrors,
    conflict: Option<&'static str>,
) -> Result<FormPage, Response> {
    let lang = ctx.page.lang;
    let meters = state
        .uc
        .list_meters
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?
        .into_iter()
        .filter(|m| m.meter.data.kind == MeterKind::Electricity && m.meter.archived_at.is_none())
        .map(|m| MeterChoice {
            id: m.meter.id.to_string(),
            name: m.meter.data.name,
        })
        .collect();
    Ok(FormPage {
        system_id: system_id.map(|s| s.to_string()),
        kinds: PvKind::ALL
            .iter()
            .map(|k| {
                (
                    k.as_str().to_owned(),
                    lang.t(&format!("pv-kind-{}", k.as_str())),
                )
            })
            .collect(),
        meters,
        months: (1..=12)
            .map(|m| {
                format::month_short(
                    lang,
                    chrono::NaiveDate::from_ymd_opt(2026, m, 1).unwrap_or_default(),
                )
            })
            .collect(),
        form,
        errors,
        conflict: conflict.map(|c| lang.code_text(c)),
        ctx: ctx.page.clone(),
    })
}

fn form_of(lang: Lang, s: &PvSystem) -> SystemForm {
    let d = &s.data;
    let opt = |v: Option<Decimal>| v.map(|v| plain(lang, v)).unwrap_or_default();
    SystemForm {
        name: d.name.clone(),
        kind: d.kind.as_str().into(),
        kwp: plain(lang, d.kwp),
        commissioned_on: date_input(d.commissioned_on),
        azimuth: d.azimuth.map(|v| v.to_string()).unwrap_or_default(),
        tilt: d.tilt.map(|v| v.to_string()).unwrap_or_default(),
        inverter: d.inverter.clone().unwrap_or_default(),
        battery_kwh: opt(d.battery_kwh),
        specific_yield: plain(lang, d.specific_yield),
        degradation_pct: plain(lang, d.degradation_pct),
        alarm_threshold_pct: d.alarm_threshold_pct.to_string(),
        shares: d.month_shares.map_or_else(
            || vec![String::new(); 12],
            |m| m.iter().map(|v| plain(lang, *v)).collect(),
        ),
        grid: s.grid().map(|g| g.to_string()).unwrap_or_default(),
        generation: s
            .with_role(PvRole::Generation)
            .map(|m| m.to_string())
            .collect(),
        battery: s
            .with_role(PvRole::Battery)
            .map(|m| m.to_string())
            .collect(),
    }
}

fn read_form(f: &FormData) -> SystemForm {
    let g = |k: &str| f.get(k).to_owned();
    let ids = |prefix: &str| {
        let mut v: Vec<String> =
            f.0.keys()
                .filter_map(|k| k.strip_prefix(prefix).map(str::to_owned))
                .collect();
        v.sort();
        v
    };
    SystemForm {
        name: g("name"),
        kind: g("kind"),
        kwp: g("kwp"),
        commissioned_on: g("commissioned_on"),
        azimuth: g("azimuth"),
        tilt: g("tilt"),
        inverter: g("inverter"),
        battery_kwh: g("battery_kwh"),
        specific_yield: g("specific_yield"),
        degradation_pct: g("degradation_pct"),
        alarm_threshold_pct: g("alarm_threshold_pct"),
        shares: (1..=12).map(|m| g(&format!("share_{m}"))).collect(),
        grid: g("grid"),
        generation: ids("gen_"),
        battery: ids("bat_"),
    }
}

fn parse_input(e: &mut FieldErrors, f: &SystemForm) -> Option<PvSystemInput> {
    let kind = f
        .kind
        .parse::<PvKind>()
        .map_err(|_| e.add("kind", "form.kind_invalid"))
        .ok();
    let kwp = required_decimal(e, "kwp", &f.kwp);
    let commissioned = required_date(e, "commissioned_on", &f.commissioned_on);
    let small = |e: &mut FieldErrors, field: &str, v: &str| {
        parse::integer::<i16>(v).unwrap_or_else(|_| {
            e.add(field, "form.integer_invalid");
            None
        })
    };
    let azimuth = small(e, "azimuth", &f.azimuth);
    let tilt = small(e, "tilt", &f.tilt);
    let battery = decimal(e, "battery_kwh", &f.battery_kwh);
    let yield_ = decimal(e, "specific_yield", &f.specific_yield).unwrap_or(DEFAULT_SPECIFIC_YIELD);
    let degradation =
        decimal(e, "degradation_pct", &f.degradation_pct).unwrap_or(DEFAULT_DEGRADATION_PCT);
    let threshold = parse::integer::<u8>(&f.alarm_threshold_pct)
        .unwrap_or_else(|_| {
            e.add("alarm_threshold_pct", "pv.threshold_invalid");
            None
        })
        .unwrap_or(DEFAULT_ALARM_THRESHOLD_PCT);
    let shares = if f.shares.iter().all(|s| s.trim().is_empty()) {
        None
    } else {
        let values: Vec<Decimal> = f
            .shares
            .iter()
            .map(|s| decimal(e, "month_shares", s).unwrap_or(Decimal::ZERO))
            .collect();
        <[Decimal; 12]>::try_from(values).ok()
    };
    let mut meters = Vec::new();
    match f.grid.parse::<MeterId>() {
        Ok(g) => meters.push((g, PvRole::Grid)),
        Err(_) => e.add("grid", "pv.grid_required"),
    }
    for (ids, role) in [
        (&f.generation, PvRole::Generation),
        (&f.battery, PvRole::Battery),
    ] {
        for id in ids {
            if let Ok(m) = id.parse::<MeterId>() {
                meters.push((m, role));
            }
        }
    }
    Some(PvSystemInput {
        data: PvSystemData {
            name: f.name.clone(),
            kind: kind?,
            kwp: kwp?,
            commissioned_on: commissioned?,
            azimuth,
            tilt,
            inverter: Some(f.inverter.clone()),
            battery_kwh: battery,
            specific_yield: yield_,
            degradation_pct: degradation,
            month_shares: shares,
            alarm_threshold_pct: threshold,
        },
        meters,
    })
}

fn split_err(err: AppError) -> Result<(FieldErrors, Option<&'static str>), AppError> {
    match err {
        AppError::Validation(e) => Ok((e, None)),
        AppError::Conflict(c) => Ok((FieldErrors::default(), Some(c))),
        other => Err(other),
    }
}

// @impl SPEC_PV_ASSIGNMENT
pub async fn new_form(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    if !ctx.page.is_owner {
        return Err(ctx.fail(AppError::Forbidden));
    }
    let lang = ctx.page.lang;
    let form = SystemForm {
        kind: PvKind::Surplus.as_str().into(),
        commissioned_on: date_input(Calendar::new(ctx.page.tz).local_date(Utc::now())),
        specific_yield: plain(lang, DEFAULT_SPECIFIC_YIELD),
        degradation_pct: plain(lang, DEFAULT_DEGRADATION_PCT),
        alarm_threshold_pct: DEFAULT_ALARM_THRESHOLD_PCT.to_string(),
        shares: vec![String::new(); 12],
        ..SystemForm::default()
    };
    Ok(render(
        &form_page(&state, &ctx, None, form, FieldErrors::default(), None).await?,
    ))
}

// @impl SPEC_PV_ASSIGNMENT
pub async fn create(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let form = read_form(&FormData::new(pairs));
    let mut e = FieldErrors::default();
    let result = match parse_input(&mut e, &form) {
        Some(i) if e.is_empty() => state.uc.create_pv_system.execute(&ctx.tenant, i).await,
        _ => Err(AppError::Validation(e)),
    };
    match result {
        Ok(id) => {
            ctx.flash("flash-pv-created").await;
            Ok(see_other(&format!("/pv/{id}")))
        }
        Err(err) => {
            let (errors, conflict) = split_err(err).map_err(|e| ctx.fail(e))?;
            let page = form_page(&state, &ctx, None, form, errors, conflict).await?;
            Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
        }
    }
}

pub async fn edit_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    if !ctx.page.is_owner {
        return Err(ctx.fail(AppError::Forbidden));
    }
    let id = parse_system(&id).map_err(|e| ctx.fail(e))?;
    let s = state
        .uc
        .get_pv_system
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let form = form_of(ctx.page.lang, &s);
    Ok(render(
        &form_page(&state, &ctx, Some(id), form, FieldErrors::default(), None).await?,
    ))
}

// @impl SPEC_PV_ASSIGNMENT
pub async fn update(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_system(&id).map_err(|e| ctx.fail(e))?;
    let form = read_form(&FormData::new(pairs));
    let mut e = FieldErrors::default();
    let result = match parse_input(&mut e, &form) {
        Some(i) if e.is_empty() => state.uc.update_pv_system.execute(&ctx.tenant, id, i).await,
        _ => Err(AppError::Validation(e)),
    };
    match result {
        Ok(()) => {
            ctx.flash("flash-pv-saved").await;
            Ok(see_other(&format!("/pv/{id}")))
        }
        Err(err) => {
            let (errors, conflict) = split_err(err).map_err(|e| ctx.fail(e))?;
            let page = form_page(&state, &ctx, Some(id), form, errors, conflict).await?;
            Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
        }
    }
}

pub async fn delete(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_system(&id).map_err(|e| ctx.fail(e))?;
    state
        .uc
        .delete_pv_system
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    ctx.flash("flash-pv-deleted").await;
    Ok(see_other("/pv"))
}

// --- Anlagenbericht ------------------------------------------------------

pub struct YieldRow {
    pub month: String,
    pub actual: String,
    pub expected: String,
    pub band: String,
    pub status: &'static str,
    pub status_label: String,
    pub history: bool,
}

pub struct EconomicsRow {
    pub label: String,
    pub generation: String,
    pub feed_in: String,
    pub savings: String,
    pub advantage: String,
    pub costs: String,
    pub balance: String,
    pub cumulative: String,
}

#[derive(Template)]
#[template(path = "pages/pv/detail.html")]
struct DetailPage {
    ctx: PageCtx,
    id: String,
    name: String,
    facts: Vec<(String, String)>,
    grid_id: String,
    yields: Vec<YieldRow>,
    forecast: Option<(i32, String, String, String)>,
    years: Vec<EconomicsRow>,
    break_even: Option<String>,
    simple_return: Option<String>,
    investments: String,
}

fn report_page(ctx: &Ctx, r: &PvReport, names: &HashMap<MeterId, String>) -> DetailPage {
    let lang = ctx.page.lang;
    let d = &r.system.data;
    let mut facts = vec![
        (
            lang.t("pv-kind"),
            lang.t(&format!("pv-kind-{}", d.kind.as_str())),
        ),
        (lang.t("pv-kwp"), format!("{} kWp", plain(lang, d.kwp))),
        (
            lang.t("pv-commissioned"),
            format::date(lang, d.commissioned_on),
        ),
        (
            lang.t("pv-specific-yield"),
            format!("{} kWh/kWp", plain(lang, d.specific_yield)),
        ),
        (
            lang.t("pv-degradation"),
            format!("{} %", plain(lang, d.degradation_pct)),
        ),
    ];
    if let Some(b) = d.battery_kwh {
        facts.push((lang.t("pv-battery-kwh"), format!("{} kWh", plain(lang, b))));
    }
    for (m, role) in &r.system.meters {
        facts.push((
            lang.t(&format!("pv-role-{}", role.as_str())),
            names.get(m).cloned().unwrap_or_default(),
        ));
    }
    let yields = r
        .yields
        .iter()
        .rev()
        .map(|y| YieldRow {
            month: format::month(lang, y.month.start),
            actual: kwh(lang, y.actual),
            expected: kwh(lang, Some(y.expected)),
            band: lang.ta(
                "pv-band-range",
                &[
                    ("lower", format::decimal(lang, y.band.lower, 0)),
                    ("upper", format::decimal(lang, y.band.upper, 0)),
                ],
            ),
            status: status_key(y.status),
            status_label: y
                .status
                .map(|s| lang.t(&format!("traffic-{}", status_key(Some(s)))))
                .unwrap_or_default(),
            history: y.source == zaehler_domain::pv::YieldSource::History,
        })
        .collect();
    let years = r
        .years
        .iter()
        .map(|y| EconomicsRow {
            label: y.period.start.format("%Y").to_string(),
            generation: kwh(lang, y.generation),
            feed_in: money(lang, y.economics.feed_in),
            savings: money(lang, y.economics.savings),
            advantage: money(lang, y.economics.advantage),
            costs: money(lang, y.economics.costs),
            balance: money(lang, y.economics.balance()),
            cumulative: money(lang, y.cumulative),
        })
        .collect();
    DetailPage {
        id: r.system.id.to_string(),
        name: d.name.clone(),
        facts,
        grid_id: r.system.grid().map(|g| g.to_string()).unwrap_or_default(),
        yields,
        forecast: r.forecast.map(|f| {
            (
                f.year,
                kwh(lang, Some(f.measured)),
                kwh(lang, Some(f.expected_rest)),
                kwh(lang, Some(f.total())),
            )
        }),
        years,
        break_even: r.break_even.map(|(date, projected)| {
            let d = format::date(lang, date);
            if projected {
                lang.ta("pv-break-even-projected", &[("date", d)])
            } else {
                d
            }
        }),
        simple_return: r.simple_return.map(|v| format::percent(lang, v, 1)),
        investments: money(lang, r.investments),
        ctx: ctx.page.clone(),
    }
}

// @impl SPEC_PV_YIELD_EXPECTATION, SPEC_PV_MONITORING, SPEC_PV_ECONOMICS
pub async fn detail(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_system(&id).map_err(|e| ctx.fail(e))?;
    let r = state
        .uc
        .pv_system_report
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let names = meter_names(&state, &ctx).await?;
    Ok(render(&report_page(&ctx, &r, &names)))
}

fn num(v: Option<Decimal>) -> Value {
    v.map_or(Value::Null, |v| {
        json!(rust_decimal::prelude::ToPrimitive::to_f64(&v.round_dp(2)))
    })
}

/// Erzeugung gegen Erwartung mit Band, letzte 24 Monate (SPEC_PV_CHARTS).
// @impl SPEC_PV_CHARTS
pub async fn yield_chart(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_system(&id).map_err(|e| ctx.fail(e))?;
    let r = state
        .uc
        .pv_system_report
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let lang = ctx.page.lang;
    let months: Vec<_> = r.yields.iter().rev().take(24).rev().collect();
    Ok(json_response(json!({
        "unit": "kWh",
        "labels": months.iter().map(|y| format::month_short(lang, y.month.start)).collect::<Vec<_>>(),
        "series": {
            "actual": { "name": lang.t("pv-generation"), "data": months.iter().map(|y| num(y.actual)).collect::<Vec<_>>() },
            "expected": { "name": lang.t("pv-expected"), "data": months.iter().map(|y| num(Some(y.expected))).collect::<Vec<_>>() },
            "lower": { "name": lang.t("pv-band-lower"), "data": months.iter().map(|y| num(Some(y.band.lower))).collect::<Vec<_>>() },
            "band": { "name": lang.t("pv-band"), "data": months.iter().map(|y| num(Some(y.band.upper - y.band.lower))).collect::<Vec<_>>() },
        }
    })))
}

/// Kumulierte Bilanz mit Amortisationszeitpunkt (SPEC_PV_CHARTS).
// @impl SPEC_PV_CHARTS
pub async fn balance_chart(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_system(&id).map_err(|e| ctx.fail(e))?;
    let r = state
        .uc
        .pv_system_report
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let lang = ctx.page.lang;
    let labels: Vec<String> = r
        .months
        .iter()
        .map(|m| format::month_short(lang, m.period.start))
        .collect();
    // gemessen und fortgeschrieben überlappen im letzten gemessenen Monat
    let last = r.months.iter().rposition(|m| !m.projected);
    let measured: Vec<Value> = r
        .months
        .iter()
        .map(|m| {
            if m.projected {
                Value::Null
            } else {
                num(Some(m.cumulative))
            }
        })
        .collect();
    let projected: Vec<Value> = r
        .months
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if m.projected || Some(i) == last {
                num(Some(m.cumulative))
            } else {
                Value::Null
            }
        })
        .collect();
    let break_even = r.break_even.and_then(|(d, _)| {
        r.months
            .iter()
            .position(|m| m.period.start <= d && d < m.period.end)
            .map(|i| labels[i].clone())
    });
    Ok(json_response(json!({
        "unit": "€",
        "labels": labels,
        "break_even": break_even,
        "break_even_label": lang.t("pv-break-even"),
        "series": {
            "measured": { "name": lang.t("pv-balance"), "data": measured },
            "projected": { "name": lang.t("pv-balance-projected"), "data": projected },
        }
    })))
}

// --- Hausanschluss -------------------------------------------------------

pub struct KpiRow {
    pub label: String,
    pub partial: bool,
    pub generation: String,
    pub self_consumption: String,
    pub export: String,
    pub import: String,
    pub consumption: String,
    pub ratio: String,
    pub autarky: String,
    pub battery: String,
    pub warning: bool,
}

#[derive(Template)]
#[template(path = "pages/pv/grid.html")]
struct GridPage {
    ctx: PageCtx,
    id: String,
    name: String,
    systems: Vec<(String, String)>,
    export_assumed: bool,
    has_battery: bool,
    months: Vec<KpiRow>,
    years: Vec<KpiRow>,
    yields: Vec<(String, String, String, String)>,
}

fn kpi_row(lang: Lang, label: String, p: &GridPeriod) -> KpiRow {
    let k: &Kpi = &p.kpi;
    KpiRow {
        label,
        partial: p.partial,
        generation: kwh(lang, k.generation),
        self_consumption: kwh(lang, k.self_consumption),
        export: kwh(lang, k.export),
        import: kwh(lang, k.import),
        consumption: kwh(lang, k.consumption),
        ratio: pct(lang, k.self_consumption_ratio),
        autarky: pct(lang, k.autarky),
        battery: p
            .battery
            .map(|b| {
                lang.ta(
                    "pv-battery-figures",
                    &[
                        ("losses", format::decimal(lang, b.losses, 0)),
                        ("efficiency", pct(lang, b.efficiency)),
                        (
                            "cycles",
                            b.full_cycles
                                .map(|c| format::decimal(lang, c, 1))
                                .unwrap_or_default(),
                        ),
                    ],
                )
            })
            .unwrap_or_default(),
        warning: k.negative_self_consumption,
    }
}

// @impl SPEC_PV_KPI, SPEC_PV_BATTERY
pub async fn grid(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let v = state
        .uc
        .grid_analysis
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let lang = ctx.page.lang;
    let months = v
        .months
        .iter()
        .rev()
        .map(|m| kpi_row(lang, format::month(lang, m.period.start), m))
        .collect();
    let mut years: Vec<KpiRow> = v
        .years
        .iter()
        .map(|y| kpi_row(lang, y.period.start.format("%Y").to_string(), y))
        .collect();
    if let Some(b) = &v.billing_year {
        years.push(kpi_row(lang, lang.t("pv-billing-year"), b));
    }
    Ok(render(&GridPage {
        id: id.to_string(),
        name: v.grid.meter.data.name.clone(),
        systems: v
            .systems
            .iter()
            .map(|s| (s.id.to_string(), s.data.name.clone()))
            .collect(),
        export_assumed: v.setup.has_generation && !v.setup.has_export_register,
        has_battery: v.setup.has_battery,
        yields: v
            .yields
            .iter()
            .rev()
            .map(|y| {
                (
                    y.name.clone(),
                    y.year.to_string(),
                    kwh(lang, y.generation),
                    y.specific
                        .map(|s| format!("{} kWh/kWp", format::decimal(lang, s, 0)))
                        .unwrap_or_default(),
                )
            })
            .collect(),
        months,
        years,
        ctx: ctx.page.clone(),
    }))
}

/// Erzeugung und Gesamtverbrauch je Monat, letzte 24 Monate (SPEC_PV_CHARTS).
// @impl SPEC_PV_CHARTS
pub async fn grid_chart(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let v = state
        .uc
        .grid_analysis
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let lang = ctx.page.lang;
    let months: Vec<&GridPeriod> = v.months.iter().rev().take(24).rev().collect();
    let series = |f: fn(&Kpi) -> Option<Decimal>| -> Vec<Value> {
        months.iter().map(|m| num(f(&m.kpi))).collect()
    };
    Ok(json_response(json!({
        "unit": "kWh",
        "labels": months.iter().map(|m| format::month_short(lang, m.period.start)).collect::<Vec<_>>(),
        "series": {
            "self_consumption": { "name": lang.t("pv-self-consumption"), "data": series(|k| k.self_consumption) },
            "export": { "name": lang.t("pv-export"), "data": series(|k| k.export) },
            "import": { "name": lang.t("pv-import"), "data": series(|k| k.import) },
        }
    })))
}

// --- Sammelerfassung -----------------------------------------------------

pub struct BulkRow {
    pub key: String,
    pub meter: String,
    pub role: String,
    pub register: String,
    pub last: String,
}

#[derive(Template)]
#[template(path = "pages/pv/readings.html")]
struct BulkPage {
    ctx: PageCtx,
    id: String,
    name: String,
    rows: Vec<BulkRow>,
    /// Offene Foto-Entwürfe (SPEC_PV_BULK_CAPTURE)
    drafts: Vec<BulkDraftRow>,
    /// Messstellen des Hausanschlusses: ID und Name
    meters: Vec<(String, String)>,
    values: HashMap<String, String>,
    warnings: Vec<String>,
    messages: Vec<String>,
}

/// Foto-Entwurf mit Zuordnung zu einer Messstelle.
pub struct BulkDraftRow {
    pub id: String,
    /// Feldname der Zuordnung
    pub key: String,
    /// Analyse abgeschlossen
    pub ready: bool,
    /// gewählte Messstelle (leer: nicht verwenden)
    pub selected: String,
    pub suggestion: String,
}

impl BulkPage {
    fn val(&self, field: &str) -> String {
        self.values.get(field).cloned().unwrap_or_default()
    }

    #[allow(clippy::unused_self)] // Askama ruft Hilfen über `self` auf
    fn same(&self, a: &str, b: &str) -> bool {
        a == b
    }
}

/// Zeilen der Foto-Entwürfe; beim ersten Aufruf übernimmt das Formular
/// Messstelle, Wert und Aufnahmezeit aus den Vorschlägen.
fn draft_rows(
    lang: Lang,
    tz: chrono_tz::Tz,
    open: &[zaehler_app::usecase::pv::BulkDraft],
    values: &mut HashMap<String, String>,
    first_visit: bool,
) -> Vec<BulkDraftRow> {
    let mut rows = Vec::new();
    for d in open {
        let key = format!("draft_{}", d.draft.id);
        if first_visit {
            if let Some(m) = d.meter {
                values.insert(key.clone(), m.to_string());
                if let (Some(r), Some(v)) = (
                    d.register,
                    d.draft.suggestion.as_ref().and_then(|s| s.value),
                ) {
                    values
                        .entry(format!("v_{m}_{r}"))
                        .or_insert_with(|| format::decimal_plain(lang, v));
                }
            }
            if let Some(taken) = d.draft.taken_at {
                values.entry("at".into()).or_insert_with(|| {
                    format::datetime_input(taken.with_timezone(&tz).naive_local())
                });
            }
        }
        let suggestion = d.draft.suggestion.as_ref().map_or_else(String::new, |s| {
            [
                s.value.map(|v| format::decimal_plain(lang, v)),
                s.register.clone(),
                s.serial_number.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
        });
        rows.push(BulkDraftRow {
            id: d.draft.id.to_string(),
            ready: d.draft.status.is_ready(),
            selected: values.get(&key).cloned().unwrap_or_default(),
            key,
            suggestion,
        });
    }
    rows
}

async fn bulk_page(
    state: &WebState,
    ctx: &Ctx,
    id: MeterId,
    values: HashMap<String, String>,
    errors: FieldErrors,
    warnings: Vec<String>,
) -> Result<BulkPage, Response> {
    let lang = ctx.page.lang;
    let meters = state
        .uc
        .record_bulk_readings
        .meters(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let mut rows = Vec::new();
    let mut name = String::new();
    for (agg, role) in &meters {
        if agg.meter.id == id {
            name.clone_from(&agg.meter.data.name);
        }
        let detail = state
            .uc
            .get_meter
            .execute(&ctx.tenant, agg.meter.id)
            .await
            .map_err(|e| ctx.fail(e))?;
        for r in &agg.registers {
            let last = detail
                .readings
                .iter()
                .filter(|x| x.register_id == r.id)
                .max_by_key(|x| x.read_at)
                .map(|x| format::decimal_plain(lang, x.value))
                .unwrap_or_default();
            rows.push(BulkRow {
                key: format!("v_{}_{}", agg.meter.id, r.id),
                meter: agg.meter.data.name.clone(),
                role: lang.t(&format!("pv-role-{}", role.as_str())),
                register: r.data.code.clone(),
                last,
            });
        }
    }
    let first_visit = values.is_empty();
    let mut values = values;
    let open = state
        .uc
        .record_bulk_readings
        .drafts(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let drafts = draft_rows(lang, ctx.page.tz, &open, &mut values, first_visit);
    values.entry("at".into()).or_insert_with(|| {
        format::datetime_input(Utc::now().with_timezone(&ctx.page.tz).naive_local())
    });
    Ok(BulkPage {
        id: id.to_string(),
        name,
        drafts,
        meters: meters
            .iter()
            .map(|(a, _)| (a.meter.id.to_string(), a.meter.data.name.clone()))
            .collect(),
        rows,
        values,
        messages: errors.0.iter().map(|e| lang.err(e)).collect(),
        warnings,
        ctx: ctx.page.clone(),
    })
}

// @impl SPEC_PV_BULK_CAPTURE
pub async fn bulk_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    if !ctx.page.can_write {
        return Err(ctx.fail(AppError::Forbidden));
    }
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    Ok(render(
        &bulk_page(
            &state,
            &ctx,
            id,
            HashMap::new(),
            FieldErrors::default(),
            Vec::new(),
        )
        .await?,
    ))
}

// @impl SPEC_PV_BULK_CAPTURE
pub async fn bulk(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let lang = ctx.page.lang;
    let mut e = FieldErrors::default();
    let at = parse::datetime_local(form.get("at")).map_or_else(
        |_| {
            e.add("at", "form.datetime_invalid");
            None
        },
        |l| Some(Calendar::new(ctx.page.tz).to_utc(l)),
    );
    let mut values = Vec::new();
    for (k, v) in &form.0 {
        let Some(rest) = k.strip_prefix("v_") else {
            continue;
        };
        if v.trim().is_empty() {
            continue;
        }
        let Some((m, r)) = rest.split_once('_') else {
            continue;
        };
        let (Ok(m), Ok(r)) = (m.parse::<MeterId>(), r.parse::<RegisterId>()) else {
            continue;
        };
        if let Some(value) = decimal(&mut e, k, v) {
            values.push((m, r, value));
        }
    }
    let result = match (at, e.is_empty()) {
        (Some(at), true) => {
            state
                .uc
                .record_bulk_readings
                .execute(
                    &ctx.tenant,
                    id,
                    BulkInput {
                        at,
                        values,
                        note: form.opt("note"),
                        confirm_warnings: form.get("confirm_warnings") == "true",
                        drafts: form
                            .0
                            .iter()
                            .filter_map(|(k, v)| {
                                let draft = k.strip_prefix("draft_")?.parse::<DraftId>().ok()?;
                                Some((draft, v.parse::<MeterId>().ok()?))
                            })
                            .collect(),
                    },
                )
                .await
        }
        _ => Err(AppError::Validation(e)),
    };
    let (errors, warnings) = match result {
        Ok(BulkOutcome::Saved(_)) => {
            ctx.flash("flash-readings-saved").await;
            return Ok(see_other(&format!("/pv/grid/{id}")));
        }
        Ok(BulkOutcome::NeedsConfirmation(w)) => (
            FieldErrors::default(),
            w.iter()
                .flat_map(|(_, checks)| checks.iter().filter_map(|c| hint(lang, &c.plausibility)))
                .collect(),
        ),
        Err(AppError::Validation(e)) => (e, Vec::new()),
        Err(e) => return Err(ctx.fail(e)),
    };
    let page = bulk_page(&state, &ctx, id, form.0, errors, warnings).await?;
    Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
}
