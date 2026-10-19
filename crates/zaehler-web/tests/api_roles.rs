//! Rollen der REST-API: Tokens wirken mit der Rolle ihres Erstellers,
//! höchstens mit der Token-Rolle (SPEC_API_AUTH).

mod common;

use axum::http::StatusCode;
use common::{User, call, test_app};

// @test SPEC_API_AUTH
#[tokio::test]
async fn harness_creates_tokens_per_role() {
    let app = test_app();
    let mut anna = User::login(&app, "api-harness").await;
    let viewer = anna.token("viewer").await;
    let res = call(&app, &viewer, "GET", "/api/v1/meters", None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.json.as_array().unwrap().is_empty());
}

/// Lesen ist für jede Rolle möglich (Dokumentliste ab Member), Schreiben
/// folgt der Rolle der Anwendungsfälle.
// @test SPEC_API_AUTH, SPEC_API_REST
#[tokio::test]
async fn viewer_tokens_read_every_collection_but_cannot_write() {
    let app = test_app();
    let mut anna = User::login(&app, "api-viewer-reads").await;
    let meter = anna.meter_via_form("Strom").await;
    let viewer = anna.token("viewer").await;
    let member = anna.token("member").await;
    for path in [
        "/api/v1/household".to_owned(),
        "/api/v1/meters".to_owned(),
        format!("/api/v1/meters/{meter}"),
        format!("/api/v1/meters/{meter}/readings"),
        format!("/api/v1/meters/{meter}/history"),
        format!("/api/v1/meters/{meter}/contracts"),
        "/api/v1/contracts".to_owned(),
        "/api/v1/costs".to_owned(),
        "/api/v1/vehicles".to_owned(),
        "/api/v1/pv".to_owned(),
        "/api/v1/pv-systems".to_owned(),
    ] {
        let res = call(&app, &viewer, "GET", &path, None).await;
        assert_eq!(res.status, StatusCode::OK, "GET {path}: {}", res.json);
    }
    let res = call(&app, &viewer, "GET", "/api/v1/documents", None).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = call(&app, &member, "GET", "/api/v1/documents", None).await;
    assert_eq!(res.status, StatusCode::OK);

    for (method, path) in [
        ("POST", format!("/api/v1/meters/{meter}/archive")),
        ("DELETE", format!("/api/v1/meters/{meter}")),
    ] {
        for token in [&viewer, &member] {
            let res = call(&app, token, method, &path, None).await;
            assert_eq!(res.status, StatusCode::FORBIDDEN, "{method} {path}");
        }
    }
}
