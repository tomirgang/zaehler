//! CSV-Import: Upload, Zuordnung, Vorschau, Übernahme
//! (SPEC_IMP_CSV, SPEC_IMP_CSV_FORMAT).

use askama::Template;
use axum::{
    extract::{Multipart, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{BlobKey, MeterId, RegisterId},
    usecase::import::{
        DateFormat, DecimalSeparator, ImportMapping, ImportPreview, ImportReport, ImportSheet,
        MappingSuggestion, RowStatus,
    },
};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format},
    pages::{FormData, field_errors, readings::hint, render, render_status, see_other},
    security::constant_time_eq,
    state::WebState,
};

/// Sitzungsschlüssel der hochgeladenen Datei.
pub const SESSION_IMPORT: &str = "import";
/// Zeilen der Vorschau mit Ergebnis `ok`, die einzeln angezeigt werden.
const OK_ROWS_SHOWN: usize = 20;
/// Höchstzahl angezeigter Zeilen mit Hinweis, Duplikat oder Fehler.
const OTHER_ROWS_SHOWN: usize = 1000;

#[derive(Template)]
#[template(path = "pages/import/upload.html")]
struct UploadPage {
    ctx: PageCtx,
    hint: String,
    errors: Vec<String>,
}

pub struct Choice {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Template)]
#[template(path = "pages/import/mapping.html")]
struct MappingPage {
    ctx: PageCtx,
    rows_label: String,
    meters: Vec<Choice>,
    meter: Option<String>,
    has_header: bool,
    header: Vec<String>,
    sample: Vec<Vec<String>>,
    date: Vec<Choice>,
    time: Vec<Choice>,
    value: Vec<Choice>,
    register: Vec<Choice>,
    serial: Vec<Choice>,
    note: Vec<Choice>,
    date_formats: Vec<Choice>,
    decimals: Vec<Choice>,
    registers: Vec<Choice>,
    errors: FieldErrors,
}

impl MappingPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }
}

pub struct RowView {
    pub line: usize,
    pub at: String,
    pub register: String,
    pub value: String,
    pub result: String,
    pub class: &'static str,
}

#[derive(Template)]
#[template(path = "pages/import/preview.html")]
struct PreviewPage {
    ctx: PageCtx,
    summary: ImportPreview,
    rows: Vec<RowView>,
    truncated: bool,
    hidden: Vec<(&'static str, String)>,
    back: String,
}

#[derive(Template)]
#[template(path = "pages/import/result.html")]
struct ResultPage {
    ctx: PageCtx,
    report: ImportReport,
    meter: String,
}

fn upload_page(ctx: &Ctx, state: &WebState, errors: Vec<String>) -> UploadPage {
    UploadPage {
        hint: ctx.page.lang.ta(
            "import-file-hint",
            &[(
                "max",
                state.cfg.max_csv_bytes.div_ceil(1024 * 1024).to_string(),
            )],
        ),
        ctx: ctx.page.clone(),
        errors,
    }
}

/// `GET /import`
// @impl SPEC_IMP_CSV
pub async fn upload_form(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    if !ctx.page.can_write {
        return Err(ctx.fail(AppError::Forbidden));
    }
    Ok(render(&upload_page(&ctx, &state, Vec::new())))
}

/// `POST /import/upload` (Multipart mit Feld `_csrf`)
// @impl SPEC_IMP_CSV, SPEC_SEC_UPLOADS, SPEC_SEC_SESSION_CSRF
pub async fn upload(
    State(state): State<WebState>,
    ctx: Ctx,
    mut multipart: Multipart,
) -> Result<Response, Response> {
    let lang = ctx.page.lang;
    let mut token = None;
    let mut file = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return Err(upload_error(&ctx, &state, e.status())),
        };
        match field.name() {
            Some("_csrf") => token = field.text().await.ok(),
            Some("file") => match field.bytes().await {
                Ok(b) => file = Some(b),
                Err(e) => return Err(upload_error(&ctx, &state, e.status())),
            },
            _ => {}
        }
    }
    if !token.is_some_and(|t| constant_time_eq(&t, &ctx.page.csrf)) {
        return Err((StatusCode::FORBIDDEN, "CSRF-Prüfung fehlgeschlagen").into_response());
    }
    let Some(file) = file.filter(|f| !f.is_empty()) else {
        let page = upload_page(&ctx, &state, vec![lang.t("err-import-empty")]);
        return Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page));
    };
    match state.uc.upload_csv.execute(&ctx.tenant, file).await {
        Ok(key) => {
            if let Err(e) = ctx.session.insert(SESSION_IMPORT, key.to_string()).await {
                tracing::warn!(error = %e, "import key not stored");
                return Err(ctx.fail(AppError::Internal("session".into())));
            }
            Ok(see_other("/import/preview"))
        }
        Err(AppError::Validation(errors)) => {
            let messages = errors.0.iter().map(|e| lang.err(e)).collect();
            let page = upload_page(&ctx, &state, messages);
            Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
        }
        Err(e) => Err(ctx.fail(e)),
    }
}

