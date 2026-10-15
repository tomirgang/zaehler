//! Driving-Adapter HTTP (ADR_022): Web-UI mit Axum, Askama und HTMX.
//!
//! Handler parsen Eingaben, rufen Anwendungsfälle aus `zaehler-app` auf und
//! rendern View-Modelle; sie greifen nie direkt auf Adapter zu.

pub mod assets;
pub mod auth;
pub mod extract;
pub mod i18n;
pub mod pages;
pub mod security;
pub mod state;

use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{DefaultBodyLimit, MatchedPath, Request},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use tower_http::{compression::CompressionLayer, timeout::TimeoutLayer, trace::TraceLayer};
use tower_sessions::{Expiry, SessionManagerLayer, SessionStore, cookie::SameSite};

pub use auth::OidcClient;
pub use state::{WebConfig, WebState};

const FORM_LIMIT: usize = 64 * 1024;

/// Vollständiger Router; `store` speichert die Sitzungen (SPEC_SEC_SESSION_CSRF).
pub fn router<S: SessionStore + Clone>(state: WebState, store: S) -> Router {
    let sessions = SessionManagerLayer::new(store)
        .with_name("zaehler_session")
        .with_http_only(true)
        .with_same_site(SameSite::Lax)
        .with_secure(state.cfg.cookie_secure)
        .with_expiry(Expiry::OnInactivity(time::Duration::hours(
            state.cfg.session_ttl_hours,
        )));

    let app = Router::new()
        .route("/", get(pages::dashboard::show))
        .route(
            "/meters",
            get(pages::meters::list).post(pages::meters::create),
        )
        .route("/meters/new", get(pages::meters::new_form))
        .route("/meters/{id}", get(pages::meters::detail))
        .route(
            "/meters/{id}/edit",
            get(pages::meters::edit_form).post(pages::meters::update),
        )
        .route(
            "/meters/{id}/swap",
            get(pages::meters::swap_form).post(pages::meters::swap),
        )
        .route("/meters/{id}/readings/new", get(pages::readings::new_form))
        .route("/meters/{id}/readings", post(pages::readings::record))
        .route("/meters/{id}/readings/check", post(pages::readings::check))
        .route("/readings/{id}/delete", post(pages::readings::delete))
        .route("/readings/{id}/photo", get(pages::readings::photo))
        .route(
            "/capture",
            get(pages::capture::upload_form)
                .post(pages::capture::analyse)
                .layer(DefaultBodyLimit::max(state.cfg.max_photo_bytes)),
        )
        .route("/capture/{id}", get(pages::capture::show))
        .route("/capture/{id}/photo", get(pages::capture::photo))
        .route("/capture/{id}/confirm", post(pages::capture::confirm))
        .route("/capture/{id}/discard", post(pages::capture::discard))
        .route("/settings", get(pages::settings::show))
        .route(
            "/settings/household",
            post(pages::settings::update_household),
        )
        .route("/settings/locale", post(pages::settings::update_locale))
        .route("/households", post(pages::settings::create_household))
        .route(
            "/households/switch",
            post(pages::settings::switch_household),
        )
        .route(
            "/households/delete",
            post(pages::settings::delete_household),
        )
        .route("/auth/login", get(auth::login_page))
        .route("/auth/oidc", get(auth::oidc_start))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/dev-login", get(auth::dev_login))
        .fallback(pages::not_found)
        // innen nach außen: Body-Limit, CSRF, Sitzung
        .layer(DefaultBodyLimit::max(FORM_LIMIT))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::csrf_guard,
        ))
        .layer(sessions);

    Router::new()
        .route("/static/{version}/{name}", get(assets::serve))
        .route("/healthz", get(pages::live))
        .route("/readyz", get(pages::ready))
        .merge(app)
        .layer(middleware::from_fn(security::security_headers))
        .layer(middleware::from_fn(track_metrics))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(120),
        ))
        .with_state(state)
}

/// HTTP-Metriken mit Route-Template statt konkreter IDs (SPEC_OPS_METRICS).
// @impl SPEC_OPS_METRICS
async fn track_metrics(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_owned(), |p| p.as_str().to_owned());
    let method = req.method().to_string();
    let response = next.run(req).await;
    let labels = [
        ("method", method),
        ("path", path),
        ("status", response.status().as_u16().to_string()),
    ];
    metrics::counter!("http_requests_total", &labels).increment(1);
    metrics::histogram!("http_request_duration_seconds", &labels)
        .record(start.elapsed().as_secs_f64());
    response
}
