//! Messstellen und Ablesungen über die REST-API schreiben und vollständig
//! lesen (SPEC_API_REST).

mod common;

use axum::http::StatusCode;
use common::{User, call, test_app};
use serde_json::{Value, json};

/// Body einer Strom-Messstelle mit allen Stammdaten.
fn meter_body(name: &str) -> Value {
    json!({
        "name": name,
        "kind": "electricity",
        "unit": "",
        "schedule": {"kind": "monthly", "value": 5},
        "max_daily_rate": "50",
        "billing_start": {"month": 3, "day": 1},
        "purpose": "heat_pump",
        "targets": {
            "annual_target": "3500",
            "tolerance_pct": 15,
            "season_weights": ["12", "11", "10", "8", "6", "5", "5", "5", "6", "8", "10", "14"]
        },
        "serial_number": "1abc 42",
        "rollover_digits": 6,
        "decimal_places": 1,
        "registers": [
            {"code": "1.8.0", "direction": "import", "start_value": "1000"},
            {"code": "2.8.0", "direction": "export", "start_value": "10"}
        ],
        "start_at": "2026-01-01T00:00:00Z"
    })
}

async fn create(app: &axum::Router, token: &str, name: &str) -> Value {
    let res = call(app, token, "POST", "/api/v1/meters", Some(meter_body(name))).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    res.json
}