fn upload_error(ctx: &Ctx, state: &WebState, status: StatusCode) -> Response {
    let key = if status == StatusCode::PAYLOAD_TOO_LARGE {
        "err-import-too_large"
    } else {
        "err-import-parse"
    };
    let page = upload_page(ctx, state, vec![ctx.page.lang.t(key)]);
    render_status(status, &page)
}

/// Schlüssel der Datei aus der Sitzung; ohne Datei zurück zum Upload.
async fn session_key(ctx: &Ctx) -> Result<BlobKey, Response> {
    match ctx.session.get::<String>(SESSION_IMPORT).await {
        Ok(Some(k)) => BlobKey::parse(&k).ok_or_else(|| see_other("/import")),
        _ => Err(see_other("/import")),
    }
}

/// Datei nicht mehr vorhanden (abgelaufen oder bereits übernommen).
async fn file_gone(ctx: &Ctx) -> Response {
    let _ = ctx.session.remove::<String>(SESSION_IMPORT).await;
    ctx.flash("flash-import-expired").await;
    see_other("/import")
}

fn column_choices(
    sheet: &ImportSheet,
    selected: Option<usize>,
    optional: bool,
    lang: Lang,
) -> Vec<Choice> {
    let mut out = Vec::new();
    if optional {
        out.push(Choice {
            value: String::new(),
            label: lang.t("import-column-none"),
            selected: selected.is_none(),
        });
    }
    for i in 0..sheet.columns {
        let first = sheet.first.get(i).map_or("", String::as_str);
        let label = if first.is_empty() {
            lang.ta("import-column", &[("n", (i + 1).to_string())])
        } else {
            lang.ta(
                "import-column-named",
                &[("n", (i + 1).to_string()), ("name", first.to_owned())],
            )
        };
        out.push(Choice {
            value: i.to_string(),
            label,
            selected: selected == Some(i),
        });
    }
    out
}

/// Werte für die Formularfelder: aus dem Formular, sonst aus dem Vorschlag.
struct Selected {
    has_header: bool,
    date: Option<usize>,
    time: Option<usize>,
    value: Option<usize>,
    register: Option<usize>,
    serial: Option<usize>,
    note: Option<usize>,
    date_format: String,
    decimal: String,
    default_register: String,
}

impl Selected {
    fn from_suggestion(s: &MappingSuggestion) -> Self {
        Self {
            has_header: s.has_header,
            date: Some(s.date),
            time: s.time,
            value: Some(s.value),
            register: s.register,
            serial: s.serial,
            note: s.note,
            date_format: s.date_format.as_str().into(),
            decimal: s.decimal.as_str().into(),
            default_register: String::new(),
        }
    }

    fn from_form(f: &FormData) -> Self {
        let col = |k: &str| f.get(k).parse::<usize>().ok();
        Self {
            has_header: f.get("has_header") == "true",
            date: col("col_date"),
            time: col("col_time"),
            value: col("col_value"),
            register: col("col_register"),
            serial: col("col_serial"),
            note: col("col_note"),
            date_format: f.get("date_format").into(),
            decimal: f.get("decimal").into(),
            default_register: f.get("default_register").into(),
        }
    }
}

