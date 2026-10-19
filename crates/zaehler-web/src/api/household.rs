//! REST-API: Haushalt und hochgeladene Dokumente (SPEC_API_REST).
//!
//! Hochladen, Erkennen und Übernehmen von Dokumenten bleiben der Oberfläche
//! vorbehalten; die API liest Dokumente und verwirft offene.

use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use zaehler_app::{
    AppError,
    ids::DocumentId,
    model::{Document, DocumentType, Household, HouseholdData},
};

use super::{
    ApiResult,
    auth::ApiCtx,
    error::{ApiError, Problem},
};
use crate::state::WebState;

pub(super) fn routes(r: OpenApiRouter<WebState>) -> OpenApiRouter<WebState> {
    r.routes(routes!(get_household, update_household))
        .routes(routes!(list_documents))
        .routes(routes!(get_document, discard_document))
        .routes(routes!(document_file))
}

/// Haushalt des Tokens.
#[derive(Debug, Serialize, ToSchema)]
pub struct HouseholdDto {
    pub id: Uuid,
    pub name: String,
    /// IANA-Zeitzone, z.B. `Europe/Berlin`
    pub timezone: String,
    /// Wirksame Rolle des Tokens: `viewer`, `member` oder `owner`
    pub role: String,
}

/// Haushalt umbenennen und Zeitzone setzen (Owner).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateHouseholdBody {
    pub name: String,
    /// IANA-Zeitzone, z.B. `Europe/Berlin`
    pub timezone: String,
}

/// Hochgeladenes Dokument mit Erkennungsergebnis.
#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentDto {
    pub id: Uuid,
    /// `invoice` oder `receipt`
    pub kind: String,
    /// `application/pdf` oder `image/jpeg`
    pub media_type: String,
    /// Größe der gespeicherten Datei in Byte
    pub size: i64,
    /// Dateiname beim Upload, nur zur Anzeige
    pub original_name: String,
    /// `uploaded`, `analysed`, `failed`, `confirmed` oder `discarded`
    pub status: String,
    /// Über die Zählernummer zugeordnete Messstelle
    pub meter_id: Option<Uuid>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    /// Modell der Erkennung; leer ohne AI-Konfiguration
    pub model: Option<String>,
    pub analysed_at: Option<DateTime<Utc>>,
    /// Vorschlag der Rechnungserkennung (Dezimalwerte als Strings)
    #[schema(value_type = Option<Object>)]
    pub extraction: Option<serde_json::Value>,
    /// Vorschlag der Belegerkennung (Dezimalwerte als Strings)
    #[schema(value_type = Option<Object>)]
    pub receipt: Option<serde_json::Value>,
}

impl From<Household> for HouseholdDto {
    fn from(h: Household) -> Self {
        Self {
            id: h.id.0,
            name: h.data.name,
            timezone: h.data.timezone.name().to_owned(),
            role: String::new(),
        }
    }
}

impl From<Document> for DocumentDto {
    fn from(d: Document) -> Self {
        Self {
            id: d.id.0,
            kind: d.kind.as_str().into(),
            media_type: d.media_type.media_type().into(),
            size: d.bytes,
            original_name: d.original_name,
            status: d.status.as_str().into(),
            meter_id: d.meter_id.map(|m| m.0),
            created_by: d.created_by.0,
            created_at: d.created_at,
            model: d.model,
            analysed_at: d.analysed_at,
            extraction: d.extraction.and_then(|x| serde_json::to_value(x).ok()),
            receipt: d.receipt.and_then(|x| serde_json::to_value(x).ok()),
        }
    }
}

async fn household_dto(s: &WebState, ctx: &ApiCtx) -> Result<HouseholdDto, ApiError> {
    let h = s.uc.get_household.execute(&ctx.tenant).await?;
    Ok(HouseholdDto {
        role: ctx.tenant.role().as_str().into(),
        ..h.into()
    })
}

