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

use crate::{extract::SESSION_CSRF, i18n::Lang, state::WebState};

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; \
                   connect-src 'self'; frame-ancestors 'none'; form-action 'self'; base-uri 'none'";

/// Setzt die Sicherheitsheader auf jede Antwort.
// @impl SPEC_SEC_HEADERS
pub async fn security_headers(req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    // Eine strengere Richtlinie des Handlers bleibt erhalten (Dokumente mit
    // `sandbox`, SPEC_DOC_UPLOADS).
    h.entry(header::CONTENT_SECURITY_POLICY)
        .or_insert(HeaderValue::from_static(CSP));
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
    // Seiten mit Haushaltsdaten nicht in gemeinsamen Caches ablegen
    let html = h
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(b"text/html"));
    // REST-API: Haushaltsdaten nie zwischenspeichern (SPEC_API_REST)
    if api {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    } else if html && !h.contains_key(header::CACHE_CONTROL) {
        h.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-cache"),
        );
    }
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

/// Schema, Host und Port eines Origins (`scheme://host[:port]`); ohne Port
/// gilt der Standardport des Schemas.
fn split_origin(origin: &str) -> Option<(&str, &str, u16)> {
    let (scheme, rest) = origin.split_once("://")?;
    let (host, port) = if rest.starts_with('[') {
        let end = rest.find(']')?;
        (&rest[..=end], rest[end + 1..].strip_prefix(':'))
    } else {
        match rest.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (rest, None),
        }
    };
    let port = match port {
        Some(p) => p.parse().ok()?,
        None => match scheme {
            "http" => 80,
            "https" => 443,
            _ => return None,
        },
    };
    Some((scheme, host, port))
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]"
}

/// Origin entspricht der öffentlichen URL. Ist diese lokal, gelten
/// `localhost`, `127.0.0.1` und `[::1]` mit gleichem Schema und Port als
/// gleichwertig.
// @impl SPEC_SEC_SESSION_CSRF
fn origin_allowed(origin: &str, public_origin: &str) -> bool {
    if origin == public_origin {
        return true;
    }
    match (split_origin(origin), split_origin(public_origin)) {
        (Some((scheme, host, port)), Some((p_scheme, p_host, p_port))) => {
            scheme == p_scheme && port == p_port && is_loopback(host) && is_loopback(p_host)
        }
        _ => false,
    }
}

/// Routen mit Multipart-Body; ihre Handler prüfen das Feld `_csrf` selbst.
const MULTIPART_ROUTES: [&str; 4] = [
    "/capture",
    "/import/upload",
    "/settings/import",
    "/documents",
];

/// Antwort bei fehlgeschlagener CSRF-Prüfung.
#[must_use]
pub fn csrf_failed(lang: Lang) -> Response {
    (StatusCode::FORBIDDEN, lang.t("error-csrf")).into_response()
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
    let lang = Lang::from_accept_language(
        req.headers()
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|v| v.to_str().ok()),
    );
    let forbidden = || csrf_failed(lang);
    if let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        && !origin_allowed(origin, &state.cfg.public_origin)
    {
        tracing::warn!(
            origin,
            expected = %state.cfg.public_origin,
            "origin mismatch"
        );
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

    // @test SPEC_SEC_SESSION_CSRF
    #[test]
    fn loopback_origins_are_equivalent_only_for_local_public_url() {
        let local = "http://localhost:8081";
        assert!(origin_allowed("http://localhost:8081", local));
        assert!(origin_allowed("http://127.0.0.1:8081", local));
        assert!(origin_allowed("http://[::1]:8081", local));
        assert!(origin_allowed(
            "http://localhost:8081",
            "http://127.0.0.1:8081"
        ));
        assert!(origin_allowed("http://127.0.0.1", "http://localhost:80"));
        // anderer Port, anderes Schema, fremder Host
        assert!(!origin_allowed("http://127.0.0.1:9999", local));
        assert!(!origin_allowed("https://127.0.0.1:8081", local));
        assert!(!origin_allowed("http://evil.example:8081", local));
        assert!(!origin_allowed("http://localhost.evil.example:8081", local));
        assert!(!origin_allowed("null", local));
        // nicht lokale öffentliche URL: nur exakt
        let public = "https://zaehler.example.com";
        assert!(origin_allowed(public, public));
        assert!(!origin_allowed("https://localhost", public));
        assert!(!origin_allowed("https://127.0.0.1:443", public));
    }
}
