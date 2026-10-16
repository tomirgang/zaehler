//! OIDC-Anmeldung gegen einen simulierten Provider: Discovery,
//! Autorisierungs-URL (Code Flow mit PKCE) und Prüfung des Rückrufs.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::json;
use tower::ServiceExt;
use tower_sessions::MemoryStore;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
use zaehler_app::{testing::memory_ports, usecase::UseCases};
use zaehler_web::{OidcClient, WebConfig, WebState, router};

async fn provider() -> MockServer {
    let server = MockServer::start().await;
    let issuer = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/auth"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/jwks"),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [] })))
        .mount(&server)
        .await;
    server
}

fn cookie(res: &axum::response::Response) -> String {
    res.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

// @test SPEC_TEN_OIDC
#[tokio::test]
async fn code_flow_with_pkce_and_state_check() {
    let server = provider().await;
    let oidc = OidcClient::discover(&server.uri(), "zaehler", "geheim", "http://localhost:8080")
        .await
        .expect("Discovery");
    let (ports, store, _clock) = memory_ports();
    let app = router(
        WebState {
            uc: Arc::new(UseCases::new(&ports)),
            readiness: store,
            cfg: Arc::new(WebConfig::new(
                "http://localhost:8080",
                false,
                false,
                24,
                "test",
            )),
            oidc: Some(Arc::new(oidc)),
            analysis: Arc::default(),
        },
        MemoryStore::default(),
    );
    let get = |uri: String, cookie: Option<String>| {
        let mut req = Request::get(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        app.clone().oneshot(req.body(Body::empty()).unwrap())
    };

    // Start: Weiterleitung zum Provider mit allen Parametern des Code Flows
    let res = get("/auth/oidc".into(), None).await.unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let session = cookie(&res);
    let location = res.headers()[header::LOCATION].to_str().unwrap().to_owned();
    assert!(location.starts_with(&format!("{}/auth?", server.uri())));
    for part in [
        "response_type=code",
        "client_id=zaehler",
        "code_challenge_method=S256",
        "code_challenge=",
        "state=",
        "nonce=",
        "redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Fauth%2Fcallback",
        "scope=openid",
    ] {
        assert!(location.contains(part), "{part} fehlt in {location}");
    }

    // Rückruf mit falschem state: abgelehnt, ohne Token-Abruf
    let res = get(
        "/auth/callback?code=abc&state=falsch".into(),
        Some(session.clone()),
    )
    .await
    .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    // der Ablauf ist verbraucht: auch ein erneuter Rückruf scheitert
    let res = get("/auth/callback?code=abc&state=falsch".into(), Some(session))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let token_requests = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/token")
        .count();
    assert_eq!(token_requests, 0);

    // ohne Entwicklungs-Login keine Anmeldung über /auth/dev-login
    let res = get("/auth/dev-login?name=x".into(), None).await.unwrap();
    assert_ne!(res.status(), StatusCode::SEE_OTHER);
}
