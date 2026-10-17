//! Das API-Token erscheint in keinem Log, die Token-ID schon (SPEC_API_AUTH).

use std::{
    io::Write,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;
use tower_sessions::MemoryStore;
use tracing_subscriber::fmt::{MakeWriter, format::FmtSpan};
use zaehler_app::{testing::memory_ports, usecase::UseCases};
use zaehler_web::{WebConfig, WebState, router};

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buffer {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

async fn send(
    app: &Router,
    cookie: &mut Option<String>,
    req: Request<Body>,
) -> (StatusCode, String) {
    let mut req = req;
    if let Some(c) = cookie.as_ref() {
        req.headers_mut().insert(header::COOKIE, c.parse().unwrap());
    }
    let res = app.clone().oneshot(req).await.unwrap();
    if let Some(set) = res.headers().get(header::SET_COOKIE) {
        *cookie = Some(set.to_str().unwrap().split(';').next().unwrap().to_owned());
    }
    let status = res.status();
    let body = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn get(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

// @test SPEC_API_AUTH
#[tokio::test]
async fn token_never_appears_in_logs() {
    let logs = Buffer::default();
    tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(FmtSpan::CLOSE)
        .with_ansi(false)
        .init();
    let (ports, store, _) = memory_ports();
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
    let app = router(state, MemoryStore::default());
    let mut cookie = None;
    send(&app, &mut cookie, get("/auth/dev-login?name=anna")).await;
    let (_, html) = send(&app, &mut cookie, get("/settings")).await;
    let start = html.find("name=\"_csrf\" value=\"").unwrap() + 20;
    let csrf = html[start..start + html[start..].find('"').unwrap()].to_owned();
    let form = serde_urlencoded::to_string([
        ("_csrf", csrf.as_str()),
        ("name", "HA"),
        ("role", "member"),
        ("expires_on", ""),
    ])
    .unwrap();
    let (_, html) = send(
        &app,
        &mut cookie,
        Request::post("/settings/tokens")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form))
            .unwrap(),
    )
    .await;
    let start = html.find("value=\"zt_").unwrap() + 7;
    let token = html[start..start + 46].to_owned();

    let mut none = None;
    for bearer in [token.clone(), format!("{}x", &token[..45])] {
        let req = Request::get("/api/v1/meters")
            .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
            .body(Body::empty())
            .unwrap();
        send(&app, &mut none, req).await;
    }
    let (status, _) = send(
        &app,
        &mut none,
        Request::get("/api/v1/meters")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(
        text.contains("api_token="),
        "Token-ID im Request-Span: {text}"
    );
    assert!(!text.contains(&token), "Token im Log");
    assert!(!text.contains(&token[3..]), "Token ohne Präfix im Log");
}
