//! Erfassung per Formular mit Plausibilitätshinweisen (SPEC_RDG_FORM).

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};
use chrono::Utc;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{MeterId, ReadingId},
    usecase::{
        meter::MeterDetail,
        reading::{ReadingsInput, RecordOutcome, RegisterCheck},
    },
};
use zaehler_domain::{
    calendar::Calendar,
    reading::{Plausibility, ReadingSource},
};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format, parse},
    pages::{FormData, field_errors, render, render_status, see_other},
    state::WebState,
};

pub struct ValueRow {
    pub register_id: String,
    pub code: String,
    pub direction: String,
    pub last: String,
    pub value: String,
    pub hint: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/readings/form.html")]
struct ReadingFormPage {
    ctx: PageCtx,
    meter_id: String,
    meter_name: String,
    unit: String,
    at: String,
    note: String,
    rows: Vec<ValueRow>,
    needs_confirm: bool,
    errors: FieldErrors,
}

impl ReadingFormPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn has_errs(&self, field: &str) -> bool {
        !self.errors.for_field(field).is_empty()
    }

    fn value_errs(&self, register_id: &str) -> Vec<String> {
        self.errs(&format!("value_{register_id}"))
    }
}

/// Fragment mit Hinweisen je Register (Antwort auf `readings/check`).
#[derive(Template)]
#[template(path = "partials/plausibility.html")]
struct PlausibilityFragment {
    hints: Vec<(String, String)>,
    errors: Vec<String>,
}

fn hint(lang: Lang, p: &Plausibility) -> Option<String> {
    match p {
        Plausibility::Ok => None,
        Plausibility::Decreasing => Some(lang.t("plaus-decreasing")),
        Plausibility::RateTooHigh => Some(lang.t("plaus-rate_too_high")),
        Plausibility::OutOfOrder => Some(lang.t("plaus-out_of_order")),
    }
}

fn rows(
    lang: Lang,
    tz: chrono_tz::Tz,
    detail: &MeterDetail,
    form: Option<&FormData>,
    checks: &[RegisterCheck],
) -> Vec<ValueRow> {
    let agg = &detail.aggregate;
    let device = agg.active_device();
    agg.registers
        .iter()
        .map(|r| {
            let last = device
                .and_then(|d| {
                    detail
                        .readings
                        .iter()
                        .filter(|x| x.register_id == r.id && x.device_id == d.id)
                        .max_by_key(|x| x.read_at)
                        .map(|x| {
                            lang.ta(
                                "readings-last",
                                &[
                                    (
                                        "value",
                                        format::decimal(lang, x.value, d.data.decimal_places),
                                    ),
                                    ("date", format::datetime_in(lang, x.read_at, tz)),
                                ],
                            )
                        })
                })
                .unwrap_or_else(|| lang.t("readings-none"));
            ValueRow {
                register_id: r.id.to_string(),
                code: r.data.code.clone(),
                direction: lang.t(&format!("direction-{}", r.data.direction.as_str())),
                last,
                value: form
                    .map(|f| f.get(&format!("value_{}", r.id)).to_owned())
                    .unwrap_or_default(),
                hint: checks
                    .iter()
                    .find(|c| c.register_id == r.id)
                    .and_then(|c| hint(lang, &c.plausibility)),
            }
        })
        .collect()
}

fn parse_meter(id: &str) -> Result<MeterId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

/// Formular in Eingaben des Kerns umsetzen.
fn parse_input(
    form: &FormData,
    detail: &MeterDetail,
    meter_id: MeterId,
    cal: Calendar,
) -> Result<ReadingsInput, FieldErrors> {
    let mut e = FieldErrors::default();
    let at = parse::datetime_local(form.get("at")).map_or_else(
        |_| {
            e.add("at", "form.datetime_invalid");
            Utc::now()
        },
        |l| cal.to_utc(l),
    );
    let mut values = Vec::new();
    for r in &detail.aggregate.registers {
        let field = format!("value_{}", r.id);
        match parse::decimal(form.get(&field)) {
            Ok(Some(v)) => values.push((r.id, v)),
            Ok(None) => {}
            Err(_) => e.add(field, "form.number_invalid"),
        }
    }
    if e.is_empty() {
        Ok(ReadingsInput {
            meter_id,
            at,
            values,
            note: form.opt("note"),
        })
    } else {
        Err(e)
    }
}

