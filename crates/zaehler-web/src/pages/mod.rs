//! Seiten-Handler und gemeinsame Hilfen.

// Handler liefern `Result<Response, Response>`; beide Seiten sind Antworten.
#![allow(clippy::result_large_err)]

pub mod analysis;
pub mod capture;
pub mod dashboard;
pub mod import;
pub mod meters;
pub mod readings;
pub mod settings;

use std::collections::HashMap;

use askama::Template;
use axum::{
    extract::State,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use zaehler_app::{AppError, FieldErrors};

use crate::{
    extract::{AnonLang, PageCtx},
    i18n::Lang,
    state::WebState,
};

/// Template als HTML; Renderfehler ergeben 500.
pub fn render(t: &impl Template) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "template rendering failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Template mit Statuscode.
pub fn render_status(status: StatusCode, t: &impl Template) -> Response {
    let mut res = render(t);
    if res.status() == StatusCode::OK {
        *res.status_mut() = status;
    }
    res
}

/// 303 See Other.
#[must_use]
pub fn see_other(location: &str) -> Response {
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, location.to_owned())],
    )
        .into_response()
}

#[derive(Template)]
#[template(path = "pages/error.html")]
struct ErrorPage<'a> {
    ctx: &'a PageCtx,
    message: String,
}

#[derive(Template)]
#[template(path = "pages/error_plain.html")]
struct PlainErrorPage {
    lang: Lang,
    static_prefix: String,
    message: String,
}

/// Abbildung von `AppError` auf HTTP (SPEC_ERR_HANDLING).
// @impl SPEC_ERR_HANDLING
pub fn error_response(ctx: &PageCtx, hx: bool, err: AppError) -> Response {
    let lang = ctx.lang;
    let (status, message) = match err {
        AppError::Unauthenticated => return crate::extract::login_redirect(hx),
        AppError::NotFound => (StatusCode::NOT_FOUND, lang.t("error-not-found")),
        AppError::Forbidden => (StatusCode::FORBIDDEN, lang.t("error-forbidden")),
        AppError::Conflict(code) => (StatusCode::CONFLICT, lang.code_text(code)),
        AppError::Validation(errors) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            errors
                .0
                .iter()
                .map(|e| lang.err(e))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        AppError::Upstream(what) => {
            let reference = uuid::Uuid::now_v7();
            tracing::error!(%reference, upstream = what, "upstream error");
            (
                StatusCode::BAD_GATEWAY,
                lang.ta("error-internal", &[("ref", reference.to_string())]),
            )
        }
        AppError::Internal(msg) => {
            let reference = uuid::Uuid::now_v7();
            tracing::error!(%reference, error = %msg, "internal error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                lang.ta("error-internal", &[("ref", reference.to_string())]),
            )
        }
    };
    render_status(status, &ErrorPage { ctx, message })
}

/// Unbekannte Pfade.
pub async fn not_found(State(state): State<WebState>, AnonLang(lang): AnonLang) -> Response {
    let page = PlainErrorPage {
        lang,
        static_prefix: format!("/static/{}", state.cfg.version),
        message: lang.t("error-not-found"),
    };
    render_status(StatusCode::NOT_FOUND, &page)
}

/// Formularfelder (bei mehrfachen Schlüsseln gilt der letzte Wert).
#[derive(Debug, Default)]
pub struct FormData(pub HashMap<String, String>);

impl FormData {
    #[must_use]
    pub fn new(pairs: Vec<(String, String)>) -> Self {
        Self(pairs.into_iter().collect())
    }

    #[must_use]
    pub fn get(&self, key: &str) -> &str {
        self.0.get(key).map_or("", String::as_str)
    }

    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    #[must_use]
    pub fn opt(&self, key: &str) -> Option<String> {
        let v = self.get(key).trim();
        (!v.is_empty()).then(|| v.to_owned())
    }
}

/// Fehlertexte eines Feldes.
#[must_use]
pub fn field_errors(lang: Lang, errors: &FieldErrors, field: &str) -> Vec<String> {
    errors
        .for_field(field)
        .into_iter()
        .map(|e| lang.err(e))
        .collect()
}

/// Liveness (SPEC_OPS_HEALTH).
// @impl SPEC_OPS_HEALTH
pub async fn live() -> &'static str {
    "ok"
}

/// Readiness über den Port `Readiness` (SPEC_OPS_HEALTH).
// @impl SPEC_OPS_HEALTH
pub async fn ready(State(state): State<WebState>) -> (StatusCode, &'static str) {
    match state.readiness.check().await {
        Ok(()) => (StatusCode::OK, "ready"),
        Err(e) => {
            tracing::warn!(error = %e, "readiness check failed");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready")
        }
    }
}
