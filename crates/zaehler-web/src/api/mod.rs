//! REST-API `/api/v1` (ADR_023, SPEC_API_REST): eigener Router mit
//! Bearer-Authentifizierung, ruft dieselben Anwendungsfälle wie die Seiten
//! auf. Die OpenAPI-Beschreibung entsteht aus den Handlern.

pub mod auth;
pub mod dto;
pub mod error;

use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::StatusCode,
    routing::get,
};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use utoipa::{
    Modify, OpenApi,
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use zaehler_app::{
    ids::{ContractId, MeterId, ReadingId, RegisterId},
    usecase::{
        api::{IngestItem, MAX_INGEST},
        reading::{ReadingsInput, RecordOutcome},
    },
};
use zaehler_domain::reading::ReadingSource;

use self::{
    auth::ApiCtx,
    dto::{
        AnalysisDto, ContractDto, CostsDto, DeviceDto, IngestBody, IngestResponse, IngestResultDto,
        MeterDetailDto, MeterDto, ReadingDto, ReadingPage, ReadingQuery, RecordReadingsBody,
        RecordedDto,
    },
    error::{ApiError, Problem},
};
use crate::state::WebState;

/// Höchstgröße eines Bodys (SPEC_API_REST).
pub const BODY_LIMIT: usize = 1024 * 1024;

/// Standard und Höchstwert von `limit` bei Ablesungen.
const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1000;

type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Zähler REST-API",
        description = "Zähler, Stände, Verträge und Auswertungen eines Haushalts. \
            Dezimalwerte als Strings, Zeitpunkte RFC 3339 in UTC.",
        version = "1"
    ),
    modifiers(&BearerAuth),
    security(("bearer" = []))
)]
struct ApiDoc;

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some("Persönliches API-Token `zt_…`"))
                    .build(),
            ),
        );
    }
}

/// Router der API und ihre OpenAPI-Beschreibung.
// @impl SPEC_API_REST
pub fn router() -> (Router<WebState>, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(list_meters))
        .routes(routes!(get_meter))
        .routes(routes!(list_readings, record_readings))
        .routes(routes!(ingest_readings))
        .routes(routes!(meter_analysis))
        .routes(routes!(meter_costs))
        .routes(routes!(list_contracts))
        .routes(routes!(get_contract))
        .split_for_parts();
    let doc = serde_json::to_value(&api).unwrap_or_default();
    let router = router
        .route(
            "/api/v1/openapi.json",
            get(move || {
                let doc = doc.clone();
                async move { Json(doc) }
            }),
        )
        .route(
            "/api/v1/{*rest}",
            axum::routing::any(|| async {
                Problem::new(StatusCode::NOT_FOUND).with_code("route.unknown")
            }),
        )
        .layer(DefaultBodyLimit::max(BODY_LIMIT));
    (router, api)
}

#[allow(clippy::result_large_err)] // wird sofort in `ApiError` umgewandelt
fn json<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Problem> {
    body.map(|Json(b)| b).map_err(|e| {
        let code = match e {
            JsonRejection::BytesRejection(_) => "body.too_large",
            JsonRejection::MissingJsonContentType(_) => "body.content_type",
            _ => "body.invalid",
        };
        if code == "body.too_large" {
            Problem::new(StatusCode::PAYLOAD_TOO_LARGE).with_code(code)
        } else {
            Problem::bad_request(code)
        }
    })
}

/// Messstellen mit Registern und letztem Stand.
#[utoipa::path(
    get,
    path = "/api/v1/meters",
    responses(
        (status = OK, body = Vec<MeterDto>),
        (status = UNAUTHORIZED, body = Problem),
    )
)]
async fn list_meters(State(s): State<WebState>, ctx: ApiCtx) -> ApiResult<Vec<MeterDto>> {
    let mut out = Vec::new();
    for m in s.uc.list_meters.execute(&ctx.tenant).await? {
        let d = s.uc.get_meter.execute(&ctx.tenant, m.meter.id).await?;
        out.push(MeterDto::new(&d.aggregate, &d.readings));
    }
    Ok(Json(out))
}

