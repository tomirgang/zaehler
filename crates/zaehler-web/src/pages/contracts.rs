//! Verträge: Formular mit Vorlagen, Positionen und Preisständen;
//! Detailseite mit Preisständen, Abschlägen, Anpassungen und Abrechnungen
//! (SPEC_TAR_CONTRACTS, SPEC_TAR_ACTUAL_SETTLEMENT).

use std::collections::HashMap;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{AdjustmentId, ContractId, InstallmentId, MeterId, PositionId, SettlementId},
    model::{Contract, ContractEntry, MeterAggregate, MonthDay},
    usecase::contract::{
        AdjustmentInput, ComponentInput, ContractInput, ContractTemplate, InstallmentInput,
        PositionInput, SettlementInput,
    },
};
use zaehler_domain::{
    meter::Direction,
    tariff::{AdjustmentKind, BasePriceInterval, PriceComponent, Tier, UnitPricing},
};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format, parse},
    pages::{FormData, field_errors, render_status, see_other},
    state::WebState,
};

/// Preisstand im Formular.
#[derive(Debug, Clone, Default)]
pub struct ComponentRow {
    /// Feldpräfix `pos{i}_c{j}`
    pub key: String,
    pub id: String,
    pub valid_from: String,
    pub base: String,
    pub interval: String,
    pub price: String,
    pub tiers: String,
    pub factor: String,
}

/// Kostenposition im Formular.
#[derive(Debug, Clone, Default)]
pub struct PositionRow {
    /// Feldpräfix `pos{i}`
    pub key: String,
    pub idx: usize,
    pub id: String,
    pub name: String,
    pub register: String,
    pub factor: String,
    pub vat: String,
    pub components: Vec<ComponentRow>,
}

/// Werte des Vertragsformulars.
#[derive(Debug, Clone, Default)]
pub struct ContractForm {
    pub template: String,
    pub direction: String,
    pub provider: String,
    pub contract_number: String,
    pub starts_on: String,
    pub ends_on: String,
    pub billing_month: String,
    pub billing_day: String,
    pub note: String,
    pub positions: Vec<PositionRow>,
}

#[derive(Template)]
#[template(path = "pages/contracts/form.html")]
struct FormPage {
    ctx: PageCtx,
    meter_id: String,
    meter_name: String,
    /// Ziel des Formulars
    action: String,
    editing: bool,
    /// (Code, Bezeichnung) der Vorlagen
    templates: Vec<(String, String)>,
    /// (Wert, Bezeichnung) der möglichen Richtungen
    directions: Vec<(String, String)>,
    /// (ID, Bezeichnung) der Register
    registers: Vec<(String, String)>,
    intervals: Vec<(String, String)>,
    form: ContractForm,
    errors: FieldErrors,
    /// Fachlicher Konflikt als Text
    conflict: Option<String>,
}

impl FormPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn has_errs(&self, field: &str) -> bool {
        !self.errors.for_field(field).is_empty()
    }

    /// Fehler eines Feldes einer Position oder eines Preisstands.
    fn ferrs(&self, key: &str, part: &str) -> Vec<String> {
        self.errs(&format!("{key}_{part}"))
    }
}

pub(crate) fn money(lang: Lang, v: Decimal) -> String {
    format!("{} €", format::decimal(lang, v, 2))
}

pub(crate) fn plain(lang: Lang, v: Decimal) -> String {
    format::decimal_plain(lang, v)
}

