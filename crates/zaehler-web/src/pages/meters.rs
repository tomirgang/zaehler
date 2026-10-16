//! Messstellen: Liste, Anlegen, Bearbeiten, Detail, Zählerwechsel
//! (SPEC_MTR_MANAGE, SPEC_MTR_SWAP_FLOW).

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{DraftId, MeterId, RegisterId},
    model::{MeterAggregate, MonthDay, Reading},
    usecase::{
        capture::DraftView,
        meter::{MeterInput, RegisterInput, SwapInput},
    },
};
use zaehler_domain::{
    calendar::Calendar,
    meter::{Direction, MeterKind},
};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format, parse},
    pages::{
        FormData,
        analysis::{self, AnalysisView},
        field_errors, render, render_status, see_other,
    },
    state::WebState,
};

const MAX_REGISTERS: usize = 20;

#[derive(Debug, Clone)]
pub struct RegisterRow {
    pub idx: usize,
    pub id: String,
    pub code: String,
    pub direction: String,
    pub start: String,
}

#[derive(Debug, Clone)]
pub struct MeterFormView {
    pub action: String,
    pub is_new: bool,
    pub locked: bool,
    pub name: String,
    pub kind: String,
    pub unit: String,
    pub serial: String,
    pub digits: String,
    pub decimals: String,
    pub interval: String,
    pub max_rate: String,
    pub billing_month: String,
    pub billing_day: String,
    pub start_at: String,
    /// Entwurf der Foto-Erfassung, aus dem die Messstelle entsteht (SPEC_AI_NEW_METER)
    pub draft: String,
    pub registers: Vec<RegisterRow>,
}

#[derive(Template)]
#[template(path = "pages/meters/form.html")]
struct MeterFormPage {
    ctx: PageCtx,
    form: MeterFormView,
    errors: FieldErrors,
    kinds: Vec<(&'static str, String)>,
}

impl MeterFormPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn has_errs(&self, field: &str) -> bool {
        !self.errors.for_field(field).is_empty()
    }

    #[allow(clippy::trivially_copy_pass_by_ref)] // Askama übergibt Argumente als Referenz
    fn reg_errs(&self, idx: &usize, part: &str) -> Vec<String> {
        self.errs(&format!("register_{idx}_{part}"))
    }

    #[allow(clippy::trivially_copy_pass_by_ref)] // Askama übergibt Argumente als Referenz
    fn has_reg_errs(&self, idx: &usize, part: &str) -> bool {
        self.has_errs(&format!("register_{idx}_{part}"))
    }

    fn kind_selected(&self, value: &str) -> bool {
        self.form.kind == value
    }

    fn kind_label(&self) -> String {
        self.ctx.lang.t(&format!("kind-{}", self.form.kind))
    }
}

pub struct MeterRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub unit: String,
    pub last: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/meters/list.html")]
struct ListPage {
    ctx: PageCtx,
    meters: Vec<MeterRow>,
}

pub struct DeviceRow {
    pub serial: String,
    pub installed: String,
    pub removed: Option<String>,
    pub digits: String,
    pub decimals: u8,
}

pub struct ReadingRow {
    pub id: String,
    pub at: String,
    pub register: String,
    pub value: String,
    pub source: String,
    pub note: String,
    pub has_photo: bool,
}

#[derive(Template)]
#[template(path = "pages/meters/detail.html")]
struct DetailPage {
    ctx: PageCtx,
    id: String,
    name: String,
    kind: String,
    unit: String,
    interval: Option<u16>,
    max_rate: Option<String>,
    billing_start: Option<String>,
    registers: Vec<(String, String)>,
    devices: Vec<DeviceRow>,
    readings: Vec<ReadingRow>,
    analysis: AnalysisView,
}

pub struct SwapRow {
    pub register_id: String,
    pub code: String,
    pub last: String,
    pub end: String,
    pub start: String,
}

#[derive(Template)]
#[template(path = "pages/meters/swap.html")]
struct SwapPage {
    ctx: PageCtx,
    id: String,
    name: String,
    at: String,
    serial: String,
    digits: String,
    decimals: String,
    rows: Vec<SwapRow>,
    errors: FieldErrors,
}

