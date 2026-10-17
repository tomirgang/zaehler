//! Belegerkennung: Vorschlag prüfen und als Ladevorgang oder
//! Kostenbuch-Eintrag übernehmen (SPEC_AI_RECEIPT).

use std::collections::HashMap;

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};
use chrono::Utc;
use rust_decimal::Decimal;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{DocumentId, MeterId},
    model::{Document, DocumentStatus, ReceiptExtraction},
    usecase::document::ReceiptEntry,
};
use zaehler_domain::{assets::VehicleCostCategory, calendar::Calendar};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format},
    pages::{
        FormData,
        asset_costs::{category_options, cost_input},
        contracts::{date_input, money, plain},
        field_errors, render, render_status, see_other,
        vehicles::{charging_input, kind_options, noon_input, prefixed, warning_of},
    },
    state::WebState,
};

pub struct SummaryView {
    pub date: String,
    pub amount: String,
    pub category: String,
    pub quantity: String,
    pub vendor: String,
    pub odometer: String,
    pub confidence: String,
}

#[derive(Template)]
#[template(path = "pages/documents/receipt.html")]
struct ReceiptPage {
    ctx: PageCtx,
    doc_id: String,
    doc_name: String,
    status: String,
    open: bool,
    failed: bool,
    ai_enabled: bool,
    summary: Option<SummaryView>,
    /// (ID, Bezeichnung) der Fahrzeuge
    vehicles: Vec<(String, String)>,
    categories: Vec<(String, String)>,
    kinds: Vec<(String, String)>,
    values: HashMap<String, String>,
    errors: FieldErrors,
    warning: Option<String>,
    /// Alle Fehler als Text (auch solche ohne eigenes Feld)
    messages: Vec<String>,
}

