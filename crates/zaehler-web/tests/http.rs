//! HTTP-Tests des Web-Adapters mit In-Memory-Adaptern (ohne Datenbank).

use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    response::Response,
};
use tower::ServiceExt;
use tower_sessions::MemoryStore;
use zaehler_app::{testing::memory_ports, usecase::UseCases};
use zaehler_web::{WebConfig, WebState, router};

fn test_app(dev_login: bool) -> Router {
    let (ports, store, _clock) = memory_ports();
    let state = WebState {
        uc: Arc::new(UseCases::new(&ports)),
        readiness: store,
        cfg: Arc::new(WebConfig::new(
            "http://localhost:8080",
            dev_login,
            false,
            24,
            "test",
        )),
        oidc: None,
    };
    router(state, MemoryStore::default())
}

/// Browser-Ersatz: hält das Sitzungscookie.
struct Client {
    app: Router,
    cookie: Option<String>,
}

impl Client {
    fn new(app: &Router) -> Self {
        Self {
            app: app.clone(),
            cookie: None,
        }
    }

    async fn send(&mut self, mut req: Request<Body>) -> Response {
        if let Some(c) = &self.cookie {
            req.headers_mut().insert(header::COOKIE, c.parse().unwrap());
        }
        let res = self.app.clone().oneshot(req).await.unwrap();
        if let Some(set) = res.headers().get(header::SET_COOKIE) {
            let pair = set.to_str().unwrap().split(';').next().unwrap().to_owned();
            self.cookie = Some(pair);
        }
        res
    }

    async fn get(&mut self, path: &str) -> Response {
        self.send(Request::get(path).body(Body::empty()).unwrap())
            .await
    }

    async fn post(&mut self, path: &str, fields: &[(&str, &str)]) -> Response {
        let body = serde_urlencoded::to_string(fields).unwrap();
        let req = Request::post(path)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        self.send(req).await
    }

    async fn login(&mut self, name: &str) {
        let res = self.get(&format!("/auth/dev-login?name={name}")).await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(location(&res), "/");
    }

    async fn csrf(&mut self) -> String {
        let html = text(self.get("/settings").await).await;
        let start = html.find("name=\"_csrf\" value=\"").expect("CSRF-Feld") + 20;
        html[start..start + html[start..].find('"').unwrap()].to_owned()
    }
}

fn location(res: &Response) -> String {
    res.headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default()
}

