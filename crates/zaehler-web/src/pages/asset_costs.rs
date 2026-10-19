//! Kostenbuch eines Fahrzeugs oder einer PV-Anlage: Liste je Jahr mit
//! Summen je Kategorie, Anlegen, Ändern und Löschen (SPEC_ASSET_COSTS,
//! ADR_020).

use std::collections::HashMap;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
};
use chrono::{Datelike, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{AssetCostId, MeterId, PvSystemId},
    model::{AssetCost, AssetRef},
    usecase::asset_cost::AssetCostInput,
};
use zaehler_domain::{
    assets::{AssetCategory, AssetType, Recurrence},
    calendar::Calendar,
};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format, parse},
    pages::{
        FormData,
        contracts::{date, date_input, decimal, money, plain, required_date},
        field_errors, render, render_status, see_other,
        vehicles::{parse_meter, warning_of},
    },
    state::WebState,
};

/// Anlage eines Kostenbuchs.
#[derive(Debug, Clone, Copy)]
enum Owner {
    Vehicle(MeterId),
    Pv(PvSystemId),
}

impl Owner {
    fn asset(self) -> AssetRef {
        match self {
            Self::Vehicle(m) => AssetRef::vehicle(m),
            Self::Pv(p) => AssetRef::pv(p),
        }
    }

    fn asset_type(self) -> AssetType {
        self.asset().asset_type
    }

    /// Pfad der Anlagenseite.
    fn base(self) -> String {
        match self {
            Self::Vehicle(m) => format!("/vehicles/{m}"),
            Self::Pv(p) => format!("/pv/{p}"),
        }
    }

    async fn name(self, state: &WebState, ctx: &Ctx) -> Result<String, Response> {
        match self {
            Self::Vehicle(m) => state
                .uc
                .get_vehicle
                .execute(&ctx.tenant, m)
                .await
                .map(|d| d.meter.meter.data.name)
                .map_err(|e| ctx.fail(e)),
            Self::Pv(p) => state
                .uc
                .get_pv_system
                .execute(&ctx.tenant, p)
                .await
                .map(|s| s.data.name)
                .map_err(|e| ctx.fail(e)),
        }
    }
}

fn vehicle(id: &str) -> Result<Owner, AppError> {
    parse_meter(id).map(Owner::Vehicle)
}

fn pv(id: &str) -> Result<Owner, AppError> {
    id.parse().map(Owner::Pv).map_err(|_| AppError::NotFound)
}

pub struct CostRow {
    pub id: String,
    pub date: String,
    pub category: String,
    pub amount: String,
    pub year_amount: String,
    pub details: String,
    pub recurring: String,
    pub document: String,
}

#[derive(Template)]
#[template(path = "pages/vehicles/costs.html")]
struct BookPage {
    ctx: PageCtx,
    /// Pfad der Anlagenseite
    base: String,
    is_vehicle: bool,
    name: String,
    year: i32,
    rows: Vec<CostRow>,
    totals: Vec<(String, String)>,
    total: String,
    categories: Vec<(String, String)>,
    /// Bearbeiteter Eintrag
    entry: Option<String>,
    values: HashMap<String, String>,
    errors: FieldErrors,
    warning: Option<String>,
}

impl BookPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn val(&self, field: &str) -> String {
        self.values.get(field).cloned().unwrap_or_default()
    }

    fn checked(&self, field: &str) -> bool {
        !self.val(field).is_empty()
    }
}

fn options(lang: Lang, asset: AssetType) -> Vec<(String, String)> {
    AssetCategory::all(asset)
        .into_iter()
        .map(|c| {
            (
                c.as_str().to_owned(),
                lang.t(&format!("cost-cat-{}", c.as_str())),
            )
        })
        .collect()
}

/// Kategorien eines Fahrzeugs (Belegerkennung).
pub(crate) fn category_options(lang: Lang) -> Vec<(String, String)> {
    options(lang, AssetType::Vehicle)
}