impl SwapPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn has_errs(&self, field: &str) -> bool {
        !self.errors.for_field(field).is_empty()
    }

    fn row_errs(&self, part: &str, register_id: &str) -> Vec<String> {
        self.errs(&format!("{part}_{register_id}"))
    }
}

fn kinds(lang: Lang) -> Vec<(&'static str, String)> {
    MeterKind::ALL
        .iter()
        .map(|k| (k.as_str(), lang.t(&format!("kind-{}", k.as_str()))))
        .collect()
}

fn now_input(tz: chrono_tz::Tz) -> String {
    format::datetime_input(Utc::now().with_timezone(&tz).naive_local())
}

fn parse_meter_id(id: &str) -> Result<MeterId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

/// Formularwerte unverändert zurückgeben (bei Fehlern).
fn view_from_form(form: &FormData, action: String, is_new: bool, locked: bool) -> MeterFormView {
    let count = form
        .get("register_count")
        .parse::<usize>()
        .unwrap_or(1)
        .clamp(1, MAX_REGISTERS);
    let registers = (0..count)
        .map(|i| RegisterRow {
            idx: i,
            id: form.get(&format!("register_{i}_id")).to_owned(),
            code: form.get(&format!("register_{i}_code")).to_owned(),
            direction: form.get(&format!("register_{i}_direction")).to_owned(),
            start: form.get(&format!("register_{i}_start")).to_owned(),
        })
        .collect();
    MeterFormView {
        action,
        is_new,
        locked,
        name: form.get("name").to_owned(),
        kind: form.get("kind").to_owned(),
        unit: form.get("unit").to_owned(),
        serial: form.get("serial_number").to_owned(),
        digits: form.get("rollover_digits").to_owned(),
        decimals: form.get("decimal_places").to_owned(),
        interval: form.get("reading_interval_days").to_owned(),
        max_rate: form.get("max_daily_rate").to_owned(),
        billing_month: form.get("billing_month").to_owned(),
        billing_day: form.get("billing_day").to_owned(),
        start_at: form.get("start_at").to_owned(),
        draft: form.get("draft").to_owned(),
        registers,
    }
}

/// Formular in Eingaben des Kerns umsetzen; Formatfehler als Feldfehler.
fn parse_input(
    form: &FormData,
    view: &MeterFormView,
    cal: Calendar,
) -> Result<MeterInput, FieldErrors> {
    let mut e = FieldErrors::default();
    let kind = view.kind.parse::<MeterKind>().unwrap_or_else(|_| {
        e.add("kind", "form.kind_invalid");
        MeterKind::Custom
    });
    let mut int = |field: &str, value: &str| -> Option<u16> {
        parse::integer::<u16>(value).unwrap_or_else(|_| {
            e.add(field, "form.integer_invalid");
            None
        })
    };
    let digits = int("rollover_digits", &view.digits).and_then(|v| u8::try_from(v).ok());
    let decimals = int("decimal_places", &view.decimals)
        .and_then(|v| u8::try_from(v).ok())
        .unwrap_or(0);
    let interval = int("reading_interval_days", &view.interval);
    let month = int("billing_start", &view.billing_month);
    let day = int("billing_start", &view.billing_day);
    let billing_start = match (month, day) {
        (None, None) => None,
        (Some(m), Some(d)) => Some(MonthDay {
            month: u8::try_from(m).unwrap_or(0),
            day: u8::try_from(d).unwrap_or(0),
        }),
        _ => {
            e.add("billing_start", "meter.billing_start_invalid");
            None
        }
    };
    let max_daily_rate = parse::decimal(&view.max_rate).unwrap_or_else(|_| {
        e.add("max_daily_rate", "form.number_invalid");
        None
    });
    let start_at = if view.start_at.trim().is_empty() {
        Utc::now()
    } else {
        parse::datetime_local(&view.start_at).map_or_else(
            |_| {
                e.add("start_at", "form.datetime_invalid");
                Utc::now()
            },
            |local| cal.to_utc(local),
        )
    };
    let mut registers = Vec::new();
    for row in &view.registers {
        let direction = row
            .direction
            .parse::<Direction>()
            .unwrap_or(Direction::Import);
        let start_value = parse::decimal(&row.start).unwrap_or_else(|_| {
            e.add(format!("register_{}_start", row.idx), "form.number_invalid");
            None
        });
        let id = if row.id.is_empty() {
            None
        } else {
            row.id.parse::<RegisterId>().ok()
        };
        registers.push(RegisterInput {
            id,
            code: row.code.clone(),
            direction,
            start_value,
        });
    }
    if e.is_empty() {
        Ok(MeterInput {
            name: view.name.clone(),
            kind,
            unit: view.unit.clone(),
            reading_interval_days: interval,
            max_daily_rate,
            billing_start,
            serial_number: form.opt("serial_number"),
            rollover_digits: digits,
            decimal_places: decimals,
            registers,
            start_at,
        })
    } else {
        Err(e)
    }
}

