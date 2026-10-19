//! REST-API: Messstellen anlegen, ändern, archivieren, löschen und Geräte
//! wechseln; einzelne Ablesungen lesen, korrigieren und löschen,
//! Änderungsverlauf und Belegfoto (SPEC_API_REST).

use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{MeterId, ReadingId, RegisterId},
    model::{MeterTargets, MonthDay},
    usecase::{
        meter::{MeterInput, RegisterInput, SwapInput},
        reading::{CorrectionInput, CorrectionOutcome, RegisterCheck, RevisionEntry},
    },
};
use zaehler_domain::{
    forecast::SeasonProfile,
    meter::{Direction, MeterKind, MeterPurpose},
    schedule::ReadingSchedule,
};

use super::{
    ApiResult,
    auth::ApiCtx,
    dto::{
        DeviceDto, MeterDetailDto, MeterDto, MonthDayDto, ReadingDto, ScheduleDto, TargetsDto,
        ValueDto,
    },
    error::{ApiError, Problem},
    json,
};
use crate::state::WebState;

pub(super) fn routes(r: OpenApiRouter<WebState>) -> OpenApiRouter<WebState> {
    r.routes(routes!(create_meter))
        .routes(routes!(update_meter, delete_meter))
        .routes(routes!(archive_meter))
        .routes(routes!(restore_meter))
        .routes(routes!(swap_device))
        .routes(routes!(meter_history))
        .routes(routes!(get_reading, correct_reading, delete_reading))
        .routes(routes!(reading_photo))
}

/// Register einer Messstelle; `id` gesetzt bei vorhandenen Registern.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterBody {
    pub id: Option<Uuid>,
    /// z.B. `1.8.0`
    pub code: String,
    /// `import` (Standard) oder `export`
    pub direction: Option<String>,
    /// Startstand beim Anlegen; beim Ändern ignoriert
    #[schema(value_type = Option<String>)]
    pub start_value: Option<Decimal>,
}

/// Stammdaten einer Messstelle wie im Formular. Beim Ändern ersetzt der
/// Body alle Stammdaten; vorhandene Register müssen mit `id` enthalten sein
/// (Register werden nie gelöscht).
#[derive(Debug, Deserialize, ToSchema)]
pub struct MeterBody {
    pub name: String,
    /// `electricity`, `gas`, `water`, `heat`, `vehicle` oder `custom`
    pub kind: String,
    /// Leer: Standard-Einheit der Art; bei `custom` erforderlich
    #[serde(default)]
    pub unit: String,
    pub schedule: Option<ScheduleDto>,
    #[schema(value_type = Option<String>)]
    pub max_daily_rate: Option<Decimal>,
    pub billing_start: Option<MonthDayDto>,
    /// `general` (Standard), `heat_pump` oder `wallbox`; nur bei Strom
    pub purpose: Option<String>,
    /// Übergeordnete Messstelle (Zwischenzähler); nur bei Strom
    pub parent_meter_id: Option<Uuid>,
    #[serde(default)]
    pub targets: TargetsDto,
    pub serial_number: Option<String>,
    /// Stellen vor dem Komma (1 bis 9) für den Überlauf
    pub rollover_digits: Option<u8>,
    /// Nachkommastellen (0 bis 3)
    #[serde(default)]
    pub decimal_places: u8,
    pub registers: Vec<RegisterBody>,
    /// Einbau und Startstände; nur beim Anlegen, Standard jetzt
    pub start_at: Option<DateTime<Utc>>,
}

/// Zählerwechsel: Endstände des alten und Startstände des neuen Geräts je
/// Register zum selben Zeitpunkt.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SwapBody {
    pub at: DateTime<Utc>,
    pub end_values: Vec<ValueDto>,
    pub start_values: Vec<ValueDto>,
    pub serial_number: Option<String>,
    pub rollover_digits: Option<u8>,
    #[serde(default)]
    pub decimal_places: u8,
    /// Übergeordnete Messstelle des neuen Geräts; fehlt bei eigenem Zähler
    pub parent_meter_id: Option<Uuid>,
}

/// Neues Gerät nach dem Zählerwechsel.
#[derive(Debug, Serialize, ToSchema)]
pub struct SwappedDto {
    pub device_id: Uuid,
}