fn row(lang: Lang, c: &AssetCost, in_year: Decimal) -> CostRow {
    let mut details = Vec::new();
    if let Some(q) = c.quantity {
        details.push(format!("{} l", format::decimal(lang, q, 2)));
    }
    if let Some(price) = c.fuel_unit_price() {
        details.push(format!("{} €/l", format::decimal(lang, price, 3)));
    }
    if let Some(o) = c.odometer {
        details.push(format!("{} km", format::decimal(lang, o, 0)));
    }
    if c.full_tank == Some(true) {
        details.push(lang.t("asset-full-tank"));
    }
    if let Some(n) = &c.note {
        details.push(n.clone());
    }
    CostRow {
        id: c.id.to_string(),
        date: format::date(lang, c.booked_on),
        category: lang.t(&format!("cost-cat-{}", c.category.as_str())),
        amount: money(lang, c.amount),
        year_amount: money(lang, in_year),
        details: details.join(", "),
        recurring: c
            .recurrence
            .map(|r| {
                let every = lang.ta(
                    "contract-every-months",
                    &[("n", r.every_months.to_string())],
                );
                match r.until {
                    Some(u) => format!(
                        "{every}, {}",
                        lang.ta("asset-until", &[("date", format::date(lang, u))])
                    ),
                    None => every,
                }
            })
            .unwrap_or_default(),
        document: c.document_id.map(|d| d.to_string()).unwrap_or_default(),
    }
}