fn register_id(detail: &Value, code: &str) -> String {
    detail["meter"]["registers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["code"] == code)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Erste Ablesung der Messstelle nach dem Startstand; liefert ihre ID.
async fn record(
    app: &axum::Router,
    token: &str,
    meter: &str,
    register: &str,
    value: &str,
) -> String {
    let res = call(
        app,
        token,
        "POST",
        &format!("/api/v1/meters/{meter}/readings"),
        Some(json!({
            "read_at": "2026-01-10T08:00:00Z",
            "values": [{"register_id": register, "value": value}]
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    res.json["reading_ids"][0].as_str().unwrap().to_owned()
}

async fn form_meter_register(app: &axum::Router, token: &str, meter: &str) -> String {
    let res = call(app, token, "GET", &format!("/api/v1/meters/{meter}"), None).await;
    assert_eq!(res.status, StatusCode::OK);
    register_id(&res.json, "main")
}

// @test SPEC_API_REST
#[tokio::test]
async fn meter_create_returns_full_master_data() {
    let app = test_app();
    let mut anna = User::login(&app, "api-meters-create").await;
    let owner = anna.token("owner").await;
    let created = create(&app, &owner, "Wärmepumpe").await;
    let id = created["meter"]["id"].as_str().unwrap();

    let res = call(&app, &owner, "GET", &format!("/api/v1/meters/{id}"), None).await;
    assert_eq!(res.status, StatusCode::OK);
    let m = &res.json["meter"];
    assert_eq!(m["name"], "Wärmepumpe");
    assert_eq!(m["kind"], "electricity");
    assert_eq!(m["unit"], "kWh");
    assert_eq!(m["purpose"], "heat_pump");
    assert_eq!(m["schedule"], json!({"kind": "monthly", "value": 5}));
    assert_eq!(m["max_daily_rate"], "50");
    assert_eq!(m["billing_start"], json!({"month": 3, "day": 1}));
    assert_eq!(m["targets"]["annual_target"], "3500");
    assert_eq!(m["targets"]["tolerance_pct"], 15);
    assert_eq!(m["targets"]["season_weights"].as_array().unwrap().len(), 12);
    assert_eq!(m["targets"]["season_weights"][0], "12");
    assert_eq!(m["serial_number"], "1ABC42");
    assert_eq!(m["parent_meter_id"], Value::Null);
    assert_eq!(m["archived_at"], Value::Null);
    assert!(m["created_at"].is_string());
    assert_eq!(m["registers"].as_array().unwrap().len(), 2);
    let export = m["registers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["code"] == "2.8.0")
        .unwrap();
    assert_eq!(export["direction"], "export");
    assert_eq!(export["last_value"], "10");
    let device = &res.json["devices"][0];
    assert_eq!(device["serial_number"], "1ABC42");
    assert_eq!(device["rollover_digits"], 6);
    assert_eq!(device["decimal_places"], 1);
    assert_eq!(device["installed_at"], "2026-01-01T00:00:00Z");
    assert_eq!(created, res.json);

    // Liste und OpenAPI enthalten beide Methoden desselben Pfads
    let list = call(&app, &owner, "GET", "/api/v1/meters", None).await;
    assert_eq!(list.json[0]["schedule"]["kind"], "monthly");
    let doc = call(&app, &owner, "GET", "/api/v1/openapi.json", None).await;
    let paths = &doc.json["paths"];
    assert!(paths["/api/v1/meters"]["get"].is_object());
    assert!(paths["/api/v1/meters"]["post"].is_object());
    assert!(paths["/api/v1/meters/{id}"]["get"].is_object());
    assert!(paths["/api/v1/meters/{id}"]["put"].is_object());
    assert!(paths["/api/v1/meters/{id}"]["delete"].is_object());
    assert!(paths["/api/v1/readings/{id}"]["put"].is_object());
}

// @test SPEC_API_REST
#[tokio::test]
async fn meter_update_archive_restore() {
    let app = test_app();
    let mut anna = User::login(&app, "api-meters-cycle").await;
    let owner = anna.token("owner").await;
    let created = create(&app, &owner, "Strom").await;
    let id = created["meter"]["id"].as_str().unwrap().to_owned();
    let import = register_id(&created, "1.8.0");
    let export = register_id(&created, "2.8.0");
    let path = format!("/api/v1/meters/{id}");

    // Ändern: Name, Plan entfernt, Ziel geändert, Register ergänzt
    let mut body = meter_body("Hausstrom");
    body["schedule"] = Value::Null;
    body["targets"] = json!({"annual_target": "4000"});
    body["registers"] = json!([
        {"id": import, "code": "1.8.0", "direction": "import"},
        {"id": export, "code": "2.8.0", "direction": "export"},
        {"code": "1.8.1", "direction": "import"}
    ]);
    let res = call(&app, &owner, "PUT", &path, Some(body.clone())).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["meter"]["name"], "Hausstrom");
    assert_eq!(res.json["meter"]["schedule"], Value::Null);
    assert_eq!(res.json["meter"]["targets"]["annual_target"], "4000");
    assert_eq!(res.json["meter"]["targets"]["season_weights"], Value::Null);
    assert_eq!(res.json["meter"]["registers"].as_array().unwrap().len(), 3);

    // vorhandene Register dürfen nicht fehlen
    body["registers"] = json!([{"code": "neu"}]);
    let res = call(&app, &owner, "PUT", &path, Some(body)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);

    // ungültige Art
    let mut bad = meter_body("Falsch");
    bad["kind"] = json!("plasma");
    let res = call(&app, &owner, "POST", "/api/v1/meters", Some(bad)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.json["errors"][0]["field"], "kind");

    let res = call(&app, &owner, "POST", &format!("{path}/archive"), None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = call(&app, &owner, "GET", &path, None).await;
    assert_eq!(res.json["meter"]["archived"], true);
    assert!(res.json["meter"]["archived_at"].is_string());
    let res = call(&app, &owner, "POST", &format!("{path}/restore"), None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = call(&app, &owner, "GET", &path, None).await;
    assert_eq!(res.json["meter"]["archived"], false);
}

// @test SPEC_API_REST
#[tokio::test]
async fn meter_swap_and_delete() {
    let app = test_app();
    let mut anna = User::login(&app, "api-meters-swap").await;
    let owner = anna.token("owner").await;
    let created = create(&app, &owner, "Strom").await;
    let id = created["meter"]["id"].as_str().unwrap().to_owned();
    let import = register_id(&created, "1.8.0");
    let export = register_id(&created, "2.8.0");
    let path = format!("/api/v1/meters/{id}");

    let res = call(
        &app,
        &owner,
        "POST",
        &format!("{path}/swap"),
        Some(json!({
            "at": "2026-01-12T10:00:00Z",
            "end_values": [
                {"register_id": import, "value": "1100"},
                {"register_id": export, "value": "12"}
            ],
            "start_values": [
                {"register_id": import, "value": "0"},
                {"register_id": export, "value": "0"}
            ],
            "serial_number": "NEU-1",
            "decimal_places": 2
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let device = res.json["device_id"].as_str().unwrap().to_owned();
    let res = call(&app, &owner, "GET", &path, None).await;
    let devices = res.json["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0]["removed_at"], "2026-01-12T10:00:00Z");
    assert_eq!(devices[1]["id"], device.as_str());
    assert_eq!(devices[1]["decimal_places"], 2);
    assert_eq!(res.json["meter"]["serial_number"], "NEU1");

    // Stände des Wechsels sind gegen Löschen gesperrt
    let readings = call(&app, &owner, "GET", &format!("{path}/readings"), None).await;
    let swap_reading = readings.json["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["device_id"] == device.as_str())
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let res = call(
        &app,
        &owner,
        "GET",
        &format!("/api/v1/readings/{swap_reading}"),
        None,
    )
    .await;
    assert_eq!(res.json["part_of_swap"], true);
    let res = call(
        &app,
        &owner,
        "DELETE",
        &format!("/api/v1/readings/{swap_reading}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);

    let res = call(&app, &owner, "DELETE", &path, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = call(&app, &owner, "GET", &path, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// @test SPEC_API_REST
#[tokio::test]
async fn reading_get_correct_history_delete() {
    let app = test_app();
    let mut anna = User::login(&app, "api-readings-cycle").await;
    let meter = anna.meter_via_form("Strom").await;
    let member = anna.token("member").await;
    let register = form_meter_register(&app, &member, &meter).await;
    let reading = record(&app, &member, &meter, &register, "1100").await;
    let path = format!("/api/v1/readings/{reading}");

    let res = call(&app, &member, "GET", &path, None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["reading"]["id"], reading.as_str());
    assert_eq!(res.json["reading"]["value"], "1100");
    assert_eq!(res.json["reading"]["source"], "api");
    assert_eq!(res.json["meter_id"], meter.as_str());
    assert_eq!(res.json["part_of_swap"], false);
    assert_eq!(res.json["has_photo"], false);

    let res = call(&app, &member, "GET", &format!("{path}/photo"), None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // fallender Stand nur mit Bestätigung
    let body = json!({"read_at": "2026-01-10T08:00:00Z", "value": "900", "note": "Tippfehler"});
    let res = call(&app, &member, "PUT", &path, Some(body.clone())).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.json["warnings"][0]["kind"], "decreasing");
    assert_eq!(res.json["warnings"][0]["register_id"], register.as_str());

    let body = json!({"read_at": "2026-01-11T09:00:00Z", "value": "1150", "note": "korrigiert"});
    let res = call(&app, &member, "PUT", &path, Some(body)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["reading"]["value"], "1150");
    assert_eq!(res.json["reading"]["read_at"], "2026-01-11T09:00:00Z");
    assert_eq!(res.json["reading"]["note"], "korrigiert");

    let history = format!("/api/v1/meters/{meter}/history");
    let res = call(&app, &member, "GET", &history, None).await;
    assert_eq!(res.status, StatusCode::OK);
    let revisions = res.json.as_array().unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0]["action"], "update");
    assert_eq!(revisions[0]["reading_id"], reading.as_str());
    assert_eq!(revisions[0]["old_value"], "1100");
    assert_eq!(revisions[0]["new_value"], "1150");
    assert_eq!(revisions[0]["new_note"], "korrigiert");
    assert_eq!(revisions[0]["changed_by_name"], "api-readings-cycle");

    let res = call(&app, &member, "DELETE", &path, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = call(&app, &member, "GET", &path, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = call(&app, &member, "GET", &history, None).await;
    let after_delete = res.json.as_array().unwrap();
    assert_eq!(after_delete.len(), 2);
    assert!(
        after_delete
            .iter()
            .any(|r| r["action"] == "delete" && r["new_value"].is_null())
    );
}

// @test SPEC_API_REST
#[tokio::test]
async fn viewer_cannot_write() {
    let app = test_app();
    let mut anna = User::login(&app, "api-meters-viewer").await;
    let owner = anna.token("owner").await;
    let viewer = anna.token("viewer").await;
    let created = create(&app, &owner, "Strom").await;
    let id = created["meter"]["id"].as_str().unwrap().to_owned();
    let register = register_id(&created, "1.8.0");
    let reading = record(&app, &owner, &id, &register, "1100").await;
    let swap = json!({
        "at": "2026-01-12T10:00:00Z",
        "end_values": [{"register_id": register, "value": "1200"}],
        "start_values": [{"register_id": register, "value": "0"}]
    });
    let correct = json!({"read_at": "2026-01-10T08:00:00Z", "value": "1101"});
    let writes = [
        ("POST", "/api/v1/meters".to_owned(), Some(meter_body("Neu"))),
        (
            "PUT",
            format!("/api/v1/meters/{id}"),
            Some(meter_body("Neu")),
        ),
        ("DELETE", format!("/api/v1/meters/{id}"), None),
        ("POST", format!("/api/v1/meters/{id}/archive"), None),
        ("POST", format!("/api/v1/meters/{id}/restore"), None),
        ("POST", format!("/api/v1/meters/{id}/swap"), Some(swap)),
        ("PUT", format!("/api/v1/readings/{reading}"), Some(correct)),
        ("DELETE", format!("/api/v1/readings/{reading}"), None),
    ];
    for (method, path, body) in writes {
        let res = call(&app, &viewer, method, &path, body).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{method} {path}");
    }
    // Lesen bleibt erlaubt
    for path in [
        format!("/api/v1/meters/{id}"),
        format!("/api/v1/meters/{id}/history"),
        format!("/api/v1/readings/{reading}"),
    ] {
        let res = call(&app, &viewer, "GET", &path, None).await;
        assert_eq!(res.status, StatusCode::OK, "{path}");
    }
    let res = call(
        &app,
        &owner,
        "GET",
        &format!("/api/v1/readings/{reading}"),
        None,
    )
    .await;
    assert_eq!(res.json["reading"]["value"], "1100");
}

// @test SPEC_API_REST
#[tokio::test]
async fn member_cannot_change_meters() {
    let app = test_app();
    let mut anna = User::login(&app, "api-meters-member").await;
    let owner = anna.token("owner").await;
    let member = anna.token("member").await;
    let created = create(&app, &owner, "Strom").await;
    let id = created["meter"]["id"].as_str().unwrap().to_owned();
    let register = register_id(&created, "1.8.0");
    let swap = json!({
        "at": "2026-01-12T10:00:00Z",
        "end_values": [
            {"register_id": register, "value": "1200"},
            {"register_id": register_id(&created, "2.8.0"), "value": "11"}
        ],
        "start_values": [{"register_id": register, "value": "0"}]
    });
    let writes = [
        ("POST", "/api/v1/meters".to_owned(), Some(meter_body("Neu"))),
        (
            "PUT",
            format!("/api/v1/meters/{id}"),
            Some(meter_body("Neu")),
        ),
        ("DELETE", format!("/api/v1/meters/{id}"), None),
        ("POST", format!("/api/v1/meters/{id}/archive"), None),
        ("POST", format!("/api/v1/meters/{id}/restore"), None),
        ("POST", format!("/api/v1/meters/{id}/swap"), Some(swap)),
    ];
    for (method, path, body) in writes {
        let res = call(&app, &member, method, &path, body).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{method} {path}");
    }
    // Ablesungen erfassen, korrigieren und löschen darf ein Mitglied
    let reading = record(&app, &member, &id, &register, "1100").await;
    let path = format!("/api/v1/readings/{reading}");
    let body = json!({"read_at": "2026-01-10T08:00:00Z", "value": "1120"});
    let res = call(&app, &member, "PUT", &path, Some(body)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let res = call(&app, &member, "DELETE", &path, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
}

// @test SPEC_API_REST
#[tokio::test]
async fn other_household_is_not_found() {
    let app = test_app();
    let mut anna = User::login(&app, "api-meters-anna").await;
    let mut ben = User::login(&app, "api-meters-ben").await;
    let anna_token = anna.token("owner").await;
    let ben_token = ben.token("owner").await;
    let created = create(&app, &anna_token, "Strom").await;
    let id = created["meter"]["id"].as_str().unwrap().to_owned();
    let register = register_id(&created, "1.8.0");
    let reading = record(&app, &anna_token, &id, &register, "1100").await;

    let requests = [
        ("GET", format!("/api/v1/meters/{id}"), None),
        (
            "PUT",
            format!("/api/v1/meters/{id}"),
            Some(meter_body("Fremd")),
        ),
        ("DELETE", format!("/api/v1/meters/{id}"), None),
        ("POST", format!("/api/v1/meters/{id}/archive"), None),
        ("POST", format!("/api/v1/meters/{id}/restore"), None),
        (
            "POST",
            format!("/api/v1/meters/{id}/swap"),
            Some(json!({"at": "2026-01-12T10:00:00Z", "end_values": [], "start_values": []})),
        ),
        ("GET", format!("/api/v1/meters/{id}/history"), None),
        ("GET", format!("/api/v1/readings/{reading}"), None),
        (
            "PUT",
            format!("/api/v1/readings/{reading}"),
            Some(json!({"read_at": "2026-01-10T08:00:00Z", "value": "1"})),
        ),
        ("DELETE", format!("/api/v1/readings/{reading}"), None),
        ("GET", format!("/api/v1/readings/{reading}/photo"), None),
    ];
    for (method, path, body) in requests {
        let res = call(&app, &ben_token, method, &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    let res = call(
        &app,
        &anna_token,
        "GET",
        &format!("/api/v1/readings/{reading}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
}
