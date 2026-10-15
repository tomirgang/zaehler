//! Kubernetes-Probes.

use axum::{extract::State, http::StatusCode};

use crate::state::AppState;

/// Liveness: Prozess läuft und beantwortet Requests.
// @impl SPEC_OPS_HEALTH
pub async fn live() -> &'static str {
    "ok"
}

/// Readiness: Datenbank erreichbar.
// @impl SPEC_OPS_HEALTH
pub async fn ready(State(state): State<AppState>) -> (StatusCode, &'static str) {
    match zaehler_db::ping(&state.db).await {
        Ok(()) => (StatusCode::OK, "ready"),
        Err(e) => {
            tracing::warn!(error = %e, "readiness check failed");
            (StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    use crate::{config::Config, routes::router, state::AppState};

    // @test SPEC_OPS_HEALTH
    #[tokio::test]
    async fn probes_respond() {
        let db = zaehler_db::connect("sqlite::memory:", 1).await.unwrap();
        let app = router(AppState {
            db,
            config: Arc::new(Config::default()),
            ai: None,
        });
        for path in ["/healthz", "/readyz", "/"] {
            let resp = app
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), 200, "{path}");
        }
    }
}