fn form_page(ctx: &Ctx, form: MeterFormView, errors: FieldErrors, status: StatusCode) -> Response {
    let page = MeterFormPage {
        kinds: kinds(ctx.page.lang),
        ctx: ctx.page.clone(),
        form,
        errors,
    };
    render_status(status, &page)
}

pub async fn list(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    let meters = state
        .uc
        .list_meters
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?;
    let lang = ctx.page.lang;
    let rows = meters
        .into_iter()
        .map(|m| MeterRow {
            id: m.meter.id.to_string(),
            name: m.meter.data.name,
            kind: lang.t(&format!("kind-{}", m.meter.data.kind.as_str())),
            unit: m.meter.data.unit,
            last: m
                .last_reading_at
                .map(|t| format::datetime_in(lang, t, ctx.page.tz)),
        })
        .collect();
    Ok(render(&ListPage {
        ctx: ctx.page,
        meters: rows,
    }))
}

fn empty_form(tz: chrono_tz::Tz) -> MeterFormView {
    MeterFormView {
        action: "/meters".into(),
        is_new: true,
        locked: false,
        name: String::new(),
        kind: MeterKind::Electricity.as_str().into(),
        unit: String::new(),
        serial: String::new(),
        digits: String::new(),
        decimals: "0".into(),
        interval: "30".into(),
        max_rate: String::new(),
        billing_month: String::new(),
        billing_day: String::new(),
        start_at: now_input(tz),
        draft: String::new(),
        registers: vec![RegisterRow {
            idx: 0,
            id: String::new(),
            code: "main".into(),
            direction: "import".into(),
            start: String::new(),
        }],
    }
}

/// Vorbelegung aus dem Vorschlag eines Foto-Entwurfs; ohne Vorschlag bleibt
/// das Formular leer, das Foto wird trotzdem übernommen.
// @impl SPEC_AI_NEW_METER
fn view_from_draft(lang: Lang, tz: chrono_tz::Tz, view: &DraftView) -> MeterFormView {
    let mut form = empty_form(tz);
    form.draft = view.draft.id.to_string();
    // Aufnahmezeit des Fotos als Zeitpunkt der Startstände (SPEC_IMG_CAPTURE_TIME)
    if let Some(at) = view.draft.taken_at {
        form.start_at = format::datetime_input(at.with_timezone(&tz).naive_local());
    }
    let Some(s) = view.draft.suggestion.as_ref() else {
        return form;
    };
    let kind = s.meter_kind.unwrap_or(MeterKind::Electricity);
    form.kind = kind.as_str().into();
    form.name = lang.t(&format!("kind-{}", kind.as_str()));
    form.unit = s
        .unit
        .clone()
        .filter(|u| u.trim() != kind.default_unit())
        .unwrap_or_default();
    form.serial = s.serial_number.clone().unwrap_or_default();
    if let Some(code) = s.register.as_ref().filter(|c| !c.trim().is_empty()) {
        code.trim().clone_into(&mut form.registers[0].code);
    }
    if let Some(v) = s.value {
        form.decimals = v.scale().min(3).to_string();
        form.registers[0].start = format::decimal_plain(lang, v);
    }
    form
}

#[derive(Deserialize)]
pub struct DraftQuery {
    draft: Option<String>,
}

