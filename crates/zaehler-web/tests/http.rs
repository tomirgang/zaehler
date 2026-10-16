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
        analysis: Arc::default(),
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
    create_meter_due(c, csrf, name, "30").await
}

/// Messstelle mit Startstand am 01.01.2026 und Ableseturnus in Tagen.
async fn create_meter_due(c: &mut Client, csrf: &str, name: &str, interval: &str) -> String {
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
                (
                    "schedule_kind",
                    if interval.is_empty() {
                        ""
                    } else {
                        "every_days"
                    },
                ),
                ("schedule_value", interval),
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

/// Messstelle mit zwei Registern; das erste ist Bezug, das zweite hat
/// die Richtung `second`. Liefert den Pfad der Detailseite.
async fn create_two_register_meter(
    c: &mut Client,
    csrf: &str,
    name: &str,
    codes: [&str; 2],
    second: &str,
) -> String {
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
                ("schedule_kind", ""),
                ("schedule_value", ""),
                ("max_daily_rate", ""),
                ("billing_month", ""),
                ("billing_day", ""),
                ("start_at", "2026-01-01T00:00"),
                ("register_count", "2"),
                ("register_0_id", ""),
                ("register_0_code", codes[0]),
                ("register_0_direction", "import"),
                ("register_0_start", "1000"),
                ("register_1_id", ""),
                ("register_1_code", codes[1]),
                ("register_1_direction", second),
                ("register_1_start", "500"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    location(&res)
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
        analysis: Arc::default(),
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
        analysis: Arc::default(),
    };
    router(state, MemoryStore::default())
}

/// Multipart-Body mit optionalem CSRF-Feld und Datei `photo`.
fn multipart(csrf: Option<&str>, photo: &[u8]) -> (String, Vec<u8>) {
    multipart_file(csrf, "photo", photo)
}

/// Multipart-Body mit optionalem CSRF-Feld und einer Datei im Feld `name`.
fn multipart_file(csrf: Option<&str>, name: &str, file: &[u8]) -> (String, Vec<u8>) {
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
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"datei\"\r\nContent-Type: application/octet-stream\r\n\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(file);
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
    // ohne `capture`: Kamera und Galerie mit Mehrfachauswahl (SPEC_UI_PWA)
    assert!(!page.contains("capture=\""));
    assert!(page.contains(" multiple required"));

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

    let html = reviewed(&mut c, &draft_page).await;
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

// @test SPEC_AI_NEW_METER
#[tokio::test]
async fn new_meter_from_photo_is_prefilled_and_keeps_the_photo() {
    let app = capture_app(1024 * 1024);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    assert!(
        text(c.get("/meters").await)
            .await
            .contains("href=\"/capture\"")
    );

    let res = upload(&mut c, "/capture", Some(&csrf), &fake_jpeg(64)).await;
    let draft_page = location(&res);
    let draft = draft_page.trim_start_matches("/capture/").to_owned();
    let html = reviewed(&mut c, &draft_page).await;
    assert!(html.contains(&format!("href=\"/meters/new?draft={draft}\"")));

    let form = text(c.get(&format!("/meters/new?draft={draft}")).await).await;
    assert!(form.contains(&format!("name=\"draft\" value=\"{draft}\"")));
    assert!(form.contains(&format!("src=\"/capture/{draft}/photo\"")));
    assert!(
        form.contains("name=\"name\" value=\"Strom\""),
        "Name aus der Art"
    );
    assert!(form.contains("name=\"decimal_places\" value=\"1\""));
    assert!(form.contains("name=\"register_0_start\" value=\"1025\""));
    assert_eq!(
        c.get("/meters/new?draft=kein-entwurf").await.status(),
        StatusCode::NOT_FOUND
    );

    let res = c
        .post(
            "/meters",
            &[
                ("_csrf", csrf.as_str()),
                ("draft", draft.as_str()),
                ("name", "Strom"),
                ("kind", "electricity"),
                ("unit", ""),
                ("serial_number", ""),
                ("rollover_digits", ""),
                ("decimal_places", "1"),
                ("schedule_kind", "every_days"),
                ("schedule_value", "30"),
                ("max_daily_rate", ""),
                ("billing_month", ""),
                ("billing_day", ""),
                ("start_at", "2026-01-10T08:00"),
                ("register_count", "1"),
                ("register_0_id", ""),
                ("register_0_code", "main"),
                ("register_0_direction", "import"),
                ("register_0_start", "1025"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let detail = text(c.get(&location(&res)).await).await;
    assert!(detail.contains("/photo\""), "Belegfoto verlinkt");
    assert_eq!(c.get(&draft_page).await.status(), StatusCode::NOT_FOUND);
}

fn import_app(max_csv_bytes: usize) -> Router {
    let (ports, store, _clock) = memory_ports();
    let state = WebState {
        uc: Arc::new(UseCases::new(&ports)),
        readiness: store,
        cfg: Arc::new(
            WebConfig::new("http://localhost:8080", true, false, 24, "test")
                .with_max_csv_bytes(max_csv_bytes),
        ),
        oidc: None,
        analysis: Arc::default(),
    };
    router(state, MemoryStore::default())
}

async fn upload_csv(c: &mut Client, csrf: Option<&str>, csv: &[u8]) -> Response {
    let (content_type, body) = multipart_file(csrf, "file", csv);
    let req = Request::post("/import/upload")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap();
    c.send(req).await
}

// @test SPEC_IMP_CSV, SPEC_IMP_CSV_FORMAT, SPEC_SEC_UPLOADS, SPEC_SEC_SESSION_CSRF
#[tokio::test]
async fn csv_import_workflow() {
    let app = import_app(64 * 1024);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    // ohne hochgeladene Datei zurück zum Upload
    assert_eq!(location(&c.get("/import/preview").await), "/import");
    let meter = create_meter(&mut c, &csrf, "Strom").await;
    assert!(text(c.get("/import").await).await.contains("name=\"file\""));

    let csv = "Datum;Zählerstand\n01.10.2025;910,0\n01.11.2025;940,0\n01.12.2025;970,0\n";
    assert_eq!(
        upload_csv(&mut c, None, csv.as_bytes()).await.status(),
        StatusCode::FORBIDDEN
    );
    let res = upload_csv(&mut c, Some(&csrf), b"\xFF\xD8\xFF\xE0").await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("UTF-8"));
    let big = vec![b'1'; 70 * 1024];
    assert_eq!(
        upload_csv(&mut c, Some(&csrf), &big).await.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );

    let res = upload_csv(&mut c, Some(&csrf), csv.as_bytes()).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), "/import/preview");
    let mapping = text(c.get("/import/preview").await).await;
    assert!(mapping.contains("3 Datenzeilen"));
    assert!(mapping.contains("name=\"has_header\" value=\"true\" checked"));
    assert!(mapping.contains(&format!("name=\"meter\" value=\"{meter}\"")));

    let fields = [
        ("_csrf", csrf.as_str()),
        ("meter", meter.as_str()),
        ("has_header", "true"),
        ("col_date", "0"),
        ("col_time", ""),
        ("col_value", "1"),
        ("col_register", ""),
        ("col_serial", ""),
        ("col_note", ""),
        ("date_format", "dmy"),
        ("decimal", "comma"),
        ("default_register", ""),
    ];
    let res = c.post("/import/preview", &fields).await;
    assert_eq!(res.status(), StatusCode::OK);
    let preview = text(res).await;
    assert_eq!(preview.matches("class=\"row-ok\"").count(), 3);
    assert!(preview.contains("action=\"/import/commit\""));

    let res = c.post("/import/commit", &fields).await;
    assert_eq!(res.status(), StatusCode::OK);
    let result = text(res).await;
    assert!(result.contains(&format!("href=\"/meters/{meter}\"")));
    let detail = text(c.get(&format!("/meters/{meter}")).await).await;
    assert_eq!(
        detail.matches("CSV-Import").count(),
        3 + 1,
        "drei Zeilen und Navigation"
    );
    // Datei verbraucht
    assert_eq!(
        location(&c.post("/import/commit", &fields).await),
        "/import"
    );

    // Datei eines anderen Haushalts ist nicht erreichbar
    let res = upload_csv(&mut c, Some(&csrf), csv.as_bytes()).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let mut bob = Client::new(&app);
    bob.login("bob").await;
    assert_eq!(location(&bob.get("/import/preview").await), "/import");
}

// @test SPEC_UI_TOPBAR, SPEC_TEN_HOUSEHOLDS
#[tokio::test]
async fn topbar_marks_the_household_and_renaming_updates_it() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let page = text(c.get("/").await).await;
    assert!(page.contains("id=\"household-label\" class=\"household-label\">Haushalt</span>"));
    assert!(page.contains("<strong class=\"household-name\">anna</strong>"));
    assert!(!page.contains("href=\"/settings#household\""));
    assert!(page.contains("Angemeldet als anna"));
    assert!(
        text(c.get("/settings").await)
            .await
            .contains("id=\"household\"")
    );

    let res = c
        .post(
            "/settings/household",
            &[
                ("_csrf", csrf.as_str()),
                ("name", "Haus am See"),
                ("timezone", "Europe/Berlin"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let page = text(c.get("/").await).await;
    assert!(page.contains("<strong class=\"household-name\">Haus am See</strong>"));
}

async fn import_archive(c: &mut Client, csrf: Option<&str>, file: &[u8]) -> Response {
    let (content_type, body) = multipart_file(csrf, "file", file);
    let req = Request::post("/settings/import")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap();
    c.send(req).await
}

// @test SPEC_BACKUP_HOUSEHOLD, SPEC_SEC_SESSION_CSRF
#[tokio::test]
async fn household_archive_download_and_import_as_new_household() {
    let app = test_app(true);
    let mut anna = Client::new(&app);
    anna.login("anna").await;
    let csrf = anna.csrf().await;
    create_meter(&mut anna, &csrf, "Strom").await;
    let settings = text(anna.get("/settings").await).await;
    assert!(settings.contains("href=\"/settings/export\" hx-boost=\"false\" download"));

    let res = anna.get("/settings/export").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/zip");
    let disposition = res.headers()[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        disposition.starts_with("attachment; filename=\"zaehler-anna-"),
        "{disposition}"
    );
    let archive = to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    assert!(archive.starts_with(b"PK"));

    let mut bob = Client::new(&app);
    bob.login("bob").await;
    let bob_csrf = bob.csrf().await;
    assert_eq!(
        import_archive(&mut bob, None, &archive).await.status(),
        StatusCode::FORBIDDEN
    );
    let res = import_archive(&mut bob, Some(&bob_csrf), b"kein archiv").await;
    assert_eq!(location(&res), "/settings");
    assert!(
        text(bob.get("/settings").await)
            .await
            .contains("kein gültiges Archiv")
    );

    let res = import_archive(&mut bob, Some(&bob_csrf), &archive).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), "/settings");
    // aktiver Haushalt ist der importierte
    let meters = text(bob.get("/meters").await).await;
    assert!(meters.contains(">Strom<"), "Messstelle aus dem Archiv");
    assert!(
        text(bob.get("/settings").await)
            .await
            .contains("id=\"hh-name\" name=\"name\" value=\"anna\"")
    );
}

// @test SPEC_UI_DASHBOARD
#[tokio::test]
async fn dashboard_renders_0_1_and_20_meters_with_overdue_first() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;

    let empty = text(c.get("/").await).await;
    assert!(empty.contains("href=\"/meters/new\"") && empty.contains("href=\"/import\""));

    let first = create_meter_due(&mut c, &csrf, "Strom", "30").await;
    let one = text(c.get("/").await).await;
    assert!(one.contains(&format!("href=\"/meters/{first}/readings/new\"")));
    assert!(one.contains("href=\"/capture\""));
    assert!(one.contains("main 1000,0 kWh"), "letzter Stand");

    for i in 2..=19 {
        create_meter_due(&mut c, &csrf, &format!("Zähler {i:02}"), "30").await;
    }
    // Startstand am 01.01., Uhr am 15.01.: nach 7 Tagen überfällig
    create_meter_due(&mut c, &csrf, "Wasser", "7").await;
    let res = c.get("/").await;
    assert_eq!(res.status(), StatusCode::OK);
    let html = text(res).await;
    assert_eq!(html.matches("<li class=\"card").count(), 20);
    let water = html.find(">Wasser<").unwrap();
    let power = html.find(">Strom<").unwrap();
    assert!(water < power, "überfällige Messstelle zuerst");
    assert!(html.contains("class=\"card overdue\""));
}

// @test SPEC_ANA_COMPARE_VIEW, SPEC_ANA_CHARTS, SPEC_ANA_REGISTER_CHARTS, SPEC_ANA_PARTIAL_MONTHS
#[tokio::test]
async fn meter_detail_shows_analysis_and_chart_data() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let meter = create_meter(&mut c, &csrf, "Strom").await;

    let html = text(c.get(&format!("/meters/{meter}?month=2026-01")).await).await;
    assert!(html.contains("id=\"analysis-title\""));
    assert!(html.contains(&format!("data-chart-url=\"/meters/{meter}/chart.json\"")));
    assert!(html.contains("/static/test/echarts.min.js"));
    assert!(html.contains("<details>"), "Tabellenalternative");
    assert!(html.contains("Kalenderjahr 2026"));

    let res = c.get(&format!("/meters/{meter}/chart.json")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
    let json: serde_json::Value = serde_json::from_str(&text(res).await).unwrap();
    assert_eq!(json["unit"], "kWh");
    assert_eq!(json["labels"].as_array().unwrap().len(), 12);
    assert_eq!(json["labels"][2], "Mär");
    assert_eq!(json["series"]["current"]["name"], "2026");
    assert_eq!(json["series"]["current_projected"]["name"], "2026 Prognose");
    assert_eq!(json["series"]["estimate"]["name"], "Geschätzt");
    assert_eq!(
        json["series"]["previous_projected"]["data"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    assert_eq!(
        json["series"]["forecast"]["data"].as_array().unwrap().len(),
        12
    );
    let js = c.get("/static/test/echarts.min.js").await;
    assert_eq!(js.status(), StatusCode::OK);
    assert_eq!(json["registers"], serde_json::json!([]), "nur ein Register");
    assert!(!html.contains("data-register="));

    // zwei Bezugsregister: Summe als Quelle, Prognose und Diagramme je Register
    let two = create_two_register_meter(&mut c, &csrf, "Doppeltarif", ["HT", "NT"], "import").await;
    let html = text(c.get(&two).await).await;
    assert!(html.contains("Quelle: Summe der Register HT, NT (Bezug)"));
    assert!(html.contains("Register HT (Bezug)"));
    assert!(html.contains("Register NT: Jahresverlauf 2026 mit Prognose"));
    assert!(html.contains("data-chart=\"monthly\" data-register=\"1\""));

    // Bezug und Einspeisung: das Bezugsregister liefert allein die Monatswerte
    // der Messstelle, wird als Quelle genannt und nicht erneut dargestellt
    let pv = create_two_register_meter(&mut c, &csrf, "PV", ["1.8.0", "2.8.0"], "export").await;
    let html = text(c.get(&pv).await).await;
    assert!(html.contains("Quelle: Register 1.8.0 (Bezug)"));
    assert!(!html.contains("data-register=\"0\""));
    assert!(!html.contains("Register 1.8.0: Monatswerte"));
    assert!(html.contains("Register 2.8.0 (Einspeisung)"));
    assert!(html.contains("data-chart=\"monthly\" data-register=\"1\""));
    let json: serde_json::Value =
        serde_json::from_str(&text(c.get(&format!("{two}/chart.json")).await).await).unwrap();
    let registers = json["registers"].as_array().unwrap();
    assert_eq!(registers.len(), 2);
    assert_eq!(registers[0]["code"], "HT");
    assert_eq!(
        registers[1]["series"]["forecast"]["data"]
            .as_array()
            .unwrap()
            .len(),
        12
    );

    let mut bob = Client::new(&app);
    bob.login("bob").await;
    assert_eq!(
        bob.get(&format!("/meters/{meter}/chart.json"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

// @test SPEC_IMG_CAPTURE_TIME
#[tokio::test]
async fn capture_time_from_photo_prefills_the_forms() {
    use zaehler_app::{port::PhotoTime, testing::StubImageNormalizer};

    let (mut ports, store, _clock) = memory_ports();
    ports.images = Arc::new(StubImageNormalizer {
        taken_at: Some(PhotoTime {
            local: chrono::NaiveDate::from_ymd_opt(2026, 1, 10)
                .unwrap()
                .and_hms_opt(8, 15, 0)
                .unwrap(),
            offset: None,
        }),
    });
    let state = WebState {
        uc: Arc::new(UseCases::new(&ports)),
        readiness: store,
        cfg: Arc::new(
            WebConfig::new("http://localhost:8080", true, false, 24, "test")
                .with_max_photo_bytes(1024 * 1024),
        ),
        oidc: None,
        analysis: Arc::default(),
    };
    let app = router(state, MemoryStore::default());
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let res = upload(&mut c, "/capture", Some(&csrf), &fake_jpeg(64)).await;
    let draft_page = location(&res);
    let draft = draft_page.trim_start_matches("/capture/").to_owned();
    let meter = create_meter(&mut c, &csrf, "Strom").await;

    let confirm = reviewed(&mut c, &format!("{draft_page}?meter={meter}")).await;
    assert!(
        confirm.contains("value=\"2026-01-10T08:15\""),
        "Zeitpunkt aus EXIF"
    );
    assert!(confirm.contains("Aufnahmezeit aus den Fotodaten"));
    let form = text(c.get(&format!("/meters/new?draft={draft}")).await).await;
    assert!(form.contains("name=\"start_at\" type=\"datetime-local\" value=\"2026-01-10T08:15\""));
}

// @test SPEC_SEC_UPLOADS
#[tokio::test]
async fn photo_upload_limit_returns_413() {
    let app = capture_app(4096);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    // einzelnes Foto über `max_photo_bytes`: Hinweis zu diesem Foto
    let res = upload(&mut c, "/capture", Some(&csrf), &fake_jpeg(8192)).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("zu groß"));
    // Upload über `max_photo_bytes` mal Höchstzahl Fotos
    let res = upload(&mut c, "/capture", Some(&csrf), &fake_jpeg(4096 * 20 + 1)).await;
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let res = upload(&mut c, "/capture", Some(&csrf), &fake_jpeg(1024)).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
}

/// Wartet, bis die Hintergrundanalyse fertig ist, und liefert die Prüfseite.
async fn reviewed(c: &mut Client, path: &str) -> String {
    for _ in 0..500 {
        let html = text(c.get(path).await).await;
        if !html.contains("hx-trigger=\"load delay:2s\"") {
            return html;
        }
        tokio::task::yield_now().await;
    }
    panic!("Analyse nicht abgeschlossen: {path}");
}

/// Multipart-Body mit mehreren Fotos im Feld `photo`.
fn multipart_photos(csrf: &str, photos: &[&[u8]]) -> (String, Vec<u8>) {
    let boundary = "zaehler-test-boundary";
    let mut body =
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n")
            .into_bytes();
    for (i, photo) in photos.iter().enumerate() {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"foto{i}.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(photo);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

// @test SPEC_AI_MULTI
#[tokio::test]
async fn several_photos_are_reviewed_one_after_another() {
    let app = capture_app(1024 * 1024);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let meter = create_meter(&mut c, &csrf, "Strom").await;
    let (content_type, body) =
        multipart_photos(&csrf, &[&fake_jpeg(64), b"kein Bild", &fake_jpeg(64)]);
    let req = Request::post("/capture")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap();
    let res = c.send(req).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let first = location(&res);
    // die Meldung erscheint einmal, auf der ersten Seite nach dem Upload
    let page = text(c.get(&first).await).await;
    assert!(
        page.contains("übersprungen"),
        "Hinweis auf das abgelehnte Foto"
    );

    let html = reviewed(&mut c, &first).await;
    assert!(
        html.contains("Foto 1 von 2"),
        "Position in der Warteschlange"
    );
    let register = {
        let start = html.find("name=\"register_id\"").unwrap();
        let rest = &html[start..];
        let v = rest.find("value=\"").unwrap() + 7;
        rest[v..v + rest[v..].find('"').unwrap()].to_owned()
    };
    let res = c
        .post(
            &format!("{first}/confirm"),
            &[
                ("_csrf", csrf.as_str()),
                ("meter_id", meter.as_str()),
                ("register_id", register.as_str()),
                ("value", "1025"),
                ("at", "2026-01-10T08:00"),
                ("note", ""),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let second = location(&res);
    assert!(
        second.starts_with("/capture/") && second != first,
        "weiter zum nächsten Foto"
    );
    let html = reviewed(&mut c, &second).await;
    assert!(html.contains("Foto 1 von 1") || !html.contains("von 2"));
    let res = c
        .post(&format!("{second}/discard"), &[("_csrf", csrf.as_str())])
        .await;
    assert_eq!(location(&res), "/capture", "keine weiteren Fotos");
    assert!(!text(c.get("/capture").await).await.contains("Jetzt prüfen"));
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
    assert_eq!(login.headers()[header::CACHE_CONTROL], "private, no-cache");
    assert_eq!(
        c.get("/gibt-es-nicht").await.status(),
        StatusCode::NOT_FOUND
    );
}

// @test SPEC_UI_BUSY
#[tokio::test]
async fn both_layouts_load_busy_indicator_script() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    let script = "<script src=\"/static/test/app.js\" defer></script>";
    assert!(text(c.get("/auth/login").await).await.contains(script));
    c.login("anna").await;
    assert!(text(c.get("/").await).await.contains(script));
    let js = text(c.get("/static/test/app.js").await).await;
    assert!(js.contains("htmx:beforeRequest") && js.contains("aria-busy"));
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
        .header(header::ORIGIN, "http://127.0.0.1:9999")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        c.send(req).await.status(),
        StatusCode::FORBIDDEN,
        "lokale Adresse mit anderem Port"
    );
    // öffentliche URL ist http://localhost:8080: 127.0.0.1 gilt als gleichwertig
    let req = Request::post("/auth/logout")
        .header("x-csrf-token", csrf.as_str())
        .header(header::ORIGIN, "http://127.0.0.1:8080")
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
            .contains("Stand passt nicht zwischen die Ablesungen davor und danach.")
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

/// Kein fehlender Schlüssel und kein offener Fluent-Platzhalter.
fn assert_translated(page: &str, what: &str) {
    assert!(
        !page.contains("Unknown localization key"),
        "{what}: fehlender Schlüssel"
    );
    assert!(
        !page.contains("{ $") && !page.contains("{$"),
        "{what}: offener Platzhalter"
    );
}

// @test SPEC_I18N
#[tokio::test]
async fn every_page_renders_in_german_and_english() {
    let app = capture_app(1024 * 1024);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let meter = create_meter(&mut c, &csrf, "Strom").await;
    let register =
        detail_register_id(&text(c.get(&format!("/meters/{meter}/readings/new")).await).await);
    let value_field = format!("value_{register}");
    let res = c
        .post(
            &format!("/meters/{meter}/readings"),
            &[
                ("_csrf", csrf.as_str()),
                ("at", "2026-01-10T10:00"),
                (value_field.as_str(), "1100,5"),
                ("note", ""),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let res = upload(&mut c, "/capture", Some(&csrf), &fake_jpeg(64)).await;
    let draft = location(&res);
    reviewed(&mut c, &draft).await;
    let res = upload_csv(
        &mut c,
        Some(&csrf),
        "Datum;Wert\n01.01.2026;1000\n".as_bytes(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let pages = [
        "/".to_owned(),
        "/meters".into(),
        "/meters/new".into(),
        format!("/meters/{meter}"),
        format!("/meters/{meter}/edit"),
        format!("/meters/{meter}/swap"),
        format!("/meters/{meter}/readings/new"),
        format!("/meters/{meter}/chart.json"),
        "/capture".into(),
        draft.clone(),
        "/import".into(),
        "/import/preview".into(),
        "/settings".into(),
    ];
    for (locale, nav) in [("de", "Übersicht"), ("en", "Overview")] {
        let res = c
            .post("/settings/locale", &[("_csrf", &csrf), ("locale", locale)])
            .await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        for path in &pages {
            let res = c.get(path).await;
            assert_eq!(res.status(), StatusCode::OK, "{locale} {path}");
            let page = text(res).await;
            assert_translated(&page, &format!("{locale} {path}"));
            if !path.ends_with("chart.json") {
                assert!(
                    page.contains(&format!("lang=\"{locale}\"")),
                    "{locale} {path}"
                );
                assert!(page.contains(nav), "{locale} {path}");
            }
        }
        let res = c.get("/meters/00000000-0000-7000-8000-000000000000").await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_translated(&text(res).await, &format!("{locale} 404"));
    }

    // ohne Anmeldung: Sprache aus `Accept-Language`
    for (accept, locale) in [("de-DE,de;q=0.9", "de"), ("en-GB,en;q=0.8", "en")] {
        let mut anon = Client::new(&app);
        let req = Request::get("/auth/login")
            .header(header::ACCEPT_LANGUAGE, accept)
            .body(Body::empty())
            .unwrap();
        let page = text(anon.send(req).await).await;
        assert_translated(&page, &format!("{locale} /auth/login"));
        assert!(page.contains(&format!("lang=\"{locale}\"")));
    }
}

// @test SPEC_UI_PWA
#[tokio::test]
async fn manifest_icons_and_service_worker_are_served() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    let req = Request::get("/manifest.webmanifest")
        .header(header::ACCEPT_LANGUAGE, "en")
        .body(Body::empty())
        .unwrap();
    let res = c.send(req).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()[header::CONTENT_TYPE],
        "application/manifest+json"
    );
    let manifest: serde_json::Value = serde_json::from_str(&text(res).await).unwrap();
    assert_eq!(manifest["display"], "standalone");
    assert_eq!(manifest["start_url"], "/");
    assert_eq!(manifest["lang"], "en");
    let icons = manifest["icons"].as_array().unwrap();
    for icon in icons {
        let src = icon["src"].as_str().unwrap();
        let res = c.get(src).await;
        assert_eq!(res.status(), StatusCode::OK, "{src}");
        assert_eq!(res.headers()[header::CONTENT_TYPE], "image/png");
        let png = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        // Breite und Höhe aus dem IHDR-Block
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!(
            format!("{width}x{height}"),
            icon["sizes"].as_str().unwrap(),
            "{src}"
        );
    }
    assert!(icons.iter().any(|i| i["purpose"] == "maskable"));
    assert!(icons.iter().any(|i| i["sizes"] == "192x192"));
    assert!(icons.iter().any(|i| i["sizes"] == "512x512"));

    let res = c.get("/sw.js").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        res.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/javascript")
    );
    let sw = text(res).await;
    assert!(
        !sw.contains("addEventListener(\"fetch\""),
        "kein Offline-Cache"
    );

    c.login("anna").await;
    let page = text(c.get("/").await).await;
    assert!(page.contains("<link rel=\"manifest\" href=\"/manifest.webmanifest\">"));
    assert!(page.contains("rel=\"apple-touch-icon\""));
    let capture = text(c.get("/capture").await).await;
    assert!(capture.contains("accept=\"image/jpeg,image/png,image/webp\" multiple"));
}

// @test SPEC_OPS_OTEL
#[tokio::test]
async fn responses_carry_a_request_id() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    let res = c.get("/healthz").await;
    let id = res.headers()["x-request-id"].to_str().unwrap().to_owned();
    assert_eq!(id.len(), 36, "UUID");
    let res = c.get("/healthz").await;
    assert_ne!(res.headers()["x-request-id"], id.as_str());
    // ID eines vorgelagerten Proxys bleibt erhalten
    let req = Request::get("/healthz")
        .header("x-request-id", "proxy-123")
        .body(Body::empty())
        .unwrap();
    assert_eq!(c.send(req).await.headers()["x-request-id"], "proxy-123");
    // zu lange oder mit Leerzeichen: neue ID
    for invalid in ["a b".to_owned(), "x".repeat(65)] {
        let req = Request::get("/healthz")
            .header("x-request-id", invalid.as_str())
            .body(Body::empty())
            .unwrap();
        let res = c.send(req).await;
        assert_eq!(res.headers()["x-request-id"].len(), 36, "{invalid}");
    }
}

/// Link aus der Seite nach dem Erzeugen einer Einladung.
fn invitation_link(html: &str) -> String {
    let start = html
        .find("id=\"invite-link\" value=\"")
        .expect("Einladungslink")
        + 24;
    html[start..start + html[start..].find('"').unwrap()].to_owned()
}

// @test SPEC_TEN_INVITES, SPEC_TEN_ROLES
#[tokio::test]
async fn invitation_workflow_with_login_redirect_and_roles() {
    let app = test_app(true);
    let mut anna = Client::new(&app);
    anna.login("anna").await;
    let csrf = anna.csrf().await;
    let res = anna
        .post(
            "/settings/invitations",
            &[("_csrf", &csrf), ("role", "member"), ("email", "")],
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let link = invitation_link(&text(res).await);
    let path = link.trim_start_matches("http://localhost:8080").to_owned();
    assert!(path.starts_with("/invite/"), "{path}");
    // offene Einladung in der Liste, Link selbst nicht mehr
    let html = text(anna.get("/settings/members").await).await;
    assert!(html.contains("/revoke"));
    assert!(!html.contains(&path));

    // Bea ist nicht angemeldet: Anmeldung, danach zurück zur Einladung
    let mut bea = Client::new(&app);
    let res = bea.get(&path).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), "/auth/login");
    let res = bea.get("/auth/dev-login?name=bea").await;
    assert_eq!(location(&res), path);
    let html = text(bea.get(&path).await).await;
    assert!(html.contains("anna"), "Name des Haushalts");
    let bea_csrf = bea.csrf().await;
    let res = bea
        .post(&format!("{path}/accept"), &[("_csrf", &bea_csrf)])
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    // Haushalt von Anna ist aktiv; Bea ist Mitglied und darf nicht einladen
    let html = text(bea.get("/settings/members").await).await;
    assert!(html.contains("anna") && html.contains("bea"));
    assert!(!html.contains("/settings/invitations"));
    let res = bea
        .post(
            "/settings/invitations",
            &[("_csrf", &bea_csrf), ("role", "owner"), ("email", "")],
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // zweites Einlösen: ungültig ohne Angabe des Grundes
    let res = bea
        .post(&format!("{path}/accept"), &[("_csrf", &bea_csrf)])
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert!(text(res).await.contains("ungültig oder abgelaufen"));

    // letzter Owner kann nicht austreten
    let res = anna.post("/settings/leave", &[("_csrf", &csrf)]).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    // Bea tritt aus
    let res = bea.post("/settings/leave", &[("_csrf", &bea_csrf)]).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let html = text(anna.get("/settings/members").await).await;
    assert!(!html.contains(">bea<"));
}

/// ID der ersten Ablesung mit Bearbeiten-Link auf der Detailseite (neueste zuerst).
fn first_editable_reading(html: &str) -> String {
    let start = html.find("href=\"/readings/").expect("Bearbeiten-Link") + 16;
    html[start..start + 36].to_owned()
}

// @test SPEC_RDG_HISTORY
#[tokio::test]
async fn reading_correction_with_confirmation_and_history() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let id = create_meter(&mut c, &csrf, "Hausstrom").await;
    let register =
        detail_register_id(&text(c.get(&format!("/meters/{id}/readings/new")).await).await);
    let value_field = format!("value_{register}");
    let res = c
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
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let detail = text(c.get(&format!("/meters/{id}")).await).await;
    assert!(!detail.contains(&format!("/meters/{id}/history")));
    let reading = first_editable_reading(&detail);
    let path = format!("/readings/{reading}/edit");
    let form = text(c.get(&path).await).await;
    assert!(form.contains("1100,5") && form.contains("2026-01-10T10:00"));

    // kleiner als der Startstand: Bestätigung nötig, nichts gespeichert
    let lower = [
        ("_csrf", csrf.as_str()),
        ("at", "2026-01-10T10:00"),
        ("value", "900"),
        ("note", "Tippfehler"),
    ];
    let res = c.post(&path, &lower).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("Trotzdem speichern"));
    let mut confirmed = lower.to_vec();
    confirmed.push(("confirm_warnings", "true"));
    let res = c.post(&path, &confirmed).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    let detail = text(c.get(&format!("/meters/{id}")).await).await;
    assert!(detail.contains("geändert") && detail.contains("900,0"));
    let history = text(c.get(&format!("/meters/{id}/history")).await).await;
    assert!(history.contains("korrigiert") && history.contains("1100,5"));
    assert!(history.contains("anna"));

    // nach dem Löschen bleibt der Verlauf
    let res = c
        .post(&format!("/readings/{reading}/delete"), &[("_csrf", &csrf)])
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let history = text(c.get(&format!("/meters/{id}/history")).await).await;
    assert!(history.contains("gelöscht") && history.contains("korrigiert"));
    assert_eq!(c.get(&path).await.status(), StatusCode::NOT_FOUND);
}

/// Einheiten mit Hochzahl erscheinen als HTML-Entität.
// @test SPEC_I18N
#[tokio::test]
async fn water_unit_rendered_with_sup3_entity() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let res = c
        .post(
            "/meters",
            &[
                ("_csrf", csrf.as_str()),
                ("name", "Wasser"),
                ("kind", "water"),
                ("unit", ""),
                ("decimal_places", "3"),
                ("start_at", "2026-01-01T00:00"),
                ("register_count", "1"),
                ("register_0_code", "main"),
                ("register_0_direction", "import"),
                ("register_0_start", "10"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let detail = text(c.get(&location(&res)).await).await;
    assert!(detail.contains("<dd>m&sup3;</dd>"), "{detail}");
    let list = text(c.get("/meters").await).await;
    assert!(list.contains("<td>m&sup3;</td>"));
    assert!(!list.contains("<td>m³</td>"));
}

// @test SPEC_NTF_EMAIL
#[tokio::test]
async fn notification_settings_page() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let html = text(c.get("/settings").await).await;
    assert!(html.contains("/settings/notifications"));
    let html = text(c.get("/settings/notifications").await).await;
    assert!(html.contains("nicht eingerichtet"), "Hinweis ohne SMTP");
    assert!(html.contains(r#"name="reminder" value="true" checked"#));
    let res = c
        .post(
            "/settings/notifications",
            &[
                ("_csrf", csrf.as_str()),
                ("email", "kein-at"),
                ("alarm", "true"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("aria-invalid"));
    let res = c
        .post(
            "/settings/notifications",
            &[
                ("_csrf", csrf.as_str()),
                ("email", "anna@privat.test"),
                ("alarm", "true"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let html = text(c.get("/settings/notifications").await).await;
    assert!(html.contains(r#"value="anna@privat.test""#));
    assert!(html.contains(r#"name="reminder" value="true">"#));
    assert!(html.contains(r#"name="alarm" value="true" checked"#));
    assert!(html.contains("Benachrichtigungen gespeichert."));
}
