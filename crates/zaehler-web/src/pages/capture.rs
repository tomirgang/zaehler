//! Foto-Erfassung: Upload, Vorschlag bestätigen oder verwerfen
//! (SPEC_AI_FLOW, SPEC_AI_DRAFTS).

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde::Deserialize;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{DraftId, MeterId, RegisterId},
    model::DraftStatus,
    usecase::{
        capture::{DraftView, MAX_PHOTOS},
        reading::{ReadingsInput, RecordOutcome},
    },
};
use zaehler_domain::{calendar::Calendar, reading::Plausibility};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format, parse},
    pages::{FormData, field_errors, readings::hint, render, render_status, see_other},
    security::constant_time_eq,
    state::WebState,
};

#[derive(Template)]
#[template(path = "pages/capture/upload.html")]
struct UploadPage {
    ctx: PageCtx,
    ai_enabled: bool,
    photo_hint: String,
    /// Ältester offener Entwurf (SPEC_AI_MULTI)
    open_draft: Option<String>,
    errors: Vec<String>,
}

pub struct SuggestionView {
    pub value: String,
    pub serial: String,
    pub register: String,
    pub unit: String,
    pub confidence: String,
}

pub struct Choice {
    pub id: String,
    pub label: String,
    pub selected: bool,
}

pub struct MeterView {
    pub id: String,
    pub name: String,
    pub unit: String,
    pub registers: Vec<Choice>,
}

#[derive(Template)]
#[template(path = "pages/capture/confirm.html")]
struct ConfirmPage {
    ctx: PageCtx,
    draft_id: String,
    failed: bool,
    suggestion: Option<SuggestionView>,
    meters: Vec<Choice>,
    meter: Option<MeterView>,
    value: String,
    at: String,
    /// Zeitpunkt stammt aus den Bildmetadaten
    at_from_photo: bool,
    note: String,
    hint: Option<String>,
    needs_confirm: bool,
    /// Position in der Warteschlange
    progress: Option<String>,
    errors: FieldErrors,
}

impl ConfirmPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn has_errs(&self, field: &str) -> bool {
        !self.errors.for_field(field).is_empty()
    }
}

fn upload_page(ctx: &Ctx, state: &WebState, errors: Vec<String>) -> UploadPage {
    UploadPage {
        ctx: ctx.page.clone(),
        ai_enabled: state.uc.analyse_photo.ai_enabled(),
        photo_hint: ctx.page.lang.ta(
            "capture-photo-hint",
            &[
                (
                    "max",
                    state.cfg.max_photo_bytes.div_ceil(1024 * 1024).to_string(),
                ),
                ("count", MAX_PHOTOS.to_string()),
            ],
        ),
        open_draft: None,
        errors,
    }
}

/// Upload-Seite mit Hinweis auf noch offene Fotos.
// @impl SPEC_AI_FLOW, SPEC_AI_MULTI
pub async fn upload_form(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    if !ctx.page.can_write {
        return Err(ctx.fail(AppError::Forbidden));
    }
    let mut page = upload_page(&ctx, &state, Vec::new());
    page.open_draft = state
        .uc
        .get_draft
        .next_open(&ctx.tenant, DraftId(uuid::Uuid::nil()))
        .await
        .map_err(|e| ctx.fail(e))?
        .map(|d| d.to_string());
    Ok(render(&page))
}

/// Fehlertext zu einem Foto mit Position und Dateiname.
fn photo_error(lang: Lang, index: usize, name: &str, message: &str) -> String {
    lang.ta(
        "capture-photo-error",
        &[
            ("n", (index + 1).to_string()),
            ("name", name.to_owned()),
            ("error", message.to_owned()),
        ],
    )
}