/// Messstelle mit Geräten.
#[utoipa::path(
    get,
    path = "/api/v1/meters/{id}",
    params(("id" = Uuid, Path, description = "Messstelle")),
    responses(
        (status = OK, body = MeterDetailDto),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn get_meter(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<MeterDetailDto> {
    let d = s.uc.get_meter.execute(&ctx.tenant, MeterId(id)).await?;
    Ok(Json(MeterDetailDto {
        meter: MeterDto::new(&d.aggregate, &d.readings),
        devices: d.aggregate.devices.iter().map(DeviceDto::from).collect(),
    }))
}

fn encode_cursor(at: DateTime<Utc>, id: ReadingId) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
        "{}|{id}",
        at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
    ))
}

fn decode_cursor(c: &str) -> Option<(DateTime<Utc>, Uuid)> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(c)
        .ok()?;
    let text = String::from_utf8(raw).ok()?;
    let (at, id) = text.split_once('|')?;
    Some((
        DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&Utc),
        id.parse().ok()?,
    ))
}

/// Ablesungen der Messstelle, chronologisch und seitenweise.
#[utoipa::path(
    get,
    path = "/api/v1/meters/{id}/readings",
    params(("id" = Uuid, Path, description = "Messstelle"), ReadingQuery),
    responses(
        (status = OK, body = ReadingPage),
        (status = BAD_REQUEST, body = Problem),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn list_readings(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    query: Result<Query<ReadingQuery>, QueryRejection>,
) -> ApiResult<ReadingPage> {
    let Query(q) = query.map_err(|_| Problem::bad_request("query.invalid"))?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return Err(Problem::bad_request("query.limit").into());
    }
    let after = match q.cursor.as_deref() {
        Some(c) => Some(decode_cursor(c).ok_or_else(|| Problem::bad_request("query.cursor"))?),
        None => None,
    };
    let d = s.uc.get_meter.execute(&ctx.tenant, MeterId(id)).await?;
    let mut matching = d
        .readings
        .iter()
        .filter(|r| q.from.is_none_or(|f| r.read_at >= f))
        .filter(|r| q.to.is_none_or(|t| r.read_at < t))
        .filter(|r| after.is_none_or(|(at, id)| (r.read_at, r.id.0) > (at, id)));
    let items: Vec<ReadingDto> = matching
        .by_ref()
        .take(limit)
        .map(ReadingDto::from)
        .collect();
    let next_cursor = if matching.next().is_some() {
        items
            .last()
            .map(|r| encode_cursor(r.read_at, ReadingId(r.id)))
    } else {
        None
    };
    Ok(Json(ReadingPage { items, next_cursor }))
}

/// Ablesungen erfassen wie das Formular; Warnungen ohne `confirm: true`
/// ergeben 422 mit `warnings`.
#[utoipa::path(
    post,
    path = "/api/v1/meters/{id}/readings",
    params(("id" = Uuid, Path, description = "Messstelle")),
    request_body = RecordReadingsBody,
    responses(
        (status = CREATED, body = RecordedDto),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = CONFLICT, body = Problem),
    )
)]
async fn record_readings(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<RecordReadingsBody>, JsonRejection>,
) -> Result<(StatusCode, Json<RecordedDto>), ApiError> {
    let b = json(body)?;
    let input = ReadingsInput {
        meter_id: MeterId(id),
        at: b.read_at,
        values: b
            .values
            .iter()
            .map(|v| (RegisterId(v.register_id), v.value))
            .collect(),
        note: b.note,
    };
    match s
        .uc
        .record_readings
        .execute(&ctx.tenant, &input, b.confirm, ReadingSource::Api)
        .await?
    {
        RecordOutcome::Saved(ids) => Ok((
            StatusCode::CREATED,
            Json(RecordedDto {
                reading_ids: ids.into_iter().map(|i| i.0).collect(),
            }),
        )),
        RecordOutcome::NeedsConfirmation(checks) => Err(Problem::warnings(&checks).into()),
    }
}

