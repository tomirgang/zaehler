//! Web-App-Manifest und Service Worker (SPEC_UI_PWA).

use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::{extract::AnonLang, state::WebState};

/// Hintergrund- und Theme-Farbe, entsprechen `--bg` und `--accent` in `app.css`.
const BACKGROUND: &str = "#f7f7f5";
const THEME: &str = "#2f6f4f";

const SERVICE_WORKER: &str = include_str!("../../static/sw.js");

/// `GET /manifest.webmanifest`: Name und Beschreibung in der Sprache des
/// Browsers, Icons unter dem versionierten Pfad der statischen Dateien.
// @impl SPEC_UI_PWA
pub async fn manifest(State(state): State<WebState>, AnonLang(lang): AnonLang) -> Response {
    let icons = format!("/static/{}", state.cfg.version);
    let body = json!({
        "name": lang.t("app-name"),
        "short_name": lang.t("app-name"),
        "description": lang.t("app-description"),
        "lang": lang.code(),
        "id": "/",
        "start_url": "/",
        "scope": "/",
        "display": "standalone",
        "background_color": BACKGROUND,
        "theme_color": THEME,
        "icons": [
            { "src": format!("{icons}/icon-192.png"), "sizes": "192x192", "type": "image/png", "purpose": "any" },
            { "src": format!("{icons}/icon-512.png"), "sizes": "512x512", "type": "image/png", "purpose": "any" },
            { "src": format!("{icons}/icon-maskable-512.png"), "sizes": "512x512", "type": "image/png", "purpose": "maskable" },
        ],
    });
    (
        [
            (header::CONTENT_TYPE, "application/manifest+json"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body.to_string(),
    )
        .into_response()
}

/// `GET /sw.js`: unter der Wurzel, damit der Service Worker die ganze App
/// abdeckt; ohne Cache, damit Änderungen sofort wirken.
// @impl SPEC_UI_PWA
pub async fn service_worker() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SERVICE_WORKER,
    )
        .into_response()
}