/// Ein oder mehrere Fotos hochladen; sie werden als wartende Entwürfe
/// gespeichert und im Hintergrund nacheinander analysiert. Danach 303 zum
/// ersten Foto. Das CSRF-Token steht im Feld `_csrf` des Multipart-Formulars
/// (SPEC_SEC_SESSION_CSRF).
// @impl SPEC_AI_FLOW, SPEC_AI_MULTI, SPEC_SEC_UPLOADS, SPEC_SEC_SESSION_CSRF
pub async fn analyse(
    State(state): State<WebState>,
    ctx: Ctx,
    mut multipart: Multipart,
) -> Result<Response, Response> {
    let lang = ctx.page.lang;
    let mut token = None;
    let mut grid = None;
    let mut photos = Vec::new();
    // Position im Upload und Dateiname je angenommenem Foto
    let mut origin: Vec<(usize, String)> = Vec::new();
    let mut errors = Vec::new();
    let mut position = 0;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return Err(upload_error(&ctx, &state, e.status())),
        };
        match field.name() {
            Some("_csrf") => token = field.text().await.ok(),
            // Rückkehr zur Sammelerfassung eines Hausanschlusses (SPEC_PV_BULK_CAPTURE)
            Some("grid") => {
                grid = field
                    .text()
                    .await
                    .ok()
                    .and_then(|g| g.parse::<MeterId>().ok());
            }
            Some("photo") => {
                let name = field.file_name().unwrap_or_default().to_owned();
                match field.bytes().await {
                    Ok(b) if b.is_empty() => {}
                    // einzelnes Foto über der Grenze: nur dieses ablehnen
                    Ok(b) if b.len() > state.cfg.max_photo_bytes => {
                        errors.push(photo_error(
                            lang,
                            position,
                            &name,
                            &lang.t("err-photo-too_large"),
                        ));
                        position += 1;
                    }
                    Ok(b) => {
                        photos.push(b);
                        origin.push((position, name));
                        position += 1;
                    }
                    Err(e) => return Err(upload_error(&ctx, &state, e.status())),
                }
            }
            _ => {}
        }
    }
    if !token.is_some_and(|t| constant_time_eq(&t, &ctx.page.csrf)) {
        return Err(crate::security::csrf_failed(ctx.page.lang));
    }
    if photos.is_empty() {
        if errors.is_empty() {
            errors.push(lang.t("err-photo-required"));
        }
        let page = upload_page(&ctx, &state, errors);
        return Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page));
    }
    let queued = match state.uc.queue_photos.execute(&ctx.tenant, photos).await {
        Ok(q) => q,
        Err(AppError::Validation(e)) => {
            let messages = e.0.iter().map(|e| lang.err(e)).collect();
            let page = upload_page(&ctx, &state, messages);
            return Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page));
        }
        Err(e) => return Err(ctx.fail(e)),
    };
    for (index, e) in &queued.rejected {
        let (pos, name) = origin
            .get(*index)
            .map_or((*index, ""), |(p, n)| (*p, n.as_str()));
        errors.push(photo_error(lang, pos, name, &lang.err(e)));
    }
    let Some(first) = queued.drafts.first() else {
        let page = upload_page(&ctx, &state, errors);
        return Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page));
    };
    state.analysis.start(state.uc.clone(), ctx.tenant.clone());
    if !errors.is_empty() {
        ctx.flash("flash-photos-rejected").await;
        tracing::info!(rejected = errors.len(), "photos rejected");
    }
    if let Some(grid) = grid {
        return Ok(see_other(&format!("/pv/grid/{grid}/readings")));
    }
    Ok(see_other(&format!("/capture/{first}")))
}

/// Body zu groß (413) oder fehlerhaftes Multipart (400).
fn upload_error(ctx: &Ctx, state: &WebState, status: StatusCode) -> Response {
    let key = if status == StatusCode::PAYLOAD_TOO_LARGE {
        "err-photo-too_large"
    } else {
        "err-photo-unsupported"
    };
    let page = upload_page(ctx, state, vec![ctx.page.lang.t(key)]);
    render_status(status, &page)
}

fn parse_draft(id: &str) -> Result<DraftId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

#[derive(Deserialize)]
pub struct MeterQuery {
    meter: Option<String>,
}

fn suggestion_view(lang: Lang, view: &DraftView) -> Option<SuggestionView> {
    let s = view.draft.suggestion.as_ref()?;
    let dash = || lang.t("capture-not-recognized");
    Some(SuggestionView {
        value: s
            .value
            .map_or_else(dash, |v| format::decimal_plain(lang, v)),
        serial: s.serial_number.clone().unwrap_or_else(dash),
        register: s.register.clone().unwrap_or_else(dash),
        unit: s.unit.clone().unwrap_or_else(dash),
        confidence: format::decimal(lang, s.confidence * rust_decimal::Decimal::ONE_HUNDRED, 0),
    })
}

