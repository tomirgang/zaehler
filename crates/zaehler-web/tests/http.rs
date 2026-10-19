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
    assert!(detail.contains("/photo/view?back="), "Belegfoto verlinkt");
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
    assert!(detail.contains("/photo/view?back="), "Belegfoto verlinkt");
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
    assert!(settings.contains("action=\"/settings/export\""));
    assert!(settings.contains("name=\"format\" value=\"csv\""));

    // CSV-Export (SPEC_IMP_EXPORT_CSV)
    let res = anna.get("/settings/export?format=csv").await;
    assert_eq!(res.status(), StatusCode::OK);
    let disposition = res.headers()[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(disposition.ends_with("-csv.zip\""), "{disposition}");
    let csv = to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    assert!(csv.starts_with(b"PK"));
    assert_eq!(
        anna.get("/settings/export?format=xml").await.status(),
        StatusCode::NOT_FOUND
    );

    let res = anna.get("/settings/export?format=json").await;
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

// @test SPEC_MTR_ARCHIVE, SPEC_MTR_DELETE
#[tokio::test]
async fn meters_can_be_archived_restored_and_deleted() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let meter = create_meter(&mut c, &csrf, "PV").await;
    let detail = text(c.get(&format!("/meters/{meter}")).await).await;
    assert!(detail.contains(&format!("action=\"/meters/{meter}/archive\"")));
    assert!(detail.contains(&format!("action=\"/meters/{meter}/delete\"")));

    let res = c
        .post(
            &format!("/meters/{meter}/archive"),
            &[("_csrf", csrf.as_str())],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let detail = text(c.get(&format!("/meters/{meter}")).await).await;
    assert!(detail.contains(&format!("action=\"/meters/{meter}/restore\"")));

    let res = c
        .post(
            &format!("/meters/{meter}/restore"),
            &[("_csrf", csrf.as_str())],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let detail = text(c.get(&format!("/meters/{meter}")).await).await;
    assert!(detail.contains(&format!("action=\"/meters/{meter}/archive\"")));

    let res = c
        .post(
            &format!("/meters/{meter}/delete"),
            &[("_csrf", csrf.as_str())],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    assert_eq!(location(&res), "/meters");
    assert_eq!(
        c.get(&format!("/meters/{meter}")).await.status(),
        StatusCode::NOT_FOUND
    );
}

// @test SPEC_UI_DASHBOARD, SPEC_UI_DASHBOARD_GROUPS
#[tokio::test]
async fn dashboard_renders_0_1_and_20_meters_grouped_by_name() {
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
    let water = html.find(">Wasser</a>").unwrap();
    let power = html.find(">Strom</a>").unwrap();
    assert!(
        power < water,
        "nach Name sortiert, überfällige bleiben an ihrem Platz"
    );
    assert!(html.contains("<h2 id=\"group-electricity\">Strom</h2>"));
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

/// Einheiten mit Hochzahl erscheinen als hochgestellte Ziffer, auch in
/// Tabellenköpfen mit tabellarischen Ziffern.
// @test SPEC_I18N
#[tokio::test]
async fn water_unit_rendered_with_superscript() {
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
    assert!(detail.contains("<dd>m<sup>3</sup></dd>"), "{detail}");
    // jede Klammer mit Einheit enthält die Hochzahl, nie nur „(m)“
    assert!(!detail.contains("(m)"));
    assert!(detail.contains("(m<sup>3</sup>)"));
    assert!(!detail.contains("&sup3;") && !detail.contains("m³<"));
    let list = text(c.get("/meters").await).await;
    assert!(list.contains("<td>m<sup>3</sup></td>"));
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

/// Felder eines Stromvertrags mit einer Position.
fn contract_fields<'a>(
    csrf: &'a str,
    provider: &'a str,
    start: &'a str,
) -> Vec<(&'a str, &'a str)> {
    vec![
        ("_csrf", csrf),
        ("template", "electricity"),
        ("positions", "1"),
        ("direction", "import"),
        ("provider", provider),
        ("contract_number", "V-1"),
        ("starts_on", start),
        ("ends_on", ""),
        ("billing_month", ""),
        ("billing_day", ""),
        ("note", ""),
        ("pos0_id", ""),
        ("pos0_name", "Strom"),
        ("pos0_register", ""),
        ("pos0_factor", "1"),
        ("pos0_vat", "19"),
        ("pos0_components", "1"),
        ("pos0_c0_id", ""),
        ("pos0_c0_valid_from", start),
        ("pos0_c0_base", "119"),
        ("pos0_c0_interval", "year"),
        ("pos0_c0_price", "0,357"),
        ("pos0_c0_tiers", ""),
        ("pos0_c0_factor", ""),
        ("action", "save"),
    ]
}

// @test SPEC_TAR_CONTRACTS, SPEC_TAR_COST_VIEW, SPEC_TAR_ACTUAL_SETTLEMENT, SPEC_SEC_TENANT_SCOPE
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn contracts_and_cost_pages() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let id = create_meter(&mut c, &csrf, "Hausstrom").await;
    let detail = text(c.get(&format!("/meters/{id}")).await).await;
    assert!(detail.contains(&format!("/meters/{id}/contracts/new?template=electricity")));
    assert!(
        !detail.contains("href=\"/costs\""),
        "Navigation ohne Verträge"
    );

    // Vorlage, Zeile hinzufügen ohne Speichern
    let form = text(
        c.get(&format!("/meters/{id}/contracts/new?template=electricity"))
            .await,
    )
    .await;
    assert!(form.contains(r#"name="pos0_name" value="Strom""#), "{form}");
    let mut add = contract_fields(&csrf, "Stadtwerke", "2026-01-01");
    add.retain(|(k, _)| *k != "action");
    add.push(("action", "add_position"));
    let res = c.post(&format!("/meters/{id}/contracts/new"), &add).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(text(res).await.contains(r#"name="pos1_name""#));

    // Anlegen
    let res = c
        .post(
            &format!("/meters/{id}/contracts/new"),
            &contract_fields(&csrf, "Stadtwerke", "2026-01-01"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let contract = location(&res);
    assert!(contract.starts_with("/contracts/"));
    let html = text(c.get(&contract).await).await;
    assert!(html.contains("Stadtwerke") && html.contains("Vertrag angelegt."));
    assert!(html.contains("0,357 €"), "Arbeitspreis");
    assert!(html.contains("Arbeitspreis 0,3000 €"), "netto bei 19 %");
    assert!(html.contains("href=\"/costs\""), "Navigation mit Verträgen");

    // Überlappung
    let res = c
        .post(
            &format!("/meters/{id}/contracts/new"),
            &contract_fields(&csrf, "Anderer", "2026-06-01"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(text(res).await.contains("überschneidet"));
    // ungültige Eingabe
    let mut bad = contract_fields(&csrf, "", "2026-01-01");
    bad.retain(|(k, _)| *k != "pos0_c0_price");
    bad.push(("pos0_c0_price", "abc"));
    let res = c.post(&format!("/meters/{id}/contracts/new"), &bad).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("Keine gültige Zahl."));

    // Abschlag, Fehler und Erfolg
    let res = c
        .post(
            &format!("{contract}/installments"),
            &[
                ("_csrf", &csrf),
                ("installment_valid_from", "2026-01-15"),
                ("installment_amount", "abc"),
                ("installment_interval", "1"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("Keine gültige Zahl."));
    let res = c
        .post(
            &format!("{contract}/installments"),
            &[
                ("_csrf", &csrf),
                ("installment_valid_from", "2026-01-15"),
                ("installment_amount", "100"),
                ("installment_interval", "1"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert!(text(c.get(&contract).await).await.contains("100,00 €"));

    // Kosten ohne Ablesung im Zeitraum, dann mit
    let html = text(c.get(&format!("/meters/{id}/costs")).await).await;
    assert!(html.contains("Ohne Ablesungen"), "{html}");
    let register =
        detail_register_id(&text(c.get(&format!("/meters/{id}/readings/new")).await).await);
    let value_field = format!("value_{register}");
    let res = c
        .post(
            &format!("/meters/{id}/readings"),
            &[
                ("_csrf", &csrf),
                ("at", "2026-01-11T00:00"),
                (&value_field, "1100"),
                ("note", ""),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let html = text(c.get(&format!("/meters/{id}/costs")).await).await;
    assert!(html.contains("Abrechnungszeitraum"), "{html}");
    assert!(html.contains("prognostiziert"));
    assert!(html.contains("erwartete"), "Saldo");
    let res = c.get(&format!("/meters/{id}/costs.json")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(text(res).await.contains("\"labels\""));
    let household = text(c.get("/costs").await).await;
    assert!(household.contains("Hausstrom") && household.contains("Kosten des Haushalts"));
    assert_eq!(
        c.get("/costs.json?year=2026").await.status(),
        StatusCode::OK
    );

    // Abrechnung
    let res = c
        .post(
            &format!("{contract}/settlements"),
            &[
                ("_csrf", &csrf),
                ("settlement_period_start", "2026-01-01"),
                ("settlement_period_end", "2026-01-10"),
                ("settlement_amount", "45,20"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let html = text(c.get(&format!("/meters/{id}/costs")).await).await;
    assert!(html.contains("45,20 €"), "abgerechnet");

    // fremder Haushalt
    let mut bob = Client::new(&app);
    bob.login("bob").await;
    let bob_csrf = bob.csrf().await;
    assert_eq!(bob.get(&contract).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        bob.get(&format!("/meters/{id}/costs")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        bob.post(&format!("{contract}/delete"), &[("_csrf", &bob_csrf)])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Löschen
    let res = c
        .post(&format!("{contract}/delete"), &[("_csrf", &csrf)])
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), format!("/meters/{id}"));
    assert_eq!(c.get(&contract).await.status(), StatusCode::NOT_FOUND);
}

fn document_app() -> Router {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use zaehler_app::{
        model::{InvoiceExtraction, InvoiceInstallment, InvoicePrice, InvoiceReading},
        testing::StubExtractor,
    };
    use zaehler_domain::tariff::BasePriceInterval;

    let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let (mut ports, store, _clock) = memory_ports();
    ports.extractor = Some(Arc::new(StubExtractor(Ok(InvoiceExtraction {
        provider: Some("Stadtwerke Süd".into()),
        contract_number: Some("V-1".into()),
        period_start: Some(day(1, 1)),
        period_end: Some(day(1, 9)),
        readings: vec![InvoiceReading {
            serial_number: None,
            register: Some("main".into()),
            date: day(1, 9),
            value: Decimal::from(1080),
        }],
        prices: vec![
            InvoicePrice {
                position: "Arbeitspreis".into(),
                valid_from: Some(day(1, 1)),
                base_price: None,
                base_interval: None,
                unit_price: Some(Decimal::new(35, 2)),
            },
            InvoicePrice {
                position: "Grundpreis".into(),
                valid_from: Some(day(1, 1)),
                base_price: Some(Decimal::from(120)),
                base_interval: Some(BasePriceInterval::Year),
                unit_price: None,
            },
        ],
        installment: Some(InvoiceInstallment {
            amount: Decimal::from(50),
            every_months: 1,
            valid_from: Some(day(2, 1)),
        }),
        total_amount: Some(Decimal::from(30)),
        confidence: Decimal::new(9, 1),
    }))));
    let state = WebState {
        uc: Arc::new(UseCases::new(&ports)),
        readiness: store,
        cfg: Arc::new(
            WebConfig::new("http://localhost:8080", true, false, 24, "test")
                .with_max_document_bytes(64 * 1024),
        ),
        oidc: None,
        analysis: Arc::default(),
    };
    router(state, MemoryStore::default())
}

async fn upload_document(c: &mut Client, csrf: Option<&str>, file: &[u8]) -> Response {
    let (content_type, body) = multipart_file(csrf, "file", file);
    let req = Request::post("/documents")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap();
    c.send(req).await
}

// @test SPEC_DOC_UPLOADS, SPEC_AI_INVOICE, SPEC_SEC_UPLOADS, SPEC_SEC_SESSION_CSRF
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn invoice_import_workflow() {
    let app = document_app();
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let meter = create_meter(&mut c, &csrf, "Hausstrom").await;
    let page = text(c.get("/documents").await).await;
    assert!(page.contains("enctype=\"multipart/form-data\""));
    assert!(page.contains("Keine offenen Dokumente."));

    // ohne CSRF-Token, kein PDF, zu groß
    let pdf = b"%PDF-1.7 Rechnung";
    assert_eq!(
        upload_document(&mut c, None, pdf).await.status(),
        StatusCode::FORBIDDEN
    );
    let res = upload_document(&mut c, Some(&csrf), b"kein Dokument").await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("Nur PDF-Dateien und Fotos"));
    let mut big = pdf.to_vec();
    big.resize(200 * 1024, b' ');
    let res = upload_document(&mut c, Some(&csrf), &big).await;
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // Hochladen und Erkennen; einzige Messstelle ist vorausgewählt
    let res = upload_document(&mut c, Some(&csrf), pdf).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let doc = location(&res);
    assert!(doc.starts_with("/documents/"));
    let page = text(c.get(&doc).await).await;
    assert!(page.contains("Stadtwerke Süd"), "{page}");
    assert!(
        page.contains(r#"name="r0_at" value="2026-01-10T00:00""#),
        "Folgetag"
    );
    assert!(page.contains(r#"name="contract_mode" value="new" checked"#));
    assert!(page.contains(&format!(r#"name="meter_id" value="{meter}""#)));
    let start = page.find(r#"name="r0_register""#).unwrap();
    let opt = start + page[start..].find("<option value=\"").unwrap() + 15;
    let register = page[opt..opt + page[opt..].find('"').unwrap()].to_owned();
    assert!(!register.is_empty());

    // Datei nur als Download in einer Sandbox
    let res = c.get(&format!("{doc}/file")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/pdf");
    assert!(
        res.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    assert!(
        res.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .starts_with("sandbox")
    );

    let mut fields = vec![
        ("_csrf", csrf.as_str()),
        ("meter_id", meter.as_str()),
        ("readings", "1"),
        ("r0_use", "1"),
        ("r0_register", register.as_str()),
        ("r0_at", "2026-01-10T00:00"),
        ("r0_value", "1080"),
        ("contract_mode", "new"),
        ("provider", "Stadtwerke Süd"),
        ("contract_number", "V-1"),
        ("starts_on", "2026-01-01"),
        ("prices", "2"),
        ("price0_use", "1"),
        ("price0_name", "Arbeitspreis"),
        ("price0_position", ""),
        ("price0_valid_from", "2026-01-01"),
        ("price0_base", ""),
        ("price0_interval", "year"),
        ("price0_price", "0,35"),
        ("price1_use", "1"),
        ("price1_name", "Grundpreis"),
        ("price1_position", ""),
        ("price1_valid_from", "2026-01-01"),
        ("price1_base", "120"),
        ("price1_interval", "year"),
        ("price1_price", ""),
        ("installment", "1"),
        ("inst_amount", "50"),
        ("inst_every", "1"),
        ("inst_from", "2026-02-01"),
        ("settlement", "1"),
        ("settle_from", "2026-01-01"),
        ("settle_to", "2026-01-09"),
        ("settle_amount", "abc"),
    ];
    // Fehler: nichts gespeichert, Formular mit Meldung
    let res = c.post(&format!("{doc}/confirm"), &fields).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("Keine gültige Zahl."));
    fields.pop();
    fields.push(("settle_amount", "30"));
    let res = c.post(&format!("{doc}/confirm"), &fields).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let contract = location(&res);
    assert!(contract.starts_with("/contracts/"));
    let html = text(c.get(&contract).await).await;
    assert!(html.contains("Stadtwerke Süd") && html.contains("Rechnung übernommen."));
    assert!(
        html.contains(&format!("{doc}/view?back=")),
        "Beleg der Abrechnung"
    );
    assert!(html.contains("50,00 €"), "Abschlag");
    let detail = text(c.get(&format!("/meters/{meter}")).await).await;
    assert!(detail.contains("1080,0"), "Ablesung aus der Rechnung");
    // übernommen: nicht mehr offen
    assert!(
        text(c.get("/documents").await)
            .await
            .contains("Keine offenen Dokumente.")
    );
    assert_eq!(
        c.post(&format!("{doc}/discard"), &[("_csrf", &csrf)])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // zweites Dokument verwerfen; anderer Haushalt sieht nichts
    let res = upload_document(&mut c, Some(&csrf), pdf).await;
    let second = location(&res);
    let res = c
        .post(&format!("{second}/discard"), &[("_csrf", &csrf)])
        .await;
    assert_eq!(location(&res), "/documents");
    let mut bob = Client::new(&app);
    bob.login("bob").await;
    assert_eq!(bob.get(&doc).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        bob.get(&format!("{doc}/file")).await.status(),
        StatusCode::NOT_FOUND
    );
}

// @test SPEC_CAR_VEHICLES, SPEC_ASSET_COSTS, SPEC_CAR_REPORT, SPEC_CAR_ENERGY_COST
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn vehicle_pages_and_cost_book() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let page = text(c.get("/").await).await;
    assert!(
        !page.contains("href=\"/vehicles\""),
        "Navigation ohne Fahrzeuge"
    );

    // ungültig: ohne Kilometerstand
    let mut fields = vec![
        ("_csrf", csrf.as_str()),
        ("name", "Familienauto"),
        ("plate", "M-ZA 1"),
        ("model", "Kombi"),
        ("drive_type", "combustion"),
        ("home_charging_meter_id", ""),
        ("pv_share_pct", ""),
        ("odometer", ""),
        ("start_at", "2025-01-01"),
    ];
    let res = c.post("/vehicles/new", &fields).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("Zahl erforderlich."));
    fields.retain(|(k, _)| *k != "odometer");
    fields.push(("odometer", "10000"));
    let res = c.post("/vehicles/new", &fields).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let vehicle = location(&res);
    assert!(vehicle.starts_with("/vehicles/"));
    let html = text(c.get(&vehicle).await).await;
    assert!(html.contains("Fahrzeug angelegt.") && html.contains("M-ZA 1"));
    assert!(html.contains("Verbrenner"));
    assert!(
        !html.contains("Ladevorgang erfassen"),
        "Verbrenner lädt nicht"
    );
    assert!(
        html.contains("href=\"/vehicles\""),
        "Navigation mit Fahrzeugen"
    );
    let list = text(c.get("/vehicles").await).await;
    assert!(list.contains("Familienauto"));

    // Kostenbuch: Laden nicht erlaubt, Tanken mit Kilometerstand als Ablesung
    let costs = format!("{vehicle}/costs");
    let entry = |category: &'static str, amount: &'static str| {
        vec![
            ("_csrf", csrf.clone()),
            ("booked_on", "2025-03-01".to_owned()),
            ("category", category.to_owned()),
            ("amount", amount.to_owned()),
            ("quantity", String::new()),
            ("odometer", String::new()),
            ("recurrence_months", String::new()),
            ("recurrence_until", String::new()),
            ("note", String::new()),
        ]
    };
    let res = post_fields(&mut c, &costs, &entry("charging", "10")).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("passt nicht zur Antriebsart"));
    let mut fuel = entry("fuel", "70,50");
    fuel.retain(|(k, _)| !matches!(*k, "quantity" | "odometer"));
    fuel.extend([
        ("quantity", "40".to_owned()),
        ("odometer", "10500".to_owned()),
        ("full_tank", "1".to_owned()),
        ("odometer_reading", "1".to_owned()),
    ]);
    let res = post_fields(&mut c, &costs, &fuel).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    assert_eq!(location(&res), format!("{costs}?year=2025"));
    let mut insurance = entry("insurance", "480");
    insurance.retain(|(k, _)| *k != "recurrence_months");
    insurance.push(("recurrence_months", "12".to_owned()));
    let res = post_fields(&mut c, &costs, &insurance).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let book = text(c.get(&format!("{costs}?year=2026")).await).await;
    assert!(
        book.contains("Versicherung") && book.contains("480,00 €"),
        "wiederkehrend"
    );
    assert!(!book.contains("70,50 €"), "einmalig nur 2025");
    let book = text(c.get(&format!("{costs}?year=2025")).await).await;
    assert!(book.contains("Kraftstoff") && book.contains("550,50 €"));
    let meter = text(c.get(&vehicle.replace("/vehicles/", "/meters/")).await).await;
    assert!(meter.contains("10500"), "Kilometerstand als Ablesung");
    assert!(meter.contains(&format!("href=\"{vehicle}\"")));

    // Bearbeiten und Löschen eines Eintrags
    let start = book.find("/costs/").unwrap() + 7;
    let entry_id = book[start..start + 36].to_owned();
    let edit = text(c.get(&format!("{costs}/{entry_id}/edit")).await).await;
    assert!(edit.contains("Eintrag bearbeiten"));
    let res = c
        .post(&format!("{costs}/{entry_id}/delete"), &[("_csrf", &csrf)])
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);

    // Antriebsart gesperrt, Auswertung vorhanden
    let mut edit = fields.clone();
    edit.retain(|(k, _)| !matches!(*k, "drive_type" | "odometer" | "start_at" | "name"));
    edit.push(("drive_type", "electric"));
    let res = c.post(&format!("{vehicle}/edit"), &edit).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("nicht mehr änderbar"));
    let html = text(c.get(&vehicle).await).await;
    assert!(html.contains("Auswertung") && html.contains("Haltedauer"));

    // anderer Haushalt sieht nichts
    let mut bob = Client::new(&app);
    bob.login("bob").await;
    assert_eq!(bob.get(&vehicle).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(bob.get(&costs).await.status(), StatusCode::NOT_FOUND);
}

// @test SPEC_CAR_MILEAGE_UI, SPEC_CAR_FUEL_PRICE_UI
#[tokio::test]
async fn vehicle_v11_mileage_and_fuel_price_workflow() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("v11-owner").await;
    let csrf = c.csrf().await;
    let res = c
        .post(
            "/vehicles/new",
            &[
                ("_csrf", &csrf),
                ("name", "v1.1 Auto"),
                ("drive_type", "combustion"),
                ("odometer", "20000"),
                ("start_at", "2025-01-01"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let vehicle = location(&res);
    let page = text(c.get(&vehicle).await).await;
    assert!(page.contains("name=\"odometer_at_purchase\" value=\"20000\""));
    let res = c
        .post(
            &format!("{vehicle}/acquisition"),
            &[
                ("_csrf", &csrf),
                ("purchased_on", "2025-01-01"),
                ("price", "30000"),
                ("depreciation_mode", "mileage"),
                ("odometer_at_purchase", "20000"),
                ("lifetime_km", "220000"),
                ("end_value", "0"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let page = text(c.get(&vehicle).await).await;
    assert!(page.contains("Aktueller Restwert"));
    assert!(page.contains("Zu wenig Daten"));
    assert!(page.contains("value=\"mileage\" checked"));
    let res = c
        .post(
            &format!("{vehicle}/profiles"),
            &[
                ("_csrf", &csrf),
                ("profile_valid_from", "2025-01-01"),
                ("profile_kwh", "6,5"),
                ("profile_energy", "fuel"),
                ("profile_loss", "0"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let costs = format!("{vehicle}/costs");
    let fuel = [
        ("_csrf", csrf.as_str()),
        ("booked_on", "2025-03-01"),
        ("category", "fuel"),
        ("quantity", "45,32"),
        ("unit_price", "1,789"),
    ];
    let res = c.post(&costs, &fuel).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let page = text(c.get(&format!("{costs}?year=2025")).await).await;
    assert!(
        page.contains("81,08 €") && page.contains("1,789 €/l"),
        "{page}"
    );
    let page = text(c.get(&vehicle).await).await;
    assert!(page.contains("1,789 €/l") && page.contains("data-chart=\"fuelPrice\""));
    assert!(page.contains("6,5 l"));
    let chart: serde_json::Value =
        serde_json::from_str(&text(c.get(&format!("{vehicle}/chart.json")).await).await).unwrap();
    assert_eq!(
        chart["series"]["fuel_price"]["data"],
        serde_json::json!([["2025-03-01", 1.789]])
    );
    let mut mismatch = fuel.to_vec();
    mismatch.push(("amount", "90"));
    let res = c.post(&costs, &mismatch).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("mehr als einen Cent"));
    let mut bob = Client::new(&app);
    bob.login("v11-other").await;
    assert_eq!(
        bob.get(&format!("{vehicle}/chart.json")).await.status(),
        StatusCode::NOT_FOUND
    );
}

// @test SPEC_AI_RECEIPT, SPEC_CAR_ENERGY_COST
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn receipt_becomes_charging_session() {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use zaehler_app::{model::ReceiptExtraction, port::RecognitionError, testing::StubExtractor};
    use zaehler_domain::assets::VehicleCostCategory;

    let (mut ports, store, _clock) = memory_ports();
    ports.extractor = Some(Arc::new(
        StubExtractor(Err(RecognitionError::Timeout)).with_receipt(Ok(ReceiptExtraction {
            date: NaiveDate::from_ymd_opt(2026, 1, 10),
            total_amount: Some(Decimal::new(1899, 2)),
            category: Some(VehicleCostCategory::Charging),
            quantity: Some(Decimal::new(32_125, 3)),
            unit_price: None,
            unit: Some("kWh".into()),
            vendor: Some("Ladepark Süd".into()),
            odometer: None,
            charging_kind: Some("dc".into()),
            confidence: Decimal::new(9, 1),
        })),
    ));
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
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let res = c
        .post(
            "/vehicles/new",
            &[
                ("_csrf", &csrf),
                ("name", "Stromer"),
                ("plate", ""),
                ("model", ""),
                ("drive_type", "electric"),
                ("home_charging_meter_id", ""),
                ("pv_share_pct", ""),
                ("odometer", "100"),
                ("start_at", "2025-01-01"),
            ],
        )
        .await;
    let vehicle = location(&res);
    let id = vehicle.trim_start_matches("/vehicles/").to_owned();
    assert!(
        text(c.get("/documents?kind=receipt").await)
            .await
            .contains(r#"value="receipt" checked"#)
    );

    // Upload als Beleg
    let boundary = "zaehler-test-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"_csrf\"\r\n\r\n{csrf}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"kind\"\r\n\r\nreceipt\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"beleg.pdf\"\r\nContent-Type: application/pdf\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(b"%PDF-1.7 Beleg");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let req = Request::post("/documents")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = c.send(req).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let doc = location(&res);
    let page = text(c.get(&doc).await).await;
    assert!(
        page.contains("Beleg prüfen") && page.contains("Ladepark Süd"),
        "{page}"
    );
    assert!(page.contains(r#"value="charging" checked"#));
    assert!(
        page.contains(&format!(r#"value="{id}" selected"#)),
        "einziges Fahrzeug"
    );
    assert!(page.contains(r#"name="charging_at" type="datetime-local" value="2026-01-10T12:00""#));

    let res = c
        .post(
            &format!("{doc}/receipt"),
            &[
                ("_csrf", &csrf),
                ("vehicle", &id),
                ("entry", "charging"),
                ("charging_at", "2026-01-10T12:00"),
                ("charging_energy", "32,125"),
                ("charging_amount", "18,99"),
                ("charging_kind", "dc"),
                ("charging_provider", "Ladepark Süd"),
                ("charging_odometer", ""),
                ("booked_on", "2026-01-10"),
                ("category", "other"),
                ("amount", "18,99"),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    assert_eq!(location(&res), vehicle);
    let html = text(c.get(&vehicle).await).await;
    assert!(
        html.contains("Beleg übernommen.") && html.contains("32,12 kWh"),
        "{html}"
    );
    assert!(
        html.contains(&format!("{doc}/view?back=")),
        "Beleg verlinkt"
    );
}

async fn post_fields(c: &mut Client, path: &str, fields: &[(&str, String)]) -> Response {
    let body: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
    c.post(path, &body).await
}

// @test SPEC_CAR_REPORT
#[tokio::test]
async fn vehicle_chart_json() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let res = c
        .post(
            "/vehicles/new",
            &[
                ("_csrf", &csrf),
                ("name", "Auto"),
                ("plate", ""),
                ("model", ""),
                ("drive_type", "combustion"),
                ("home_charging_meter_id", ""),
                ("pv_share_pct", ""),
                ("odometer", "100"),
                ("start_at", "2025-01-01"),
            ],
        )
        .await;
    let vehicle = location(&res);
    let html = text(c.get(&vehicle).await).await;
    assert!(html.contains(&format!("data-chart-url=\"{vehicle}/chart.json\"")));
    let json: serde_json::Value =
        serde_json::from_str(&text(c.get(&format!("{vehicle}/chart.json")).await).await).unwrap();
    assert_eq!(json["unit"], "€");
    assert_eq!(json["labels"][0], "2025");
    assert!(json["series"]["depreciation"]["data"].is_array());
}

// @test SPEC_PV_ASSIGNMENT, SPEC_PV_BULK_CAPTURE, SPEC_PV_KPI, SPEC_PV_CHARTS, SPEC_PV_ECONOMICS
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn pv_pages_bulk_capture_and_charts() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let grid =
        create_two_register_meter(&mut c, &csrf, "Hausanschluss", ["1.8.0", "2.8.0"], "export")
            .await
            .trim_start_matches("/meters/")
            .to_owned();
    let battery =
        create_two_register_meter(&mut c, &csrf, "Speicher", ["charge", "discharge"], "export")
            .await
            .trim_start_matches("/meters/")
            .to_owned();
    let house = create_meter(&mut c, &csrf, "Wohnung").await;
    let res = c
        .post(
            "/meters",
            &[
                ("_csrf", csrf.as_str()),
                ("name", "PV Dach"),
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
                ("register_0_code", "2.8.0"),
                ("register_0_direction", "export"),
                ("register_0_start", "0"),
            ],
        )
        .await;
    let generation = location(&res).trim_start_matches("/meters/").to_owned();
    let form = text(c.get("/pv/new").await).await;
    assert!(form.contains("PV Dach") && form.contains("share_12"));

    let gen_key = format!("gen_{generation}");
    let bat_key = format!("bat_{battery}");
    let mut fields = vec![
        ("_csrf", csrf.clone()),
        ("name", "Dach Süd".to_owned()),
        ("kind", "surplus".to_owned()),
        ("kwp", "9,84".to_owned()),
        ("commissioned_on", "2026-01-01".to_owned()),
        ("azimuth", "180".to_owned()),
        ("tilt", "30".to_owned()),
        ("inverter", String::new()),
        ("battery_kwh", "10".to_owned()),
        ("specific_yield", "950".to_owned()),
        ("degradation_pct", "0,5".to_owned()),
        ("alarm_threshold_pct", "50".to_owned()),
        ("grid", house.clone()),
        (gen_key.as_str(), "1".to_owned()),
        (bat_key.as_str(), "1".to_owned()),
    ];
    // Hausanschluss ohne Einspeiseregister: nur Steckersolargerät
    let res = post_fields(&mut c, "/pv/new", &fields).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(res).await.contains("nur ein Steckersolargerät"));
    fields.retain(|(k, _)| *k != "grid");
    fields.push(("grid", grid.clone()));
    let res = post_fields(&mut c, "/pv/new", &fields).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let system = location(&res);
    assert!(system.starts_with("/pv/"));
    let html = text(c.get(&system).await).await;
    assert!(html.contains("PV-Anlage angelegt.") && html.contains("Dach Süd"));
    assert!(html.contains("Wirtschaftlichkeit"));
    assert!(
        text(c.get("/").await).await.contains("href=\"/pv\""),
        "Navigation mit PV"
    );

    // Sammelerfassung mit gemeinsamem Zeitpunkt
    let page = text(c.get(&format!("/pv/grid/{grid}/readings")).await).await;
    let keys: Vec<String> = page
        .match_indices("name=\"v_")
        .map(|(i, _)| {
            let start = i + 6;
            page[start..start + page[start..].find('"').unwrap()].to_owned()
        })
        .collect();
    assert_eq!(keys.len(), 5, "Hausanschluss 2, Erzeugung 1, Speicher 2");
    let mut bulk = vec![
        ("_csrf", csrf.clone()),
        ("at", "2026-01-10T00:00".to_owned()),
        ("note", String::new()),
    ];
    for (k, v) in keys.iter().zip(["1200", "900", "1000", "1300", "770"]) {
        bulk.push((k.as_str(), v.to_owned()));
    }
    let res = post_fields(&mut c, &format!("/pv/grid/{grid}/readings"), &bulk).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    assert_eq!(location(&res), format!("/pv/grid/{grid}"));
    let html = text(c.get(&format!("/pv/grid/{grid}")).await).await;
    // EV = 1000 - 400 - (300 - 270) = 570, V = 200 + 570
    assert!(
        html.contains("570 kWh") && html.contains("770 kWh"),
        "{html}"
    );
    assert!(html.contains("bis zur letzten Ablesung"));
    let json: serde_json::Value =
        serde_json::from_str(&text(c.get(&format!("/pv/grid/{grid}/chart.json")).await).await)
            .unwrap();
    assert!(json["series"]["self_consumption"]["data"].is_array());
    for path in [
        format!("{system}/yield.json"),
        format!("{system}/balance.json"),
    ] {
        let res = c.get(&path).await;
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        let v: serde_json::Value = serde_json::from_str(&text(res).await).unwrap();
        assert!(v["labels"].is_array(), "{path}");
    }
    // Auswertung des Hausanschlusses auf den Gesamtverbrauch
    let detail = text(c.get(&format!("/meters/{grid}")).await).await;
    assert!(detail.contains("beziehen sich auf den Gesamtverbrauch"));

    // Kostenbuch der Anlage mit PV-Kategorien
    let res = post_fields(
        &mut c,
        &format!("{system}/costs"),
        &[
            ("_csrf", csrf.clone()),
            ("booked_on", "2026-01-01".to_owned()),
            ("category", "investment".to_owned()),
            ("amount", "14500".to_owned()),
            ("recurrence_months", String::new()),
            ("recurrence_until", String::new()),
            ("note", String::new()),
        ],
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let book = text(c.get(&format!("{system}/costs?year=2026")).await).await;
    assert!(book.contains("Investition") && book.contains("14500,00 €"));
    assert!(!book.contains("vollgetankt"), "keine Fahrzeugfelder");
    assert!(text(c.get(&system).await).await.contains("14500,00 €"));

    // Dashboard-Karte und fremder Haushalt
    let home = text(c.get("/").await).await;
    assert!(home.contains("PV: Hausanschluss"), "{home}");
    let mut bob = Client::new(&app);
    bob.login("bob").await;
    assert_eq!(bob.get(&system).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        bob.get(&format!("/pv/grid/{grid}")).await.status(),
        StatusCode::NOT_FOUND
    );
}

/// JSON-Anfrage mit optionalem Bearer-Token.
async fn api(
    c: &mut Client,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value, Response) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let res = c.send(req).await;
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = to_bytes(res.into_body(), 1 << 22).await.unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    let mut echo = Response::new(Body::empty());
    *echo.headers_mut() = headers;
    (status, json, echo)
}

/// Erzeugt über die Einstellungsseite ein Token und liest es aus der Antwort.
async fn create_token(c: &mut Client, csrf: &str, role: &str) -> String {
    let res = c
        .post(
            "/settings/tokens",
            &[
                ("_csrf", csrf),
                ("name", "Home Assistant"),
                ("role", role),
                ("expires_on", ""),
            ],
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let html = text(res).await;
    let start = html.find("value=\"zt_").expect("Token einmal angezeigt") + 7;
    html[start..start + 46].to_owned()
}

// @test SPEC_API_AUTH, SPEC_API_REST, SPEC_API_DAILY_READINGS
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn rest_api_with_bearer_tokens() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let meter = create_meter(&mut c, &csrf, "Strom API").await;
    assert!(
        text(c.get("/settings").await)
            .await
            .contains("/settings/tokens")
    );
    let token = create_token(&mut c, &csrf, "member").await;
    assert!(token.starts_with("zt_"));
    let page = text(c.get("/settings/tokens").await).await;
    assert!(!page.contains(&token), "Token erscheint nur einmal");
    assert!(page.contains(&token[..8]));

    // ohne Bearer, Cookie allein genügt nicht
    let (status, body, res) = api(&mut c, "GET", "/api/v1/meters", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["status"], 401);
    assert_eq!(res.headers()[header::WWW_AUTHENTICATE], "Bearer");
    assert_eq!(
        res.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    let (status, ..) = api(&mut c, "GET", "/api/v1/meters", Some("zt_falsch"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Lesen
    let (status, meters, _) = api(&mut c, "GET", "/api/v1/meters", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{meters}");
    assert_eq!(meters[0]["id"], meter.as_str());
    assert_eq!(meters[0]["registers"][0]["code"], "main");
    assert_eq!(meters[0]["registers"][0]["last_value"], "1000");
    let register = meters[0]["registers"][0]["id"].as_str().unwrap().to_owned();
    let (status, detail, _) = api(
        &mut c,
        "GET",
        &format!("/api/v1/meters/{meter}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["devices"].as_array().unwrap().len(), 1);

    // automatische Stände: zwei an einem Tag ergeben eine Ablesung
    let ingest = serde_json::json!({ "readings": [
        { "register_id": register, "read_at": "2026-01-10T06:00:00Z", "value": "1010.5" },
        { "register_id": register, "read_at": "2026-01-10T18:00:00Z", "value": "1012" },
        { "serial": "unbekannt", "register": "main", "read_at": "2026-01-10T18:00:00Z", "value": "1" },
        { "read_at": "2026-01-10T18:00:00Z", "value": "1" }
    ]});
    let (status, out, _) = api(
        &mut c,
        "POST",
        "/api/v1/readings",
        Some(&token),
        Some(ingest),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let results = out["results"].as_array().unwrap();
    assert_eq!(results[0]["status"], "stored");
    assert_eq!(results[1]["status"], "replaced");
    assert_eq!(results[1]["reading_id"], results[0]["reading_id"]);
    assert_eq!(results[2]["error"], "api.serial_unknown");
    assert_eq!(results[3]["error"], "api.target_invalid");

    // Ablesungen seitenweise
    let path = format!("/api/v1/meters/{meter}/readings?limit=1");
    let (status, first, _) = api(&mut c, "GET", &path, Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["items"][0]["value"], "1000");
    let cursor = first["next_cursor"].as_str().unwrap().to_owned();
    let (_, second, _) = api(
        &mut c,
        "GET",
        &format!("{path}&cursor={cursor}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(second["items"][0]["value"], "1012");
    assert_eq!(second["items"][0]["source"], "api");
    assert!(second["next_cursor"].is_null());
    let (status, ..) = api(
        &mut c,
        "GET",
        &format!("{path}&cursor=kaputt"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Erfassen wie das Formular: Warnung ohne confirm, dann gespeichert
    let record = |confirm: bool| {
        serde_json::json!({
            "read_at": "2026-01-12T08:00:00Z",
            "values": [{ "register_id": register, "value": "900" }],
            "confirm": confirm
        })
    };
    let readings = format!("/api/v1/meters/{meter}/readings");
    let (status, problem, _) =
        api(&mut c, "POST", &readings, Some(&token), Some(record(false))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem["warnings"][0]["kind"], "decreasing");
    let (status, saved, _) = api(&mut c, "POST", &readings, Some(&token), Some(record(true))).await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["reading_ids"].as_array().unwrap().len(), 1);
    let (status, problem, _) = api(
        &mut c,
        "POST",
        &readings,
        Some(&token),
        Some(serde_json::json!({ "values": "kein Array" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(problem["code"], "body.invalid");

    // Auswertung, Kosten, Verträge
    for p in [
        format!("/api/v1/meters/{meter}/analysis"),
        format!("/api/v1/meters/{meter}/costs"),
        "/api/v1/contracts".to_owned(),
    ] {
        let (status, body, _) = api(&mut c, "GET", &p, Some(&token), None).await;
        assert_eq!(status, StatusCode::OK, "{p}: {body}");
    }
    let (status, body, _) = api(
        &mut c,
        "GET",
        &format!("/api/v1/contracts/{}", uuid::Uuid::now_v7()),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["code"].is_null());

    // Viewer-Token darf nicht schreiben
    let viewer = create_token(&mut c, &csrf, "viewer").await;
    let (status, ..) = api(&mut c, "POST", &readings, Some(&viewer), Some(record(true))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // fremder Haushalt sieht die Messstelle nicht
    let mut bea = Client::new(&app);
    bea.login("bea").await;
    let bea_csrf = bea.csrf().await;
    let beas = create_token(&mut bea, &bea_csrf, "owner").await;
    let (status, ..) = api(
        &mut bea,
        "GET",
        &format!("/api/v1/meters/{meter}"),
        Some(&beas),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list, _) = api(&mut bea, "GET", "/api/v1/meters", Some(&beas), None).await;
    assert_eq!(list, serde_json::json!([]));

    // Widerruf des neuesten Tokens (Viewer); das andere gilt weiter
    let page = text(c.get("/settings/tokens").await).await;
    let start = page.find("/settings/tokens/").unwrap() + 17;
    let id = &page[start..start + 36];
    let res = c
        .post(
            &format!("/settings/tokens/{id}/revoke"),
            &[("_csrf", &csrf)],
        )
        .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let (status, ..) = api(&mut c, "GET", "/api/v1/meters", Some(&viewer), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, ..) = api(&mut c, "GET", "/api/v1/meters", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);

    // unbekannte Route
    let (status, body, _) = api(&mut c, "GET", "/api/v1/unbekannt", Some(&token), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "route.unknown");
}

/// Jeder Pfad der OpenAPI-Beschreibung ist im Router vorhanden und jede
/// Route des Routers beschrieben.
// @test SPEC_API_REST
#[tokio::test]
async fn openapi_matches_router() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    let (status, doc, _) = api(&mut c, "GET", "/api/v1/openapi.json", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(doc["openapi"].as_str().unwrap().starts_with("3.1"));
    assert!(doc["components"]["securitySchemes"]["bearer"].is_object());
    let paths = doc["paths"].as_object().unwrap();
    let mut documented: Vec<(String, String)> = Vec::new();
    for (path, item) in paths {
        for method in item.as_object().unwrap().keys() {
            documented.push((method.to_uppercase(), path.clone()));
        }
    }
    documented.sort();
    let expected: Vec<(String, String)> = [
        ("DELETE", "/api/v1/asset-costs/{id}"),
        ("DELETE", "/api/v1/contracts/{id}"),
        ("DELETE", "/api/v1/contracts/{id}/{kind}/{entry}"),
        ("DELETE", "/api/v1/documents/{id}"),
        ("DELETE", "/api/v1/meters/{id}"),
        ("DELETE", "/api/v1/pv-systems/{id}"),
        ("DELETE", "/api/v1/readings/{id}"),
        ("DELETE", "/api/v1/vehicles/{id}/acquisition"),
        ("DELETE", "/api/v1/vehicles/{id}/charging/{session}"),
        ("DELETE", "/api/v1/vehicles/{id}/profiles/{profile}"),
        ("GET", "/api/v1/asset-costs/{id}"),
        ("GET", "/api/v1/contracts"),
        ("GET", "/api/v1/contracts/{id}"),
        ("GET", "/api/v1/costs"),
        ("GET", "/api/v1/documents"),
        ("GET", "/api/v1/documents/{id}"),
        ("GET", "/api/v1/documents/{id}/file"),
        ("GET", "/api/v1/household"),
        ("GET", "/api/v1/meters"),
        ("GET", "/api/v1/meters/{id}"),
        ("GET", "/api/v1/meters/{id}/analysis"),
        ("GET", "/api/v1/meters/{id}/contract-templates"),
        ("GET", "/api/v1/meters/{id}/contracts"),
        ("GET", "/api/v1/meters/{id}/costs"),
        ("GET", "/api/v1/meters/{id}/history"),
        ("GET", "/api/v1/meters/{id}/readings"),
        ("GET", "/api/v1/pv"),
        ("GET", "/api/v1/pv-grid/{id}/analysis"),
        ("GET", "/api/v1/pv-grid/{id}/meters"),
        ("GET", "/api/v1/pv-systems"),
        ("GET", "/api/v1/pv-systems/{id}"),
        ("GET", "/api/v1/pv-systems/{id}/costs"),
        ("GET", "/api/v1/pv-systems/{id}/report"),
        ("GET", "/api/v1/readings/{id}"),
        ("GET", "/api/v1/readings/{id}/photo"),
        ("GET", "/api/v1/vehicles"),
        ("GET", "/api/v1/vehicles/{id}"),
        ("GET", "/api/v1/vehicles/{id}/charging"),
        ("GET", "/api/v1/vehicles/{id}/costs"),
        ("GET", "/api/v1/vehicles/{id}/report"),
        ("POST", "/api/v1/contracts/{id}/adjustments"),
        ("POST", "/api/v1/contracts/{id}/installments"),
        (
            "POST",
            "/api/v1/contracts/{id}/positions/{position}/price-components",
        ),
        ("POST", "/api/v1/contracts/{id}/settlements"),
        ("POST", "/api/v1/meters"),
        ("POST", "/api/v1/meters/{id}/archive"),
        ("POST", "/api/v1/meters/{id}/contracts"),
        ("POST", "/api/v1/meters/{id}/readings"),
        ("POST", "/api/v1/meters/{id}/restore"),
        ("POST", "/api/v1/meters/{id}/swap"),
        ("POST", "/api/v1/pv-grid/{id}/readings"),
        ("POST", "/api/v1/pv-systems"),
        ("POST", "/api/v1/pv-systems/{id}/costs"),
        ("POST", "/api/v1/readings"),
        ("POST", "/api/v1/vehicles"),
        ("POST", "/api/v1/vehicles/{id}/charging"),
        ("POST", "/api/v1/vehicles/{id}/costs"),
        ("POST", "/api/v1/vehicles/{id}/profiles"),
        ("PUT", "/api/v1/asset-costs/{id}"),
        ("PUT", "/api/v1/contracts/{id}"),
        ("PUT", "/api/v1/household"),
        ("PUT", "/api/v1/meters/{id}"),
        ("PUT", "/api/v1/pv-systems/{id}"),
        ("PUT", "/api/v1/readings/{id}"),
        ("PUT", "/api/v1/vehicles/{id}"),
        ("PUT", "/api/v1/vehicles/{id}/acquisition"),
    ]
    .iter()
    .map(|(m, p)| ((*m).to_owned(), (*p).to_owned()))
    .collect();
    assert_eq!(documented, expected);
    // jeder beschriebene Pfad trifft eine Route (401 statt 404 oder 405)
    for (method, path) in &documented {
        let concrete = ["{id}", "{entry}", "{position}", "{profile}", "{session}"]
            .iter()
            .fold(path.replace("{kind}", "installments"), |p, k| {
                p.replace(k, &uuid::Uuid::now_v7().to_string())
            });
        let (status, body, _) = api(&mut c, method, &concrete, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}: {body}");
    }
    // und die Router-Tabelle des Moduls stimmt mit der Beschreibung überein
    let (_, api_doc) = zaehler_web::api::router();
    assert_eq!(api_doc.paths.paths.len(), paths.len());
}

/// Fotos in der Sammelerfassung: Upload mit Rückkehr zum Formular, Vorschlag,
/// Zuordnung zur Messstelle und Speichern mit Belegfoto.
// @test SPEC_PV_BULK_CAPTURE
#[tokio::test]
async fn pv_bulk_capture_with_photos() {
    let app = capture_app(1024 * 1024);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let grid =
        create_two_register_meter(&mut c, &csrf, "Hausanschluss", ["1.8.0", "2.8.0"], "export")
            .await
            .trim_start_matches("/meters/")
            .to_owned();
    let pv = create_meter(&mut c, &csrf, "PV Dach").await;
    let gen_key = format!("gen_{pv}");
    let fields = vec![
        ("_csrf", csrf.clone()),
        ("name", "Dach".to_owned()),
        ("kind", "surplus".to_owned()),
        ("kwp", "5".to_owned()),
        ("commissioned_on", "2026-01-01".to_owned()),
        ("azimuth", String::new()),
        ("tilt", String::new()),
        ("inverter", String::new()),
        ("battery_kwh", String::new()),
        ("specific_yield", "950".to_owned()),
        ("degradation_pct", "0,5".to_owned()),
        ("alarm_threshold_pct", "50".to_owned()),
        ("grid", grid.clone()),
        (gen_key.as_str(), "1".to_owned()),
    ];
    let res = post_fields(&mut c, "/pv/new", &fields).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);

    // Upload mit Hausanschluss: zurück zur Sammelerfassung
    let (content_type, mut body) = multipart_photos(&csrf, &[&fake_jpeg(64)]);
    let grid_field = format!(
        "--zaehler-test-boundary\r\nContent-Disposition: form-data; name=\"grid\"\r\n\r\n{grid}\r\n"
    );
    body.splice(0..0, grid_field.into_bytes());
    let req = Request::post("/capture")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap();
    let res = c.send(req).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let form_path = format!("/pv/grid/{grid}/readings");
    assert_eq!(location(&res), form_path);
    let mut page = String::new();
    for _ in 0..500 {
        page = text(c.get(&form_path).await).await;
        if !page.contains("Wird erkannt") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let start = page.find("/capture/").expect("Foto in der Sammelerfassung") + 9;
    let draft = page[start..start + 36].to_owned();
    assert!(page.contains(&format!("name=\"draft_{draft}\"")), "{page}");
    assert!(page.contains("Erkannt: 1025"), "Vorschlag angezeigt");

    // Zuordnung zur PV-Messstelle, Wert aus dem Foto
    let reading_form = text(c.get(&format!("/meters/{pv}/readings/new")).await).await;
    let register = detail_register_id(&reading_form);
    let draft_key = format!("draft_{draft}");
    let value_key = format!("v_{pv}_{register}");
    let fields = vec![
        ("_csrf", csrf.clone()),
        ("at", "2026-01-10T12:00".to_owned()),
        (value_key.as_str(), "1025,0".to_owned()),
        ("note", String::new()),
        (draft_key.as_str(), pv.clone()),
    ];
    let res = post_fields(&mut c, &form_path, &fields).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "{}", text(res).await);
    let history = text(c.get(&format!("/meters/{pv}")).await).await;
    assert!(history.contains("/photo"), "Belegfoto an der Ablesung");
    let page = text(c.get(&form_path).await).await;
    assert!(!page.contains(&draft), "Entwurf abgeschlossen");
}

/// Breadcrumb-Leiste aus dem HTML einer Seite (ohne Tags).
fn breadcrumb(html: &str) -> Option<String> {
    let start = html.find("<nav class=\"breadcrumb\"")?;
    let end = start + html[start..].find("</nav>")?;
    let mut out = String::new();
    let mut in_tag = false;
    for ch in html[start..end].chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push('|');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    Some(
        out.split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" › "),
    )
}

// @test SPEC_UI_BREADCRUMB
#[tokio::test]
async fn pages_show_a_breadcrumb_with_the_overview_as_root() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let csrf = c.csrf().await;
    let id = create_meter(&mut c, &csrf, "Hausstrom").await;

    let home = text(c.get("/").await).await;
    assert_eq!(breadcrumb(&home), None, "Übersicht ohne Leiste");

    for (path, expected) in [
        ("/meters".to_owned(), "Übersicht › Messstellen"),
        (
            format!("/meters/{id}"),
            "Übersicht › Messstellen › Hausstrom",
        ),
        (
            format!("/meters/{id}/edit"),
            "Übersicht › Messstellen › Hausstrom › Messstelle bearbeiten",
        ),
        (
            format!("/meters/{id}/readings/new"),
            "Übersicht › Messstellen › Hausstrom › Stand erfassen",
        ),
        ("/settings".to_owned(), "Übersicht › Einstellungen"),
        (
            "/settings/members".to_owned(),
            "Übersicht › Einstellungen › Mitglieder",
        ),
        (
            "/vehicles/new".to_owned(),
            "Übersicht › Fahrzeuge › Fahrzeug anlegen",
        ),
        ("/capture".to_owned(), "Übersicht › Foto erfassen"),
    ] {
        let res = c.get(&path).await;
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        let html = text(res).await;
        assert_eq!(breadcrumb(&html).as_deref(), Some(expected), "{path}");
        assert_eq!(html.matches("class=\"breadcrumb\"").count(), 1, "{path}");
        assert!(html.contains("aria-current=\"page\""), "{path}");
    }
}

// Jede Seite mit dem Hauptlayout außer Übersicht und Fehlerseite hat eine
// Breadcrumb-Leiste.
// @test SPEC_UI_BREADCRUMB
#[test]
fn every_page_template_defines_a_breadcrumb() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/pages");
    let mut missing = Vec::new();
    let mut dirs = vec![root.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let html = std::fs::read_to_string(&path).unwrap();
            let exempt = ["dashboard.html", "error.html"].contains(&rel.as_str());
            if html.contains("layout/base.html") && !exempt && !html.contains("bc::here(") {
                missing.push(rel);
            }
        }
    }
    assert!(missing.is_empty(), "ohne Breadcrumb: {missing:?}");
}

// @test SPEC_UI_MENU
#[tokio::test]
async fn topbar_has_a_menu_toggle_for_small_screens() {
    let app = test_app(true);
    let mut c = Client::new(&app);
    c.login("anna").await;
    let html = text(c.get("/").await).await;
    assert!(html.contains(
        "<button type=\"button\" class=\"menu-toggle secondary\" aria-controls=\"topnav\" aria-expanded=\"false\">"
    ));
    let nav = html.find("id=\"topnav\"").unwrap();
    assert!(html[nav..].find("href=\"/settings\"").is_some());
    assert!(html[nav..].find("action=\"/auth/logout\"").is_some());
}

// @test SPEC_UI_PHOTO_VIEW, SPEC_SEC_TENANT_SCOPE
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn reading_photo_opens_on_a_view_page_with_a_safe_back_link() {
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
    let link = format!("/readings/{id}/photo/view?back=/meters/{meter}");
    assert!(html.contains(&link), "Link auf die Ansichtsseite");
    assert!(!html.contains("target=\"_blank\""));

    let page = text(anna.get(&link).await).await;
    assert!(page.contains(&format!(
        "<img class=\"photo-view-image\" src=\"/readings/{id}/photo\""
    )));
    assert!(page.contains(&format!("href=\"/meters/{meter}\" data-close")));
    assert!(
        page.contains("<dd>1001 kWh</dd>") || page.contains("<dd>1001,0 kWh</dd>"),
        "{page}"
    );
    assert_eq!(
        breadcrumb(&page).as_deref(),
        Some("Übersicht › Strom › Foto der Ablesung")
    );
    // fremde Ziele fallen auf die Messstelle zurück
    for bad in [
        "%2F%2Fevil.example",
        "https%3A%2F%2Fevil.example",
        "%2F%5Cevil.example",
    ] {
        let page = text(
            anna.get(&format!("/readings/{id}/photo/view?back={bad}"))
                .await,
        )
        .await;
        assert!(
            page.contains(&format!("href=\"/meters/{meter}\" data-close")),
            "{bad}"
        );
        assert!(!page.contains("evil.example"), "{bad}");
    }
    // Ablesung ohne Foto und fremder Haushalt
    let start = detail.readings[0].id;
    assert_eq!(
        anna.get(&format!("/readings/{start}/photo/view"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let mut bob = Client::new(&app);
    bob.login("bob").await;
    assert_eq!(
        bob.get(&format!("/readings/{id}/photo/view"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn setup_wizard_persists_and_resumes_an_owner_draft() {
    let app = test_app(true);
    let mut owner = Client::new(&app);
    owner.login("setup-owner").await;
    let started = owner.get("/setup").await;
    assert_eq!(started.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&started), "/setup/grid");

    let page = text(owner.get("/setup/grid").await).await;
    assert!(page.contains("setup-photo"));
    let csrf = owner.csrf().await;
    let saved = owner
        .post(
            "/setup/grid",
            &[
                ("_csrf", &csrf),
                ("enabled", "true"),
                ("name", "Grid meter"),
                ("action", "next"),
            ],
        )
        .await;
    assert_eq!(saved.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&saved), "/setup/heat_pump");

    let resumed = text(owner.get("/setup/grid").await).await;
    assert!(resumed.contains("value=\"Grid meter\""));
    let mut another_household = Client::new(&app);
    another_household.login("setup-other").await;
    let isolated = text(another_household.get("/setup/grid").await).await;
    assert!(!isolated.contains("value=\"Grid meter\""));
    let discarded = owner.post("/setup/discard", &[("_csrf", &csrf)]).await;
    assert_eq!(discarded.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&discarded), "/");
}

#[tokio::test]
async fn setup_photo_without_ai_is_kept_for_manual_entry() {
    let app = test_app(true);
    let mut owner = Client::new(&app);
    owner.login("setup-photo-owner").await;
    let _ = owner.get("/setup").await;
    let csrf = owner.csrf().await;
    let boundary = "SetupPhotoBoundary";
    let mut body = Vec::new();
    for (name, value) in [
        ("_csrf", csrf.as_str()),
        ("object_key", "grid"),
        ("return_step", "grid"),
    ] {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"meter.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&[0xff, 0xd8, 0xff]);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let uploaded = owner
        .send(
            Request::post("/setup/photo")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(uploaded.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&uploaded), "/setup/grid");
    let page = text(owner.get("/setup/grid").await).await;
    assert!(
        page.contains("Foto im Setup-Entwurf gespeichert.")
            || page.contains("Photo saved in the setup draft.")
    );
    assert!(page.contains("id=\"start_value\""));
}

#[tokio::test]
async fn setup_wizard_offers_selects_photo_slots_and_existing_meters() {
    let app = test_app(true);
    let mut owner = Client::new(&app);
    owner.login("setup-extend-owner").await;
    let _ = owner.get("/setup").await;
    let csrf = owner.csrf().await;

    let pv = text(owner.get("/setup/pv").await).await;
    assert!(pv.contains("<select id=\"kind\""));
    assert!(pv.contains("value=\"full_feed_in\""));
    assert!(pv.contains("id=\"setup-photo-pv_battery_charge\""));
    assert!(pv.contains("id=\"setup-photo-pv_battery_discharge\""));
    let vehicle = text(owner.get("/setup/vehicle").await).await;
    assert!(vehicle.contains("<select id=\"drive_type\""));
    assert!(vehicle.contains("id=\"acquisition_price\""));
    let contracts = text(owner.get("/setup/contracts").await).await;
    assert!(contracts.contains("<select id=\"direction\""));
    assert!(contracts.contains("<select id=\"second_meter\""));

    let first = owner
        .post(
            "/setup/grid",
            &[
                ("_csrf", &csrf),
                ("enabled", "true"),
                ("name", "Hausanschluss"),
                ("start_value", "100"),
                ("action", "next"),
            ],
        )
        .await;
    assert_eq!(first.status(), StatusCode::SEE_OTHER);
    let done = owner.post("/setup/complete", &[("_csrf", &csrf)]).await;
    assert_eq!(done.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&done), "/");

    let _ = owner.get("/setup").await;
    let grid = text(owner.get("/setup/grid").await).await;
    assert!(grid.contains("<select id=\"existing_grid\""));
    assert!(grid.contains("Hausanschluss"));
    let media = text(owner.get("/setup/media").await).await;
    assert!(media.contains("value=\"gas\""));
}

// @test SPEC_SETUP_FLOW
#[tokio::test]
async fn setup_fields_follow_the_choices_and_every_meter_can_be_existing() {
    let app = test_app(true);
    let mut owner = Client::new(&app);
    owner.login("setup-conditional-owner").await;
    let csrf = owner.csrf().await;
    let grid = create_meter(&mut owner, &csrf, "Strom").await;
    let _ = owner.get("/setup").await;

    let page = text(owner.get("/setup/grid").await).await;
    assert!(page.contains("data-show-if=\"enabled;existing_grid=;has_export\""));
    assert!(page.contains("data-show-form=\"setup-form\""));
    assert!(
        page.contains("class=\"required\""),
        "Pflichtfelder markiert"
    );

    let heat = text(owner.get("/setup/heat_pump").await).await;
    assert!(heat.contains("<select id=\"existing_heat_pump\""));
    assert!(heat.contains(&format!("value=\"{grid}\"")));
    assert!(heat.contains("data-show-if=\"enabled;existing_heat_pump=\""));
    let saved = owner
        .post(
            "/setup/heat_pump",
            &[
                ("_csrf", csrf.as_str()),
                ("enabled", "true"),
                ("existing_heat_pump", grid.as_str()),
                ("action", "next"),
            ],
        )
        .await;
    assert_eq!(saved.status(), StatusCode::SEE_OTHER);
    let heat = text(owner.get("/setup/heat_pump").await).await;
    assert!(heat.contains(&format!("value=\"{grid}\" selected")));

    let pv = text(owner.get("/setup/pv").await).await;
    assert!(pv.contains("<select id=\"battery_meter\""));
    assert!(pv.contains("data-show-if=\"enabled;battery_meter=new\""));
    let media = text(owner.get("/setup/media").await).await;
    assert!(media.contains("<select id=\"existing_media\""));

    let vehicle = text(owner.get("/setup/vehicle").await).await;
    assert!(vehicle.contains("<option value=\"wallbox\""));
    assert!(vehicle.contains("data-show-if=\"enabled;has_acquisition;acquisition_mode=mileage\""));
    // Verbrenner: Wallbox-Angaben ohne JavaScript werden ignoriert
    let saved = owner
        .post(
            "/setup/vehicle",
            &[
                ("_csrf", csrf.as_str()),
                ("enabled", "true"),
                ("name", "Golf"),
                ("model", "Golf"),
                ("drive_type", "combustion"),
                ("home_meter", "wallbox"),
                ("wallbox_name", "Wallbox"),
                ("action", "next"),
            ],
        )
        .await;
    assert_eq!(saved.status(), StatusCode::SEE_OTHER);
    let vehicle = text(owner.get("/setup/vehicle").await).await;
    assert!(!vehicle.contains("value=\"Wallbox\""));
}

#[tokio::test]
async fn setup_skip_reports_removed_photos() {
    let app = test_app(true);
    let mut owner = Client::new(&app);
    owner.login("setup-skip-owner").await;
    let _ = owner.get("/setup").await;
    let csrf = owner.csrf().await;
    let boundary = "SetupSkipBoundary";
    let mut body = Vec::new();
    for (name, value) in [
        ("_csrf", csrf.as_str()),
        ("object_key", "heat_pump"),
        ("return_step", "heat_pump"),
    ] {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"m.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&[0xff, 0xd8, 0xff]);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let uploaded = owner
        .send(
            Request::post("/setup/photo")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(uploaded.status(), StatusCode::SEE_OTHER);
    let skipped = owner
        .post("/setup/heat_pump", &[("_csrf", &csrf), ("action", "skip")])
        .await;
    assert_eq!(location(&skipped), "/setup/pv?removed=1");
    let page = text(owner.get("/setup/pv?removed=1").await).await;
    assert!(page.contains("role=\"status\""));
    let heat = text(owner.get("/setup/heat_pump").await).await;
    assert!(!heat.contains("Foto im Setup-Entwurf gespeichert."));
    assert!(!heat.contains("Photo saved in the setup draft."));
}