/// Haushalt des Tokens mit der wirksamen Rolle.
#[utoipa::path(
    get,
    path = "/api/v1/household",
    tag = "household",
    responses(
        (status = OK, body = HouseholdDto),
        (status = UNAUTHORIZED, body = Problem),
    )
)]
async fn get_household(State(s): State<WebState>, ctx: ApiCtx) -> ApiResult<HouseholdDto> {
    Ok(Json(household_dto(&s, &ctx).await?))
}

/// Haushalt umbenennen und Zeitzone ändern (Owner). Löschen ist nur in der
/// Oberfläche möglich.
#[utoipa::path(
    put,
    path = "/api/v1/household",
    tag = "household",
    request_body = UpdateHouseholdBody,
    responses(
        (status = OK, body = HouseholdDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn update_household(
    State(s): State<WebState>,
    ctx: ApiCtx,
    body: Result<Json<UpdateHouseholdBody>, JsonRejection>,
) -> ApiResult<HouseholdDto> {
    let b = super::json(body)?;
    let timezone = b
        .timezone
        .trim()
        .parse::<Tz>()
        .map_err(|_| AppError::field("timezone", "form.timezone_invalid"))?;
    s.uc.update_household
        .execute(
            &ctx.tenant,
            HouseholdData {
                name: b.name,
                timezone,
            },
        )
        .await?;
    Ok(Json(household_dto(&s, &ctx).await?))
}

/// Offene (weder übernommene noch verworfene) Dokumente (Member).
#[utoipa::path(
    get,
    path = "/api/v1/documents",
    tag = "documents",
    responses(
        (status = OK, body = Vec<DocumentDto>),
        (status = FORBIDDEN, body = Problem),
    )
)]
async fn list_documents(State(s): State<WebState>, ctx: ApiCtx) -> ApiResult<Vec<DocumentDto>> {
    let docs = s.uc.list_documents.execute(&ctx.tenant).await?;
    Ok(Json(docs.into_iter().map(DocumentDto::from).collect()))
}

/// Dokument mit Vorschlag, auch übernommene und verworfene (Viewer).
#[utoipa::path(
    get,
    path = "/api/v1/documents/{id}",
    tag = "documents",
    params(("id" = Uuid, Path, description = "Dokument")),
    responses(
        (status = OK, body = DocumentDto),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn get_document(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<DocumentDto> {
    let d =
        s.uc.get_document
            .execute(&ctx.tenant, DocumentId(id))
            .await?;
    Ok(Json(d.into()))
}

/// Offenes Dokument verwerfen (Member): Status `discarded`, die Datei
/// entfernt das Housekeeping später. Übernommene oder bereits verworfene
/// Dokumente liefern 404.
#[utoipa::path(
    delete,
    path = "/api/v1/documents/{id}",
    tag = "documents",
    params(("id" = Uuid, Path, description = "Dokument")),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn discard_document(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.uc.discard_document
        .execute(&ctx.tenant, DocumentId(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Dateiname für `Content-Disposition`: nur ASCII, Endung nach Dateiart
/// (wie die Seite `/documents/{id}/file`).
fn attachment_name(name: &str, kind: DocumentType) -> String {
    let stem: String = name
        .rsplit_once('.')
        .map_or(name, |(s, _)| s)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    format!("{stem}.{}", kind.extension())
}

/// Gespeicherte Datei, nur als Download (Viewer).
// @impl SPEC_DOC_UPLOADS
#[utoipa::path(
    get,
    path = "/api/v1/documents/{id}/file",
    tag = "documents",
    params(("id" = Uuid, Path, description = "Dokument")),
    responses(
        (status = OK, description = "PDF oder JPEG als Download", content(
            (Vec<u8> = "application/pdf"),
            (Vec<u8> = "image/jpeg"),
        )),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn document_file(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let (kind, bytes, name) =
        s.uc.get_document_file
            .execute(&ctx.tenant, DocumentId(id))
            .await?;
    let disposition = HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"",
        attachment_name(&name, kind)
    ))
    .unwrap_or(HeaderValue::from_static("attachment"));
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(kind.media_type()),
            ),
            (header::CONTENT_DISPOSITION, disposition),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("sandbox; default-src 'none'"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store"),
            ),
        ],
        bytes,
    )
        .into_response())
}