/// View-Modell; `form` überschreibt die Werte aus dem Entwurf.
fn confirm_page(
    ctx: &Ctx,
    view: &DraftView,
    form: Option<&FormData>,
    plausibility: Option<&Plausibility>,
    needs_confirm: bool,
    errors: FieldErrors,
) -> ConfirmPage {
    let lang = ctx.page.lang;
    let selected_meter = view.meter.as_ref().map(|m| m.aggregate.meter.id);
    let selected_register = form
        .and_then(|f| f.get("register_id").parse::<RegisterId>().ok())
        .or(view.register);
    let suggested_value = view
        .draft
        .suggestion
        .as_ref()
        .and_then(|s| s.value)
        .map(|v| format::decimal_plain(lang, v))
        .unwrap_or_default();
    ConfirmPage {
        draft_id: view.draft.id.to_string(),
        failed: view.draft.status == DraftStatus::Failed,
        suggestion: suggestion_view(lang, view),
        meters: view
            .meters
            .iter()
            .map(|m| Choice {
                id: m.meter.id.to_string(),
                label: m.meter.data.name.clone(),
                selected: Some(m.meter.id) == selected_meter,
            })
            .collect(),
        meter: view.meter.as_ref().map(|d| MeterView {
            id: d.aggregate.meter.id.to_string(),
            name: d.aggregate.meter.data.name.clone(),
            unit: d.aggregate.meter.data.unit.clone(),
            registers: d
                .aggregate
                .registers
                .iter()
                .map(|r| Choice {
                    id: r.id.to_string(),
                    label: r.data.code.clone(),
                    selected: Some(r.id) == selected_register,
                })
                .collect(),
        }),
        value: form.map_or(suggested_value, |f| f.get("value").to_owned()),
        // Vorgabe: Aufnahmezeit des Fotos, sonst jetzt (SPEC_IMG_CAPTURE_TIME)
        at: form.map_or_else(
            || {
                let at = view.draft.taken_at.unwrap_or_else(Utc::now);
                format::datetime_input(at.with_timezone(&ctx.page.tz).naive_local())
            },
            |f| f.get("at").to_owned(),
        ),
        at_from_photo: form.is_none() && view.draft.taken_at.is_some(),
        note: form.map(|f| f.get("note").to_owned()).unwrap_or_default(),
        hint: plausibility
            .or(view.plausibility.as_ref())
            .and_then(|p| hint(lang, p)),
        needs_confirm,
        progress: progress(lang, view),
        errors,
        ctx: ctx.page.clone(),
    }
}

/// „Foto 2 von 5“, falls mehrere Fotos offen sind (SPEC_AI_MULTI).
fn progress(lang: Lang, view: &DraftView) -> Option<String> {
    let (pos, total) = view.position();
    (total > 1).then(|| {
        lang.ta(
            "capture-progress",
            &[("n", pos.to_string()), ("total", total.to_string())],
        )
    })
}

#[derive(Template)]
#[template(path = "pages/capture/waiting.html")]
struct WaitingPage {
    ctx: PageCtx,
    draft_id: String,
    progress: Option<String>,
}

/// Bestätigungsformular; `?meter=` wählt eine andere Messstelle. Ist das
/// Foto noch nicht analysiert, eine Warteseite, die sich selbst aktualisiert.
// @impl SPEC_AI_DRAFTS, SPEC_AI_MULTI
pub async fn show(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<MeterQuery>,
) -> Result<Response, Response> {
    let id = parse_draft(&id).map_err(|e| ctx.fail(e))?;
    let meter = q.meter.as_deref().and_then(|m| m.parse::<MeterId>().ok());
    let view = state
        .uc
        .get_draft
        .execute(&ctx.tenant, id, meter)
        .await
        .map_err(|e| ctx.fail(e))?;
    if !view.draft.status.is_ready() {
        // nimmt eine unterbrochene Warteschlange wieder auf
        state.analysis.start(state.uc.clone(), ctx.tenant.clone());
        return Ok(render(&WaitingPage {
            draft_id: id.to_string(),
            progress: progress(ctx.page.lang, &view),
            ctx: ctx.page.clone(),
        }));
    }
    Ok(render(&confirm_page(
        &ctx,
        &view,
        None,
        None,
        false,
        FieldErrors::default(),
    )))
}