fn values_of(lang: Lang, c: &AssetCost) -> HashMap<String, String> {
    let opt = |x: Option<Decimal>| x.map(|x| plain(lang, x)).unwrap_or_default();
    [
        ("booked_on", date_input(c.booked_on)),
        ("category", c.category.as_str().to_owned()),
        ("amount", plain(lang, c.amount)),
        ("quantity", opt(c.quantity)),
        ("unit_price", opt(c.fuel_unit_price())),
        ("odometer", opt(c.odometer)),
        (
            "full_tank",
            if c.full_tank == Some(true) {
                "1".into()
            } else {
                String::new()
            },
        ),
        (
            "recurrence_months",
            c.recurrence
                .map(|r| r.every_months.to_string())
                .unwrap_or_default(),
        ),
        (
            "recurrence_until",
            c.recurrence
                .and_then(|r| r.until)
                .map(date_input)
                .unwrap_or_default(),
        ),
        ("note", c.note.clone().unwrap_or_default()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

#[allow(clippy::too_many_arguments)] // Bestandteile der Seite
async fn book_page(
    state: &WebState,
    ctx: &Ctx,
    owner: Owner,
    year: i32,
    entry: Option<AssetCostId>,
    values: HashMap<String, String>,
    errors: FieldErrors,
    warning: Option<String>,
) -> Result<BookPage, Response> {
    let lang = ctx.page.lang;
    let name = owner.name(state, ctx).await?;
    let book = state
        .uc
        .list_asset_costs
        .execute(&ctx.tenant, owner.asset(), year)
        .await
        .map_err(|e| ctx.fail(e))?;
    let mut values = values;
    if values.is_empty() {
        values.insert(
            "booked_on".into(),
            date_input(Calendar::new(ctx.page.tz).local_date(Utc::now())),
        );
    }
    Ok(BookPage {
        base: owner.base(),
        is_vehicle: matches!(owner, Owner::Vehicle(_)),
        name,
        year,
        rows: book
            .entries
            .iter()
            .rev()
            .map(|(c, v)| row(lang, c, *v))
            .collect(),
        totals: book
            .totals
            .iter()
            .map(|(c, v)| (lang.t(&format!("cost-cat-{}", c.as_str())), money(lang, *v)))
            .collect(),
        total: money(lang, book.total),
        categories: options(lang, owner.asset_type()),
        entry: entry.map(|e| e.to_string()),
        values,
        errors,
        warning,
        ctx: ctx.page.clone(),
    })
}

#[derive(Deserialize)]
pub struct YearQuery {
    year: Option<i32>,
}

fn this_year(ctx: &Ctx) -> i32 {
    Calendar::new(ctx.page.tz).local_date(Utc::now()).year()
}

/// Eintrag eines Fahrzeugs aus den Formularfeldern (auch Belegerkennung).
pub(crate) fn cost_input(e: &mut FieldErrors, form: &FormData) -> Option<AssetCostInput> {
    cost_input_for(e, form, AssetType::Vehicle)
}

fn cost_input_for(
    e: &mut FieldErrors,
    form: &FormData,
    asset: AssetType,
) -> Option<AssetCostInput> {
    let booked_on = required_date(e, "booked_on", form.get("booked_on"));
    let category = AssetCategory::parse(asset, form.get("category")).or_else(|| {
        e.add("category", "form.kind_invalid");
        None
    });
    let amount = decimal(e, "amount", form.get("amount"));
    let quantity = decimal(e, "quantity", form.get("quantity"));
    let unit_price = decimal(e, "unit_price", form.get("unit_price"));
    let odometer = decimal(e, "odometer", form.get("odometer"));
    let every = parse::integer::<u8>(form.get("recurrence_months")).unwrap_or_else(|_| {
        e.add("recurrence_months", "form.integer_invalid");
        None
    });
    let until = date(e, "recurrence_until", form.get("recurrence_until"));
    Some(AssetCostInput {
        booked_on: booked_on?,
        category: category?,
        amount,
        quantity,
        unit_price,
        odometer,
        full_tank: form.has("full_tank"),
        recurrence: every.map(|every_months| Recurrence {
            every_months,
            until,
        }),
        note: form.opt("note"),
        odometer_as_reading: form.has("odometer_reading"),
        confirm_warnings: form.get("confirm_warnings") == "true",
    })
}

async fn after(
    state: &WebState,
    ctx: &Ctx,
    owner: Owner,
    entry: Option<AssetCostId>,
    form: FormData,
    result: Result<Option<String>, AppError>,
    flash: &str,
) -> Result<Response, Response> {
    let year = form
        .get("booked_on")
        .get(..4)
        .and_then(|y| y.parse().ok())
        .unwrap_or_else(|| this_year(ctx));
    let (errors, warning) = match result {
        Ok(None) => {
            ctx.flash(flash).await;
            return Ok(see_other(&format!("{}/costs?year={year}", owner.base())));
        }
        Ok(Some(w)) => (FieldErrors::default(), Some(w)),
        Err(AppError::Validation(e)) => (e, None),
        Err(e) => return Err(ctx.fail(e)),
    };
    let page = book_page(state, ctx, owner, year, entry, form.0, errors, warning).await?;
    Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
}

fn parse_entry(entry: &str) -> Result<AssetCostId, AppError> {
    entry.parse().map_err(|_| AppError::NotFound)
}

/// Eintrag der Anlage.
async fn owned(
    state: &WebState,
    ctx: &Ctx,
    owner: Owner,
    entry: &str,
) -> Result<AssetCost, Response> {
    let entry = parse_entry(entry).map_err(|e| ctx.fail(e))?;
    let c = state
        .uc
        .get_asset_cost
        .execute(&ctx.tenant, entry)
        .await
        .map_err(|e| ctx.fail(e))?;
    if c.asset != owner.asset() {
        return Err(ctx.fail(AppError::NotFound));
    }
    Ok(c)
}

// @impl SPEC_ASSET_COSTS
async fn list_for(
    state: WebState,
    ctx: Ctx,
    owner: Owner,
    q: YearQuery,
) -> Result<Response, Response> {
    let year = q.year.unwrap_or_else(|| this_year(&ctx));
    let page = book_page(
        &state,
        &ctx,
        owner,
        year,
        None,
        HashMap::new(),
        FieldErrors::default(),
        None,
    )
    .await?;
    Ok(render(&page))
}

// @impl SPEC_ASSET_COSTS
async fn add_for(
    state: WebState,
    ctx: Ctx,
    owner: Owner,
    pairs: Vec<(String, String)>,
) -> Result<Response, Response> {
    let form = FormData::new(pairs);
    let mut e = FieldErrors::default();
    let result = match cost_input_for(&mut e, &form, owner.asset_type()) {
        Some(input) if e.is_empty() => state
            .uc
            .add_asset_cost
            .execute(&ctx.tenant, owner.asset(), input)
            .await
            .map(|o| warning_of(ctx.page.lang, o)),
        _ => Err(AppError::Validation(e)),
    };
    after(&state, &ctx, owner, None, form, result, "flash-cost-saved").await
}

async fn edit_for(
    state: WebState,
    ctx: Ctx,
    owner: Owner,
    entry: String,
) -> Result<Response, Response> {
    let c = owned(&state, &ctx, owner, &entry).await?;
    let page = book_page(
        &state,
        &ctx,
        owner,
        c.booked_on.year(),
        Some(c.id),
        values_of(ctx.page.lang, &c),
        FieldErrors::default(),
        None,
    )
    .await?;
    Ok(render(&page))
}

// @impl SPEC_ASSET_COSTS
async fn update_for(
    state: WebState,
    ctx: Ctx,
    owner: Owner,
    entry: String,
    pairs: Vec<(String, String)>,
) -> Result<Response, Response> {
    let c = owned(&state, &ctx, owner, &entry).await?;
    let form = FormData::new(pairs);
    let mut e = FieldErrors::default();
    let result = match cost_input_for(&mut e, &form, owner.asset_type()) {
        Some(input) if e.is_empty() => state
            .uc
            .update_asset_cost
            .execute(&ctx.tenant, c.id, input)
            .await
            .map(|()| None),
        _ => Err(AppError::Validation(e)),
    };
    after(
        &state,
        &ctx,
        owner,
        Some(c.id),
        form,
        result,
        "flash-cost-saved",
    )
    .await
}

async fn delete_for(
    state: WebState,
    ctx: Ctx,
    owner: Owner,
    entry: String,
) -> Result<Response, Response> {
    let c = owned(&state, &ctx, owner, &entry).await?;
    state
        .uc
        .delete_asset_cost
        .execute(&ctx.tenant, c.id)
        .await
        .map_err(|e| ctx.fail(e))?;
    ctx.flash("flash-entry-deleted").await;
    Ok(see_other(&format!(
        "{}/costs?year={}",
        owner.base(),
        c.booked_on.year()
    )))
}

// --- Fahrzeuge -------------------------------------------------------------

pub async fn list(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<YearQuery>,
) -> Result<Response, Response> {
    let owner = vehicle(&id).map_err(|e| ctx.fail(e))?;
    list_for(state, ctx, owner, q).await
}

pub async fn add(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let owner = vehicle(&id).map_err(|e| ctx.fail(e))?;
    add_for(state, ctx, owner, pairs).await
}

pub async fn edit_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
) -> Result<Response, Response> {
    let owner = vehicle(&id).map_err(|e| ctx.fail(e))?;
    edit_for(state, ctx, owner, entry).await
}

pub async fn update(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let owner = vehicle(&id).map_err(|e| ctx.fail(e))?;
    update_for(state, ctx, owner, entry, pairs).await
}

pub async fn delete(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
) -> Result<Response, Response> {
    let owner = vehicle(&id).map_err(|e| ctx.fail(e))?;
    delete_for(state, ctx, owner, entry).await
}

// --- PV-Anlagen ------------------------------------------------------------

pub async fn pv_list(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<YearQuery>,
) -> Result<Response, Response> {
    let owner = pv(&id).map_err(|e| ctx.fail(e))?;
    list_for(state, ctx, owner, q).await
}

pub async fn pv_add(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let owner = pv(&id).map_err(|e| ctx.fail(e))?;
    add_for(state, ctx, owner, pairs).await
}

pub async fn pv_edit_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
) -> Result<Response, Response> {
    let owner = pv(&id).map_err(|e| ctx.fail(e))?;
    edit_for(state, ctx, owner, entry).await
}

pub async fn pv_update(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let owner = pv(&id).map_err(|e| ctx.fail(e))?;
    update_for(state, ctx, owner, entry, pairs).await
}

pub async fn pv_delete(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
) -> Result<Response, Response> {
    let owner = pv(&id).map_err(|e| ctx.fail(e))?;
    delete_for(state, ctx, owner, entry).await
}
