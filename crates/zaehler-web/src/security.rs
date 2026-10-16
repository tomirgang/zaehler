//! Sicherheitsheader und CSRF-Prüfung (SPEC_SEC_HEADERS, SPEC_SEC_SESSION_CSRF).

use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use tower_sessions::Session;
use uuid::Uuid;

use crate::{extract::SESSION_CSRF, state::WebState};

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; \
                   connect-src 'self'; frame-ancestors 'none'; form-action 'self'; base-uri 'none'";

/// Setzt die Sicherheitsheader auf jede Antwort.
// @impl SPEC_SEC_HEADERS
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    h.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(self), geolocation=()"),
    );
    res
}

/// Zufälliges Token (2 × 122 Bit aus UUIDv4), Base64URL.
#[must_use]
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Vergleich in konstanter Zeit.
pub(crate) fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

const MAX_FORM_BYTES: usize = 1024 * 1024;

/// Routen mit Multipart-Body; ihre Handler prüfen das Feld `_csrf` selbst.
const MULTIPART_ROUTES: [&str; 2] = ["/capture", "/import/upload"];

fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "CSRF-Prüfung fehlgeschlagen").into_response()
}

/// Prüft bei zustandsändernden Requests das CSRF-Token (Header
/// `X-CSRF-Token` oder Formularfeld `_csrf`) und den `Origin`-Header.
// @impl SPEC_SEC_SESSION_CSRF
pub async fn csrf_guard(
    State(state): State<WebState>,
    session: Session,
    req: Request,
    next: Next,
) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    if let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        && origin != state.cfg.public_origin
    {
        tracing::warn!(origin, "origin mismatch");
        return forbidden();
    }
    let Ok(Some(expected)) = session.get::<String>(SESSION_CSRF).await else {
        return forbidden();
    };
    if let Some(token) = req
        .headers()
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
    {
        return if constant_time_eq(token, &expected) {
            next.run(req).await
        } else {
            forbidden()
        };
    }
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    // Multipart (Foto-Upload): das Formularfeld `_csrf` prüft der Handler, da
    // der Body hier nicht gepuffert wird. Nur für ausdrücklich genannte Routen.
    if content_type.starts_with("multipart/form-data") {
        return if MULTIPART_ROUTES.contains(&req.uri().path()) {
            next.run(req).await
        } else {
            forbidden()
        };
    }
    if !content_type.starts_with("application/x-www-form-urlencoded") {
        return forbidden();
    }
    let (parts, body) = req.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_FORM_BYTES).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let token = serde_urlencoded::from_bytes::<Vec<(String, String)>>(&bytes)
        .ok()
        .and_then(|pairs| {
            pairs
                .into_iter()
                .find(|(k, _)| k == "_csrf")
                .map(|(_, v)| v)
        });
    match token {
        Some(t) if constant_time_eq(&t, &expected) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
        _ => forbidden(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_compared_exactly() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(constant_time_eq(&a, &a.clone()));
        assert!(!constant_time_eq(&a, &b));
        assert!(!constant_time_eq(&a, &a[..42]));
    }
}