/// Foto des Entwurfs als JPEG.
// @impl SPEC_SEC_UPLOADS
pub async fn photo(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_draft(&id).map_err(|e| ctx.fail(e))?;
    let jpeg = state
        .uc
        .get_draft_photo
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        jpeg,
    )
        .into_response())
}

fn parse_confirm(form: &FormData, cal: Calendar) -> Result<ReadingsInput, FieldErrors> {
    let mut e = FieldErrors::default();
    let meter_id = form.get("meter_id").parse::<MeterId>();
    let register = form.get("register_id").parse::<RegisterId>();
    if register.is_err() {
        e.add("register_id", "capture.register_required");
    }
    let value = match parse::decimal(form.get("value")) {
        Ok(Some(v)) => Some(v),
        Ok(None) => {
            e.add("value", "reading.values_required");
            None
        }
        Err(_) => {
            e.add("value", "form.number_invalid");
            None
        }
    };
    let at = parse::datetime_local(form.get("at")).map_or_else(
        |_| {
            e.add("at", "form.datetime_invalid");
            Utc::now()
        },
        |l| cal.to_utc(l),
    );
    match (meter_id, register, value) {
        (Ok(meter_id), Ok(register), Some(value)) if e.is_empty() => Ok(ReadingsInput {
            meter_id,
            at,
            values: vec![(register, value)],
            note: form.opt("note"),
        }),
        _ => Err(e),
    }
}

/// Speichern; bei Warnungen erneut mit Bestätigungsfeld.
// @impl SPEC_AI_DRAFTS, SPEC_RDG_PLAUSIBILITY
pub async fn confirm(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_draft(&id).map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let meter = form.get("meter_id").parse::<MeterId>().ok();
    let view = state
        .uc
        .get_draft
        .execute(&ctx.tenant, id, meter)
        .await
        .map_err(|e| ctx.fail(e))?;
    let rerender = |p: Option<&Plausibility>, needs_confirm: bool, errors: FieldErrors| {
        render_status(
            StatusCode::UNPROCESSABLE_ENTITY,
            &confirm_page(&ctx, &view, Some(&form), p, needs_confirm, errors),
        )
    };
    let input = match parse_confirm(&form, Calendar::new(ctx.page.tz)) {
        Ok(i) => i,
        Err(errors) => return Ok(rerender(None, false, errors)),
    };
    let confirmed = form.get("confirm_warnings") == "true";
    match state
        .uc
        .confirm_draft
        .execute(&ctx.tenant, id, &input, confirmed)
        .await
    {
        Ok(RecordOutcome::Saved(_)) => {
            ctx.flash("flash-readings-saved").await;
            // weiter mit dem nächsten offenen Foto (SPEC_AI_MULTI)
            match state.uc.get_draft.next_open(&ctx.tenant, id).await {
                Ok(Some(next)) => Ok(see_other(&format!("/capture/{next}"))),
                Ok(None) => Ok(see_other(&format!("/meters/{}", input.meter_id))),
                Err(e) => Err(ctx.fail(e)),
            }
        }
        Ok(RecordOutcome::NeedsConfirmation(checks)) => Ok(rerender(
            checks.first().map(|c| &c.plausibility),
            true,
            FieldErrors::default(),
        )),
        Err(AppError::Validation(errors)) => Ok(rerender(None, false, errors)),
        Err(e) => Err(ctx.fail(e)),
    }
}

// @impl SPEC_AI_DRAFTS, SPEC_AI_MULTI
pub async fn discard(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_draft(&id).map_err(|e| ctx.fail(e))?;
    state
        .uc
        .discard_draft
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    ctx.flash("flash-draft-discarded").await;
    match state.uc.get_draft.next_open(&ctx.tenant, id).await {
        Ok(Some(next)) => Ok(see_other(&format!("/capture/{next}"))),
        Ok(None) => Ok(see_other("/capture")),
        Err(e) => Err(ctx.fail(e)),
    }
}
