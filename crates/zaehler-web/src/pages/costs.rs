//! Kostenansicht je Messstelle und Haushalt mit Diagramm
//! (SPEC_TAR_COST_VIEW, SPEC_TAR_INSTALLMENT_SETTLEMENT,
//! SPEC_TAR_ACTUAL_SETTLEMENT, SPEC_TAR_SUBMETER_COST).

use askama::Template;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::Days;
use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde::Deserialize;
use serde_json::{Value, json};
use zaehler_app::{
    AppError,
    ids::MeterId,
    usecase::cost::{
        ContractCost, HouseholdCostView, MeterCostView, MonthCost, PeriodCost, SettlementCheck,
    },
};
use zaehler_domain::{cost::CostBreakdown, period::Period};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format},
    pages::{contracts::template_links, render_status},
    state::WebState,
};

fn money(lang: Lang, v: Decimal) -> String {
    format!("{} €", format::decimal(lang, v, 2))
}

/// Zeitraum `[start, end)` als „01.01.2026 bis 31.12.2026“.
fn period_text(lang: Lang, p: Period) -> String {
    let last = p.end.checked_sub_days(Days::new(1)).unwrap_or(p.end);
    lang.ta(
        "contract-period",
        &[
            ("start", format::date(lang, p.start)),
            ("end", format::date(lang, last)),
        ],
    )
}

/// Zeile der Kostentabelle: ein Preisstand einer Position, gemessen oder
/// prognostiziert.
pub struct SegmentRow {
    pub position: String,
    pub period: String,
    pub quantity: String,
    pub factor: String,
    pub base: String,
    pub work: String,
    pub total: String,
    pub estimated: bool,
}

pub struct InstallmentView {
    pub current: String,
    pub paid: String,
    pub open: String,
    /// Positiv: Nachzahlung, negativ: Erstattung
    pub balance: Option<String>,
    pub balance_label: String,
    pub suggestion: Option<String>,
}

pub struct ContractCostView {
    pub id: String,
    pub provider: String,
    pub period: String,
    pub rows: Vec<SegmentRow>,
    pub measured: String,
    pub estimated: String,
    pub adjustments: Option<String>,
    pub total: String,
    pub coverage: String,
    pub has_estimate: bool,
    pub quantity: Option<String>,
    pub unit_price: Option<String>,
    pub installments: Option<InstallmentView>,
    /// Keine Ablesungen, keine Kostenrechnung möglich
    pub missing: bool,
}

pub struct ShareRow {
    pub id: String,
    pub name: String,
    pub quantity: String,
    pub unit_price: String,
    pub cost: String,
    pub estimated: bool,
}

pub struct SettlementRow {
    pub provider: String,
    pub period: String,
    pub amount: String,
    pub calculated: Option<String>,
    pub calculated_estimated: bool,
    pub difference: Option<String>,
    pub difference_rel: Option<String>,
    pub forecast_on: String,
    pub forecast: Option<String>,
}

pub struct YearView {
    pub import: Option<String>,
    pub import_estimated: bool,
    pub export: Option<String>,
    pub coverage: String,
    pub without_contract: bool,
}

pub struct MonthRow {
    pub label: String,
    pub measured: String,
    pub estimated: String,
    pub export: String,
}

#[derive(Template)]
#[template(path = "pages/costs/meter.html")]
struct MeterPage {
    ctx: PageCtx,
    id: String,
    name: String,
    year: i32,
    has_contracts: bool,
    templates: Vec<(String, String)>,
    import: Option<ContractCostView>,
    export: Option<ContractCostView>,
    year_view: YearView,
    months: Vec<MonthRow>,
    submeters: Vec<ShareRow>,
    parent: Option<(String, ShareRow, String)>,
    settlements: Vec<SettlementRow>,
}

fn rows(lang: Lang, c: &ContractCost, b: &CostBreakdown, unit: &str) -> Vec<SegmentRow> {
    b.segments
        .iter()
        .map(|s| SegmentRow {
            position: c
                .contract
                .data
                .positions
                .get(s.position)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            period: period_text(lang, s.period),
            quantity: format!("{} {unit}", format::decimal(lang, s.quantity, 1)),
            factor: format::decimal_plain(lang, s.factor),
            base: money(lang, s.base),
            work: money(lang, s.work),
            total: money(lang, s.total()),
            estimated: s.estimated,
        })
        .collect()
}

