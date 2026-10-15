//! Server-gerenderte Seiten (Askama + HTMX, ADR_002).

use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

use crate::state::AppState;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexPage {
    version: &'static str,
    db_backend: &'static str,
}

pub async fn index(State(state): State<AppState>) -> Response {
    render(&IndexPage {
        version: env!("CARGO_PKG_VERSION"),
        db_backend: state.config.db_backend(),
    })
}

fn render(t: &impl Template) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "template rendering failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