/// Ablesung mit Messstelle.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReadingDetailDto {
    pub reading: ReadingDto,
    pub meter_id: Uuid,
    /// End- oder Startstand eines Zählerwechsels; eine Zeitänderung
    /// verschiebt alle Stände des Wechsels, Löschen ist gesperrt
    pub part_of_swap: bool,
    /// Belegfoto unter `/api/v1/readings/{id}/photo`
    pub has_photo: bool,
}

/// Korrektur einer Ablesung; ersetzt Zeitpunkt, Wert und Notiz (ohne
/// `note` wird die Notiz entfernt).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CorrectReadingBody {
    pub read_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub value: Decimal,
    pub note: Option<String>,
    /// Plausibilitätswarnung bestätigen
    #[serde(default)]
    pub confirm: bool,
}

/// Revision einer Ablesung.
#[derive(Debug, Serialize, ToSchema)]
pub struct RevisionDto {
    pub id: Uuid,
    pub reading_id: Uuid,
    pub register_id: Uuid,
    /// `update` oder `delete`
    pub action: String,
    pub changed_by: Uuid,
    pub changed_by_name: Option<String>,
    pub changed_at: DateTime<Utc>,
    pub old_read_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub old_value: Decimal,
    pub old_note: Option<String>,
    /// Fehlt beim Löschen
    pub new_read_at: Option<DateTime<Utc>>,
    #[schema(value_type = Option<String>)]
    pub new_value: Option<Decimal>,
    pub new_note: Option<String>,
}

impl From<&RevisionEntry> for RevisionDto {
    fn from(e: &RevisionEntry) -> Self {
        let r = &e.revision;
        Self {
            id: r.id.0,
            reading_id: r.reading_id.0,
            register_id: r.register_id.0,
            action: r.action.as_str().into(),
            changed_by: r.changed_by.0,
            changed_by_name: e.changed_by_name.clone(),
            changed_at: r.changed_at,
            old_read_at: r.old_read_at,
            old_value: r.old_value,
            old_note: r.old_note.clone(),
            new_read_at: r.new_read_at,
            new_value: r.new_value,
            new_note: r.new_note.clone(),
        }
    }
}

/// Eingaben des Formulars aus dem Body; ungültige Aufzählungen ergeben
/// Feldfehler (422).
fn meter_input(b: MeterBody) -> Result<MeterInput, AppError> {
    let mut e = FieldErrors::default();
    let kind = if let Ok(k) = b.kind.parse::<MeterKind>() {
        k
    } else {
        e.add("kind", "form.kind_invalid");
        MeterKind::Custom
    };
    let purpose = match b.purpose.as_deref() {
        None | Some("") => MeterPurpose::General,
        Some(p) => p.parse().unwrap_or_else(|_| {
            e.add("purpose", "meter.purpose_invalid");
            MeterPurpose::General
        }),
    };
    let schedule = b.schedule.and_then(|s| {
        let plan = ReadingSchedule::from_parts(&s.kind, s.value);
        if plan.is_none() {
            e.add("schedule", "meter.schedule_invalid");
        }
        plan
    });
    let season_weights = b.targets.season_weights.and_then(|w| {
        if let Ok(monthly_weights) = <[Decimal; 12]>::try_from(w) {
            Some(SeasonProfile { monthly_weights })
        } else {
            e.add("season_weights", "meter.season_invalid");
            None
        }
    });
    let mut registers = Vec::with_capacity(b.registers.len());
    for (i, r) in b.registers.into_iter().enumerate() {
        let direction = match r.direction.as_deref() {
            None | Some("") => Direction::Import,
            Some(d) => d.parse().unwrap_or_else(|_| {
                e.add(format!("register_{i}_direction"), "meter.direction_invalid");
                Direction::Import
            }),
        };
        registers.push(RegisterInput {
            id: r.id.map(RegisterId),
            code: r.code,
            direction,
            start_value: r.start_value,
        });
    }
    e.into_result()?;
    Ok(MeterInput {
        name: b.name,
        kind,
        unit: b.unit,
        schedule,
        max_daily_rate: b.max_daily_rate,
        billing_start: b.billing_start.map(|m| MonthDay {
            month: m.month,
            day: m.day,
        }),
        purpose,
        parent_meter_id: b.parent_meter_id.map(MeterId),
        targets: MeterTargets {
            season_weights,
            annual_target: b.targets.annual_target,
            tolerance_pct: b.targets.tolerance_pct,
        },
        serial_number: b.serial_number,
        rollover_digits: b.rollover_digits,
        decimal_places: b.decimal_places,
        registers,
        start_at: b.start_at.unwrap_or_else(Utc::now),
    })
}