async fn mapping_page(
    state: &WebState,
    ctx: &Ctx,
    key: &BlobKey,
    meter: Option<MeterId>,
    selected: Option<Selected>,
    errors: FieldErrors,
) -> Result<MappingPage, Response> {
    let lang = ctx.page.lang;
    let sheet = match state.uc.preview_import.sheet(&ctx.tenant, key).await {
        Ok(s) => s,
        Err(AppError::NotFound) => return Err(file_gone(ctx).await),
        Err(e) => return Err(ctx.fail(e)),
    };
    let meters: Vec<_> = state
        .uc
        .list_meters
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?
        .into_iter()
        .filter(|m| !m.meter.archived)
        .collect();
    let meter = meter
        .filter(|id| meters.iter().any(|m| m.meter.id == *id))
        .or_else(|| (meters.len() == 1).then(|| meters[0].meter.id));
    let registers = match meter {
        Some(id) => {
            state
                .uc
                .get_meter
                .execute(&ctx.tenant, id)
                .await
                .map_err(|e| ctx.fail(e))?
                .aggregate
                .registers
        }
        None => Vec::new(),
    };
    let sel = selected.unwrap_or_else(|| Selected::from_suggestion(&sheet.suggestion));
    let choice = |value: &str, label: String, current: &str| Choice {
        selected: value == current,
        value: value.to_owned(),
        label,
    };
    Ok(MappingPage {
        rows_label: lang.ta("import-rows", &[("n", sheet.rows.to_string())]),
        meters: meters
            .iter()
            .map(|m| Choice {
                value: m.meter.id.to_string(),
                label: m.meter.data.name.clone(),
                selected: Some(m.meter.id) == meter,
            })
            .collect(),
        meter: meter.map(|m| m.to_string()),
        has_header: sel.has_header,
        header: if sheet.suggestion.has_header {
            sheet.first.clone()
        } else {
            Vec::new()
        },
        sample: sheet.sample.clone(),
        date: column_choices(&sheet, sel.date, false, lang),
        time: column_choices(&sheet, sel.time, true, lang),
        value: column_choices(&sheet, sel.value, false, lang),
        register: column_choices(&sheet, sel.register, true, lang),
        serial: column_choices(&sheet, sel.serial, true, lang),
        note: column_choices(&sheet, sel.note, true, lang),
        date_formats: DateFormat::ALL
            .iter()
            .map(|f| {
                choice(
                    f.as_str(),
                    lang.t(&format!("import-date-{}", f.as_str())),
                    &sel.date_format,
                )
            })
            .collect(),
        decimals: [DecimalSeparator::Comma, DecimalSeparator::Point]
            .iter()
            .map(|d| {
                choice(
                    d.as_str(),
                    lang.t(&format!("import-decimal-{}", d.as_str())),
                    &sel.decimal,
                )
            })
            .collect(),
        registers: std::iter::once(choice(
            "",
            lang.t("import-register-auto"),
            &sel.default_register,
        ))
        .chain(registers.iter().map(|r| {
            choice(
                &r.id.to_string(),
                r.data.code.clone(),
                &sel.default_register,
            )
        }))
        .collect(),
        errors,
        ctx: ctx.page.clone(),
    })
}

#[derive(Deserialize)]
pub struct MeterQuery {
    meter: Option<String>,
}

/// `GET /import/preview`: Zuordnung der Spalten; `?meter=` wählt die Messstelle.
// @impl SPEC_IMP_CSV_FORMAT
pub async fn mapping(
    State(state): State<WebState>,
    ctx: Ctx,
    Query(q): Query<MeterQuery>,
) -> Result<Response, Response> {
    let key = session_key(&ctx).await?;
    let meter = q.meter.as_deref().and_then(|m| m.parse().ok());
    let page = mapping_page(&state, &ctx, &key, meter, None, FieldErrors::default()).await?;
    Ok(render(&page))
}

/// Formular in eine Zuordnung umsetzen.
fn parse_mapping(f: &FormData) -> Result<ImportMapping, FieldErrors> {
    let mut e = FieldErrors::default();
    let col = |k: &str| f.get(k).parse::<usize>().ok();
    let meter_id = f.get("meter").parse::<MeterId>();
    if meter_id.is_err() {
        e.add("meter", "import.meter_required");
    }
    let date = col("col_date");
    if date.is_none() {
        e.add("col_date", "import.column_invalid");
    }
    let value = col("col_value");
    if value.is_none() {
        e.add("col_value", "import.column_invalid");
    }
    let date_format = f.get("date_format").parse::<DateFormat>();
    let decimal = f.get("decimal").parse::<DecimalSeparator>();
    if date_format.is_err() || decimal.is_err() {
        e.add("date_format", "form.kind_invalid");
    }
    let default_register = match f.get("default_register") {
        "" => None,
        s => s.parse::<RegisterId>().map_or_else(
            |_| {
                e.add("default_register", "import.register_unknown");
                None
            },
            Some,
        ),
    };
    let (Ok(meter_id), Some(date), Some(value), Ok(date_format), Ok(decimal)) =
        (meter_id, date, value, date_format, decimal)
    else {
        return Err(e);
    };
    if !e.is_empty() {
        return Err(e);
    }
    Ok(ImportMapping {
        meter_id,
        has_header: f.get("has_header") == "true",
        date,
        time: col("col_time"),
        value,
        register: col("col_register"),
        serial: col("col_serial"),
        note: col("col_note"),
        date_format,
        decimal,
        default_register,
    })
}

/// Felder der Zuordnung für den Übernahmeschritt.
fn hidden_fields(f: &FormData) -> Vec<(&'static str, String)> {
    [
        "meter",
        "has_header",
        "col_date",
        "col_time",
        "col_value",
        "col_register",
        "col_serial",
        "col_note",
        "date_format",
        "decimal",
        "default_register",
    ]
    .into_iter()
    .map(|k| (k, f.get(k).to_owned()))
    .collect()
}

