//! REST-API für Fahrzeuge und das Kostenbuch von Fahrzeugen und PV-Anlagen.

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{User, call, test_app};

/// Browser-Sitzung desselben Nutzers für Formulare, die die API nicht
/// abdeckt (PV-Anlage anlegen).
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
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
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

    async fn csrf(&self) -> String {
        let res = self
            .app
            .clone()
            .oneshot(
                Request::get("/settings")
                    .header(header::COOKIE, &self.cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let html =
            String::from_utf8(to_bytes(res.into_body(), 1 << 24).await.unwrap().to_vec()).unwrap();
        let start = html.find("name=\"_csrf\" value=\"").expect("CSRF-Feld") + 20;
        html[start..start + html[start..].find('"').unwrap()].to_owned()
    }

    /// Steckersolargerät mit Hausanschluss `grid` und Erzeugung `generation`.
    async fn pv_system(&self, grid: &str, generation: &str) -> String {
        let csrf = self.csrf().await;
        let gen_key = format!("gen_{generation}");
        let body = serde_urlencoded::to_string([
            ("_csrf", csrf.as_str()),
            ("name", "Balkon"),
            ("kind", "plug_in"),
            ("kwp", "0,8"),
            ("commissioned_on", "2026-01-01"),
            ("azimuth", "180"),
            ("tilt", "30"),
            ("inverter", ""),
            ("battery_kwh", ""),
            ("specific_yield", ""),
            ("degradation_pct", ""),
            ("alarm_threshold_pct", "50"),
            ("grid", grid),
            (gen_key.as_str(), "1"),
        ])
        .unwrap();
        let res = self
            .app
            .clone()
            .oneshot(
                Request::post("/pv/new")
                    .header(header::COOKIE, &self.cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let location = res
            .headers()
            .get(header::LOCATION)
            .map(|l| l.to_str().unwrap().to_owned());
        let text =
            String::from_utf8(to_bytes(res.into_body(), 1 << 24).await.unwrap().to_vec()).unwrap();
        assert_eq!(status, StatusCode::SEE_OTHER, "{text}");
        location.unwrap().trim_start_matches("/pv/").to_owned()
    }
}

fn vehicle_body(drive: &str) -> Value {
    json!({
        "name": "Familienauto",
        "plate": "M-AB 123",
        "model": "Kombi",
        "drive_type": drive,
        "odometer": "10000",
        "start_at": "2026-01-01T00:00:00Z",
    })
}

async fn create_vehicle(app: &Router, token: &str, drive: &str) -> String {
    let res = call(
        app,
        token,
        "POST",
        "/api/v1/vehicles",
        Some(vehicle_body(drive)),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    res.json["meter"]["id"].as_str().unwrap().to_owned()
}

fn field_codes(v: &Value) -> Vec<(String, String)> {
    v["errors"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| {
                    (
                        e["field"].as_str().unwrap().to_owned(),
                        e["code"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

// @test SPEC_API_REST
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn vehicle_lifecycle_with_profiles_acquisition_charging_and_report() {
    let app = test_app();
    let mut anna = User::login(&app, "anna").await;
    let owner = anna.token("owner").await;
    let home = anna.meter_via_form("Strom").await;

    let res = call(
        &app,
        &owner,
        "POST",
        "/api/v1/vehicles",
        Some(vehicle_body("rocket")),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        field_codes(&res.json),
        [("drive_type".into(), "form.kind_invalid".into())]
    );

    let mut body = vehicle_body("plugin_hybrid");
    body["home_charging_meter_id"] = json!(home);
    body["pv_share_pct"] = json!("20");
    let res = call(&app, &owner, "POST", "/api/v1/vehicles", Some(body)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let id = res.json["meter"]["id"].as_str().unwrap().to_owned();
    assert_eq!(res.json["meter"]["kind"], "vehicle");
    assert_eq!(res.json["meter"]["unit"], "km");
    assert_eq!(res.json["data"]["drive_type"], "plugin_hybrid");
    assert_eq!(res.json["data"]["home_charging_meter_id"], json!(home));
    assert_eq!(res.json["data"]["pv_share_pct"], "20");
    assert_eq!(res.json["initial_odometer"], "10000");

    let res = call(&app, &owner, "GET", "/api/v1/vehicles", None).await;
    assert_eq!(res.status, StatusCode::OK);
    let list = res.json.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], json!(id));
    assert_eq!(list[0]["name"], "Familienauto");
    assert_eq!(list[0]["data"]["plate"], "M-AB 123");

    // Stammdaten ändern
    let res = call(
        &app,
        &owner,
        "PUT",
        &format!("/api/v1/vehicles/{id}"),
        Some(json!({"plate": "M-CD 9", "model": "Kombi", "drive_type": "plugin_hybrid"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["data"]["plate"], "M-CD 9");
    assert_eq!(res.json["data"]["home_charging_meter_id"], Value::Null);

    // Verbrauchsprofile
    let profiles = format!("/api/v1/vehicles/{id}/profiles");
    let res = call(
        &app,
        &owner,
        "POST",
        &profiles,
        Some(json!({
            "valid_from": "2026-01-01",
            "kwh_per_100km": "18.5",
            "charging_loss_pct": "10",
            "electric_share_pct": "60",
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["energy"], "electric");
    assert_eq!(res.json["kwh_per_100km"], "18.5");
    let profile = res.json["id"].as_str().unwrap().to_owned();
    let res = call(
        &app,
        &owner,
        "POST",
        &profiles,
        Some(json!({"valid_from": "2026-01-02", "energy": "fuel", "kwh_per_100km": "6.2"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["charging_loss_pct"], "0");
    let res = call(
        &app,
        &owner,
        "POST",
        &profiles,
        Some(json!({"valid_from": "2026-01-02", "energy": "fuel", "kwh_per_100km": "6"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        field_codes(&res.json),
        [("profile_valid_from".into(), "vehicle.profile_exists".into())]
    );
    let res = call(
        &app,
        &owner,
        "POST",
        &profiles,
        Some(json!({"valid_from": "2026-02-01", "energy": "gas", "kwh_per_100km": "6"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let res = call(
        &app,
        &owner,
        "DELETE",
        &format!("{profiles}/{profile}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);

    // Anschaffung
    let acquisition = format!("/api/v1/vehicles/{id}/acquisition");
    let res = call(
        &app,
        &owner,
        "PUT",
        &acquisition,
        Some(json!({"purchased_on": "2026-01-01", "price": "30000", "residual_value": "12000"})),
    )
    .await;
    // Restwert nur zusammen mit dem Halteende
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let fields: Vec<String> = field_codes(&res.json).into_iter().map(|f| f.0).collect();
    assert!(fields.contains(&"planned_end".to_owned()), "{fields:?}");
    let res = call(
        &app,
        &owner,
        "PUT",
        &acquisition,
        Some(json!({
            "purchased_on": "2026-01-01",
            "price": "30000",
            "mode": "time",
            "first_registration": "2024-06-01",
            "residual_value": "12000",
            "planned_end": "2032-01-01",
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["mode"], "time");
    assert_eq!(res.json["first_registration"], "2024-06-01");

    // Ladevorgänge; ein kleinerer Kilometerstand braucht die Bestätigung
    let charging = format!("/api/v1/vehicles/{id}/charging");
    let mut session = json!({
        "charged_at": "2026-01-10T10:00:00Z",
        "energy": "30.5",
        "amount": "15.25",
        "odometer": "9000",
        "provider": "Stadtwerke",
        "kind": "dc",
        "odometer_as_reading": true,
    });
    let res = call(&app, &owner, "POST", &charging, Some(session.clone())).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.json);
    assert_eq!(res.json["code"], "reading.warnings", "{}", res.json);
    assert_eq!(res.json["warnings"][0]["kind"], "decreasing");
    session["confirm"] = json!(true);
    let res = call(&app, &owner, "POST", &charging, Some(session)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["kind"], "dc");
    assert_eq!(res.json["source"], "form");
    assert_eq!(res.json["energy"], "30.5");
    let first = res.json["id"].as_str().unwrap().to_owned();
    let res = call(
        &app,
        &owner,
        "POST",
        &charging,
        Some(json!({"charged_at": "2026-02-10T10:00:00Z", "energy": "20", "amount": "0"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["kind"], "ac");
    let res = call(
        &app,
        &owner,
        "POST",
        &charging,
        Some(json!({
            "charged_at": "2026-02-10T10:00:00Z",
            "energy": "20",
            "amount": "0",
            "kind": "x",
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let res = call(&app, &owner, "GET", &charging, None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json.as_array().unwrap().len(), 2);
    let res = call(&app, &owner, "DELETE", &format!("{charging}/{first}"), None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);

    // Detail mit allem
    let res = call(&app, &owner, "GET", &format!("/api/v1/vehicles/{id}"), None).await;
    assert_eq!(res.status, StatusCode::OK);
    let d = &res.json;
    assert_eq!(d["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(d["profiles"][0]["energy"], "fuel");
    assert_eq!(d["acquisition"]["price"], "30000");
    assert_eq!(d["charging"].as_array().unwrap().len(), 1);
    assert_eq!(d["meter"]["registers"][0]["code"], "km");

    // Antriebsart nach dem ersten Ladevorgang gesperrt
    let res = call(
        &app,
        &owner,
        "PUT",
        &format!("/api/v1/vehicles/{id}"),
        Some(json!({"drive_type": "electric"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.json["code"], "vehicle.drive_type_locked");

    // Auswertung
    let res = call(
        &app,
        &owner,
        "GET",
        &format!("/api/v1/vehicles/{id}/report"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let r = &res.json;
    assert!(r["today"].is_string());
    assert!(!r["years"].as_array().unwrap().is_empty());
    assert!(r["years"][0]["cost"]["fixed"].is_string());
    assert_eq!(r["residual"]["mode"], "time");
    assert!(r["fuel_price"].is_object(), "Plug-in-Hybrid tankt");

    let res = call(&app, &owner, "DELETE", &acquisition, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = call(&app, &owner, "GET", &format!("/api/v1/vehicles/{id}"), None).await;
    assert_eq!(res.json["acquisition"], Value::Null);

    // Kein Fahrzeug: Strom-Messstelle
    let res = call(
        &app,
        &owner,
        "GET",
        &format!("/api/v1/vehicles/{home}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// @test SPEC_API_REST
#[tokio::test]
async fn combustion_vehicle_rejects_charging() {
    let app = test_app();
    let mut anna = User::login(&app, "anna").await;
    let owner = anna.token("owner").await;
    let id = create_vehicle(&app, &owner, "combustion").await;
    let res = call(
        &app,
        &owner,
        "POST",
        &format!("/api/v1/vehicles/{id}/charging"),
        Some(json!({"charged_at": "2026-02-10T10:00:00Z", "energy": "20", "amount": "5"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        field_codes(&res.json),
        [("drive_type".into(), "vehicle.no_charging".into())]
    );
}

// @test SPEC_API_REST
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn vehicle_cost_book_crud() {
    let app = test_app();
    let mut anna = User::login(&app, "anna").await;
    let owner = anna.token("owner").await;
    let id = create_vehicle(&app, &owner, "combustion").await;
    let costs = format!("/api/v1/vehicles/{id}/costs");

    // Kraftstoff: Literpreis aus Betrag und Menge
    let res = call(
        &app,
        &owner,
        "POST",
        &costs,
        Some(json!({
            "booked_on": "2026-03-01",
            "category": "fuel",
            "amount": "75.00",
            "quantity": "50",
            "odometer": "10500",
            "full_tank": true,
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let fuel = res.json["id"].as_str().unwrap().to_owned();
    assert_eq!(res.json["asset_type"], "vehicle");
    assert_eq!(res.json["asset_id"], json!(id));
    assert_eq!(res.json["unit_price"], "1.50");
    assert_eq!(res.json["full_tank"], true);

    // Wiederkehrend vierteljährlich
    let res = call(
        &app,
        &owner,
        "POST",
        &costs,
        Some(json!({
            "booked_on": "2026-01-15",
            "category": "tax",
            "amount": "50",
            "recurrence": {"every_months": 3},
            "note": "Kfz-Steuer",
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let tax = res.json["id"].as_str().unwrap().to_owned();
    assert_eq!(res.json["recurrence"]["every_months"], 3);

    // Ungültig: Laden beim Verbrenner, unbekannte und PV-Kategorie
    for category in ["charging", "nope", "meter_rent"] {
        let res = call(
            &app,
            &owner,
            "POST",
            &costs,
            Some(json!({"booked_on": "2026-03-01", "category": category, "amount": "1"})),
        )
        .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{category}");
        assert_eq!(field_codes(&res.json)[0].0, "category");
    }

    // Kilometerstand als Ablesung: kleiner als der Startstand → Warnung
    let mut entry = json!({
        "booked_on": "2026-01-10",
        "category": "workshop",
        "amount": "300",
        "odometer": "9000",
        "odometer_as_reading": true,
    });
    let res = call(&app, &owner, "POST", &costs, Some(entry.clone())).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.json["code"], "reading.warnings");
    entry["confirm"] = json!(true);
    let res = call(&app, &owner, "POST", &costs, Some(entry)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);

    let res = call(&app, &owner, "GET", &format!("{costs}?year=2026"), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let book = &res.json;
    assert_eq!(book["year"], 2026);
    assert_eq!(book["entries"].as_array().unwrap().len(), 3);
    let tax_entry = book["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == json!(tax))
        .unwrap();
    assert_eq!(tax_entry["year_amount"], "200");
    assert_eq!(tax_entry["note"], "Kfz-Steuer");
    assert_eq!(book["total"], "575.00");
    let res = call(&app, &owner, "GET", &format!("{costs}?year=2025"), None).await;
    assert_eq!(res.json["entries"].as_array().unwrap().len(), 0);
    let res = call(&app, &owner, "GET", &costs, None).await;
    assert_eq!(res.status, StatusCode::OK);
    let res = call(&app, &owner, "GET", &format!("{costs}?year=abc"), None).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);

    // Einzeln lesen, ändern, löschen
    let item = format!("/api/v1/asset-costs/{fuel}");
    let res = call(&app, &owner, "GET", &item, None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["category"], "fuel");
    let res = call(
        &app,
        &owner,
        "PUT",
        &item,
        Some(json!({
            "booked_on": "2026-03-02",
            "category": "fuel",
            "quantity": "40",
            "unit_price": "1.8",
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["amount"], "72.0");
    assert_eq!(res.json["booked_on"], "2026-03-02");
    assert_eq!(res.json["full_tank"], false);
    let res = call(
        &app,
        &owner,
        "PUT",
        &item,
        Some(json!({"booked_on": "2026-03-02", "category": "investment", "amount": "1"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let res = call(&app, &owner, "DELETE", &item, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = call(&app, &owner, "GET", &item, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = call(&app, &owner, "DELETE", &item, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// @test SPEC_API_REST
#[tokio::test]
async fn pv_cost_book_crud() {
    let app = test_app();
    let mut anna = User::login(&app, "anna").await;
    let owner = anna.token("owner").await;
    let viewer = anna.token("viewer").await;
    let grid = anna.meter_via_form("Hausanschluss").await;
    let generation = anna.meter_via_form("Balkon PV").await;
    let pv = Browser::login(&app, "anna")
        .await
        .pv_system(&grid, &generation)
        .await;
    let costs = format!("/api/v1/pv-systems/{pv}/costs");

    let res = call(
        &app,
        &owner,
        "POST",
        &costs,
        Some(json!({"booked_on": "2026-01-01", "category": "investment", "amount": "600"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["asset_type"], "pv_system");
    assert_eq!(res.json["asset_id"], json!(pv));
    let entry = res.json["id"].as_str().unwrap().to_owned();

    // Fahrzeugkategorie, Kilometerstand und Menge nur beim Fahrzeug
    let res = call(
        &app,
        &owner,
        "POST",
        &costs,
        Some(json!({"booked_on": "2026-01-01", "category": "fuel", "amount": "1"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let res = call(
        &app,
        &owner,
        "POST",
        &costs,
        Some(json!({
            "booked_on": "2026-01-01",
            "category": "other",
            "amount": "1",
            "odometer": "5",
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        field_codes(&res.json),
        [("odometer".into(), "asset.odometer_vehicle_only".into())]
    );

    let res = call(&app, &viewer, "GET", &format!("{costs}?year=2026"), None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["total"], "600");
    assert_eq!(res.json["totals"][0]["category"], "investment");

    let item = format!("/api/v1/asset-costs/{entry}");
    let res = call(
        &app,
        &owner,
        "PUT",
        &item,
        Some(json!({"booked_on": "2026-01-01", "category": "maintenance", "amount": "80"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["category"], "maintenance");
    let res = call(&app, &viewer, "DELETE", &item, None).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = call(&app, &owner, "DELETE", &item, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);

    // Keine PV-Anlage: Messstelle bzw. Fahrzeug-ID
    let res = call(
        &app,
        &owner,
        "GET",
        &format!("/api/v1/pv-systems/{grid}/costs"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// @test SPEC_API_REST
#[tokio::test]
#[allow(clippy::too_many_lines)] // alle Schreibzugriffe je Rolle
async fn roles_limit_vehicle_and_cost_writes() {
    let app = test_app();
    let mut anna = User::login(&app, "anna").await;
    let owner = anna.token("owner").await;
    let member = anna.token("member").await;
    let viewer = anna.token("viewer").await;
    let id = create_vehicle(&app, &owner, "electric").await;
    let base = format!("/api/v1/vehicles/{id}");
    let res = call(
        &app,
        &owner,
        "POST",
        &format!("{base}/profiles"),
        Some(json!({"valid_from": "2026-01-01", "kwh_per_100km": "17"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED);
    let profile = res.json["id"].as_str().unwrap().to_owned();

    let charging = json!({"charged_at": "2026-02-01T10:00:00Z", "energy": "10", "amount": "4"});
    let cost = json!({"booked_on": "2026-02-01", "category": "insurance", "amount": "300"});
    let acquisition = json!({
        "purchased_on": "2026-01-01",
        "price": "40000",
        "mode": "mileage",
        "odometer_at_purchase": "10000",
        "lifetime_km": "250000",
    });

    // Viewer liest, schreibt nicht
    for path in [
        "/api/v1/vehicles".to_owned(),
        base.clone(),
        format!("{base}/charging"),
        format!("{base}/report"),
        format!("{base}/costs"),
    ] {
        let res = call(&app, &viewer, "GET", &path, None).await;
        assert_eq!(res.status, StatusCode::OK, "{path}");
    }
    let writes = [
        (
            "POST",
            "/api/v1/vehicles".to_owned(),
            Some(vehicle_body("electric")),
        ),
        ("PUT", base.clone(), Some(json!({"drive_type": "electric"}))),
        (
            "POST",
            format!("{base}/profiles"),
            Some(json!({"valid_from": "2026-02-01", "kwh_per_100km": "17"})),
        ),
        ("DELETE", format!("{base}/profiles/{profile}"), None),
        (
            "PUT",
            format!("{base}/acquisition"),
            Some(acquisition.clone()),
        ),
        ("DELETE", format!("{base}/acquisition"), None),
        ("POST", format!("{base}/charging"), Some(charging.clone())),
        ("POST", format!("{base}/costs"), Some(cost.clone())),
    ];
    for (method, path, body) in &writes {
        let res = call(&app, &viewer, method, path, body.clone()).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "viewer {method} {path}");
    }

    // Member: Ladevorgänge und Kostenbuch, aber keine Stammdaten
    for (method, path, body) in &writes[..6] {
        let res = call(&app, &member, method, path, body.clone()).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "member {method} {path}");
    }
    let res = call(
        &app,
        &member,
        "POST",
        &format!("{base}/charging"),
        Some(charging),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let session = res.json["id"].as_str().unwrap().to_owned();
    let res = call(
        &app,
        &member,
        "POST",
        &format!("{base}/costs"),
        Some(cost.clone()),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let entry = res.json["id"].as_str().unwrap().to_owned();
    let item = format!("/api/v1/asset-costs/{entry}");
    let res = call(&app, &viewer, "PUT", &item, Some(cost.clone())).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = call(&app, &member, "PUT", &item, Some(cost)).await;
    assert_eq!(res.status, StatusCode::OK);
    let res = call(
        &app,
        &viewer,
        "DELETE",
        &format!("{base}/charging/{session}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = call(
        &app,
        &member,
        "DELETE",
        &format!("{base}/charging/{session}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = call(&app, &member, "DELETE", &item, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);

    // Owner darf die Stammdaten
    let res = call(
        &app,
        &owner,
        "PUT",
        &format!("{base}/acquisition"),
        Some(acquisition),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["mode"], "mileage");
    let res = call(&app, &owner, "GET", &format!("{base}/report"), None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["residual"]["mode"], "mileage");
    assert_eq!(res.json["fuel_price"], Value::Null, "Elektro tankt nicht");
}

// @test SPEC_API_REST
#[tokio::test]
#[allow(clippy::too_many_lines)] // alle Zugriffe auf fremde IDs
async fn foreign_household_ids_are_not_found() {
    let app = test_app();
    let mut anna = User::login(&app, "anna").await;
    let owner = anna.token("owner").await;
    let id = create_vehicle(&app, &owner, "plugin_hybrid").await;
    let res = call(
        &app,
        &owner,
        "POST",
        &format!("/api/v1/vehicles/{id}/profiles"),
        Some(json!({"valid_from": "2026-01-01", "kwh_per_100km": "17"})),
    )
    .await;
    let profile = res.json["id"].as_str().unwrap().to_owned();
    let res = call(
        &app,
        &owner,
        "POST",
        &format!("/api/v1/vehicles/{id}/charging"),
        Some(json!({"charged_at": "2026-02-01T10:00:00Z", "energy": "10", "amount": "4"})),
    )
    .await;
    let session = res.json["id"].as_str().unwrap().to_owned();
    let cost = json!({"booked_on": "2026-02-01", "category": "insurance", "amount": "300"});
    let res = call(
        &app,
        &owner,
        "POST",
        &format!("/api/v1/vehicles/{id}/costs"),
        Some(cost.clone()),
    )
    .await;
    let entry = res.json["id"].as_str().unwrap().to_owned();
    let grid = anna.meter_via_form("Hausanschluss").await;
    let generation = anna.meter_via_form("Balkon PV").await;
    let pv = Browser::login(&app, "anna")
        .await
        .pv_system(&grid, &generation)
        .await;

    let mut bob = User::login(&app, "bob").await;
    let foreign = bob.token("owner").await;
    let res = call(&app, &foreign, "GET", "/api/v1/vehicles", None).await;
    assert_eq!(res.json, json!([]));
    let base = format!("/api/v1/vehicles/{id}");
    let item = format!("/api/v1/asset-costs/{entry}");
    let requests = [
        ("GET", base.clone(), None),
        (
            "PUT",
            base.clone(),
            Some(json!({"drive_type": "plugin_hybrid"})),
        ),
        (
            "POST",
            format!("{base}/profiles"),
            Some(json!({"valid_from": "2026-02-01", "kwh_per_100km": "17"})),
        ),
        (
            "PUT",
            format!("{base}/acquisition"),
            Some(json!({
                "purchased_on": "2026-01-01",
                "price": "1",
                "residual_value": "0",
                "planned_end": "2030-01-01",
            })),
        ),
        ("DELETE", format!("{base}/acquisition"), None),
        ("GET", format!("{base}/charging"), None),
        (
            "POST",
            format!("{base}/charging"),
            Some(json!({"charged_at": "2026-02-01T10:00:00Z", "energy": "10", "amount": "4"})),
        ),
        ("GET", format!("{base}/report"), None),
        ("GET", format!("{base}/costs"), None),
        ("POST", format!("{base}/costs"), Some(cost.clone())),
        ("GET", format!("/api/v1/pv-systems/{pv}/costs"), None),
        (
            "POST",
            format!("/api/v1/pv-systems/{pv}/costs"),
            Some(json!({"booked_on": "2026-01-01", "category": "other", "amount": "1"})),
        ),
        ("GET", item.clone(), None),
        ("PUT", item.clone(), Some(cost)),
        ("DELETE", item.clone(), None),
    ];
    for (method, path, body) in requests {
        let res = call(&app, &foreign, method, &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {path}");
    }

    // Löschen fremder Profile und Ladevorgänge wirkt nicht
    call(
        &app,
        &foreign,
        "DELETE",
        &format!("{base}/profiles/{profile}"),
        None,
    )
    .await;
    call(
        &app,
        &foreign,
        "DELETE",
        &format!("{base}/charging/{session}"),
        None,
    )
    .await;
    let res = call(&app, &owner, "GET", &base, None).await;
    assert_eq!(res.json["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(res.json["charging"].as_array().unwrap().len(), 1);
    let res = call(&app, &owner, "GET", &item, None).await;
    assert_eq!(res.status, StatusCode::OK);
}