async fn meter_detail(s: &WebState, ctx: &ApiCtx, id: MeterId) -> Result<MeterDetailDto, ApiError> {
    let d = s.uc.get_meter.execute(&ctx.tenant, id).await?;
    Ok(MeterDetailDto {
        meter: MeterDto::new(&d.aggregate, &d.readings),
        devices: d.aggregate.devices.iter().map(DeviceDto::from).collect(),
    })
}

async fn reading_detail(
    s: &WebState,
    ctx: &ApiCtx,
    id: ReadingId,
) -> Result<ReadingDetailDto, ApiError> {
    let rc = s.uc.get_reading.execute(&ctx.tenant, id).await?;
    Ok(ReadingDetailDto {
        reading: ReadingDto::from(&rc.reading),
        meter_id: rc.detail.aggregate.meter.id.0,
        part_of_swap: rc.part_of_swap,
        has_photo: rc.reading.photo_key.is_some(),
    })
}

/// Messstelle mit Gerät, Registern und Startständen anlegen.
#[utoipa::path(
    post,
    path = "/api/v1/meters",
    request_body = MeterBody,
    responses(
        (status = CREATED, body = MeterDetailDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    ),
    tag = "meters"
)]
async fn create_meter(
    State(s): State<WebState>,
    ctx: ApiCtx,
    body: Result<Json<MeterBody>, JsonRejection>,
) -> Result<(StatusCode, Json<MeterDetailDto>), ApiError> {
    let input = meter_input(json(body)?)?;
    let id = s.uc.create_meter.execute(&ctx.tenant, input).await?;
    Ok((StatusCode::CREATED, Json(meter_detail(&s, &ctx, id).await?)))
}

/// Stammdaten ändern; Art, Einheit und Register-Richtung sind nach der
/// ersten Ablesung fest.
#[utoipa::path(
    put,
    path = "/api/v1/meters/{id}",
    params(("id" = Uuid, Path, description = "Messstelle")),
    request_body = MeterBody,
    responses(
        (status = OK, body = MeterDetailDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    ),
    tag = "meters"
)]
async fn update_meter(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<MeterBody>, JsonRejection>,
) -> ApiResult<MeterDetailDto> {
    let input = meter_input(json(body)?)?;
    s.uc.update_meter
        .execute(&ctx.tenant, MeterId(id), input)
        .await?;
    Ok(Json(meter_detail(&s, &ctx, MeterId(id)).await?))
}