impl ReceiptPage {
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

fn summary(lang: Lang, x: &ReceiptExtraction) -> SummaryView {
    let dash = || lang.t("capture-not-recognized");
    SummaryView {
        date: x.date.map_or_else(dash, |d| format::date(lang, d)),
        amount: x.total_amount.map_or_else(dash, |v| money(lang, v)),
        category: x
            .category
            .map_or_else(dash, |c| lang.t(&format!("cost-cat-{}", c.as_str()))),
        quantity: x.quantity.map_or_else(dash, |q| {
            format!(
                "{} {}",
                format::decimal_plain(lang, q),
                x.unit.clone().unwrap_or_default()
            )
        }),
        vendor: x.vendor.clone().unwrap_or_else(dash),
        odometer: x
            .odometer
            .map_or_else(dash, |o| format!("{} km", format::decimal(lang, o, 0))),
        confidence: format::decimal(lang, x.confidence * Decimal::ONE_HUNDRED, 0),
    }
}

/// Formularwerte aus dem Vorschlag.
fn values_of(
    lang: Lang,
    x: Option<&ReceiptExtraction>,
    today: chrono::NaiveDate,
) -> HashMap<String, String> {
    let mut v = HashMap::new();
    let day = x.and_then(|x| x.date).unwrap_or(today);
    let p = |d: Option<Decimal>| d.map(|d| plain(lang, d)).unwrap_or_default();
    let charging = x.is_some_and(ReceiptExtraction::is_charging);
    v.insert(
        "entry".into(),
        if charging { "charging" } else { "cost" }.into(),
    );
    v.insert("charging_at".into(), noon_input(day));
    v.insert("booked_on".into(), date_input(day));
    let amount = p(x.and_then(|x| x.total_amount));
    v.insert("charging_amount".into(), amount.clone());
    v.insert("amount".into(), amount);
    let quantity = p(x.and_then(|x| x.quantity));
    v.insert("charging_energy".into(), quantity.clone());
    let category = x
        .and_then(|x| x.category)
        .filter(|c| *c != VehicleCostCategory::Charging)
        .unwrap_or(VehicleCostCategory::Other);
    if category == VehicleCostCategory::Fuel {
        v.insert("quantity".into(), quantity);
        v.insert("full_tank".into(), "1".into());
    }
    v.insert("category".into(), category.as_str().into());
    let odometer = p(x.and_then(|x| x.odometer));
    v.insert("charging_odometer".into(), odometer.clone());
    v.insert("odometer".into(), odometer);
    let vendor = x.and_then(|x| x.vendor.clone()).unwrap_or_default();
    v.insert("charging_provider".into(), vendor.clone());
    v.insert("note".into(), vendor);
    v.insert(
        "charging_kind".into(),
        x.and_then(|x| x.charging_kind.clone())
            .unwrap_or_else(|| "ac".into()),
    );
    v
}

async fn page(
    state: &WebState,
    ctx: &Ctx,
    doc: &Document,
    values: Option<HashMap<String, String>>,
    errors: FieldErrors,
    warning: Option<String>,
) -> Result<ReceiptPage, Response> {
    let lang = ctx.page.lang;
    let vehicles: Vec<(String, String)> = state
        .uc
        .list_vehicles
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?
        .into_iter()
        .filter(|v| v.vehicle.is_some() && v.meter.archived_at.is_none())
        .map(|v| (v.meter.id.to_string(), v.meter.data.name))
        .collect();
    let today = Calendar::new(ctx.page.tz).local_date(Utc::now());
    let mut values = values.unwrap_or_else(|| values_of(lang, doc.receipt.as_ref(), today));
    // genau ein Fahrzeug: vorbelegt
    if !values.contains_key("vehicle")
        && let [(id, _)] = vehicles.as_slice()
    {
        values.insert("vehicle".into(), id.clone());
    }
    Ok(ReceiptPage {
        doc_id: doc.id.to_string(),
        doc_name: doc.original_name.clone(),
        status: lang.t(&format!("document-status-{}", doc.status.as_str())),
        open: doc.status.is_open(),
        failed: doc.status == DocumentStatus::Failed,
        ai_enabled: state.uc.analyse_document.ai_enabled(),
        summary: doc.receipt.as_ref().map(|x| summary(lang, x)),
        vehicles,
        categories: category_options(lang),
        kinds: kind_options(),
        messages: errors.0.iter().map(|e| lang.err(e)).collect(),
        values,
        errors,
        warning,
        ctx: ctx.page.clone(),
    })
}

/// Prüfseite eines Belegs.
// @impl SPEC_AI_RECEIPT
pub(crate) async fn show(state: &WebState, ctx: &Ctx, doc: Document) -> Result<Response, Response> {
    Ok(render(
        &page(state, ctx, &doc, None, FieldErrors::default(), None).await?,
    ))
}

/// Übernahme als Ladevorgang oder Kostenbuch-Eintrag.
// @impl SPEC_AI_RECEIPT
pub async fn confirm(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id: DocumentId = id.parse().map_err(|_| ctx.fail(AppError::NotFound))?;
    let doc = state
        .uc
        .get_document
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let mut e = FieldErrors::default();
    let vehicle = form.get("vehicle").parse::<MeterId>().map_err(|_| {
        e.add("vehicle", "receipt.vehicle_required");
    });
    let charging = form.get("entry") == "charging";
    let entry = if charging {
        charging_input(&mut e, &form, Calendar::new(ctx.page.tz)).map(ReceiptEntry::Charging)
    } else {
        cost_input(&mut e, &form).map(ReceiptEntry::Cost)
    };
    let result = match (vehicle, entry, e.is_empty()) {
        (Ok(vehicle), Some(entry), true) => state
            .uc
            .confirm_receipt
            .execute(&ctx.tenant, id, vehicle, entry)
            .await
            .map(|o| (vehicle, warning_of(ctx.page.lang, o))),
        _ => Err(AppError::Validation(e)),
    };
    let (errors, warning) = match result {
        Ok((vehicle, None)) => {
            ctx.flash("flash-receipt-saved").await;
            let to = if charging {
                format!("/vehicles/{vehicle}")
            } else {
                format!("/vehicles/{vehicle}/costs")
            };
            return Ok(see_other(&to));
        }
        Ok((_, Some(w))) => (FieldErrors::default(), Some(w)),
        Err(AppError::Validation(e)) if charging => (prefixed(e, "charging_"), None),
        Err(AppError::Validation(e)) => (e, None),
        Err(e) => return Err(ctx.fail(e)),
    };
    let page = page(&state, &ctx, &doc, Some(form.0), errors, warning).await?;
    Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
}