// @impl SPEC_MTR_MANAGE, SPEC_AI_NEW_METER
pub async fn new_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Query(q): Query<DraftQuery>,
) -> Result<Response, Response> {
    let form = match q.draft.as_deref().filter(|d| !d.is_empty()) {
        None => empty_form(ctx.page.tz),
        Some(d) => {
            let id = d
                .parse::<DraftId>()
                .map_err(|_| ctx.fail(AppError::NotFound))?;
            let view = state
                .uc
                .get_draft
                .execute(&ctx.tenant, id, None)
                .await
                .map_err(|e| ctx.fail(e))?;
            view_from_draft(ctx.page.lang, ctx.page.tz, &view)
        }
    };
    Ok(form_page(
        &ctx,
        form,
        FieldErrors::default(),
        StatusCode::OK,
    ))
}

fn add_register_row(view: &mut MeterFormView) {
    if view.registers.len() < MAX_REGISTERS {
        view.registers.push(RegisterRow {
            idx: view.registers.len(),
            id: String::new(),
            code: String::new(),
            direction: "import".into(),
            start: String::new(),
        });
    }
}

pub async fn create(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let form = FormData::new(pairs);
    let mut view = view_from_form(&form, "/meters".into(), true, false);
    if form.has("add_register") {
        add_register_row(&mut view);
        return form_page(&ctx, view, FieldErrors::default(), StatusCode::OK);
    }
    let input = match parse_input(&form, &view, Calendar::new(ctx.page.tz)) {
        Ok(i) => i,
        Err(errors) => return form_page(&ctx, view, errors, StatusCode::UNPROCESSABLE_ENTITY),
    };
    let result = if view.draft.is_empty() {
        state.uc.create_meter.execute(&ctx.tenant, input).await
    } else {
        match view.draft.parse::<DraftId>() {
            Ok(draft) => {
                state
                    .uc
                    .create_meter
                    .execute_from_draft(&ctx.tenant, input, draft)
                    .await
            }
            Err(_) => Err(AppError::NotFound),
        }
    };
    match result {
        Ok(id) => {
            ctx.flash("flash-meter-created").await;
            see_other(&format!("/meters/{id}"))
        }
        Err(AppError::Validation(errors)) => {
            form_page(&ctx, view, errors, StatusCode::UNPROCESSABLE_ENTITY)
        }
        Err(e) => ctx.fail(e),
    }
}

fn source_label(lang: Lang, r: &Reading) -> String {
    lang.t(&format!("source-{}", r.source.as_str()))
}

#[derive(Deserialize)]
pub struct MonthQuery {
    month: Option<String>,
}

/// Detailseite mit Auswertung; `?month=2026-03` wählt den Bezugsmonat.
// @impl SPEC_ANA_COMPARE_VIEW
pub async fn detail(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<MonthQuery>,
) -> Result<Response, Response> {
    let id = parse_meter_id(&id).map_err(|e| ctx.fail(e))?;
    let month = q.month.as_deref().and_then(analysis::parse_month);
    let result = state
        .uc
        .meter_analysis
        .execute(&ctx.tenant, id, month)
        .await
        .map_err(|e| ctx.fail(e))?;
    let analysis = analysis::view(ctx.page.lang, &result);
    let detail = result.detail;
    let lang = ctx.page.lang;
    let tz = ctx.page.tz;
    let agg = &detail.aggregate;
    let data = &agg.meter.data;
    let registers = agg
        .registers
        .iter()
        .map(|r| {
            (
                r.data.code.clone(),
                lang.t(&format!("direction-{}", r.data.direction.as_str())),
            )
        })
        .collect();
    let devices = agg
        .devices
        .iter()
        .rev()
        .map(|d| DeviceRow {
            serial: d.data.serial_number.clone().unwrap_or_default(),
            installed: format::datetime_in(lang, d.data.installed_at, tz),
            removed: d.removed_at.map(|t| format::datetime_in(lang, t, tz)),
            digits: d
                .data
                .rollover_digits
                .map(|x| x.to_string())
                .unwrap_or_default(),
            decimals: d.data.decimal_places,
        })
        .collect();
    let readings = detail
        .readings
        .iter()
        .rev()
        .map(|r| ReadingRow {
            id: r.id.to_string(),
            at: format::datetime_in(lang, r.read_at, tz),
            register: agg
                .register(r.register_id)
                .map(|x| x.data.code.clone())
                .unwrap_or_default(),
            value: format::decimal(
                lang,
                r.value,
                agg.device(r.device_id).map_or(3, |d| d.data.decimal_places),
            ),
            source: source_label(lang, r),
            note: r.note.clone().unwrap_or_default(),
            has_photo: r.photo_key.is_some(),
        })
        .collect();
    let page = DetailPage {
        id: id.to_string(),
        name: data.name.clone(),
        kind: lang.t(&format!("kind-{}", data.kind.as_str())),
        unit: data.unit.clone(),
        interval: data.reading_interval_days,
        max_rate: data.max_daily_rate.map(|r| format::decimal_plain(lang, r)),
        billing_start: data
            .billing_start
            .map(|md| format!("{:02}.{:02}.", md.day, md.month)),
        registers,
        devices,
        readings,
        analysis,
        ctx: ctx.page,
    };
    Ok(render(&page))
}

