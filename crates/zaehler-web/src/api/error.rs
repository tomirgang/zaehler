//! Fehler der REST-API als `application/problem+json` (RFC 9457).

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;
use zaehler_app::{AppError, usecase::reading::RegisterCheck};
use zaehler_domain::reading::Plausibility;

/// Fehlercode je Feld (nicht übersetzt).
#[derive(Debug, Serialize, ToSchema)]
pub struct FieldProblem {
    pub field: String,
    pub code: String,
}

/// Plausibilitätswarnung je Register.
#[derive(Debug, Serialize, ToSchema)]
pub struct WarningDto {
    pub register_id: Uuid,
    /// `decreasing`, `rate_too_high` oder `out_of_order`
    pub kind: String,
}

/// Problem-Details nach RFC 9457.
#[derive(Debug, Serialize, ToSchema)]
pub struct Problem {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub status: u16,
    /// Fachlicher Fehlercode, z.B. `reading.duplicate`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Referenz im Log bei internen Fehlern
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldProblem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<WarningDto>,
}

impl Problem {
    #[must_use]
    pub fn new(status: StatusCode) -> Self {
        Self {
            kind: "about:blank".into(),
            title: status.canonical_reason().unwrap_or_default().into(),
            status: status.as_u16(),
            code: None,
            reference: None,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_code(mut self, code: &str) -> Self {
        self.code = Some(code.to_owned());
        self
    }

    /// Ohne gültiges Bearer-Token (SPEC_API_AUTH).
    #[must_use]
    pub fn unauthenticated() -> Self {
        Self::new(StatusCode::UNAUTHORIZED)
    }

    /// Ungültiger Body oder Parameter.
    #[must_use]
    pub fn bad_request(code: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST).with_code(code)
    }

    /// Plausibilitätswarnungen ohne `confirm: true`.
    #[must_use]
    pub fn warnings(checks: &[RegisterCheck]) -> Self {
        let mut p = Self::new(StatusCode::UNPROCESSABLE_ENTITY).with_code("reading.warnings");
        p.warnings = checks
            .iter()
            .filter(|c| c.plausibility != Plausibility::Ok)
            .map(|c| WarningDto {
                register_id: c.register_id.0,
                kind: match c.plausibility {
                    Plausibility::Decreasing => "decreasing",
                    Plausibility::RateTooHigh => "rate_too_high",
                    Plausibility::OutOfOrder => "out_of_order",
                    Plausibility::Ok => "ok",
                }
                .into(),
            })
            .collect();
        p
    }
}

impl From<AppError> for Problem {
    fn from(e: AppError) -> Self {
        match e {
            AppError::NotFound => Self::new(StatusCode::NOT_FOUND),
            AppError::Forbidden => Self::new(StatusCode::FORBIDDEN),
            AppError::Unauthenticated => Self::unauthenticated(),
            AppError::Validation(f) => {
                let mut p = Self::new(StatusCode::UNPROCESSABLE_ENTITY);
                p.errors =
                    f.0.into_iter()
                        .map(|x| FieldProblem {
                            field: x.field,
                            code: x.code.into(),
                        })
                        .collect();
                p
            }
            AppError::Conflict(code) => Self::new(StatusCode::CONFLICT).with_code(code),
            AppError::Upstream(code) => Self::new(StatusCode::SERVICE_UNAVAILABLE).with_code(code),
            AppError::Internal(msg) => {
                let reference = Uuid::now_v7();
                tracing::error!(%reference, error = %msg, "api request failed");
                let mut p = Self::new(StatusCode::INTERNAL_SERVER_ERROR);
                p.reference = Some(reference.to_string());
                p
            }
        }
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let unauthorized = status == StatusCode::UNAUTHORIZED;
        let mut res = (status, Json(self)).into_response();
        let headers = res.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        if unauthorized {
            headers.insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        res
    }
}

/// Fehlerantwort der Handler; `Problem` liegt auf dem Heap, damit
/// `Result` klein bleibt.
#[derive(Debug)]
pub struct ApiError(pub Box<Problem>);

impl From<Problem> for ApiError {
    fn from(p: Problem) -> Self {
        Self(Box::new(p))
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        Problem::from(e).into()
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}
