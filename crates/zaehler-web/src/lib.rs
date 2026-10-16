//! Driving-Adapter HTTP (ADR_022): Web-UI mit Axum, Askama und HTMX.
//!
//! Handler parsen Eingaben, rufen Anwendungsfälle aus `zaehler-app` auf und
//! rendern View-Modelle; sie greifen nie direkt auf Adapter zu.

pub mod assets;
pub mod auth;
pub mod extract;
pub mod i18n;
pub mod pages;
pub mod queue;
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
use tower_http::{
    compression::CompressionLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tower_sessions::{Expiry, SessionManagerLayer, SessionStore, cookie::SameSite};
use tracing::{Span, field::Empty};

pub use auth::OidcClient;
pub use state::{WebConfig, WebState};

const FORM_LIMIT: usize = 64 * 1024;

/// Vollständiger Router; `store` speichert die Sitzungen (SPEC_SEC_SESSION_CSRF).
#[allow(clippy::too_many_lines)] // Routentabelle, eine Route je Zeile
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
        .route("/meters/{id}/chart.json", get(pages::analysis::chart))
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
        .route(
            "/readings/{id}/edit",
            get(pages::readings::edit_form).post(pages::readings::edit),
        )
        .route("/meters/{id}/history", get(pages::readings::history))
        .route("/readings/{id}/photo", get(pages::readings::photo))
        .route(
            "/capture",
            get(pages::capture::upload_form)
                .post(pages::capture::analyse)
                // je Foto `max_photo_bytes`, im Handler geprüft (SPEC_AI_MULTI)
                .layer(DefaultBodyLimit::max(
                    state
                        .cfg
                        .max_photo_bytes
                        .saturating_mul(zaehler_app::usecase::capture::MAX_PHOTOS),
                )),
        )
        .route("/import", get(pages::import::upload_form))
        .route(
            "/import/upload",
            post(pages::import::upload).layer(DefaultBodyLimit::max(state.cfg.max_csv_bytes)),
        )
        .route(
            "/import/preview",
            get(pages::import::mapping).post(pages::import::preview),
        )
        .route("/import/commit", post(pages::import::commit))
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
        .route("/settings/members", get(pages::members::show))
        .route(
            "/settings/members/{user}/role",
            post(pages::members::change_role),
        )
        .route(
            "/settings/members/{user}/remove",
            post(pages::members::remove),
        )
        .route("/settings/leave", post(pages::members::leave))
        .route(
            "/settings/notifications",
            get(pages::notifications::show).post(pages::notifications::update),
        )
        .route(
            "/settings/invitations",
            post(pages::members::create_invitation),
        )
        .route(
            "/settings/invitations/{id}/revoke",
            post(pages::members::revoke_invitation),
        )
        .route("/invite/{token}", get(pages::members::show_invitation))
        .route(
            "/invite/{token}/accept",
            post(pages::members::accept_invitation),
        )
        .route("/settings/export", get(pages::backup::export))
        .route(
            "/settings/import",
            post(pages::backup::import).layer(DefaultBodyLimit::max(state.cfg.max_backup_bytes)),
        )
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

    let trace_parent = state.cfg.trace_parent;
    Router::new()
        .route("/static/{version}/{name}", get(assets::serve))
        .route("/manifest.webmanifest", get(pages::pwa::manifest))
        .route("/sw.js", get(pages::pwa::service_worker))
        .route("/healthz", get(pages::live))
        .route("/readyz", get(pages::ready))
        .merge(app)
        .layer(middleware::from_fn(security::security_headers))
        .layer(middleware::from_fn(track_metrics))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(move |req: &Request| request_span(req, trace_parent))
                .on_response(record_status),
        )
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(120),
        ))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(middleware::map_request(drop_invalid_request_id))
        .with_state(state)
}

/// Übernimmt eine Request-ID eines vorgelagerten Proxys nur mit höchstens
/// 64 sichtbaren ASCII-Zeichen; sonst wird eine neue erzeugt (SPEC_OPS_OTEL).
async fn drop_invalid_request_id(mut req: Request) -> Request {
    let valid = req.headers().get("x-request-id").is_none_or(|v| {
        let b = v.as_bytes();
        !b.is_empty() && b.len() <= 64 && b.iter().all(u8::is_ascii_graphic)
    });
    if !valid {
        req.headers_mut().remove("x-request-id");
    }
    req
}

/// Span je Request mit Route-Template und Request-ID; Haushalt und Nutzer
/// trägt der Extraktor `Ctx` nach (SPEC_OPS_OTEL).
// @impl SPEC_OPS_OTEL
fn request_span(req: &Request, trace_parent: Option<state::TraceParent>) -> Span {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str);
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let span = tracing::info_span!(
        "http_request",
        otel.name = %format_args!("{} {route}", req.method()),
        otel.kind = "server",
        otel.status_code = Empty,
        http.request.method = %req.method(),
        http.route = route,
        http.response.status_code = Empty,
        request_id,
        household = Empty,
        user = Empty,
    );
    if let Some(set_parent) = trace_parent {
        set_parent(req.headers(), &span);
    }
    span
}

fn record_status(res: &Response, latency: Duration, span: &Span) {
    span.record("http.response.status_code", res.status().as_u16());
    if res.status().is_server_error() {
        span.record("otel.status_code", "ERROR");
    }
    tracing::debug!(latency_ms = latency.as_millis(), "request finished");
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