fn view_from_aggregate(agg: &MeterAggregate, lang: Lang, locked: bool) -> MeterFormView {
    let device = agg.active_device();
    let data = &agg.meter.data;
    MeterFormView {
        action: format!("/meters/{}/edit", agg.meter.id),
        is_new: false,
        locked,
        name: data.name.clone(),
        kind: data.kind.as_str().into(),
        unit: data.unit.clone(),
        serial: device
            .and_then(|d| d.data.serial_number.clone())
            .unwrap_or_default(),
        digits: device
            .and_then(|d| d.data.rollover_digits)
            .map(|x| x.to_string())
            .unwrap_or_default(),
        decimals: device.map_or(0, |d| d.data.decimal_places).to_string(),
        interval: data
            .reading_interval_days
            .map(|x| x.to_string())
            .unwrap_or_default(),
        max_rate: data
            .max_daily_rate
            .map(|r| format::decimal_plain(lang, r))
            .unwrap_or_default(),
        billing_month: data
            .billing_start
            .map(|md| md.month.to_string())
            .unwrap_or_default(),
        billing_day: data
            .billing_start
            .map(|md| md.day.to_string())
            .unwrap_or_default(),
        start_at: String::new(),
        draft: String::new(),
        registers: agg
            .registers
            .iter()
            .enumerate()
            .map(|(i, r)| RegisterRow {
                idx: i,
                id: r.id.to_string(),
                code: r.data.code.clone(),
                direction: r.data.direction.as_str().into(),
                start: String::new(),
            })
            .collect(),
    }
}

pub async fn edit_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_meter_id(&id).map_err(|e| ctx.fail(e))?;
    let detail = state
        .uc
        .get_meter
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let view = view_from_aggregate(
        &detail.aggregate,
        ctx.page.lang,
        !detail.readings.is_empty(),
    );
    Ok(form_page(
        &ctx,
        view,
        FieldErrors::default(),
        StatusCode::OK,
    ))
}

pub async fn update(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let Ok(id) = parse_meter_id(&id) else {
        return ctx.fail(AppError::NotFound);
    };
    let form = FormData::new(pairs);
    let locked = form.get("locked") == "true";
    let mut view = view_from_form(&form, format!("/meters/{id}/edit"), false, locked);
    if form.has("add_register") {
        add_register_row(&mut view);
        return form_page(&ctx, view, FieldErrors::default(), StatusCode::OK);
    }
    let input = match parse_input(&form, &view, Calendar::new(ctx.page.tz)) {
        Ok(i) => i,
        Err(errors) => return form_page(&ctx, view, errors, StatusCode::UNPROCESSABLE_ENTITY),
    };
    match state.uc.update_meter.execute(&ctx.tenant, id, input).await {
        Ok(()) => {
            ctx.flash("flash-meter-updated").await;
            see_other(&format!("/meters/{id}"))
        }
        Err(AppError::Validation(errors)) => {
            form_page(&ctx, view, errors, StatusCode::UNPROCESSABLE_ENTITY)
        }
        Err(e) => ctx.fail(e),
    }
}

fn last_text(
    lang: Lang,
    agg: &MeterAggregate,
    readings: &[Reading],
    register: RegisterId,
    tz: chrono_tz::Tz,
) -> String {
    let Some(device) = agg.active_device() else {
        return String::new();
    };
    readings
        .iter()
        .filter(|r| r.register_id == register && r.device_id == device.id)
        .max_by_key(|r| r.read_at)
        .map_or_else(
            || lang.t("readings-none"),
            |r| {
                lang.ta(
                    "readings-last",
                    &[
                        (
                            "value",
                            format::decimal(lang, r.value, device.data.decimal_places),
                        ),
                        ("date", format::datetime_in(lang, r.read_at, tz)),
                    ],
                )
            },
        )
}