pub async fn new_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let meter_id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let detail = state
        .uc
        .get_meter
        .execute(&ctx.tenant, meter_id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let page = ReadingFormPage {
        meter_id: meter_id.to_string(),
        meter_name: detail.aggregate.meter.data.name.clone(),
        unit: detail.aggregate.meter.data.unit.clone(),
        at: format::datetime_input(Utc::now().with_timezone(&ctx.page.tz).naive_local()),
        note: String::new(),
        rows: rows(ctx.page.lang, ctx.page.tz, &detail, None, &[]),
        needs_confirm: false,
        errors: FieldErrors::default(),
        ctx: ctx.page,
    };
    Ok(render(&page))
}

fn form_page(
    ctx: &Ctx,
    detail: &MeterDetail,
    form: &FormData,
    checks: &[RegisterCheck],
    errors: FieldErrors,
    needs_confirm: bool,
) -> Response {
    let page = ReadingFormPage {
        meter_id: detail.aggregate.meter.id.to_string(),
        meter_name: detail.aggregate.meter.data.name.clone(),
        unit: detail.aggregate.meter.data.unit.clone(),
        at: form.get("at").to_owned(),
        note: form.get("note").to_owned(),
        rows: rows(ctx.page.lang, ctx.page.tz, detail, Some(form), checks),
        needs_confirm,
        errors,
        ctx: ctx.page.clone(),
    };
    render_status(StatusCode::UNPROCESSABLE_ENTITY, &page)
}

/// Speichern; bei Warnungen erneut mit Bestätigungsfeld.
// @impl SPEC_RDG_FORM
pub async fn record(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let meter_id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let detail = state
        .uc
        .get_meter
        .execute(&ctx.tenant, meter_id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let input = match parse_input(&form, &detail, meter_id, Calendar::new(ctx.page.tz)) {
        Ok(i) => i,
        Err(errors) => return Ok(form_page(&ctx, &detail, &form, &[], errors, false)),
    };
    let confirmed = form.get("confirm_warnings") == "true";
    match state
        .uc
        .record_readings
        .execute(&ctx.tenant, &input, confirmed, ReadingSource::Manual)
        .await
    {
        Ok(RecordOutcome::Saved(_)) => {
            ctx.flash("flash-readings-saved").await;
            Ok(see_other(&format!("/meters/{meter_id}")))
        }
        Ok(RecordOutcome::NeedsConfirmation(checks)) => Ok(form_page(
            &ctx,
            &detail,
            &form,
            &checks,
            FieldErrors::default(),
            true,
        )),
        Err(AppError::Validation(errors)) => {
            Ok(form_page(&ctx, &detail, &form, &[], errors, false))
        }
        Err(e) => Err(ctx.fail(e)),
    }
}

/// Plausibilität prüfen, ohne zu speichern (HTMX-Fragment).
// @impl SPEC_RDG_FORM
pub async fn check(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let meter_id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let detail = state
        .uc
        .get_meter
        .execute(&ctx.tenant, meter_id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let lang = ctx.page.lang;
    let codes = |checks: &[RegisterCheck]| -> Vec<(String, String)> {
        checks
            .iter()
            .filter_map(|c| {
                let code = detail.aggregate.register(c.register_id)?.data.code.clone();
                hint(lang, &c.plausibility).map(|h| (code, h))
            })
            .collect()
    };
    let fragment = match parse_input(&form, &detail, meter_id, Calendar::new(ctx.page.tz)) {
        Err(errors) => PlausibilityFragment {
            hints: Vec::new(),
            errors: errors.0.iter().map(|e| lang.err(e)).collect(),
        },
        Ok(input) if input.values.is_empty() => PlausibilityFragment {
            hints: Vec::new(),
            errors: Vec::new(),
        },
        Ok(input) => match state.uc.check_readings.execute(&ctx.tenant, &input).await {
            Ok(checks) => PlausibilityFragment {
                hints: codes(&checks),
                errors: Vec::new(),
            },
            Err(AppError::Validation(errors)) => PlausibilityFragment {
                hints: Vec::new(),
                errors: errors.0.iter().map(|e| lang.err(e)).collect(),
            },
            Err(e) => return Err(ctx.fail(e)),
        },
    };
    Ok(render(&fragment))
}

pub async fn delete(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let reading_id: ReadingId = id.parse().map_err(|_| ctx.fail(AppError::NotFound))?;
    let meter_id = state
        .uc
        .delete_reading
        .execute(&ctx.tenant, reading_id)
        .await
        .map_err(|e| ctx.fail(e))?;
    ctx.flash("flash-reading-deleted").await;
    Ok(see_other(&format!("/meters/{meter_id}")))
}
