//! Gemeinsame Hilfen der API-Tests: App mit In-Memory-Adaptern, Anmeldung
//! über den Entwicklungs-Login und Tokens über die Einstellungsseite.

#![allow(dead_code)]

use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use serde_json::Value;
use tower::ServiceExt;
use tower_sessions::MemoryStore;
use zaehler_app::{testing::memory_ports, usecase::UseCases};
use zaehler_web::{WebConfig, WebState, router};

pub fn test_app() -> Router {
    let (ports, store, _clock) = memory_ports();
    let state = WebState {
        uc: Arc::new(UseCases::new(&ports)),
        readiness: store,
        cfg: Arc::new(WebConfig::new(
            "http://localhost:8080",
            true,
            false,
            24,
            "test",
        )),
        oidc: None,
        analysis: Arc::default(),
    };
    router(state, MemoryStore::default())
}

/// Antwort der API.
pub struct ApiResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub bytes: Vec<u8>,
    /// `Null`, wenn der Body kein JSON ist
    pub json: Value,
}

/// Browser-Sitzung eines Nutzers (Entwicklungs-Login), um Tokens zu erzeugen.
pub struct User {
    app: Router,
    cookie: Option<String>,
}

impl User {
    /// Meldet `name` an; jeder neue Name erhält einen eigenen Haushalt, in
    /// dem er Owner ist.
    pub async fn login(app: &Router, name: &str) -> Self {
        let mut u = Self {
            app: app.clone(),
            cookie: None,
        };
        let res = u
            .send(
                Request::get(format!("/auth/dev-login?name={name}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "dev-login");
        u
    }

    async fn send(&mut self, mut req: Request<Body>) -> ApiResponse {
        if let Some(c) = &self.cookie {
            req.headers_mut().insert(header::COOKIE, c.parse().unwrap());
        }
        let res = self.app.clone().oneshot(req).await.unwrap();
        if let Some(set) = res.headers().get(header::SET_COOKIE) {
            let pair = set.to_str().unwrap().split(';').next().unwrap().to_owned();
            self.cookie = Some(pair);
        }
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = to_bytes(res.into_body(), 1 << 24).await.unwrap().to_vec();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        ApiResponse {
            status,
            headers,
            bytes,
            json,
        }
    }

    async fn csrf(&mut self) -> String {
        let res = self
            .send(Request::get("/settings").body(Body::empty()).unwrap())
            .await;
        let html = String::from_utf8(res.bytes).unwrap();
        let start = html.find("name=\"_csrf\" value=\"").expect("CSRF-Feld") + 20;
        html[start..start + html[start..].find('"').unwrap()].to_owned()
    }

    /// Erzeugt über `/settings/tokens` ein Token mit der Rolle
    /// `viewer`, `member` oder `owner`.
    pub async fn token(&mut self, role: &str) -> String {
        let csrf = self.csrf().await;
        let body = serde_urlencoded::to_string([
            ("_csrf", csrf.as_str()),
            ("name", "Test"),
            ("role", role),
            ("expires_on", ""),
        ])
        .unwrap();
        let res = self
            .send(
                Request::post("/settings/tokens")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status, StatusCode::OK, "token page");
        let html = String::from_utf8(res.bytes).unwrap();
        let start = html.find("value=\"zt_").expect("Token einmal angezeigt") + 7;
        html[start..start + 46].to_owned()
    }

    /// Legt über das Formular eine Strom-Messstelle mit Register `main`
    /// (Bezug, Startstand 1000 am 01.01.2026) an und liefert ihre ID.
    pub async fn meter_via_form(&mut self, name: &str) -> String {
        let csrf = self.csrf().await;
        let body = serde_urlencoded::to_string([
            ("_csrf", csrf.as_str()),
            ("name", name),
            ("kind", "electricity"),
            ("unit", ""),
            ("serial_number", ""),
            ("rollover_digits", ""),
            ("decimal_places", "1"),
            ("schedule_kind", ""),
            ("schedule_value", ""),
            ("max_daily_rate", ""),
            ("billing_month", ""),
            ("billing_day", ""),
            ("start_at", "2026-01-01T00:00"),
            ("register_count", "1"),
            ("register_0_id", ""),
            ("register_0_code", "main"),
            ("register_0_direction", "import"),
            ("register_0_start", "1000"),
        ])
        .unwrap();
        let res = self
            .send(
                Request::post("/meters")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "meter form");
        res.headers[header::LOCATION]
            .to_str()
            .unwrap()
            .trim_start_matches("/meters/")
            .to_owned()
    }
}

/// API-Aufruf mit Bearer-Token und optionalem JSON-Body.
pub async fn call(
    app: &Router,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> ApiResponse {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = to_bytes(res.into_body(), 1 << 24).await.unwrap().to_vec();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    ApiResponse {
        status,
        headers,
        bytes,
        json,
    }
}
