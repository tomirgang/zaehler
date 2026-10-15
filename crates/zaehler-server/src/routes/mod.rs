mod health;
mod ui;

use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{MatchedPath, Request},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use metrics_exporter_prometheus::PrometheusHandle;
use tower_http::{compression::CompressionLayer, timeout::TimeoutLayer, trace::TraceLayer};

use crate::{assets, state::AppState};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(ui::index))
        .route("/static/{name}", get(assets::serve))
        .route("/healthz", get(health::live))
        .route("/readyz", get(health::ready))
        // TODO(REQ_TEN_002): OIDC-Login (/auth/login, /auth/callback) und Session-Middleware.
        .layer(middleware::from_fn(track_metrics))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(120),
        ))
        .with_state(state)
}

/// Separater Listener, damit `/metrics` nicht über den Ingress erreichbar ist.
pub fn metrics_router(handle: PrometheusHandle) -> Router {
    Router::new().route("/metrics", get(move || std::future::ready(handle.render())))
}

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