fn contract_view(lang: Lang, c: &ContractCost, unit: &str) -> ContractCostView {
    let b = c.breakdown.as_ref();
    let inst = c.installments.as_ref().map(|i| {
        let label = match i.balance {
            Some(v) if v > Decimal::ZERO => lang.t("costs-balance-due"),
            Some(v) if v < Decimal::ZERO => lang.t("costs-balance-refund"),
            _ => lang.t("costs-balance"),
        };
        InstallmentView {
            current: i.current.as_ref().map_or_else(
                || lang.t("installments-none"),
                |r| {
                    lang.ta(
                        "costs-installment-current",
                        &[
                            ("amount", money(lang, r.installment.amount)),
                            ("n", r.installment.every_months.to_string()),
                        ],
                    )
                },
            ),
            paid: money(lang, i.paid),
            open: money(lang, i.open),
            balance: i.balance.map(|v| money(lang, v.abs())),
            balance_label: label,
            suggestion: i.suggestion.map(|s| {
                lang.ta(
                    "costs-suggestion-value",
                    &[("amount", money(lang, s)), ("n", i.interval.to_string())],
                )
            }),
        }
    });
    ContractCostView {
        id: c.contract.id.to_string(),
        provider: c.contract.data.provider.clone(),
        period: c.period.map(|p| period_text(lang, p)).unwrap_or_default(),
        rows: b.map(|b| rows(lang, c, b, unit)).unwrap_or_default(),
        measured: money(lang, b.map_or(Decimal::ZERO, CostBreakdown::measured)),
        estimated: money(lang, b.map_or(Decimal::ZERO, CostBreakdown::estimated)),
        adjustments: b
            .filter(|b| !b.adjustments.is_zero())
            .map(|b| money(lang, -b.adjustments)),
        total: money(lang, b.map_or(Decimal::ZERO, CostBreakdown::total)),
        coverage: format::percent(lang, b.map_or(Decimal::ZERO, CostBreakdown::coverage), 0),
        has_estimate: b.is_some_and(CostBreakdown::has_estimate),
        quantity: c
            .quantity
            .map(|q| format!("{} {unit}", format::decimal(lang, q.value, 0))),
        unit_price: c
            .unit_price()
            .map(|p| format!("{} €/{unit}", format::decimal(lang, p, 4))),
        installments: inst,
        missing: b.is_none(),
    }
}

fn settlement_row(lang: Lang, s: &SettlementCheck) -> SettlementRow {
    SettlementRow {
        provider: s.provider.clone(),
        period: period_text(lang, s.settlement.period()),
        amount: money(lang, s.settlement.amount),
        calculated: s.calculated.as_ref().map(|c| money(lang, c.total())),
        calculated_estimated: s
            .calculated
            .as_ref()
            .is_some_and(CostBreakdown::has_estimate),
        difference: s
            .difference
            .map(|d| format!("{} €", format::signed(lang, d, 2))),
        difference_rel: s.difference_rel.map(|r| format::percent(lang, r, 1)),
        forecast_on: format::date(lang, s.forecast_on),
        forecast: s.forecast.map(|f| money(lang, f)),
    }
}

fn year_view(lang: Lang, import: Option<PeriodCost>, export: Option<PeriodCost>) -> YearView {
    YearView {
        import: import.map(|c| money(lang, c.total())),
        import_estimated: import.is_some_and(|c| c.has_estimate()),
        export: export.map(|c| money(lang, c.total())),
        coverage: format::percent(lang, import.map_or(Decimal::ZERO, |c| c.coverage), 0),
        without_contract: import.is_some_and(|c| c.without_contract),
    }
}

fn month_rows(lang: Lang, months: &[MonthCost]) -> Vec<MonthRow> {
    months
        .iter()
        .map(|m| MonthRow {
            label: format::month(lang, m.month.start),
            measured: format::decimal(lang, m.measured, 2),
            estimated: format::decimal(lang, m.estimated, 2),
            export: format::decimal(lang, m.export, 2),
        })
        .collect()
}

fn parse_meter_id(id: &str) -> Result<MeterId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

fn meter_page(ctx: &Ctx, v: &MeterCostView, templates: Vec<(String, String)>) -> MeterPage {
    let lang = ctx.page.lang;
    let unit = v.meter.meter.data.unit.clone();
    let share = |s: &zaehler_app::usecase::cost::SubmeterShare| ShareRow {
        id: s.meter_id.to_string(),
        name: s.name.clone(),
        quantity: format!("{} {unit}", format::decimal(lang, s.quantity.value, 0)),
        unit_price: format!("{} €/{unit}", format::decimal(lang, s.unit_price, 4)),
        cost: money(lang, s.cost()),
        estimated: s.quantity.estimated,
    };
    MeterPage {
        id: v.meter.meter.id.to_string(),
        name: v.meter.meter.data.name.clone(),
        year: chrono::Datelike::year(&v.today),
        has_contracts: !v.contracts.is_empty(),
        templates,
        import: v.import.as_ref().map(|c| contract_view(lang, c, &unit)),
        export: v.export.as_ref().map(|c| contract_view(lang, c, &unit)),
        year_view: year_view(lang, v.year_import, v.year_export),
        months: month_rows(lang, &v.months),
        submeters: v.submeters.iter().map(share).collect(),
        parent: v
            .parent_share
            .as_ref()
            .map(|(name, s)| (name.clone(), share(s), period_text(lang, s.period))),
        settlements: v
            .settlements
            .iter()
            .map(|s| settlement_row(lang, s))
            .collect(),
        ctx: ctx.page.clone(),
    }
}

