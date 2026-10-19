//! REST-API Photovoltaik: Anlagen, Bericht, Kennzahlen und Sammelerfassung
//! (SPEC_API_REST).

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use common::{User, call, test_app};
use serde_json::{Value, json};
use tower::ServiceExt;

/// Browser-Sitzung desselben Nutzers wie [`User`], um Messstellen mit
/// beliebigen Registern über das Formular anzulegen.
struct Browser {
    app: Router,
    cookie: String,
}

impl Browser {
    async fn login(app: &Router, name: &str) -> Self {
        let res = app
            .clone()
            .oneshot(
                Request::get(format!("/auth/dev-login?name={name}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER, "dev-login");
        let cookie = res.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        Self {
            app: app.clone(),
            cookie,
        }
    }

    async fn send(&self, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
        let mut req = req;
        req.headers_mut()
            .insert(header::COOKIE, self.cookie.parse().unwrap());
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = to_bytes(res.into_body(), 1 << 24).await.unwrap();
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    async fn csrf(&self) -> String {
        let (_, _, html) = self
            .send(Request::get("/settings").body(Body::empty()).unwrap())
            .await;
        let start = html.find("name=\"_csrf\" value=\"").expect("CSRF-Feld") + 20;
        html[start..start + html[start..].find('"').unwrap()].to_owned()
    }

    /// Strom-Messstelle mit Registern `(code, direction, start)`, Startstand
    /// am 01.01.2026.
    async fn meter(&self, name: &str, registers: &[(&str, &str, &str)]) -> String {
        let csrf = self.csrf().await;
        let mut fields: Vec<(String, String)> = [
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
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        fields.push(("register_count".into(), registers.len().to_string()));
        for (i, (code, dir, start)) in registers.iter().enumerate() {
            fields.push((format!("register_{i}_id"), String::new()));
            fields.push((format!("register_{i}_code"), (*code).to_owned()));
            fields.push((format!("register_{i}_direction"), (*dir).to_owned()));
            fields.push((format!("register_{i}_start"), (*start).to_owned()));
        }
        let (status, headers, _) = self
            .send(
                Request::post("/meters")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(serde_urlencoded::to_string(&fields).unwrap()))
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::SEE_OTHER, "meter form");
        headers[header::LOCATION]
            .to_str()
            .unwrap()
            .trim_start_matches("/meters/")
            .to_owned()
    }
}

/// Haushalt mit Hausanschluss (Bezug + Einspeisung), Erzeugungszähler und
/// Speicher.
struct Plant {
    name: String,
    user: User,
    owner: String,
    grid: String,
    generation: String,
    battery: String,
}

impl Plant {
    async fn new(app: &Router, name: &str) -> Self {
        let mut user = User::login(app, name).await;
        let owner = user.token("owner").await;
        let b = Browser::login(app, name).await;
        let grid = b
            .meter(
                "Hausanschluss",
                &[("main", "import", "1000"), ("feed", "export", "0")],
            )
            .await;
        let generation = b.meter("PV", &[("gen", "export", "0")]).await;
        let battery = b
            .meter(
                "Speicher",
                &[("charge", "import", "0"), ("discharge", "export", "0")],
            )
            .await;
        Self {
            name: name.to_owned(),
            user,
            owner,
            grid,
            generation,
            battery,
        }
    }

    fn body(&self, name: &str) -> Value {
        json!({
            "name": name,
            "kind": "surplus",
            "kwp": "9.8",
            "commissioned_on": "2025-06-01",
            "azimuth": 180,
            "tilt": 30,
            "inverter": "WR 10",
            "battery_kwh": "10",
            "meters": [
                {"meter_id": self.grid, "role": "grid"},
                {"meter_id": self.generation, "role": "generation"},
                {"meter_id": self.battery, "role": "battery"},
            ],
        })
    }

    async fn create(&self, app: &Router) -> String {
        let res = call(
            app,
            &self.owner,
            "POST",
            "/api/v1/pv-systems",
            Some(self.body("Dach")),
        )
        .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
        res.json["id"].as_str().unwrap().to_owned()
    }

    async fn register(&self, app: &Router, meter: &str, code: &str) -> String {
        let res = call(
            app,
            &self.owner,
            "GET",
            &format!("/api/v1/meters/{meter}"),
            None,
        )
        .await;
        res.json["meter"]["registers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["code"] == code)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

// @test SPEC_API_REST
#[tokio::test]
async fn pv_system_crud_with_assignment() {
    let app = test_app();
    let p = Plant::new(&app, "pv-crud").await;
    let id = p.create(&app).await;

    let res = call(
        &app,
        &p.owner,
        "GET",
        &format!("/api/v1/pv-systems/{id}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    let s = &res.json;
    assert_eq!(s["name"], "Dach");
    assert_eq!(s["kind"], "surplus");
    assert_eq!(s["kwp"], "9.8");
    assert_eq!(s["commissioned_on"], "2025-06-01");
    assert_eq!(s["azimuth"], 180);
    assert_eq!(s["specific_yield"], "950");
    assert_eq!(s["degradation_pct"], "0.5");
    assert_eq!(s["alarm_threshold_pct"], 50);
    assert_eq!(s["month_shares"], Value::Null);
    assert_eq!(s["effective_month_shares"].as_array().unwrap().len(), 12);
    let meters = s["meters"].as_array().unwrap();
    assert_eq!(meters.len(), 3);
    assert!(
        meters
            .iter()
            .any(|m| m["meter_id"] == p.battery.as_str() && m["role"] == "battery")
    );

    let list = call(&app, &p.owner, "GET", "/api/v1/pv-systems", None).await;
    assert_eq!(list.json.as_array().unwrap().len(), 1);

    // ohne Speicher, eigene Monatsanteile und Ertragswerte
    let mut body = p.body("Dach Süd");
    body["meters"].as_array_mut().unwrap().pop();
    body["battery_kwh"] = Value::Null;
    body["specific_yield"] = json!("1000");
    body["degradation_pct"] = json!("0.3");
    body["alarm_threshold_pct"] = json!(40);
    body["month_shares"] = json!([
        "2", "4", "8", "10", "12", "14", "14", "12", "10", "7", "4", "3"
    ]);
    let res = call(
        &app,
        &p.owner,
        "PUT",
        &format!("/api/v1/pv-systems/{id}"),
        Some(body),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["name"], "Dach Süd");
    assert_eq!(res.json["specific_yield"], "1000");
    assert_eq!(res.json["alarm_threshold_pct"], 40);
    assert_eq!(res.json["month_shares"][6], "14");
    assert_eq!(res.json["meters"].as_array().unwrap().len(), 2);

    let res = call(
        &app,
        &p.owner,
        "DELETE",
        &format!("/api/v1/pv-systems/{id}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = call(
        &app,
        &p.owner,
        "GET",
        &format!("/api/v1/pv-systems/{id}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// @test SPEC_API_REST
#[tokio::test]
async fn pv_system_validation_and_conflicts() {
    let app = test_app();
    let p = Plant::new(&app, "pv-valid").await;

    let mut body = p.body("X");
    body["kind"] = json!("windrad");
    let res = call(&app, &p.owner, "POST", "/api/v1/pv-systems", Some(body)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.json["errors"][0]["field"], "kind");

    let mut body = p.body("X");
    body["meters"] = json!([{"meter_id": p.grid, "role": "grid"}]);
    let res = call(&app, &p.owner, "POST", "/api/v1/pv-systems", Some(body)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        res.json["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["code"] == "pv.generation_required")
    );

    let mut body = p.body("X");
    body["month_shares"] = json!(["50", "50"]);
    let res = call(&app, &p.owner, "POST", "/api/v1/pv-systems", Some(body)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.json["errors"][0]["field"], "month_shares");

    let mut body = p.body("X");
    body["meters"][1]["role"] = json!("sonne");
    let res = call(&app, &p.owner, "POST", "/api/v1/pv-systems", Some(body)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);

    let res = call(
        &app,
        &p.owner,
        "POST",
        "/api/v1/pv-systems",
        Some(json!({})),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);

    // Erzeugung und Speicher gehören höchstens einer Anlage
    p.create(&app).await;
    let res = call(
        &app,
        &p.owner,
        "POST",
        "/api/v1/pv-systems",
        Some(p.body("Zweite")),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.json["code"], "pv.meter_assigned");
}

// @test SPEC_API_REST
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn pv_bulk_readings_with_warnings() {
    let app = test_app();
    let p = Plant::new(&app, "pv-bulk").await;
    p.create(&app).await;
    let main = p.register(&app, &p.grid, "main").await;
    let feed = p.register(&app, &p.grid, "feed").await;
    let gen_reg = p.register(&app, &p.generation, "gen").await;

    let res = call(
        &app,
        &p.owner,
        "GET",
        &format!("/api/v1/pv-grid/{}/meters", p.grid),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    let roles: Vec<&str> = res
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["grid", "generation", "battery"]);
    assert_eq!(res.json[0]["meter"]["id"], p.grid.as_str());

    let path = format!("/api/v1/pv-grid/{}/readings", p.grid);
    let res = call(
        &app,
        &p.owner,
        "POST",
        &path,
        Some(json!({
            "read_at": "2026-01-10T10:00:00Z",
            "values": [
                {"meter_id": p.grid, "register_id": main, "value": "1050"},
                {"meter_id": p.grid, "register_id": feed, "value": "20"},
                {"meter_id": p.generation, "register_id": gen_reg, "value": "80"},
            ],
            "note": "Sammelerfassung",
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["saved"], 3);

    // fallender Stand: Warnung, nichts gespeichert
    let decreasing = json!({
        "read_at": "2026-01-12T10:00:00Z",
        "values": [
            {"meter_id": p.grid, "register_id": main, "value": "1040"},
            {"meter_id": p.generation, "register_id": gen_reg, "value": "90"},
        ],
    });
    let res = call(&app, &p.owner, "POST", &path, Some(decreasing.clone())).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.json["code"], "reading.warnings");
    assert_eq!(res.json["warnings"][0]["register_id"], main.as_str());
    assert_eq!(res.json["warnings"][0]["kind"], "decreasing");
    let readings = call(
        &app,
        &p.owner,
        "GET",
        &format!("/api/v1/meters/{}/readings", p.generation),
        None,
    )
    .await;
    let count = readings.json["items"].as_array().unwrap().len();
    let mut confirmed = decreasing;
    confirmed["confirm"] = json!(true);
    let res = call(&app, &p.owner, "POST", &path, Some(confirmed)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["saved"], 2);
    let readings = call(
        &app,
        &p.owner,
        "GET",
        &format!("/api/v1/meters/{}/readings", p.generation),
        None,
    )
    .await;
    assert_eq!(readings.json["items"].as_array().unwrap().len(), count + 1);
    let last = readings.json["items"].as_array().unwrap().last().unwrap();
    assert_eq!(last["source"], "api", "Quelle der API-Sammelerfassung");

    // Messstelle außerhalb des Hausanschlusses
    let other = p.user_meter(&app).await;
    let res = call(
        &app,
        &p.owner,
        "POST",
        &path,
        Some(json!({
            "read_at": "2026-01-13T10:00:00Z",
            "values": [{"meter_id": other.0, "register_id": other.1, "value": "1001"}],
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = call(
        &app,
        &p.owner,
        "POST",
        &path,
        Some(json!({"read_at": "2026-01-13T10:00:00Z", "values": []})),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
}

// @test SPEC_API_REST
#[tokio::test]
async fn pv_report_analysis_and_overview() {
    let app = test_app();
    let p = Plant::new(&app, "pv-report").await;
    let id = p.create(&app).await;
    let gen_reg = p.register(&app, &p.generation, "gen").await;
    let res = call(
        &app,
        &p.owner,
        "POST",
        &format!("/api/v1/pv-grid/{}/readings", p.grid),
        Some(json!({
            "read_at": "2026-01-10T10:00:00Z",
            "values": [{"meter_id": p.generation, "register_id": gen_reg, "value": "80"}],
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let other = p.user_meter(&app).await;

    let res = call(
        &app,
        &p.owner,
        "GET",
        &format!("/api/v1/pv-grid/{}/analysis", p.grid),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let a = &res.json;
    assert_eq!(a["grid_meter_id"], p.grid.as_str());
    assert_eq!(a["systems"][0]["id"], id.as_str());
    assert_eq!(a["setup"]["has_generation"], true);
    assert_eq!(a["setup"]["has_export_register"], true);
    assert_eq!(a["setup"]["has_battery"], true);
    assert_eq!(a["months"][0]["start"], "2026-01-01");
    // laufender Monat ist noch nicht vollständig gemessen
    assert!(a["months"][0].get("generation").is_some());
    assert!(a["years"].is_array());
    assert!(a["yields"].is_array());

    let res = call(
        &app,
        &p.owner,
        "GET",
        &format!("/api/v1/pv-systems/{id}/report"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let r = &res.json;
    assert_eq!(r["system"]["id"], id.as_str());
    assert_eq!(r["grid_name"], "Hausanschluss");
    // Inbetriebnahme Juni 2025 bis Januar 2026
    assert_eq!(r["yields"][0]["month"], "2025-06");
    assert!(r["yields"][0]["expected"].is_string());
    assert!(r["years"].is_array());
    assert!(r["months"].is_array());
    assert!(r["investments"].is_string());

    let res = call(&app, &p.owner, "GET", "/api/v1/pv", None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json[0]["grid_meter_id"], p.grid.as_str());
    assert_eq!(res.json[0]["name"], "Hausanschluss");

    // Messstelle ohne Anlage ist kein Hausanschluss
    let res = call(
        &app,
        &p.owner,
        "GET",
        &format!("/api/v1/pv-grid/{}/analysis", other.0),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

impl Plant {
    /// Weitere Messstelle ohne Anlage: ID und Register `main`.
    async fn user_meter(&self, app: &Router) -> (String, String) {
        let b = Browser::login(app, &self.name).await;
        let m = b.meter("Garage", &[("main", "import", "1000")]).await;
        let r = self.register(app, &m, "main").await;
        (m, r)
    }
}

// @test SPEC_API_REST
#[tokio::test]
async fn pv_roles_viewer_member_owner() {
    let app = test_app();
    let mut p = Plant::new(&app, "pv-roles").await;
    let id = p.create(&app).await;
    let viewer = p.user.token("viewer").await;
    let member = p.user.token("member").await;
    let main = p.register(&app, &p.grid, "main").await;
    let sys = format!("/api/v1/pv-systems/{id}");
    let bulk = format!("/api/v1/pv-grid/{}/readings", p.grid);
    let reading = json!({
        "read_at": "2026-01-10T10:00:00Z",
        "values": [{"meter_id": p.grid, "register_id": main, "value": "1010"}],
    });

    for token in [&viewer, &member] {
        for path in [
            "/api/v1/pv".to_owned(),
            "/api/v1/pv-systems".to_owned(),
            sys.clone(),
            format!("{sys}/report"),
            format!("/api/v1/pv-grid/{}/analysis", p.grid),
            format!("/api/v1/pv-grid/{}/meters", p.grid),
        ] {
            let res = call(&app, token, "GET", &path, None).await;
            assert_eq!(res.status, StatusCode::OK, "GET {path}");
        }
        let res = call(&app, token, "POST", "/api/v1/pv-systems", Some(p.body("N"))).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
        let res = call(&app, token, "PUT", &sys, Some(p.body("N"))).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
        let res = call(&app, token, "DELETE", &sys, None).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
    }

    let res = call(&app, &viewer, "POST", &bulk, Some(reading.clone())).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = call(&app, &member, "POST", &bulk, Some(reading)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
}

// @test SPEC_API_REST
#[tokio::test]
async fn pv_foreign_household_is_not_found() {
    let app = test_app();
    let p = Plant::new(&app, "pv-own").await;
    let id = p.create(&app).await;
    let main = p.register(&app, &p.grid, "main").await;
    let mut stranger = User::login(&app, "pv-stranger").await;
    let foreign = stranger.token("owner").await;
    let sys = format!("/api/v1/pv-systems/{id}");

    for path in [
        sys.clone(),
        format!("{sys}/report"),
        format!("/api/v1/pv-grid/{}/analysis", p.grid),
        format!("/api/v1/pv-grid/{}/meters", p.grid),
    ] {
        let res = call(&app, &foreign, "GET", &path, None).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "GET {path}");
    }
    let res = call(&app, &foreign, "PUT", &sys, Some(p.body("N"))).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = call(&app, &foreign, "DELETE", &sys, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = call(
        &app,
        &foreign,
        "POST",
        &format!("/api/v1/pv-grid/{}/readings", p.grid),
        Some(json!({
            "read_at": "2026-01-10T10:00:00Z",
            "values": [{"meter_id": p.grid, "register_id": main, "value": "1010"}],
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    // fremde Messstellen in der Zuordnung
    let res = call(
        &app,
        &foreign,
        "POST",
        "/api/v1/pv-systems",
        Some(p.body("N")),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let list = call(&app, &foreign, "GET", "/api/v1/pv-systems", None).await;
    assert_eq!(list.json, json!([]));
    let cards = call(&app, &foreign, "GET", "/api/v1/pv", None).await;
    assert_eq!(cards.json, json!([]));
}
