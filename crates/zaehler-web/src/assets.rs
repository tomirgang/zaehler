//! Eingebettete statische Dateien unter `/static/{version}/{name}`.

use axum::{
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

const ASSETS: &[(&str, &str, &[u8])] = &[
    (
        "app.css",
        "text/css; charset=utf-8",
        include_bytes!("../static/app.css"),
    ),
    (
        "app.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../static/app.js"),
    ),
    (
        "htmx.min.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../static/htmx.min.js"),
    ),
    (
        "charts.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../static/charts.js"),
    ),
    // Apache ECharts 5.6.0 (Apache-2.0), nur auf Seiten mit Diagramm geladen
    (
        "echarts.min.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../static/echarts.min.js"),
    ),
    // App-Icons (SPEC_UI_PWA), erzeugt mit `tools/make_icons.py`
    (
        "icon.svg",
        "image/svg+xml",
        include_bytes!("../static/icon.svg"),
    ),
    (
        "icon-192.png",
        "image/png",
        include_bytes!("../static/icon-192.png"),
    ),
    (
        "icon-512.png",
        "image/png",
        include_bytes!("../static/icon-512.png"),
    ),
    (
        "icon-maskable-512.png",
        "image/png",
        include_bytes!("../static/icon-maskable-512.png"),
    ),
    (
        "apple-touch-icon.png",
        "image/png",
        include_bytes!("../static/apple-touch-icon.png"),
    ),
];

/// Die Version im Pfad macht Dateien unveränderlich cachebar; in
/// Debug-Builds wird nicht gecacht.
pub async fn serve(Path((_version, name)): Path<(String, String)>) -> Response {
    let cache = if cfg!(debug_assertions) {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    match ASSETS.iter().find(|(n, ..)| *n == name) {
        Some((_, mime, body)) => (
            [
                (header::CONTENT_TYPE, *mime),
                (header::CACHE_CONTROL, cache),
            ],
            *body,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