// @impl SPEC_TAR_COST_VIEW, SPEC_TAR_SUBMETER_COST, SPEC_TAR_ACTUAL_SETTLEMENT
pub async fn meter(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_meter_id(&id).map_err(|e| ctx.fail(e))?;
    let view = state
        .uc
        .meter_costs
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let templates = if view.contracts.is_empty() && ctx.page.is_owner {
        let (_, t) = state
            .uc
            .contract_templates
            .execute(&ctx.tenant, id)
            .await
            .map_err(|e| ctx.fail(e))?;
        template_links(ctx.page.lang, &t)
    } else {
        Vec::new()
    };
    Ok(render_status(
        StatusCode::OK,
        &meter_page(&ctx, &view, templates),
    ))
}

/// Monatskosten für `charts.js` (Typ `costs`).
fn chart_json(lang: Lang, months: &[MonthCost]) -> Value {
    let series = |f: fn(&MonthCost) -> Decimal| -> Vec<Value> {
        months
            .iter()
            .map(|m| {
                let v = f(m);
                if v.is_zero() {
                    Value::Null
                } else {
                    json!(v.round_dp(2).to_f64())
                }
            })
            .collect()
    };
    json!({
        "unit": "€",
        "labels": months.iter().map(|m| format::month_short(lang, m.month.start)).collect::<Vec<_>>(),
        "series": {
            "measured": { "name": lang.t("costs-measured"), "data": series(|m| m.measured) },
            "estimated": { "name": lang.t("costs-estimated"), "data": series(|m| m.estimated) },
            "export": { "name": lang.t("costs-export"), "data": series(|m| m.export) },
        }
    })
}

fn json_response(v: Value) -> Response {
    ([(header::CACHE_CONTROL, "private, no-store")], Json(v)).into_response()
}

// @impl SPEC_TAR_COST_VIEW
pub async fn meter_chart(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_meter_id(&id).map_err(|e| ctx.fail(e))?;
    let view = state
        .uc
        .meter_costs
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    Ok(json_response(chart_json(ctx.page.lang, &view.months)))
}

#[derive(Debug, Deserialize)]
pub struct YearQuery {
    pub year: Option<i32>,
}

pub struct HouseholdRow {
    pub id: String,
    pub name: String,
    pub import: Option<String>,
    pub import_estimated: bool,
    pub export: Option<String>,
    pub without_contract: bool,
    pub included: Option<(String, String, bool)>,
}

#[derive(Template)]
#[template(path = "pages/costs/household.html")]
struct HouseholdPage {
    ctx: PageCtx,
    year: i32,
    rows: Vec<HouseholdRow>,
    import_total: String,
    export_total: Option<String>,
    net_total: String,
    has_estimate: bool,
    months: Vec<MonthRow>,
}

fn household_page(ctx: &Ctx, v: &HouseholdCostView) -> HouseholdPage {
    let lang = ctx.page.lang;
    let export = v.export_total();
    HouseholdPage {
        year: v.year,
        rows: v
            .rows
            .iter()
            .map(|r| HouseholdRow {
                id: r.meter_id.to_string(),
                name: r.name.clone(),
                import: r.import.map(|c| money(lang, c.total())),
                import_estimated: r.import.is_some_and(|c| c.has_estimate()),
                export: r.export.map(|c| money(lang, c.total())),
                without_contract: r.import.is_some_and(|c| c.without_contract),
                included: r
                    .included_in
                    .as_ref()
                    .map(|(parent, c)| (parent.clone(), money(lang, c.value), c.estimated)),
            })
            .collect(),
        import_total: money(lang, v.import_total()),
        export_total: (!export.is_zero()).then(|| money(lang, export)),
        net_total: money(lang, v.import_total() + export),
        has_estimate: v.has_estimate(),
        months: month_rows(lang, &v.months),
        ctx: ctx.page.clone(),
    }
}

// @impl SPEC_TAR_COST_VIEW
pub async fn household(
    State(state): State<WebState>,
    ctx: Ctx,
    Query(q): Query<YearQuery>,
) -> Result<Response, Response> {
    let view = state
        .uc
        .household_costs
        .execute(&ctx.tenant, q.year.filter(|y| (1970..=9999).contains(y)))
        .await
        .map_err(|e| ctx.fail(e))?;
    Ok(render_status(StatusCode::OK, &household_page(&ctx, &view)))
}

// @impl SPEC_TAR_COST_VIEW
pub async fn household_chart(
    State(state): State<WebState>,
    ctx: Ctx,
    Query(q): Query<YearQuery>,
) -> Result<Response, Response> {
    let view = state
        .uc
        .household_costs
        .execute(&ctx.tenant, q.year.filter(|y| (1970..=9999).contains(y)))
        .await
        .map_err(|e| ctx.fail(e))?;
    Ok(json_response(chart_json(ctx.page.lang, &view.months)))
}
