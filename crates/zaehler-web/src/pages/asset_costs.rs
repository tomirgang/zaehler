//! Kostenbuch eines Fahrzeugs: Liste je Jahr mit Summen je Kategorie,
//! Anlegen, Ändern und Löschen (SPEC_ASSET_COSTS).

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
    ids::{AssetCostId, MeterId},
    model::{AssetCost, AssetRef},
    usecase::asset_cost::AssetCostInput,
};
use zaehler_domain::{
    assets::{AssetCategory, AssetType, Recurrence, VehicleCostCategory},
    calendar::Calendar,
};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format, parse},
    pages::{
        FormData,
        contracts::{date, date_input, decimal, money, plain, required_date},
        field_errors, render, render_status, see_other,
        vehicles::{parse_meter, required_decimal, warning_of},
    },
    state::WebState,
};

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
    id: String,
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

pub(crate) fn category_options(lang: Lang) -> Vec<(String, String)> {
    VehicleCostCategory::ALL
        .iter()
        .map(|c| {
            (
                c.as_str().to_owned(),
                lang.t(&format!("cost-cat-{}", c.as_str())),
            )
        })
        .collect()
}

fn row(lang: Lang, c: &AssetCost, in_year: Decimal) -> CostRow {
    let mut details = Vec::new();
    if let Some(q) = c.quantity {
        details.push(format!("{} l", format::decimal(lang, q, 2)));
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
    id: MeterId,
    year: i32,
    entry: Option<AssetCostId>,
    values: HashMap<String, String>,
    errors: FieldErrors,
    warning: Option<String>,
) -> Result<BookPage, Response> {
    let lang = ctx.page.lang;
    let fail = |e| ctx.fail(e);
    let detail = state
        .uc
        .get_vehicle
        .execute(&ctx.tenant, id)
        .await
        .map_err(fail)?;
    let book = state
        .uc
        .list_asset_costs
        .execute(&ctx.tenant, AssetRef::vehicle(id), year)
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
        id: id.to_string(),
        name: detail.meter.meter.data.name.clone(),
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
        categories: category_options(lang),
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

// @impl SPEC_ASSET_COSTS
pub async fn list(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<YearQuery>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let year = q.year.unwrap_or_else(|| this_year(&ctx));
    let page = book_page(
        &state,
        &ctx,
        id,
        year,
        None,
        HashMap::new(),
        FieldErrors::default(),
        None,
    )
    .await?;
    Ok(render(&page))
}

/// Eintrag aus den Formularfeldern.
pub(crate) fn cost_input(e: &mut FieldErrors, form: &FormData) -> Option<AssetCostInput> {
    let booked_on = required_date(e, "booked_on", form.get("booked_on"));
    let category = AssetCategory::parse(AssetType::Vehicle, form.get("category")).or_else(|| {
        e.add("category", "form.kind_invalid");
        None
    });
    let amount = required_decimal(e, "amount", form.get("amount"));
    let quantity = decimal(e, "quantity", form.get("quantity"));
    let odometer = decimal(e, "odometer", form.get("odometer"));
    let every = parse::integer::<u8>(form.get("recurrence_months")).unwrap_or_else(|_| {
        e.add("recurrence_months", "form.integer_invalid");
        None
    });
    let until = date(e, "recurrence_until", form.get("recurrence_until"));
    Some(AssetCostInput {
        booked_on: booked_on?,
        category: category?,
        amount: amount?,
        quantity,
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
    id: MeterId,
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
            return Ok(see_other(&format!("/vehicles/{id}/costs?year={year}")));
        }
        Ok(Some(w)) => (FieldErrors::default(), Some(w)),
        Err(AppError::Validation(e)) => (e, None),
        Err(e) => return Err(ctx.fail(e)),
    };
    let page = book_page(state, ctx, id, year, entry, form.0, errors, warning).await?;
    Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
}

// @impl SPEC_ASSET_COSTS
pub async fn add(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let mut e = FieldErrors::default();
    let result = match cost_input(&mut e, &form) {
        Some(input) if e.is_empty() => state
            .uc
            .add_asset_cost
            .execute(&ctx.tenant, AssetRef::vehicle(id), input)
            .await
            .map(|o| warning_of(ctx.page.lang, o)),
        _ => Err(AppError::Validation(e)),
    };
    after(&state, &ctx, id, None, form, result, "flash-cost-saved").await
}

fn parse_entry(entry: &str) -> Result<AssetCostId, AppError> {
    entry.parse().map_err(|_| AppError::NotFound)
}

/// Eintrag im Formular der Kostenbuch-Seite bearbeiten.
pub async fn edit_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let entry = parse_entry(&entry).map_err(|e| ctx.fail(e))?;
    let c = state
        .uc
        .get_asset_cost
        .execute(&ctx.tenant, entry)
        .await
        .map_err(|e| ctx.fail(e))?;
    if c.asset != AssetRef::vehicle(id) {
        return Err(ctx.fail(AppError::NotFound));
    }
    let page = book_page(
        &state,
        &ctx,
        id,
        c.booked_on.year(),
        Some(entry),
        values_of(ctx.page.lang, &c),
        FieldErrors::default(),
        None,
    )
    .await?;
    Ok(render(&page))
}

// @impl SPEC_ASSET_COSTS
pub async fn update(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let entry = parse_entry(&entry).map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let mut e = FieldErrors::default();
    let result = match cost_input(&mut e, &form) {
        Some(input) if e.is_empty() => state
            .uc
            .update_asset_cost
            .execute(&ctx.tenant, entry, input)
            .await
            .map(|()| None),
        _ => Err(AppError::Validation(e)),
    };
    after(
        &state,
        &ctx,
        id,
        Some(entry),
        form,
        result,
        "flash-cost-saved",
    )
    .await
}

pub async fn delete(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let entry = parse_entry(&entry).map_err(|e| ctx.fail(e))?;
    let c = state
        .uc
        .get_asset_cost
        .execute(&ctx.tenant, entry)
        .await
        .map_err(|e| ctx.fail(e))?;
    if c.asset != AssetRef::vehicle(id) {
        return Err(ctx.fail(AppError::NotFound));
    }
    state
        .uc
        .delete_asset_cost
        .execute(&ctx.tenant, entry)
        .await
        .map_err(|e| ctx.fail(e))?;
    ctx.flash("flash-entry-deleted").await;
    Ok(see_other(&format!(
        "/vehicles/{id}/costs?year={}",
        c.booked_on.year()
    )))
}