pub async fn swap_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_meter_id(&id).map_err(|e| ctx.fail(e))?;
    let detail = state
        .uc
        .get_meter
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let lang = ctx.page.lang;
    let rows = detail
        .aggregate
        .registers
        .iter()
        .map(|r| SwapRow {
            register_id: r.id.to_string(),
            code: r.data.code.clone(),
            last: last_text(lang, &detail.aggregate, &detail.readings, r.id, ctx.page.tz),
            end: String::new(),
            start: "0".into(),
        })
        .collect();
    let decimals = detail
        .aggregate
        .active_device()
        .map_or(0, |d| d.data.decimal_places);
    let page = SwapPage {
        id: id.to_string(),
        name: detail.aggregate.meter.data.name.clone(),
        at: now_input(ctx.page.tz),
        serial: String::new(),
        digits: String::new(),
        decimals: decimals.to_string(),
        rows,
        errors: FieldErrors::default(),
        ctx: ctx.page,
    };
    Ok(render(&page))
}

pub async fn swap(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_meter_id(&id).map_err(|e| ctx.fail(e))?;
    let detail = state
        .uc
        .get_meter
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let lang = ctx.page.lang;
    let mut errors = FieldErrors::default();
    let cal = Calendar::new(ctx.page.tz);
    let at: Option<DateTime<Utc>> = parse::datetime_local(form.get("at"))
        .map(|l| cal.to_utc(l))
        .ok();
    if at.is_none() {
        errors.add("at", "form.datetime_invalid");
    }
    let digits = parse::integer::<u8>(form.get("rollover_digits")).unwrap_or_else(|_| {
        errors.add("rollover_digits", "form.integer_invalid");
        None
    });
    let decimals = parse::integer::<u8>(form.get("decimal_places")).unwrap_or_else(|_| {
        errors.add("decimal_places", "form.integer_invalid");
        None
    });
    let mut end_values = Vec::new();
    let mut start_values = Vec::new();
    let mut rows = Vec::new();
    for r in &detail.aggregate.registers {
        let end_field = format!("end_{}", r.id);
        let start_field = format!("start_{}", r.id);
        match parse::decimal(form.get(&end_field)) {
            Ok(Some(v)) => end_values.push((r.id, v)),
            Ok(None) => {}
            Err(_) => errors.add(&end_field, "form.number_invalid"),
        }
        match parse::decimal(form.get(&start_field)) {
            Ok(Some(v)) => start_values.push((r.id, v)),
            Ok(None) => {}
            Err(_) => errors.add(&start_field, "form.number_invalid"),
        }
        rows.push(SwapRow {
            register_id: r.id.to_string(),
            code: r.data.code.clone(),
            last: last_text(lang, &detail.aggregate, &detail.readings, r.id, ctx.page.tz),
            end: form.get(&end_field).to_owned(),
            start: form.get(&start_field).to_owned(),
        });
    }
    let mut page = SwapPage {
        id: id.to_string(),
        name: detail.aggregate.meter.data.name.clone(),
        at: form.get("at").to_owned(),
        serial: form.get("serial_number").to_owned(),
        digits: form.get("rollover_digits").to_owned(),
        decimals: form.get("decimal_places").to_owned(),
        rows,
        errors: FieldErrors::default(),
        ctx: ctx.page.clone(),
    };
    let Some(at) = at.filter(|_| errors.is_empty()) else {
        page.errors = errors;
        return Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page));
    };
    let input = SwapInput {
        at,
        end_values,
        serial_number: form.opt("serial_number"),
        rollover_digits: digits,
        decimal_places: decimals.unwrap_or(0),
        start_values,
    };
    match state.uc.swap_device.execute(&ctx.tenant, id, input).await {
        Ok(_) => {
            ctx.flash("flash-swap-done").await;
            Ok(see_other(&format!("/meters/{id}")))
        }
        Err(AppError::Validation(errors)) => {
            page.errors = errors;
            Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
        }
        Err(e) => Err(ctx.fail(e)),
    }
}
