//! REST-API: Haushalt und Dokumente (SPEC_API_REST).

mod common;

use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tower::ServiceExt;
use tower_sessions::MemoryStore;
use zaehler_app::{
    model::InvoiceExtraction,
    testing::{StubExtractor, memory_ports},
    usecase::UseCases,
};
use zaehler_web::{WebConfig, WebState, router};

use common::{User, call, test_app};

const PDF: &[u8] = b"%PDF-1.7 Rechnung";

/// App mit einer Rechnungserkennung, die immer denselben Vorschlag liefert.
fn document_app() -> Router {
    let (mut ports, store, _clock) = memory_ports();
    ports.extractor = Some(Arc::new(StubExtractor(Ok(InvoiceExtraction {
        provider: Some("Stadtwerke Süd".into()),
        contract_number: Some("V-1".into()),
        period_start: None,
        period_end: None,
        readings: vec![],
        prices: vec![],
        installment: None,
        total_amount: Some(Decimal::new(3050, 2)),
        confidence: Decimal::new(9, 1),
    }))));
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

/// Browser-Sitzung zum Hochladen über das Formular; derselbe Name wie bei
/// `User::login` ergibt denselben Nutzer und Haushalt.
struct Browser {
    app: Router,
    cookie: Option<String>,
}

impl Browser {
    async fn login(app: &Router, name: &str) -> Self {
        let mut b = Self {
            app: app.clone(),
            cookie: None,
        };
        let (status, _, _) = b
            .send(
                Request::get(format!("/auth/dev-login?name={name}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::SEE_OTHER, "dev-login");
        b
    }

    async fn send(&mut self, mut req: Request<Body>) -> (StatusCode, Option<String>, Vec<u8>) {
        if let Some(c) = &self.cookie {
            req.headers_mut().insert(header::COOKIE, c.parse().unwrap());
        }
        let res = self.app.clone().oneshot(req).await.unwrap();
        if let Some(set) = res.headers().get(header::SET_COOKIE) {
            let pair = set.to_str().unwrap().split(';').next().unwrap().to_owned();
            self.cookie = Some(pair);
        }
        let status = res.status();
        let location = res
            .headers()
            .get(header::LOCATION)
            .map(|l| l.to_str().unwrap().to_owned());
        let bytes = to_bytes(res.into_body(), 1 << 24).await.unwrap().to_vec();
        (status, location, bytes)
    }

    async fn csrf(&mut self) -> String {
        let (_, _, bytes) = self
            .send(Request::get("/settings").body(Body::empty()).unwrap())
            .await;
        let html = String::from_utf8(bytes).unwrap();
        let start = html.find("name=\"_csrf\" value=\"").expect("CSRF-Feld") + 20;
        html[start..start + html[start..].find('"').unwrap()].to_owned()
    }

    /// Lädt `file` als Rechnung hoch und liefert die ID des Dokuments.
    async fn upload(&mut self, filename: &str, file: &[u8]) -> String {
        let csrf = self.csrf().await;
        let boundary = "zaehler-test-boundary";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(file);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let (status, location, _) = self
            .send(
                Request::post("/documents")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::SEE_OTHER, "upload");
        location
            .unwrap()
            .trim_start_matches("/documents/")
            .to_owned()
    }
}

fn header(res: &common::ApiResponse, name: header::HeaderName) -> &str {
    res.headers[name].to_str().unwrap()
}

fn error_fields(body: &Value) -> Vec<&str> {
    body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect()
}

// @test SPEC_API_REST
#[tokio::test]
async fn household_read_and_update() {
    let app = test_app();
    let mut anna = User::login(&app, "anna").await;
    let owner = anna.token("owner").await;
    let member = anna.token("member").await;
    let viewer = anna.token("viewer").await;

    let res = call(&app, &owner, "GET", "/api/v1/household", None).await;
    assert_eq!(res.status, StatusCode::OK);
    let id = res.json["id"].as_str().unwrap().to_owned();
    assert!(!res.json["name"].as_str().unwrap().is_empty());
    assert!(res.json["timezone"].as_str().unwrap().contains('/'));
    assert_eq!(res.json["role"], "owner");

    let res = call(&app, &viewer, "GET", "/api/v1/household", None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["id"], id.as_str());
    assert_eq!(res.json["role"], "viewer");

    let body = json!({ "name": "  Ferienhaus ", "timezone": "Europe/Vienna" });
    for token in [&viewer, &member] {
        let res = call(&app, token, "PUT", "/api/v1/household", Some(body.clone())).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
    }

    let res = call(
        &app,
        &owner,
        "PUT",
        "/api/v1/household",
        Some(json!({ "name": "Ferienhaus", "timezone": "Mars/Olympus" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_fields(&res.json), ["timezone"]);

    let res = call(
        &app,
        &owner,
        "PUT",
        "/api/v1/household",
        Some(json!({ "name": "  ", "timezone": "Europe/Vienna" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_fields(&res.json), ["name"]);

    let res = call(
        &app,
        &owner,
        "PUT",
        "/api/v1/household",
        Some(json!({ "name": "Ferienhaus" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);

    let res = call(&app, &owner, "PUT", "/api/v1/household", Some(body)).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(
        res.json,
        json!({ "id": id, "name": "Ferienhaus", "timezone": "Europe/Vienna", "role": "owner" })
    );
    let res = call(&app, &member, "GET", "/api/v1/household", None).await;
    assert_eq!(res.json["name"], "Ferienhaus");
    assert_eq!(res.json["timezone"], "Europe/Vienna");

    // ein anderer Haushalt bleibt unverändert
    let mut bert = User::login(&app, "bert").await;
    let bert = bert.token("owner").await;
    let res = call(&app, &bert, "GET", "/api/v1/household", None).await;
    assert_ne!(res.json["id"], id.as_str());
    assert_ne!(res.json["name"], "Ferienhaus");
}

// @test SPEC_API_REST
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn documents_read_download_and_discard() {
    let app = document_app();
    let mut browser = Browser::login(&app, "anna").await;
    let doc = browser.upload("Rechnung März.pdf", PDF).await;
    let mut anna = User::login(&app, "anna").await;
    let owner = anna.token("owner").await;
    let member = anna.token("member").await;
    let viewer = anna.token("viewer").await;
    let path = format!("/api/v1/documents/{doc}");

    // Liste offener Dokumente ab Member
    let res = call(&app, &viewer, "GET", "/api/v1/documents", None).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = call(&app, &member, "GET", "/api/v1/documents", None).await;
    assert_eq!(res.status, StatusCode::OK);
    let list = res.json.as_array().unwrap();
    assert_eq!(list.len(), 1);
    let d = &list[0];
    assert_eq!(d["id"], doc.as_str());
    assert_eq!(d["kind"], "invoice");
    assert_eq!(d["media_type"], "application/pdf");
    assert_eq!(d["size"], PDF.len());
    assert_eq!(d["original_name"], "Rechnung März.pdf");
    assert_eq!(d["status"], "analysed");
    assert_eq!(d["meter_id"], Value::Null);
    assert!(d["created_at"].is_string());
    assert!(d["analysed_at"].is_string());
    assert_eq!(d["extraction"]["provider"], "Stadtwerke Süd");
    assert_eq!(d["extraction"]["total_amount"], "30.50");
    assert_eq!(d["receipt"], Value::Null);

    // Einzelabruf und Datei ab Viewer
    let res = call(&app, &viewer, "GET", &path, None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(&res.json, d);

    let res = call(&app, &viewer, "GET", &format!("{path}/file"), None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.bytes, PDF);
    assert_eq!(header(&res, header::CONTENT_TYPE), "application/pdf");
    assert_eq!(header(&res, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    assert_eq!(
        header(&res, header::CONTENT_DISPOSITION),
        "attachment; filename=\"Rechnung_M_rz.pdf\""
    );

    // fremder Haushalt und unbekannte IDs
    let mut bert = User::login(&app, "bert").await;
    let bert = bert.token("owner").await;
    for (method, p) in [
        ("GET", path.clone()),
        ("GET", format!("{path}/file")),
        ("DELETE", path.clone()),
    ] {
        let res = call(&app, &bert, method, &p, None).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {p}");
    }
    let res = call(&app, &bert, "GET", "/api/v1/documents", None).await;
    assert_eq!(res.json, json!([]));
    let unknown = format!("/api/v1/documents/{}", uuid::Uuid::now_v7());
    let res = call(&app, &owner, "GET", &unknown, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = call(&app, &owner, "GET", &format!("{unknown}/file"), None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // Verwerfen ab Member; danach nur noch lesbar
    let res = call(&app, &viewer, "DELETE", &path, None).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = call(&app, &member, "DELETE", &path, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(res.bytes.is_empty());
    let res = call(&app, &owner, "DELETE", &path, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = call(&app, &member, "GET", "/api/v1/documents", None).await;
    assert_eq!(res.json, json!([]));
    let res = call(&app, &viewer, "GET", &path, None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["status"], "discarded");
}

// @test SPEC_API_REST
#[tokio::test]
async fn document_without_recognition_is_failed() {
    let app = test_app();
    let mut browser = Browser::login(&app, "anna").await;
    let doc = browser.upload("../scan.pdf", PDF).await;
    let mut anna = User::login(&app, "anna").await;
    let member = anna.token("member").await;
    let res = call(
        &app,
        &member,
        "GET",
        &format!("/api/v1/documents/{doc}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["status"], "failed");
    assert_eq!(res.json["original_name"], "scan.pdf");
    assert_eq!(res.json["model"], Value::Null);
    assert_eq!(res.json["extraction"], Value::Null);
    let res = call(
        &app,
        &member,
        "GET",
        &format!("/api/v1/documents/{doc}/file"),
        None,
    )
    .await;
    assert_eq!(
        header(&res, header::CONTENT_DISPOSITION),
        "attachment; filename=\"scan.pdf\""
    );
}