async fn text(res: Response) -> String {
    String::from_utf8(to_bytes(res.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap()
}

async fn create_meter(c: &mut Client, csrf: &str, name: &str) -> String {
    let res = c
        .post(
            "/meters",
            &[
                ("_csrf", csrf),
                ("name", name),
                ("kind", "electricity"),
                ("unit", ""),
                ("serial_number", ""),
                ("rollover_digits", ""),
                ("decimal_places", "1"),
                ("reading_interval_days", "30"),
                ("max_daily_rate", "50"),
                ("billing_month", ""),
                ("billing_day", ""),
                ("start_at", "2026-01-01T00:00"),
                ("register_count", "1"),
                ("register_0_id", ""),
                ("register_0_code", "main"),
                ("register_0_direction", "import"),
                ("register_0_start", "1000"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let loc = location(&res);
    loc.trim_start_matches("/meters/").to_owned()
}

// @test SPEC_SEC_UPLOADS, SPEC_SEC_TENANT_SCOPE
#[tokio::test]
async fn reading_photos_are_served_only_to_their_household() {
    use zaehler_app::{
        ids::{BlobKey, MeterId, ReadingId},
        model::{Identity, Reading},
    };
    use zaehler_domain::reading::ReadingSource;

    let (ports, store, _clock) = memory_ports();
    let uc = Arc::new(UseCases::new(&ports));
    let state = WebState {
        uc: uc.clone(),
        readiness: store,
        cfg: Arc::new(WebConfig::new(
            "http://localhost:8080",
            true,
            false,
            24,
            "test",
        )),
        oidc: None,
    };
    let app = router(state, MemoryStore::default());
    let mut anna = Client::new(&app);
    anna.login("anna").await;
    let csrf = anna.csrf().await;
    let meter: MeterId = create_meter(&mut anna, &csrf, "Strom")
        .await
        .parse()
        .unwrap();

    // Ablesung mit Foto direkt über die Ports (Foto-Erfassung folgt mit WP_MVP_07)
    let identity = Identity {
        issuer: "dev".into(),
        subject: "anna".into(),
        email: None,
        display_name: "anna".into(),
        locale: "de".into(),
    };
    let user = uc.sign_in.execute(&identity).await.unwrap().user.id;
    let tenant = uc.resolve_tenant.execute(user, None).await.unwrap().tenant;
    let detail = uc.get_meter.execute(&tenant, meter).await.unwrap();
    let id = ReadingId(uuid::Uuid::now_v7());
    let key = BlobKey::reading_photo(tenant.household_id(), id);
    ports
        .blobs
        .put(&key, bytes::Bytes::from_static(b"\xFF\xD8\xFFjpeg"))
        .await
        .unwrap();
    let mut uow = ports.uow.begin().await.unwrap();
    uow.readings()
        .insert(
            &tenant,
            &[Reading {
                id,
                register_id: detail.aggregate.registers[0].id,
                device_id: detail.aggregate.devices[0].id,
                read_at: detail.readings[0].read_at + chrono::Duration::days(1),
                value: rust_decimal::Decimal::new(10_010, 1),
                source: ReadingSource::Photo,
                note: None,
                photo_key: Some(key),
                ai_confidence: None,
                created_by: Some(user),
            }],
        )
        .await
        .unwrap();
    uow.commit().await.unwrap();

    let html = text(anna.get(&format!("/meters/{meter}")).await).await;
    assert!(html.contains(&format!("/readings/{id}/photo")));
    let res = anna.get(&format!("/readings/{id}/photo")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(res.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        to_bytes(res.into_body(), 1024).await.unwrap(),
        &b"\xFF\xD8\xFFjpeg"[..]
    );
    // Ablesung ohne Foto, unbekannte und ungültige IDs
    let start = detail.readings[0].id;
    for path in [
        format!("/readings/{start}/photo"),
        format!("/readings/{}/photo", uuid::Uuid::now_v7()),
        "/readings/x/photo".to_owned(),
    ] {
        assert_eq!(
            anna.get(&path).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }

    let mut bob = Client::new(&app);
    bob.login("bob").await;
    let res = bob.get(&format!("/readings/{id}/photo")).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

fn capture_app(max_photo_bytes: usize) -> Router {
    use rust_decimal::Decimal;
    use zaehler_app::{model::Recognition, testing::StubRecognizer};
    use zaehler_domain::meter::MeterKind;

    let (mut ports, store, _clock) = memory_ports();
    ports.recognizer = Some(Arc::new(StubRecognizer(Ok(Recognition {
        value: Some(Decimal::new(10_250, 1)),
        serial_number: None,
        register: Some("main".into()),
        unit: Some("kWh".into()),
        meter_kind: Some(MeterKind::Electricity),
        confidence: Decimal::new(87, 2),
    }))));
    let state = WebState {
        uc: Arc::new(UseCases::new(&ports)),
        readiness: store,
        cfg: Arc::new(
            WebConfig::new("http://localhost:8080", true, false, 24, "test")
                .with_max_photo_bytes(max_photo_bytes),
        ),
        oidc: None,
    };
    router(state, MemoryStore::default())
}

/// Multipart-Body mit optionalem CSRF-Feld und Datei `photo`.
fn multipart(csrf: Option<&str>, photo: &[u8]) -> (String, Vec<u8>) {
    let boundary = "zaehler-test-boundary";
    let mut body = Vec::new();
    if let Some(t) = csrf {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{t}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"z.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(photo);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn upload(c: &mut Client, path: &str, csrf: Option<&str>, photo: &[u8]) -> Response {
    let (content_type, body) = multipart(csrf, photo);
    let req = Request::post(path)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap();
    c.send(req).await
}

/// JPEG-Signatur, aufgefüllt auf `len` Byte (der Stub-Normalisierer prüft nur
/// die Signatur).
fn fake_jpeg(len: usize) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF];
    v.resize(len, 0);
    v
}

// @test SPEC_AI_FLOW, SPEC_AI_DRAFTS, SPEC_SEC_UPLOADS, SPEC_SEC_SESSION_CSRF
#[tokio::test]
async fn photo_capture_workflow() {
    let app = capture_app(1024 * 1024);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let meter = create_meter(&mut c, &csrf, "Strom").await;

    let page = text(c.get("/capture").await).await;
    assert!(page.contains("enctype=\"multipart/form-data\""));
    assert!(page.contains("capture=\"environment\""));

    // ohne Token und auf anderen Routen wird Multipart abgelehnt
    let photo = fake_jpeg(200 * 1024);
    assert_eq!(
        upload(&mut c, "/capture", None, &photo).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        upload(&mut c, "/capture", Some("falsch"), &photo)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        upload(
            &mut c,
            &format!("/meters/{meter}/readings"),
            Some(&csrf),
            &photo
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    // keine Bilddatei
    let res = upload(&mut c, "/capture", Some(&csrf), b"Zaehlerstand 12345").await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("JPEG"));

    // 200 KiB liegen über dem allgemeinen Formular-Limit, aber unter dem Foto-Limit
    let res = upload(&mut c, "/capture", Some(&csrf), &photo).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let draft_page = location(&res);
    assert!(draft_page.starts_with("/capture/"));

    let html = text(c.get(&draft_page).await).await;
    assert!(html.contains("1025"), "Vorschlag sichtbar");
    assert!(html.contains("87 %"), "Konfidenz sichtbar");
    assert!(html.contains(&format!("value=\"{meter}\"")), "zugeordnet");
    let res = c.get(&format!("{draft_page}/photo")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "image/jpeg");

    // Bestätigung mit Registerwahl aus dem Formular
    let register = {
        let start = html.find("name=\"register_id\"").unwrap();
        let rest = &html[start..];
        let v = rest.find("value=\"").unwrap() + 7;
        rest[v..v + rest[v..].find('"').unwrap()].to_owned()
    };
    let res = c
        .post(
            &format!("{draft_page}/confirm"),
            &[
                ("_csrf", csrf.as_str()),
                ("meter_id", meter.as_str()),
                ("register_id", register.as_str()),
                ("value", "1025,0"),
                ("at", "2026-01-10T08:00"),
                ("note", ""),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    assert_eq!(location(&res), format!("/meters/{meter}"));
    let detail = text(c.get(&format!("/meters/{meter}")).await).await;
    assert!(detail.contains("/photo\""), "Belegfoto verlinkt");
    // Entwurf ist abgeschlossen
    assert_eq!(c.get(&draft_page).await.status(), StatusCode::NOT_FOUND);

    // Verwerfen
    let res = upload(&mut c, "/capture", Some(&csrf), &fake_jpeg(64)).await;
    let second = location(&res);
    let res = c
        .post(&format!("{second}/discard"), &[("_csrf", csrf.as_str())])
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), "/capture");
    assert_eq!(
        c.get(&format!("{second}/photo")).await.status(),
        StatusCode::NOT_FOUND
    );

    // fremder Haushalt
    let mut bob = Client::new(&app);
    bob.login("bob").await;
    let res = upload(&mut c, "/capture", Some(&csrf), &fake_jpeg(64)).await;
    let third = location(&res);
    assert_eq!(bob.get(&third).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        bob.get(&format!("{third}/photo")).await.status(),
        StatusCode::NOT_FOUND
    );
}

// @test SPEC_SEC_UPLOADS
#[tokio::test]
async fn photo_upload_limit_returns_413() {
    let app = capture_app(4096);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let res = upload(&mut c, "/capture", Some(&csrf), &fake_jpeg(8192)).await;
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let res = upload(&mut c, "/capture", Some(&csrf), &fake_jpeg(1024)).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

// @test SPEC_OPS_HEALTH, SPEC_SEC_HEADERS
#[tokio::test]
async fn probes_assets_and_security_headers() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    assert_eq!(c.get("/healthz").await.status(), StatusCode::OK);
    assert_eq!(c.get("/readyz").await.status(), StatusCode::OK);
    let css = c.get("/static/test/app.css").await;
    assert_eq!(css.status(), StatusCode::OK);
    assert_eq!(
        css.headers()[header::CONTENT_TYPE],
        "text/css; charset=utf-8"
    );
    assert_eq!(
        c.get("/static/test/unbekannt.js").await.status(),
        StatusCode::NOT_FOUND
    );
    let login = c.get("/auth/login").await;
    let csp = login.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.contains("script-src 'self'") && !csp.contains("unsafe-inline"));
    assert_eq!(login.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        c.get("/gibt-es-nicht").await.status(),
        StatusCode::NOT_FOUND
    );
}

// @test SPEC_TEN_DEV_LOGIN, SPEC_ERR_HANDLING
#[tokio::test]
async fn anonymous_requests_are_redirected_and_dev_login_can_be_disabled() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    let res = c.get("/").await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), "/auth/login");
    let hx = c
        .send(
            Request::get("/meters")
                .header("hx-request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(hx.headers()["hx-redirect"], "/auth/login");

    let disabled = test_app(false);
    let mut d = Client::new(&disabled);
    assert_eq!(
        d.get("/auth/dev-login?name=x").await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        text(d.get("/auth/login").await)
            .await
            .contains("Keine Anmeldung konfiguriert")
    );
}

// @test SPEC_SEC_SESSION_CSRF
#[tokio::test]
async fn csrf_and_origin_are_enforced() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let base = [
        ("name", "Strom"),
        ("kind", "electricity"),
        ("register_count", "1"),
        ("register_0_code", "main"),
    ];
    assert_eq!(
        c.post("/meters", &base).await.status(),
        StatusCode::FORBIDDEN,
        "ohne Token"
    );
    let mut wrong = base.to_vec();
    wrong.push(("_csrf", "falsch"));
    assert_eq!(
        c.post("/meters", &wrong).await.status(),
        StatusCode::FORBIDDEN,
        "falsches Token"
    );

    let req = Request::post("/auth/logout")
        .header("x-csrf-token", csrf.as_str())
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        c.send(req).await.status(),
        StatusCode::FORBIDDEN,
        "fremder Origin"
    );
    let req = Request::post("/auth/logout")
        .header("x-csrf-token", csrf.as_str())
        .header(header::ORIGIN, "http://localhost:8080")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        c.send(req).await.status(),
        StatusCode::SEE_OTHER,
        "Header-Token und eigener Origin"
    );
}

// @test SPEC_MTR_MANAGE, SPEC_RDG_FORM, SPEC_MTR_SWAP_FLOW
#[tokio::test]
#[allow(clippy::too_many_lines)] // zusammenhängender Ablauf wie im Browser
async fn meter_and_reading_workflow() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;

    // Validierung: Formular erneut mit Fehlern
    let res = c
        .post(
            "/meters",
            &[
                ("_csrf", &csrf),
                ("name", ""),
                ("kind", "gas"),
                ("register_count", "1"),
                ("register_0_code", ""),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = text(res).await;
    assert!(html.contains("Name mit 1 bis 80 Zeichen erforderlich."));
    assert!(html.contains("has-error"));

    let id = create_meter(&mut c, &csrf, "Hausstrom").await;
    let detail = text(c.get(&format!("/meters/{id}")).await).await;
    assert!(
        detail.contains("Hausstrom") && detail.contains("1000,0"),
        "Startstand mit einer Nachkommastelle"
    );
    assert!(detail.contains("Messstelle angelegt."), "Flash-Meldung");

    // Übersicht mit Karte und Fälligkeit
    let dashboard = text(c.get("/").await).await;
    assert!(dashboard.contains(&format!("/meters/{id}/readings/new")));

    // Erfassung: Formular, Prüfung, Speichern
    assert_eq!(
        c.get(&format!("/meters/{id}/readings/new")).await.status(),
        StatusCode::OK
    );
    let register =
        detail_register_id(&text(c.get(&format!("/meters/{id}/readings/new")).await).await);
    let value_field = format!("value_{register}");
    let ok = c
        .post(
            &format!("/meters/{id}/readings"),
            &[
                ("_csrf", &csrf),
                ("at", "2026-01-10T10:00"),
                (&value_field, "1100,5"),
                ("note", ""),
            ],
        )
        .await;
    assert_eq!(ok.status(), StatusCode::SEE_OTHER);
    assert!(
        text(c.get(&format!("/meters/{id}")).await)
            .await
            .contains("1100,5")
    );

    let check = c
        .post(
            &format!("/meters/{id}/readings/check"),
            &[
                ("_csrf", &csrf),
                ("at", "2026-01-11T10:00"),
                (&value_field, "1000"),
            ],
        )
        .await;
    assert!(
        text(check)
            .await
            .contains("Stand ist kleiner als der vorherige.")
    );

    let lower = [
        ("_csrf", csrf.as_str()),
        ("at", "2026-01-11T10:00"),
        (value_field.as_str(), "1000"),
    ];
    let res = c.post(&format!("/meters/{id}/readings"), &lower).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("Trotzdem speichern"));
    let mut confirmed = lower.to_vec();
    confirmed.push(("confirm_warnings", "true"));
    assert_eq!(
        c.post(&format!("/meters/{id}/readings"), &confirmed)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );

    let bad = c
        .post(
            &format!("/meters/{id}/readings"),
            &[
                ("_csrf", &csrf),
                ("at", "2026-01-12T10:00"),
                (&value_field, "1.234,5"),
            ],
        )
        .await;
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(bad).await.contains("Keine gültige Zahl."));

    // Zählerwechsel
    assert_eq!(
        c.get(&format!("/meters/{id}/swap")).await.status(),
        StatusCode::OK
    );
    let end = format!("end_{register}");
    let start = format!("start_{register}");
    let swap = c
        .post(
            &format!("/meters/{id}/swap"),
            &[
                ("_csrf", &csrf),
                ("at", "2026-01-13T12:00"),
                (&end, "1050"),
                (&start, "0"),
                ("decimal_places", "2"),
                ("serial_number", "NEU"),
            ],
        )
        .await;
    assert_eq!(swap.status(), StatusCode::SEE_OTHER, "{}", text(swap).await);
    let detail = text(c.get(&format!("/meters/{id}")).await).await;
    assert!(detail.contains("NEU") && detail.contains("0,00"));
}

fn detail_register_id(form_html: &str) -> String {
    let start = form_html.find("name=\"value_").expect("Wertfeld") + 12;
    form_html[start..start + 36].to_owned()
}

// @test SPEC_SEC_TENANT_SCOPE
#[tokio::test]
async fn meters_of_other_households_are_not_found() {
    let app = test_app(true);
    let mut anna = Client::new(&app);
    anna.login("anna").await;
    let csrf = anna.csrf().await;
    let id = create_meter(&mut anna, &csrf, "Annas Wasser").await;

    let mut bob = Client::new(&app);
    bob.login("bob").await;
    assert_eq!(
        bob.get(&format!("/meters/{id}")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        bob.get(&format!("/meters/{id}/edit")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        !text(bob.get("/meters").await)
            .await
            .contains("Annas Wasser")
    );
    assert_eq!(
        bob.get("/meters/kein-uuid").await.status(),
        StatusCode::NOT_FOUND
    );
}

// @test SPEC_I18N, SPEC_TEN_HOUSEHOLDS, SPEC_PRIV_HOUSEHOLD_DELETE
#[tokio::test]
async fn settings_language_households_and_logout() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;

    assert_eq!(
        c.post("/settings/locale", &[("_csrf", &csrf), ("locale", "en")])
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert!(text(c.get("/").await).await.contains("Overview"));

    assert_eq!(
        c.post(
            "/households",
            &[
                ("_csrf", &csrf),
                ("new_name", "Ferienwohnung"),
                ("new_timezone", "Europe/Vienna")
            ]
        )
        .await
        .status(),
        StatusCode::SEE_OTHER
    );
    let page = text(c.get("/settings").await).await;
    assert!(
        page.contains("Ferienwohnung") && page.contains("Europe/Vienna"),
        "neuer Haushalt aktiv"
    );
    let bad_tz = c
        .post(
            "/settings/household",
            &[
                ("_csrf", &csrf),
                ("name", "X"),
                ("timezone", "Mars/Olympus"),
            ],
        )
        .await;
    assert_eq!(bad_tz.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let wrong = c
        .post(
            "/households/delete",
            &[("_csrf", &csrf), ("confirm", "falsch")],
        )
        .await;
    assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        c.post(
            "/households/delete",
            &[("_csrf", &csrf), ("confirm", "Ferienwohnung")]
        )
        .await
        .status(),
        StatusCode::SEE_OTHER
    );
    assert!(
        !text(c.get("/settings").await)
            .await
            .contains("Ferienwohnung")
    );

    let res = c.post("/auth/logout", &[("_csrf", &csrf)]).await;
    assert_eq!(location(&res), "/auth/login?logged_out=true");
    assert_eq!(c.get("/").await.status(), StatusCode::SEE_OTHER);
}