/// Messstelle mit allen Ablesungen, Belegfotos und Verträgen endgültig
/// löschen (SPEC_MTR_DELETE).
#[utoipa::path(
    delete,
    path = "/api/v1/meters/{id}",
    params(("id" = Uuid, Path, description = "Messstelle")),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
    ),
    tag = "meters"
)]
async fn delete_meter(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.uc.delete_meter.execute(&ctx.tenant, MeterId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Messstelle archivieren (SPEC_MTR_ARCHIVE).
#[utoipa::path(
    post,
    path = "/api/v1/meters/{id}/archive",
    params(("id" = Uuid, Path, description = "Messstelle")),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
    ),
    tag = "meters"
)]
async fn archive_meter(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.uc.archive_meter.execute(&ctx.tenant, MeterId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Archivierung aufheben (SPEC_MTR_ARCHIVE).
#[utoipa::path(
    post,
    path = "/api/v1/meters/{id}/restore",
    params(("id" = Uuid, Path, description = "Messstelle")),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
    ),
    tag = "meters"
)]
async fn restore_meter(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.uc.restore_meter.execute(&ctx.tenant, MeterId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Zählerwechsel (SPEC_MTR_SWAP_FLOW).
#[utoipa::path(
    post,
    path = "/api/v1/meters/{id}/swap",
    params(("id" = Uuid, Path, description = "Messstelle")),
    request_body = SwapBody,
    responses(
        (status = CREATED, body = SwappedDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    ),
    tag = "meters"
)]
async fn swap_device(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<SwapBody>, JsonRejection>,
) -> Result<(StatusCode, Json<SwappedDto>), ApiError> {
    let b = json(body)?;
    let values = |v: &[ValueDto]| -> Vec<(RegisterId, Decimal)> {
        v.iter()
            .map(|x| (RegisterId(x.register_id), x.value))
            .collect()
    };
    let input = SwapInput {
        at: b.at,
        end_values: values(&b.end_values),
        serial_number: b.serial_number,
        rollover_digits: b.rollover_digits,
        decimal_places: b.decimal_places,
        start_values: values(&b.start_values),
        parent_meter_id: b.parent_meter_id.map(MeterId),
    };
    let device =
        s.uc.swap_device
            .execute(&ctx.tenant, MeterId(id), input)
            .await?;
    Ok((
        StatusCode::CREATED,
        Json(SwappedDto {
            device_id: device.0,
        }),
    ))
}

/// Änderungsverlauf der Ablesungen, neueste zuerst; enthält gelöschte
/// Ablesungen (SPEC_RDG_HISTORY).
#[utoipa::path(
    get,
    path = "/api/v1/meters/{id}/history",
    params(("id" = Uuid, Path, description = "Messstelle")),
    responses(
        (status = OK, body = Vec<RevisionDto>),
        (status = NOT_FOUND, body = Problem),
    ),
    tag = "meters"
)]
async fn meter_history(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<Vec<RevisionDto>> {
    let list =
        s.uc.reading_history
            .execute(&ctx.tenant, MeterId(id))
            .await?;
    Ok(Json(list.iter().map(RevisionDto::from).collect()))
}

/// Einzelne Ablesung.
#[utoipa::path(
    get,
    path = "/api/v1/readings/{id}",
    params(("id" = Uuid, Path, description = "Ablesung")),
    responses(
        (status = OK, body = ReadingDetailDto),
        (status = NOT_FOUND, body = Problem),
    ),
    tag = "meters"
)]
async fn get_reading(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<ReadingDetailDto> {
    Ok(Json(reading_detail(&s, &ctx, ReadingId(id)).await?))
}

/// Zeitpunkt, Wert und Notiz korrigieren; schreibt eine Revision.
/// Warnungen ohne `confirm: true` ergeben 422 mit `warnings`.
#[utoipa::path(
    put,
    path = "/api/v1/readings/{id}",
    params(("id" = Uuid, Path, description = "Ablesung")),
    request_body = CorrectReadingBody,
    responses(
        (status = OK, body = ReadingDetailDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    ),
    tag = "meters"
)]
async fn correct_reading(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<CorrectReadingBody>, JsonRejection>,
) -> ApiResult<ReadingDetailDto> {
    let b = json(body)?;
    let id = ReadingId(id);
    let current = s.uc.get_reading.execute(&ctx.tenant, id).await?;
    let input = CorrectionInput {
        at: b.read_at,
        value: b.value,
        note: b.note,
    };
    match s
        .uc
        .correct_reading
        .execute(&ctx.tenant, id, &input, b.confirm)
        .await?
    {
        CorrectionOutcome::Saved(_) => Ok(Json(reading_detail(&s, &ctx, id).await?)),
        CorrectionOutcome::NeedsConfirmation(plausibility) => {
            Err(Problem::warnings(&[RegisterCheck {
                register_id: current.reading.register_id,
                plausibility,
            }])
            .into())
        }
    }
}

/// Ablesung löschen; schreibt eine Revision. Stände eines Zählerwechsels
/// sind gesperrt (409).
#[utoipa::path(
    delete,
    path = "/api/v1/readings/{id}",
    params(("id" = Uuid, Path, description = "Ablesung")),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
    ),
    tag = "meters"
)]
async fn delete_reading(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.uc.delete_reading
        .execute(&ctx.tenant, ReadingId(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Belegfoto einer Ablesung als JPEG; 404 ohne Foto (SPEC_SEC_UPLOADS).
#[utoipa::path(
    get,
    path = "/api/v1/readings/{id}/photo",
    params(("id" = Uuid, Path, description = "Ablesung")),
    responses(
        (status = OK, body = Vec<u8>, content_type = "image/jpeg"),
        (status = NOT_FOUND, body = Problem),
    ),
    tag = "meters"
)]
async fn reading_photo(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let jpeg =
        s.uc.get_reading_photo
            .execute(&ctx.tenant, ReadingId(id))
            .await?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "private, max-age=3600"),
        ],
        jpeg,
    )
        .into_response())
}
