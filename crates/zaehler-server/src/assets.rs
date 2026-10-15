//! Eingebettete statische Dateien, damit das Binary kein externes Asset-Verzeichnis benötigt.

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
        "htmx.min.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../static/htmx.min.js"),
    ),
];

pub async fn serve(Path(name): Path<String>) -> Response {
    match ASSETS.iter().find(|(n, ..)| *n == name) {
        Some((_, mime, body)) => (
            [
                (header::CONTENT_TYPE, *mime),
                (header::CACHE_CONTROL, "public, max-age=86400"),
            ],
            *body,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