/// Automatisch gelieferte Stände; je Register, Tag und Token höchstens eine
/// Ablesung (SPEC_API_DAILY_READINGS).
#[utoipa::path(
    post,
    path = "/api/v1/readings",
    request_body = IngestBody,
    responses(
        (status = OK, body = IngestResponse),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = PAYLOAD_TOO_LARGE, body = Problem),
    )
)]
async fn ingest_readings(
    State(s): State<WebState>,
    ctx: ApiCtx,
    body: Result<Json<IngestBody>, JsonRejection>,
) -> ApiResult<IngestResponse> {
    let b = json(body)?;
    if b.readings.is_empty() || b.readings.len() > MAX_INGEST {
        return Err(Problem::bad_request("api.count_invalid").into());
    }
    // Einträge ohne gültiges Ziel erhalten einen Fehler, ohne die übrigen
    // zu blockieren
    let mut items = Vec::new();
    let mut slots = Vec::with_capacity(b.readings.len());
    for r in &b.readings {
        if let Some(target) = r.target() {
            slots.push(Some(items.len()));
            items.push(IngestItem {
                target,
                at: r.read_at,
                value: r.value,
            });
        } else {
            slots.push(None);
        }
    }
    let results = if items.is_empty() {
        Vec::new()
    } else {
        s.uc.ingest_readings
            .execute(&ctx.tenant, ctx.token, &items)
            .await?
    };
    Ok(Json(IngestResponse {
        results: slots
            .into_iter()
            .map(|slot| match slot.and_then(|i| results.get(i)) {
                Some(r) => IngestResultDto::from(r),
                None => IngestResultDto {
                    status: "error".into(),
                    reading_id: None,
                    error: Some("api.target_invalid".into()),
                },
            })
            .collect(),
    }))
}

/// Monatsserie, Vergleiche, Prognose und Ampel.
#[utoipa::path(
    get,
    path = "/api/v1/meters/{id}/analysis",
    params(("id" = Uuid, Path, description = "Messstelle")),
    responses((status = OK, body = AnalysisDto), (status = NOT_FOUND, body = Problem))
)]
async fn meter_analysis(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<AnalysisDto> {
    let a =
        s.uc.meter_analysis
            .execute(&ctx.tenant, MeterId(id), None)
            .await?;
    Ok(Json(AnalysisDto::from(&a)))
}

/// Kostenansicht (SPEC_TAR_COST_VIEW).
#[utoipa::path(
    get,
    path = "/api/v1/meters/{id}/costs",
    params(("id" = Uuid, Path, description = "Messstelle")),
    responses((status = OK, body = CostsDto), (status = NOT_FOUND, body = Problem))
)]
async fn meter_costs(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<CostsDto> {
    let v = s.uc.meter_costs.execute(&ctx.tenant, MeterId(id)).await?;
    Ok(Json(CostsDto::from(&v)))
}

/// Verträge des Haushalts.
#[utoipa::path(
    get,
    path = "/api/v1/contracts",
    responses((status = OK, body = Vec<ContractDto>))
)]
async fn list_contracts(State(s): State<WebState>, ctx: ApiCtx) -> ApiResult<Vec<ContractDto>> {
    let list = s.uc.list_contracts.execute(&ctx.tenant).await?;
    Ok(Json(list.iter().map(ContractDto::from).collect()))
}

/// Vertrag mit Positionen, Preisständen und Abschlägen.
#[utoipa::path(
    get,
    path = "/api/v1/contracts/{id}",
    params(("id" = Uuid, Path, description = "Vertrag")),
    responses((status = OK, body = ContractDto), (status = NOT_FOUND, body = Problem))
)]
async fn get_contract(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<ContractDto> {
    let d =
        s.uc.get_contract
            .execute(&ctx.tenant, ContractId(id))
            .await?;
    Ok(Json(ContractDto::from(&d.contract)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trip() {
        let at = Utc::now();
        let id = ReadingId(Uuid::now_v7());
        assert_eq!(decode_cursor(&encode_cursor(at, id)), Some((at, id.0)));
        assert_eq!(decode_cursor("kein cursor"), None);
    }
}