pub(crate) fn date_input(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub(crate) fn interval_options(lang: Lang) -> Vec<(String, String)> {
    BasePriceInterval::ALL
        .iter()
        .map(|i| {
            (
                i.as_str().to_owned(),
                lang.t(&format!("interval-{}", i.as_str())),
            )
        })
        .collect()
}

fn blank_component(key: String, valid_from: &str) -> ComponentRow {
    ComponentRow {
        key,
        valid_from: valid_from.to_owned(),
        interval: BasePriceInterval::Year.as_str().to_owned(),
        ..ComponentRow::default()
    }
}

fn blank_position(idx: usize, valid_from: &str) -> PositionRow {
    PositionRow {
        key: format!("pos{idx}"),
        idx,
        factor: "1".into(),
        components: vec![blank_component(format!("pos{idx}_c0"), valid_from)],
        ..PositionRow::default()
    }
}

/// Tiers als Text: je Zeile „Obergrenze;Preis“, die letzte ohne Obergrenze.
fn tiers_text(lang: Lang, tiers: &[Tier]) -> String {
    tiers
        .iter()
        .map(|t| {
            format!(
                "{};{}",
                t.up_to.map(|u| plain(lang, u)).unwrap_or_default(),
                plain(lang, t.price)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn component_row(lang: Lang, key: String, id: String, c: &PriceComponent) -> ComponentRow {
    let (price, tiers) = match &c.unit_pricing {
        UnitPricing::Flat { price } => (plain(lang, *price), String::new()),
        UnitPricing::Tiered { tiers } => (String::new(), tiers_text(lang, tiers)),
        UnitPricing::Dynamic { fallback, .. } => (plain(lang, *fallback), String::new()),
    };
    ComponentRow {
        key,
        id,
        valid_from: date_input(c.valid_from),
        base: plain(lang, c.base_price),
        interval: c.base_interval.as_str().to_owned(),
        price,
        tiers,
        factor: c.factor.map(|f| plain(lang, f)).unwrap_or_default(),
    }
}

/// Formular aus einer Vorlage; Namen in der Sprache des Nutzers.
fn from_template(lang: Lang, t: &ContractTemplate, today: NaiveDate) -> ContractForm {
    let start = date_input(today);
    ContractForm {
        template: t.code.to_owned(),
        direction: t.direction.as_str().to_owned(),
        starts_on: start.clone(),
        positions: t
            .positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut row = blank_position(i, &start);
                row.name = lang.t(&format!("tpl-pos-{}", p.name));
                row.register = p.register_id.map(|r| r.to_string()).unwrap_or_default();
                row.factor = p.factor.map(|f| plain(lang, f)).unwrap_or_default();
                p.base_interval
                    .as_str()
                    .clone_into(&mut row.components[0].interval);
                if !p.base {
                    row.components[0].base = "0".into();
                }
                if !p.work {
                    row.components[0].price = "0".into();
                }
                row
            })
            .collect(),
        ..ContractForm::default()
    }
}

fn from_contract(lang: Lang, c: &Contract) -> ContractForm {
    let d = &c.data;
    ContractForm {
        template: String::new(),
        direction: d.direction.as_str().to_owned(),
        provider: d.provider.clone(),
        contract_number: d.contract_number.clone().unwrap_or_default(),
        starts_on: date_input(d.starts_on),
        ends_on: d.ends_on.map(date_input).unwrap_or_default(),
        billing_month: d.billing_start.month.to_string(),
        billing_day: d.billing_start.day.to_string(),
        note: d.note.clone().unwrap_or_default(),
        positions: d
            .positions
            .iter()
            .enumerate()
            .map(|(i, p)| PositionRow {
                key: format!("pos{i}"),
                idx: i,
                id: p.id.to_string(),
                name: p.name.clone(),
                register: p.register_id.map(|r| r.to_string()).unwrap_or_default(),
                factor: plain(lang, p.factor),
                vat: p.vat_pct.map(|v| plain(lang, v)).unwrap_or_default(),
                components: p
                    .components
                    .iter()
                    .enumerate()
                    .map(|(j, x)| {
                        component_row(lang, format!("pos{i}_c{j}"), x.id.to_string(), &x.component)
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Formularwerte aus den gesendeten Feldern.
fn read_form(form: &FormData) -> ContractForm {
    let count = |key: &str| form.get(key).parse::<usize>().unwrap_or(0).min(64);
    let positions = (0..count("positions"))
        .map(|i| {
            let key = format!("pos{i}");
            let f = |part: &str| form.get(&format!("{key}_{part}")).to_owned();
            PositionRow {
                idx: i,
                id: f("id"),
                name: f("name"),
                register: f("register"),
                factor: f("factor"),
                vat: f("vat"),
                components: (0..count(&format!("{key}_components")))
                    .map(|j| {
                        let ckey = format!("{key}_c{j}");
                        let g = |part: &str| form.get(&format!("{ckey}_{part}")).to_owned();
                        ComponentRow {
                            id: g("id"),
                            valid_from: g("valid_from"),
                            base: g("base"),
                            interval: g("interval"),
                            price: g("price"),
                            tiers: g("tiers"),
                            factor: g("factor"),
                            key: ckey,
                        }
                    })
                    .collect(),
                key,
            }
        })
        .collect();
    ContractForm {
        template: form.get("template").to_owned(),
        direction: form.get("direction").to_owned(),
        provider: form.get("provider").to_owned(),
        contract_number: form.get("contract_number").to_owned(),
        starts_on: form.get("starts_on").to_owned(),
        ends_on: form.get("ends_on").to_owned(),
        billing_month: form.get("billing_month").to_owned(),
        billing_day: form.get("billing_day").to_owned(),
        note: form.get("note").to_owned(),
        positions,
    }
}

/// Positionen und Preisstände neu nummerieren (nach Hinzufügen/Entfernen).
fn renumber(form: &mut ContractForm) {
    for (i, p) in form.positions.iter_mut().enumerate() {
        p.idx = i;
        p.key = format!("pos{i}");
        for (j, c) in p.components.iter_mut().enumerate() {
            c.key = format!("pos{i}_c{j}");
        }
    }
}

/// Zeilen hinzufügen oder entfernen; `true`, wenn das Formular nur neu
/// angezeigt wird.
fn apply_action(form: &mut ContractForm, action: &str) -> bool {
    let start = form.starts_on.clone();
    let handled = if action == "add_position" {
        let idx = form.positions.len();
        form.positions.push(blank_position(idx, &start));
        true
    } else if let Some(i) = action
        .strip_prefix("add_component:")
        .and_then(|i| i.parse::<usize>().ok())
    {
        if let Some(p) = form.positions.get_mut(i) {
            p.components.push(blank_component(String::new(), ""));
        }
        true
    } else if let Some(i) = action
        .strip_prefix("remove_position:")
        .and_then(|i| i.parse::<usize>().ok())
    {
        if i < form.positions.len() {
            form.positions.remove(i);
        }
        true
    } else {
        false
    };
    renumber(form);
    handled
}

pub(crate) fn decimal(e: &mut FieldErrors, field: &str, value: &str) -> Option<Decimal> {
    parse::decimal(value).unwrap_or_else(|_| {
        e.add(field, "form.number_invalid");
        None
    })
}

pub(crate) fn date(e: &mut FieldErrors, field: &str, value: &str) -> Option<NaiveDate> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    NaiveDate::parse_from_str(v, "%Y-%m-%d").map_or_else(
        |_| {
            e.add(field, "form.date_invalid");
            None
        },
        Some,
    )
}

pub(crate) fn required_date(e: &mut FieldErrors, field: &str, value: &str) -> Option<NaiveDate> {
    let d = date(e, field, value);
    if value.trim().is_empty() {
        e.add(field, "form.date_required");
    }
    d
}

/// Staffel aus Zeilen „Obergrenze;Preis“; die letzte Zeile ohne Obergrenze.
fn parse_tiers(e: &mut FieldErrors, field: &str, text: &str) -> Vec<Tier> {
    let mut tiers = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (up, price) = line.split_once(';').unwrap_or(("", line));
        let up_to = parse::decimal(up).ok();
        let price = parse::decimal(price).ok().flatten();
        let (Some(up_to), Some(price)) = (up_to, price) else {
            e.add(field, "form.tiers_invalid");
            return Vec::new();
        };
        tiers.push(Tier { up_to, price });
    }
    tiers
}

fn parse_component(
    e: &mut FieldErrors,
    row: &ComponentRow,
    default_from: Option<NaiveDate>,
) -> Option<ComponentInput> {
    let k = &row.key;
    let valid_from = if row.valid_from.trim().is_empty() {
        default_from
    } else {
        date(e, &format!("{k}_valid_from"), &row.valid_from)
    };
    if valid_from.is_none() && row.valid_from.trim().is_empty() {
        e.add(format!("{k}_valid_from"), "form.date_required");
    }
    let base = decimal(e, &format!("{k}_base"), &row.base).unwrap_or(Decimal::ZERO);
    let interval = row
        .interval
        .parse::<BasePriceInterval>()
        .unwrap_or(BasePriceInterval::Year);
    let unit_pricing = if row.tiers.trim().is_empty() {
        UnitPricing::Flat {
            price: decimal(e, &format!("{k}_price"), &row.price).unwrap_or(Decimal::ZERO),
        }
    } else {
        UnitPricing::Tiered {
            tiers: parse_tiers(e, &format!("{k}_tiers"), &row.tiers),
        }
    };
    let factor = decimal(e, &format!("{k}_factor"), &row.factor);
    Some(ComponentInput {
        id: row.id.parse().ok(),
        valid_from: valid_from?,
        base_price: base,
        base_interval: interval,
        unit_pricing,
        factor,
    })
}

fn component_empty(c: &ComponentRow) -> bool {
    [&c.valid_from, &c.base, &c.price, &c.tiers, &c.factor]
        .iter()
        .all(|v| v.trim().is_empty())
}

/// Eingaben des Formulars; leere Positionen und Preisstände entfallen.
// @impl SPEC_TAR_CONTRACTS, SPEC_I18N
fn parse_input(form: &ContractForm) -> Result<ContractInput, FieldErrors> {
    let mut e = FieldErrors::default();
    let direction = form.direction.parse::<Direction>().unwrap_or_else(|_| {
        e.add("direction", "form.kind_invalid");
        Direction::Import
    });
    let starts_on = required_date(&mut e, "starts_on", &form.starts_on);
    let ends_on = date(&mut e, "ends_on", &form.ends_on);
    let month = parse::integer::<u8>(&form.billing_month);
    let day = parse::integer::<u8>(&form.billing_day);
    let billing_start = match (month, day) {
        (Ok(None), Ok(None)) => None,
        (Ok(Some(month)), Ok(Some(day))) => Some(MonthDay { month, day }),
        _ => {
            e.add("billing_start", "meter.billing_start_invalid");
            None
        }
    };
    let mut positions = Vec::new();
    for p in &form.positions {
        let comps: Vec<&ComponentRow> = p
            .components
            .iter()
            .filter(|c| !component_empty(c))
            .collect();
        if p.name.trim().is_empty() && comps.is_empty() {
            continue;
        }
        let k = &p.key;
        let factor = if p.factor.trim().is_empty() {
            e.add(format!("{k}_factor"), "form.number_required");
            None
        } else {
            decimal(&mut e, &format!("{k}_factor"), &p.factor)
        };
        let vat = decimal(&mut e, &format!("{k}_vat"), &p.vat);
        let register_id = if p.register.is_empty() {
            None
        } else {
            p.register.parse().map_or_else(
                |_| {
                    e.add(format!("{k}_register"), "form.kind_invalid");
                    None
                },
                Some,
            )
        };
        let components = comps
            .iter()
            .filter_map(|c| parse_component(&mut e, c, starts_on))
            .collect();
        positions.push(PositionInput {
            id: p.id.parse::<PositionId>().ok(),
            name: p.name.clone(),
            register_id,
            factor: factor.unwrap_or(Decimal::ONE),
            vat_pct: vat,
            components,
        });
    }
    match (e.is_empty(), starts_on) {
        (true, Some(starts_on)) => Ok(ContractInput {
            direction,
            provider: form.provider.clone(),
            contract_number: Some(form.contract_number.clone()),
            starts_on,
            ends_on,
            billing_start,
            note: Some(form.note.clone()),
            positions,
        }),
        _ => Err(e),
    }
}

fn parse_contract_id(id: &str) -> Result<ContractId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

fn parse_meter_id(id: &str) -> Result<MeterId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

#[allow(clippy::too_many_arguments)] // Bestandteile der Seite
fn form_page(
    ctx: &Ctx,
    agg: &MeterAggregate,
    templates: &[ContractTemplate],
    action: String,
    editing: bool,
    form: ContractForm,
    errors: FieldErrors,
    conflict: Option<&'static str>,
    status: StatusCode,
) -> Response {
    let lang = ctx.page.lang;
    let mut directions: Vec<(String, String)> = Vec::new();
    for d in [Direction::Import, Direction::Export] {
        if agg.registers.iter().any(|r| r.data.direction == d) {
            directions.push((
                d.as_str().to_owned(),
                lang.t(&format!("contract-direction-{}", d.as_str())),
            ));
        }
    }
    let page = FormPage {
        meter_id: agg.meter.id.to_string(),
        meter_name: agg.meter.data.name.clone(),
        action,
        editing,
        templates: templates
            .iter()
            .map(|t| (t.code.to_owned(), lang.t(&format!("tpl-{}", t.code))))
            .collect(),
        directions,
        registers: agg
            .registers
            .iter()
            .map(|r| {
                (
                    r.id.to_string(),
                    format!(
                        "{} ({})",
                        r.data.code,
                        lang.t(&format!("direction-{}", r.data.direction.as_str()))
                    ),
                )
            })
            .collect(),
        intervals: interval_options(lang),
        form,
        errors,
        conflict: conflict.map(|c| lang.code_text(c)),
        ctx: ctx.page.clone(),
    };
    render_status(status, &page)
}

#[derive(Debug, Deserialize)]
pub struct TemplateQuery {
    pub template: Option<String>,
}

/// Formular für einen neuen Vertrag, vorbelegt mit einer Vorlage.
// @impl SPEC_TAR_CONTRACTS
pub async fn new_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<TemplateQuery>,
) -> Result<Response, Response> {
    if !ctx.page.is_owner {
        return Err(ctx.fail(AppError::Forbidden));
    }
    let id = parse_meter_id(&id).map_err(|e| ctx.fail(e))?;
    let (agg, templates) = state
        .uc
        .contract_templates
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let today = chrono::Utc::now().with_timezone(&ctx.page.tz).date_naive();
    let chosen = q
        .template
        .as_deref()
        .and_then(|c| templates.iter().find(|t| t.code == c))
        .or_else(|| templates.first());
    let form = chosen.map_or_else(
        || {
            let start = date_input(today);
            ContractForm {
                direction: Direction::Import.as_str().into(),
                starts_on: start.clone(),
                positions: vec![blank_position(0, &start)],
                ..ContractForm::default()
            }
        },
        |t| from_template(ctx.page.lang, t, today),
    );
    Ok(form_page(
        &ctx,
        &agg,
        &templates,
        format!("/meters/{id}/contracts/new"),
        false,
        form,
        FieldErrors::default(),
        None,
        StatusCode::OK,
    ))
}

/// Antwort auf Validierungs- und Konfliktfehler: Formular erneut anzeigen.
fn retry(
    ctx: &Ctx,
    agg: &MeterAggregate,
    templates: &[ContractTemplate],
    action: String,
    editing: bool,
    form: ContractForm,
    err: AppError,
) -> Response {
    match err {
        AppError::Validation(errors) => form_page(
            ctx,
            agg,
            templates,
            action,
            editing,
            form,
            errors,
            None,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        AppError::Conflict(code) => form_page(
            ctx,
            agg,
            templates,
            action,
            editing,
            form,
            FieldErrors::default(),
            Some(code),
            StatusCode::CONFLICT,
        ),
        other => ctx.fail(other),
    }
}

// @impl SPEC_TAR_CONTRACTS
pub async fn create(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_meter_id(&id).map_err(|e| ctx.fail(e))?;
    let data = FormData::new(pairs);
    let (agg, templates) = state
        .uc
        .contract_templates
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let mut form = read_form(&data);
    let action = format!("/meters/{id}/contracts/new");
    if apply_action(&mut form, data.get("action")) {
        return Ok(form_page(
            &ctx,
            &agg,
            &templates,
            action,
            false,
            form,
            FieldErrors::default(),
            None,
            StatusCode::OK,
        ));
    }
    let result = match parse_input(&form) {
        Ok(input) => {
            state
                .uc
                .create_contract
                .execute(&ctx.tenant, id, input)
                .await
        }
        Err(e) => Err(AppError::Validation(e)),
    };
    match result {
        Ok(contract) => {
            ctx.flash("flash-contract-created").await;
            Ok(see_other(&format!("/contracts/{contract}")))
        }
        Err(e) => Ok(retry(&ctx, &agg, &templates, action, false, form, e)),
    }
}

// @impl SPEC_TAR_CONTRACTS
pub async fn edit_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    if !ctx.page.is_owner {
        return Err(ctx.fail(AppError::Forbidden));
    }
    let id = parse_contract_id(&id).map_err(|e| ctx.fail(e))?;
    let detail = state
        .uc
        .get_contract
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let form = from_contract(ctx.page.lang, &detail.contract);
    Ok(form_page(
        &ctx,
        &detail.meter,
        &[],
        format!("/contracts/{id}/edit"),
        true,
        form,
        FieldErrors::default(),
        None,
        StatusCode::OK,
    ))
}

// @impl SPEC_TAR_CONTRACTS
pub async fn update(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_contract_id(&id).map_err(|e| ctx.fail(e))?;
    let data = FormData::new(pairs);
    let detail = state
        .uc
        .get_contract
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let mut form = read_form(&data);
    let action = format!("/contracts/{id}/edit");
    if apply_action(&mut form, data.get("action")) {
        return Ok(form_page(
            &ctx,
            &detail.meter,
            &[],
            action,
            true,
            form,
            FieldErrors::default(),
            None,
            StatusCode::OK,
        ));
    }
    let result = match parse_input(&form) {
        Ok(input) => {
            state
                .uc
                .update_contract
                .execute(&ctx.tenant, id, input)
                .await
        }
        Err(e) => Err(AppError::Validation(e)),
    };
    match result {
        Ok(()) => {
            ctx.flash("flash-contract-updated").await;
            Ok(see_other(&format!("/contracts/{id}")))
        }
        Err(e) => Ok(retry(&ctx, &detail.meter, &[], action, true, form, e)),
    }
}

pub async fn delete(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_contract_id(&id).map_err(|e| ctx.fail(e))?;
    let meter = state
        .uc
        .delete_contract
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    ctx.flash("flash-contract-deleted").await;
    Ok(see_other(&format!("/meters/{meter}")))
}

/// Zeile eines Preisstands auf der Detailseite.
pub struct ComponentView {
    pub valid_from: String,
    pub base: String,
    pub work: String,
    pub factor: Option<String>,
    /// Netto-Werte bei hinterlegtem MwSt-Satz (REQ_TAR_010)
    pub net: Option<String>,
}

pub struct PositionView {
    pub id: String,
    pub name: String,
    pub register: String,
    pub factor: String,
    pub vat: Option<String>,
    pub components: Vec<ComponentView>,
}

pub struct EntryView {
    pub id: String,
    pub cells: Vec<String>,
}

#[derive(Template)]
#[template(path = "pages/contracts/detail.html")]
struct DetailPage {
    ctx: PageCtx,
    id: String,
    meter_id: String,
    meter_name: String,
    provider: String,
    facts: Vec<(String, String)>,
    positions: Vec<PositionView>,
    installments: Vec<EntryView>,
    adjustments: Vec<EntryView>,
    settlements: Vec<EntryView>,
    intervals: Vec<(String, String)>,
    adjustment_kinds: Vec<(String, String)>,
    /// Gesendete Werte eines fehlgeschlagenen Formulars
    values: HashMap<String, String>,
    errors: FieldErrors,
}

impl DetailPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn val(&self, field: &str) -> String {
        self.values.get(field).cloned().unwrap_or_default()
    }

    /// Feldwert mit Vorgabe für ein leeres Formular.
    fn val_or(&self, field: &str, default: &str) -> String {
        self.values
            .get(field)
            .cloned()
            .unwrap_or_else(|| default.to_owned())
    }
}

fn unit_price(lang: Lang, pricing: &UnitPricing) -> String {
    match pricing {
        UnitPricing::Flat { price } => format!("{} €", plain(lang, *price)),
        UnitPricing::Tiered { tiers } => tiers
            .iter()
            .map(|t| match t.up_to {
                Some(up) => lang.ta(
                    "contract-tier-up-to",
                    &[("up_to", plain(lang, up)), ("price", plain(lang, t.price))],
                ),
                None => lang.ta("contract-tier-rest", &[("price", plain(lang, t.price))]),
            })
            .collect::<Vec<_>>()
            .join("; "),
        UnitPricing::Dynamic { fallback, .. } => format!("{} €", plain(lang, *fallback)),
    }
}

fn net(v: Decimal, vat: Decimal) -> Decimal {
    v * Decimal::ONE_HUNDRED / (Decimal::ONE_HUNDRED + vat)
}

#[allow(clippy::too_many_lines)] // Abbildung aller Abschnitte der Seite
fn detail_page(
    ctx: &Ctx,
    contract: &Contract,
    meter: &MeterAggregate,
    values: HashMap<String, String>,
    errors: FieldErrors,
) -> DetailPage {
    let lang = ctx.page.lang;
    let d = &contract.data;
    let date = |x: NaiveDate| format::date(lang, x);
    let mut facts = vec![
        (
            lang.t("contract-direction"),
            lang.t(&format!("contract-direction-{}", d.direction.as_str())),
        ),
        (
            lang.t("contract-term"),
            match d.ends_on {
                Some(end) => lang.ta(
                    "contract-term-bounded",
                    &[("start", date(d.starts_on)), ("end", date(end))],
                ),
                None => lang.ta("contract-term-open", &[("start", date(d.starts_on))]),
            },
        ),
        (
            lang.t("contract-billing-start"),
            format!("{:02}.{:02}.", d.billing_start.day, d.billing_start.month),
        ),
    ];
    if let Some(n) = &d.contract_number {
        facts.insert(0, (lang.t("contract-number"), n.clone()));
    }
    if let Some(n) = &d.note {
        facts.push((lang.t("contract-note"), n.clone()));
    }
    let positions = d
        .positions
        .iter()
        .map(|p| PositionView {
            id: p.id.to_string(),
            name: p.name.clone(),
            register: p
                .register_id
                .and_then(|r| meter.register(r))
                .map_or_else(|| lang.t("contract-all-registers"), |r| r.data.code.clone()),
            factor: plain(lang, p.factor),
            vat: p.vat_pct.map(|v| format!("{} %", plain(lang, v))),
            components: p
                .components
                .iter()
                .map(|c| {
                    let x = &c.component;
                    let base = format!(
                        "{} € / {}",
                        plain(lang, x.base_price),
                        lang.t(&format!("interval-{}", x.base_interval.as_str()))
                    );
                    ComponentView {
                        valid_from: date(x.valid_from),
                        base,
                        work: unit_price(lang, &x.unit_pricing),
                        factor: x.factor.map(|f| plain(lang, f)),
                        net: p.vat_pct.map(|vat| {
                            let work = match &x.unit_pricing {
                                UnitPricing::Flat { price } => {
                                    format::decimal(lang, net(*price, vat), 4)
                                }
                                _ => "-".into(),
                            };
                            lang.ta(
                                "contract-net",
                                &[
                                    ("base", format::decimal(lang, net(x.base_price, vat), 2)),
                                    ("work", work),
                                ],
                            )
                        }),
                    }
                })
                .collect(),
        })
        .collect();
    let installments = contract
        .installments
        .iter()
        .rev()
        .map(|i| EntryView {
            id: i.id.to_string(),
            cells: vec![
                date(i.installment.valid_from),
                money(lang, i.installment.amount),
                lang.ta(
                    "contract-every-months",
                    &[("n", i.installment.every_months.to_string())],
                ),
            ],
        })
        .collect();
    let adjustments = contract
        .adjustments
        .iter()
        .rev()
        .map(|a| EntryView {
            id: a.id.to_string(),
            cells: vec![
                date(a.adjustment.booked_on),
                lang.t(&format!("adjustment-{}", a.adjustment.kind.as_str())),
                money(lang, a.adjustment.amount),
                a.note.clone().unwrap_or_default(),
            ],
        })
        .collect();
    let settlements = contract
        .settlements
        .iter()
        .rev()
        .map(|s| EntryView {
            id: s.id.to_string(),
            cells: vec![
                lang.ta(
                    "contract-period",
                    &[("start", date(s.period_start)), ("end", date(s.period_end))],
                ),
                money(lang, s.amount),
                // Beleg aus dem Rechnungsimport (SPEC_AI_INVOICE)
                s.document_id.map(|d| d.to_string()).unwrap_or_default(),
            ],
        })
        .collect();
    DetailPage {
        id: contract.id.to_string(),
        meter_id: meter.meter.id.to_string(),
        meter_name: meter.meter.data.name.clone(),
        provider: d.provider.clone(),
        facts,
        positions,
        installments,
        adjustments,
        settlements,
        intervals: interval_options(lang),
        adjustment_kinds: AdjustmentKind::ALL
            .iter()
            .map(|k| {
                (
                    k.as_str().to_owned(),
                    lang.t(&format!("adjustment-{}", k.as_str())),
                )
            })
            .collect(),
        values,
        errors,
        ctx: ctx.page.clone(),
    }
}

// @impl SPEC_TAR_CONTRACTS
pub async fn detail(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_contract_id(&id).map_err(|e| ctx.fail(e))?;
    let detail = state
        .uc
        .get_contract
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let page = detail_page(
        &ctx,
        &detail.contract,
        &detail.meter,
        HashMap::new(),
        FieldErrors::default(),
    );
    Ok(render_status(StatusCode::OK, &page))
}

/// Nach einem Eintrag zurück zur Detailseite; bei Eingabefehlern die
/// Detailseite mit Fehlern und den gesendeten Werten.
async fn after_entry(
    state: &WebState,
    ctx: &Ctx,
    id: ContractId,
    form: FormData,
    result: Result<(), AppError>,
    flash: &str,
) -> Result<Response, Response> {
    let errors = match result {
        Ok(()) => {
            ctx.flash(flash).await;
            return Ok(see_other(&format!("/contracts/{id}")));
        }
        Err(AppError::Validation(e)) => e,
        Err(AppError::Conflict(code)) => {
            let mut e = FieldErrors::default();
            e.add("entry", code);
            e
        }
        Err(e) => return Err(ctx.fail(e)),
    };
    let detail = state
        .uc
        .get_contract
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let page = detail_page(ctx, &detail.contract, &detail.meter, form.0, errors);
    Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
}

// @impl SPEC_TAR_CONTRACTS
pub async fn add_component(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, position)): Path<(String, String)>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_contract_id(&id).map_err(|e| ctx.fail(e))?;
    let position: PositionId = position.parse().map_err(|_| ctx.fail(AppError::NotFound))?;
    let form = FormData::new(pairs);
    let row = ComponentRow {
        key: "component".into(),
        id: String::new(),
        valid_from: form.get("component_valid_from").to_owned(),
        base: form.get("component_base").to_owned(),
        interval: form.get("component_interval").to_owned(),
        price: form.get("component_price").to_owned(),
        tiers: form.get("component_tiers").to_owned(),
        factor: form.get("component_factor").to_owned(),
    };
    let mut e = FieldErrors::default();
    let input = parse_component(&mut e, &row, None);
    let result = match input {
        Some(input) if e.is_empty() => {
            state
                .uc
                .add_price_component
                .execute(&ctx.tenant, id, position, input)
                .await
        }
        _ => Err(AppError::Validation(e)),
    };
    let mut form = form;
    form.0
        .insert("component_position".into(), position.to_string());
    after_entry(&state, &ctx, id, form, result, "flash-component-added").await
}

// @impl SPEC_TAR_INSTALLMENT_SETTLEMENT
pub async fn add_installment(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_contract_id(&id).map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let mut e = FieldErrors::default();
    let valid_from = required_date(
        &mut e,
        "installment_valid_from",
        form.get("installment_valid_from"),
    );
    let amount = decimal(&mut e, "installment_amount", form.get("installment_amount"));
    if amount.is_none() && e.for_field("installment_amount").is_empty() {
        e.add("installment_amount", "form.number_required");
    }
    let every = parse::integer::<u8>(form.get("installment_interval")).unwrap_or_else(|_| {
        e.add("installment_interval", "form.integer_invalid");
        None
    });
    let result = match (valid_from, amount, e.is_empty()) {
        (Some(valid_from), Some(amount), true) => {
            state
                .uc
                .set_installment
                .execute(
                    &ctx.tenant,
                    id,
                    InstallmentInput {
                        valid_from,
                        amount,
                        every_months: every.unwrap_or(1),
                    },
                )
                .await
        }
        _ => Err(AppError::Validation(e)),
    };
    after_entry(&state, &ctx, id, form, result, "flash-installment-saved").await
}

// @impl SPEC_TAR_COST_VIEW
pub async fn add_adjustment(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_contract_id(&id).map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let mut e = FieldErrors::default();
    let kind = form
        .get("adjustment_kind")
        .parse::<AdjustmentKind>()
        .unwrap_or_else(|_| {
            e.add("adjustment_kind", "form.kind_invalid");
            AdjustmentKind::Bonus
        });
    let booked_on = required_date(
        &mut e,
        "adjustment_booked_on",
        form.get("adjustment_booked_on"),
    );
    let amount = decimal(&mut e, "adjustment_amount", form.get("adjustment_amount"));
    if amount.is_none() && e.for_field("adjustment_amount").is_empty() {
        e.add("adjustment_amount", "form.number_required");
    }
    let result = match (booked_on, amount, e.is_empty()) {
        (Some(booked_on), Some(amount), true) => {
            state
                .uc
                .add_adjustment
                .execute(
                    &ctx.tenant,
                    id,
                    AdjustmentInput {
                        kind,
                        booked_on,
                        amount,
                        note: form.opt("adjustment_note"),
                    },
                )
                .await
        }
        _ => Err(AppError::Validation(e)),
    };
    after_entry(&state, &ctx, id, form, result, "flash-adjustment-saved").await
}

// @impl SPEC_TAR_ACTUAL_SETTLEMENT
pub async fn add_settlement(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_contract_id(&id).map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let mut e = FieldErrors::default();
    let start = required_date(
        &mut e,
        "settlement_period_start",
        form.get("settlement_period_start"),
    );
    let end = required_date(
        &mut e,
        "settlement_period_end",
        form.get("settlement_period_end"),
    );
    let amount = decimal(&mut e, "settlement_amount", form.get("settlement_amount"));
    if amount.is_none() && e.for_field("settlement_amount").is_empty() {
        e.add("settlement_amount", "form.number_required");
    }
    let result = match (start, end, amount, e.is_empty()) {
        (Some(period_start), Some(period_end), Some(amount), true) => {
            state
                .uc
                .record_settlement
                .execute(
                    &ctx.tenant,
                    id,
                    SettlementInput {
                        period_start,
                        period_end,
                        amount,
                    },
                )
                .await
        }
        _ => Err(AppError::Validation(e)),
    };
    after_entry(&state, &ctx, id, form, result, "flash-settlement-saved").await
}

/// Abschlag, Anpassung oder Abrechnung löschen.
pub async fn delete_entry(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, kind, entry)): Path<(String, String, String)>,
) -> Result<Response, Response> {
    let id = parse_contract_id(&id).map_err(|e| ctx.fail(e))?;
    let not_found = || ctx.fail(AppError::NotFound);
    let entry = match kind.as_str() {
        "installments" => {
            ContractEntry::Installment(InstallmentId(entry.parse().map_err(|_| not_found())?))
        }
        "adjustments" => {
            ContractEntry::Adjustment(AdjustmentId(entry.parse().map_err(|_| not_found())?))
        }
        "settlements" => {
            ContractEntry::Settlement(SettlementId(entry.parse().map_err(|_| not_found())?))
        }
        _ => return Err(not_found()),
    };
    state
        .uc
        .delete_contract_entry
        .execute(&ctx.tenant, id, entry)
        .await
        .map_err(|e| ctx.fail(e))?;
    ctx.flash("flash-entry-deleted").await;
    Ok(see_other(&format!("/contracts/{id}")))
}

/// Zeile der Vertragsliste auf der Detailseite der Messstelle.
pub struct ContractRow {
    pub id: String,
    pub provider: String,
    pub direction: String,
    pub term: String,
    pub active: bool,
}

/// Verträge der Messstelle für die Detailseite (SPEC_TAR_CONTRACTS).
#[must_use]
pub fn contract_rows(lang: Lang, contracts: &[Contract], today: NaiveDate) -> Vec<ContractRow> {
    contracts
        .iter()
        .rev()
        .map(|c| ContractRow {
            id: c.id.to_string(),
            provider: c.data.provider.clone(),
            direction: lang.t(&format!("contract-direction-{}", c.data.direction.as_str())),
            term: match c.data.ends_on {
                Some(end) => lang.ta(
                    "contract-term-bounded",
                    &[
                        ("start", format::date(lang, c.data.starts_on)),
                        ("end", format::date(lang, end)),
                    ],
                ),
                None => lang.ta(
                    "contract-term-open",
                    &[("start", format::date(lang, c.data.starts_on))],
                ),
            },
            active: c.active_on(today),
        })
        .collect()
}

/// Vorlagen als (Code, Bezeichnung) für „Vertrag anlegen“.
#[must_use]
pub fn template_links(lang: Lang, templates: &[ContractTemplate]) -> Vec<(String, String)> {
    templates
        .iter()
        .map(|t| (t.code.to_owned(), lang.t(&format!("tpl-{}", t.code))))
        .collect()
}