fn row_view(lang: Lang, tz: chrono_tz::Tz, r: &zaehler_app::usecase::import::RowResult) -> RowView {
    let (result, class) = match &r.status {
        RowStatus::Ok => (lang.t("import-row-ok"), "ok"),
        RowStatus::Warning(p) => (
            format!(
                "{}: {}",
                lang.t("import-row-warning"),
                hint(lang, p).unwrap_or_default()
            ),
            "warning",
        ),
        RowStatus::Duplicate => (lang.t("import-row-duplicate"), "duplicate"),
        RowStatus::Error(e) => (
            format!("{}: {}", lang.t("import-row-error"), lang.err(e)),
            "error",
        ),
    };
    RowView {
        line: r.line,
        at: r
            .at
            .map(|t| format::datetime_in(lang, t, tz))
            .unwrap_or_default(),
        register: r.register.clone().unwrap_or_default(),
        value: r
            .value
            .map(|v| format::decimal_plain(lang, v))
            .unwrap_or_default(),
        result,
        class,
    }
}

/// Mapping prüfen und Vorschau rendern bzw. Zuordnung mit Fehlern.
async fn checked_mapping(
    state: &WebState,
    ctx: &Ctx,
    key: &BlobKey,
    form: &FormData,
) -> Result<ImportMapping, Response> {
    match parse_mapping(form) {
        Ok(m) => Ok(m),
        Err(errors) => {
            let meter = form.get("meter").parse().ok();
            let page = mapping_page(
                state,
                ctx,
                key,
                meter,
                Some(Selected::from_form(form)),
                errors,
            )
            .await?;
            Err(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
        }
    }
}

async fn import_error(
    state: &WebState,
    ctx: &Ctx,
    key: &BlobKey,
    form: &FormData,
    e: AppError,
) -> Response {
    match e {
        // Datei abgelaufen oder bereits übernommen; sonst fehlt die Messstelle
        AppError::NotFound => match state.uc.preview_import.sheet(&ctx.tenant, key).await {
            Err(AppError::NotFound) => file_gone(ctx).await,
            _ => ctx.fail(AppError::NotFound),
        },
        AppError::Validation(errors) => {
            let meter = form.get("meter").parse().ok();
            match mapping_page(
                state,
                ctx,
                key,
                meter,
                Some(Selected::from_form(form)),
                errors,
            )
            .await
            {
                Ok(page) => render_status(StatusCode::UNPROCESSABLE_ENTITY, &page),
                Err(r) => r,
            }
        }
        other => ctx.fail(other),
    }
}

/// `POST /import/preview`: Prüfergebnis je Zeile.
// @impl SPEC_IMP_CSV, SPEC_IMP_CSV_FORMAT
pub async fn preview(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let form = FormData::new(pairs);
    let key = session_key(&ctx).await?;
    let mapping = checked_mapping(&state, &ctx, &key, &form).await?;
    let summary = match state
        .uc
        .preview_import
        .execute(&ctx.tenant, &key, &mapping)
        .await
    {
        Ok(s) => s,
        Err(e) => return Err(import_error(&state, &ctx, &key, &form, e).await),
    };
    let (lang, tz) = (ctx.page.lang, ctx.page.tz);
    let mut shown_ok = 0;
    let mut shown_other = 0;
    let mut truncated = false;
    let mut rows = Vec::new();
    for r in &summary.rows {
        let show = if r.status == RowStatus::Ok {
            shown_ok += 1;
            shown_ok <= OK_ROWS_SHOWN
        } else {
            shown_other += 1;
            shown_other <= OTHER_ROWS_SHOWN
        };
        if show {
            rows.push(row_view(lang, tz, r));
        } else {
            truncated = true;
        }
    }
    Ok(render(&PreviewPage {
        rows,
        truncated,
        hidden: hidden_fields(&form),
        back: format!("/import/preview?meter={}", mapping.meter_id),
        summary,
        ctx: ctx.page.clone(),
    }))
}

/// `POST /import/commit`: Übernahme; Hinweiszeilen nur mit `confirm_warnings`.
// @impl SPEC_IMP_CSV
pub async fn commit(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let form = FormData::new(pairs);
    let key = session_key(&ctx).await?;
    let mapping = checked_mapping(&state, &ctx, &key, &form).await?;
    let confirmed = form.get("confirm_warnings") == "true";
    match state
        .uc
        .commit_import
        .execute(&ctx.tenant, &key, &mapping, confirmed)
        .await
    {
        Ok(report) => {
            let _ = ctx.session.remove::<String>(SESSION_IMPORT).await;
            Ok(render(&ResultPage {
                report,
                meter: mapping.meter_id.to_string(),
                ctx: ctx.page.clone(),
            }))
        }
        Err(e) => Err(import_error(&state, &ctx, &key, &form, e).await),
    }
}
