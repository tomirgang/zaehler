//! REST-API: Verträge, Einträge und Haushaltskosten (SPEC_API_REST).
mod common;

use axum::{Router, http::StatusCode};
use common::{User, call, test_app};
use serde_json::{Value, json};

fn contract_body(provider: &str) -> Value {
    json!({
        "direction": "import",
        "provider": provider,
        "contract_number": "V-42",
        "starts_on": "2026-01-01",
        "billing_start": "01-01",
        "note": "Testvertrag",
        "positions": [{
            "name": "Strom",
            "vat_pct": "19",
            "components": [{
                "valid_from": "2026-01-01",
                "base_price": "120",
                "base_interval": "year",
                "unit_pricing": {"kind": "flat", "price": "0.30"}
            }]
        }]
    })
}

/// Owner mit Messstelle, Tokens aller Rollen und einem Vertrag.
struct Setup {
    app: Router,
    meter: String,
    owner: String,
    member: String,
    viewer: String,
    contract: Value,
}

async fn setup(name: &str) -> Setup {
    let app = test_app();
    let mut user = User::login(&app, name).await;
    let meter = user.meter_via_form("Strom").await;
    let owner = user.token("owner").await;
    let member = user.token("member").await;
    let viewer = user.token("viewer").await;
    let res = call(
        &app,
        &owner,
        "POST",
        &format!("/api/v1/meters/{meter}/contracts"),
        Some(contract_body("Stadtwerke")),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    Setup {
        app,
        meter,
        owner,
        member,
        viewer,
        contract: res.json,
    }
}

fn id(v: &Value) -> &str {
    v["id"].as_str().unwrap()
}

// @test SPEC_API_REST
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn contract_lifecycle() {
    let s = setup("anna").await;
    let c = &s.contract;
    let cid = id(c);
    assert_eq!(c["meter_id"], s.meter.as_str());
    assert_eq!(c["direction"], "import");
    assert_eq!(c["provider"], "Stadtwerke");
    assert_eq!(c["contract_number"], "V-42");
    assert_eq!(c["starts_on"], "2026-01-01");
    assert_eq!(c["ends_on"], Value::Null);
    assert_eq!(c["billing_start"], "01-01");
    assert_eq!(c["note"], "Testvertrag");
    let pos = &c["positions"][0];
    assert_eq!(pos["name"], "Strom");
    assert_eq!(pos["factor"], "1");
    assert_eq!(pos["vat_pct"], "19");
    assert_eq!(pos["components"][0]["base_price"], "120");
    assert_eq!(pos["components"][0]["base_interval"], "year");
    assert_eq!(pos["components"][0]["unit_pricing"]["kind"], "flat");
    assert_eq!(pos["components"][0]["unit_pricing"]["price"], "0.30");

    // Lesen: einzeln, je Messstelle, im Haushalt
    let res = call(
        &s.app,
        &s.viewer,
        "GET",
        &format!("/api/v1/contracts/{cid}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(&res.json, c);
    let res = call(
        &s.app,
        &s.viewer,
        "GET",
        &format!("/api/v1/meters/{}/contracts", s.meter),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json.as_array().unwrap().len(), 1);
    assert_eq!(&res.json[0], c);

    // Ändern: Position und Preisstand bleiben über ihre IDs erhalten
    let pos_id = id(pos).to_owned();
    let comp_id = id(&pos["components"][0]).to_owned();
    let mut body = contract_body("Neuer Versorger");
    body["ends_on"] = json!("2027-12-31");
    body["positions"][0]["id"] = json!(pos_id);
    body["positions"][0]["components"][0]["id"] = json!(comp_id);
    body["positions"][0]["components"][0]["unit_pricing"] =
        json!({"kind": "tiered", "tiers": [{"up_to": "1000", "price": "0.25"}, {"price": "0.35"}]});
    let res = call(
        &s.app,
        &s.owner,
        "PUT",
        &format!("/api/v1/contracts/{cid}"),
        Some(body),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["provider"], "Neuer Versorger");
    assert_eq!(res.json["ends_on"], "2027-12-31");
    assert_eq!(res.json["positions"][0]["id"], pos_id.as_str());
    let comp = &res.json["positions"][0]["components"][0];
    assert_eq!(comp["id"], comp_id.as_str());
    assert_eq!(comp["unit_pricing"]["kind"], "tiered");
    assert_eq!(comp["unit_pricing"]["tiers"][1]["price"], "0.35");

    // Preisstand
    let res = call(
        &s.app,
        &s.owner,
        "POST",
        &format!("/api/v1/contracts/{cid}/positions/{pos_id}/price-components"),
        Some(json!({
            "valid_from": "2026-07-01",
            "base_price": "10",
            "base_interval": "month",
            "unit_pricing": {"kind": "flat", "price": "0.32"},
            "factor": "1.5"
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let comps = &res.json["positions"][0]["components"];
    assert_eq!(comps.as_array().unwrap().len(), 2);
    assert_eq!(comps[1]["valid_from"], "2026-07-01");
    assert_eq!(comps[1]["base_interval"], "month");
    assert_eq!(comps[1]["factor"], "1.5");

    // Abschlag, Anpassung, Abrechnung anlegen und wieder löschen
    let res = call(
        &s.app,
        &s.owner,
        "POST",
        &format!("/api/v1/contracts/{cid}/installments"),
        Some(json!({"valid_from": "2026-01-01", "amount": "80", "every_months": 2})),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let inst = &res.json["installments"][0];
    assert_eq!(inst["amount"], "80");
    assert_eq!(inst["every_months"], 2);
    let inst_id = id(inst).to_owned();

    let res = call(
        &s.app,
        &s.member,
        "POST",
        &format!("/api/v1/contracts/{cid}/adjustments"),
        Some(
            json!({"kind": "bonus", "booked_on": "2026-03-01", "amount": "50", "note": "Neukunde"}),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let adj = &res.json["adjustments"][0];
    assert_eq!(adj["kind"], "bonus");
    assert_eq!(adj["note"], "Neukunde");
    let adj_id = id(adj).to_owned();

    let res = call(
        &s.app,
        &s.member,
        "POST",
        &format!("/api/v1/contracts/{cid}/settlements"),
        Some(json!({"period_start": "2026-01-01", "period_end": "2026-12-31", "amount": "910.50"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let set = &res.json["settlements"][0];
    assert_eq!(set["period_end"], "2026-12-31");
    assert_eq!(set["amount"], "910.50");
    assert_eq!(set["document_id"], Value::Null);
    let set_id = id(set).to_owned();

    for (kind, entry, token) in [
        ("adjustments", &adj_id, &s.member),
        ("settlements", &set_id, &s.member),
        ("installments", &inst_id, &s.owner),
    ] {
        let path = format!("/api/v1/contracts/{cid}/{kind}/{entry}");
        let res = call(&s.app, token, "DELETE", &path, None).await;
        assert_eq!(res.status, StatusCode::NO_CONTENT, "{kind}");
        let res = call(&s.app, token, "DELETE", &path, None).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{kind} erneut");
    }
    let res = call(
        &s.app,
        &s.viewer,
        "GET",
        &format!("/api/v1/contracts/{cid}"),
        None,
    )
    .await;
    for list in ["installments", "adjustments", "settlements"] {
        assert!(res.json[list].as_array().unwrap().is_empty(), "{list}");
    }

    // Löschen
    let path = format!("/api/v1/contracts/{cid}");
    let res = call(&s.app, &s.owner, "DELETE", &path, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = call(&s.app, &s.viewer, "GET", &path, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = call(&s.app, &s.owner, "DELETE", &path, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// @test SPEC_API_REST
#[tokio::test]
async fn invalid_input_is_rejected() {
    let s = setup("bert").await;
    let cid = id(&s.contract);
    let meter_path = format!("/api/v1/meters/{}/contracts", s.meter);
    let mut body = contract_body("X");
    body["direction"] = json!("sideways");
    body["billing_start"] = json!("13");
    body["positions"][0]["components"][0]["base_interval"] = json!("week");
    let res = call(&s.app, &s.owner, "POST", &meter_path, Some(body)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let fields: Vec<&str> = res.json["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, ["direction", "billing_start", "pos0_c0_interval"]);

    // Validierung des Anwendungsfalls
    let mut body = contract_body("");
    body["positions"] = json!([]);
    let res = call(&s.app, &s.owner, "POST", &meter_path, Some(body)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);

    // überlappender Vertrag gleicher Richtung
    let res = call(
        &s.app,
        &s.owner,
        "POST",
        &meter_path,
        Some(contract_body("Y")),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.json);

    let res = call(
        &s.app,
        &s.member,
        "POST",
        &format!("/api/v1/contracts/{cid}/adjustments"),
        Some(json!({"kind": "gift", "booked_on": "2026-03-01", "amount": "5"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.json["errors"][0]["field"], "adjustment_kind");

    let res = call(
        &s.app,
        &s.owner,
        "POST",
        &format!("/api/v1/contracts/{cid}/installments"),
        Some(json!({"valid_from": "2026-01-01", "amount": "5", "every_months": 13})),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);

    let res = call(
        &s.app,
        &s.owner,
        "POST",
        &format!("/api/v1/contracts/{cid}/installments"),
        Some(json!({"amount": "5"})),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);

    let res = call(
        &s.app,
        &s.owner,
        "DELETE",
        &format!("/api/v1/contracts/{cid}/positions/{cid}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// @test SPEC_API_REST
#[tokio::test]
#[allow(clippy::too_many_lines)] // alle Schreibzugriffe je Rolle
async fn roles_are_enforced() {
    let s = setup("carla").await;
    let cid = id(&s.contract);
    let pos = id(&s.contract["positions"][0]);
    let meter_path = format!("/api/v1/meters/{}/contracts", s.meter);
    let installment = json!({"valid_from": "2026-01-01", "amount": "80"});
    let adjustment = json!({"kind": "credit", "booked_on": "2026-03-01", "amount": "5"});
    let settlement =
        json!({"period_start": "2026-01-01", "period_end": "2026-12-31", "amount": "1"});
    let component =
        json!({"valid_from": "2026-05-01", "unit_pricing": {"kind": "flat", "price": "0.3"}});
    let writes = [
        ("POST", meter_path.clone(), Some(contract_body("Z"))),
        (
            "PUT",
            format!("/api/v1/contracts/{cid}"),
            Some(contract_body("Z")),
        ),
        (
            "POST",
            format!("/api/v1/contracts/{cid}/positions/{pos}/price-components"),
            Some(component),
        ),
        (
            "POST",
            format!("/api/v1/contracts/{cid}/installments"),
            Some(installment),
        ),
        (
            "POST",
            format!("/api/v1/contracts/{cid}/adjustments"),
            Some(adjustment.clone()),
        ),
        (
            "POST",
            format!("/api/v1/contracts/{cid}/settlements"),
            Some(settlement.clone()),
        ),
        ("DELETE", format!("/api/v1/contracts/{cid}"), None),
    ];
    for (method, path, body) in &writes {
        let res = call(&s.app, &s.viewer, method, path, body.clone()).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "viewer {method} {path}");
    }
    // Member: nur Anpassungen und Abrechnungen
    for (i, (method, path, body)) in writes.iter().enumerate() {
        let res = call(&s.app, &s.member, method, path, body.clone()).await;
        let expected = if matches!(i, 4 | 5) {
            StatusCode::CREATED
        } else {
            StatusCode::FORBIDDEN
        };
        assert_eq!(res.status, expected, "member {method} {path}");
    }
    let res = call(
        &s.app,
        &s.owner,
        "GET",
        &format!("/api/v1/contracts/{cid}"),
        None,
    )
    .await;
    let adj = id(&res.json["adjustments"][0]).to_owned();
    let res = call(
        &s.app,
        &s.owner,
        "POST",
        &format!("/api/v1/contracts/{cid}/installments"),
        Some(json!({"valid_from": "2026-01-01", "amount": "80"})),
    )
    .await;
    let inst = id(&res.json["installments"][0]).to_owned();
    for (path, token) in [
        (
            format!("/api/v1/contracts/{cid}/adjustments/{adj}"),
            &s.viewer,
        ),
        (
            format!("/api/v1/contracts/{cid}/installments/{inst}"),
            &s.viewer,
        ),
        (
            format!("/api/v1/contracts/{cid}/installments/{inst}"),
            &s.member,
        ),
    ] {
        let res = call(&s.app, token, "DELETE", &path, None).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{path}");
    }
    let res = call(
        &s.app,
        &s.member,
        "DELETE",
        &format!("/api/v1/contracts/{cid}/adjustments/{adj}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);

    // Lesen genügt Viewer
    for path in [
        meter_path.clone(),
        format!("/api/v1/meters/{}/contract-templates", s.meter),
        "/api/v1/costs".to_owned(),
    ] {
        let res = call(&s.app, &s.viewer, "GET", &path, None).await;
        assert_eq!(res.status, StatusCode::OK, "{path}");
    }
}

// @test SPEC_API_REST
#[tokio::test]
async fn other_household_is_not_found() {
    let s = setup("dora").await;
    let cid = id(&s.contract);
    let pos = id(&s.contract["positions"][0]);
    let mut other = User::login(&s.app, "emil").await;
    let token = other.token("owner").await;
    let res = call(
        &s.app,
        &s.owner,
        "POST",
        &format!("/api/v1/contracts/{cid}/installments"),
        Some(json!({"valid_from": "2026-01-01", "amount": "80"})),
    )
    .await;
    let inst = id(&res.json["installments"][0]).to_owned();
    let component =
        json!({"valid_from": "2026-05-01", "unit_pricing": {"kind": "flat", "price": "0.3"}});
    let requests = [
        ("GET", format!("/api/v1/meters/{}/contracts", s.meter), None),
        (
            "POST",
            format!("/api/v1/meters/{}/contracts", s.meter),
            Some(contract_body("Z")),
        ),
        (
            "GET",
            format!("/api/v1/meters/{}/contract-templates", s.meter),
            None,
        ),
        ("GET", format!("/api/v1/contracts/{cid}"), None),
        (
            "PUT",
            format!("/api/v1/contracts/{cid}"),
            Some(contract_body("Z")),
        ),
        (
            "POST",
            format!("/api/v1/contracts/{cid}/positions/{pos}/price-components"),
            Some(component),
        ),
        (
            "POST",
            format!("/api/v1/contracts/{cid}/installments"),
            Some(json!({"valid_from": "2026-02-01", "amount": "80"})),
        ),
        (
            "POST",
            format!("/api/v1/contracts/{cid}/adjustments"),
            Some(json!({"kind": "bonus", "booked_on": "2026-03-01", "amount": "5"})),
        ),
        (
            "POST",
            format!("/api/v1/contracts/{cid}/settlements"),
            Some(json!({"period_start": "2026-01-01", "period_end": "2026-12-31", "amount": "1"})),
        ),
        (
            "DELETE",
            format!("/api/v1/contracts/{cid}/installments/{inst}"),
            None,
        ),
        ("DELETE", format!("/api/v1/contracts/{cid}"), None),
    ];
    for (method, path, body) in requests {
        let res = call(&s.app, &token, method, &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    // Daten unverändert
    let res = call(
        &s.app,
        &s.viewer,
        "GET",
        &format!("/api/v1/contracts/{cid}"),
        None,
    )
    .await;
    assert_eq!(res.json["provider"], "Stadtwerke");
    assert_eq!(res.json["installments"].as_array().unwrap().len(), 1);
    let res = call(&s.app, &token, "GET", "/api/v1/contracts", None).await;
    assert_eq!(res.json, json!([]));
}

// @test SPEC_API_REST
#[tokio::test]
async fn household_costs_and_templates() {
    let s = setup("frida").await;
    let res = call(&s.app, &s.viewer, "GET", "/api/v1/costs", None).await;
    assert_eq!(res.json["rows"], json!([]), "ohne Verbrauch");
    let meter = call(
        &s.app,
        &s.viewer,
        "GET",
        &format!("/api/v1/meters/{}", s.meter),
        None,
    )
    .await;
    let register = meter.json["meter"]["registers"][0]["id"].clone();
    let res = call(
        &s.app,
        &s.member,
        "POST",
        &format!("/api/v1/meters/{}/readings", s.meter),
        Some(json!({
            "read_at": "2026-01-10T12:00:00Z",
            "values": [{"register_id": register, "value": "1100"}]
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);

    let res = call(&s.app, &s.viewer, "GET", "/api/v1/costs?year=2026", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["year"], 2026);
    let rows = res.json["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["meter_id"], s.meter.as_str());
    assert_eq!(rows[0]["name"], "Strom");
    assert!(rows[0]["import"]["total"].is_string());
    assert!(res.json["import_total"].is_string());
    assert!(res.json["net_total"].is_string());
    assert_eq!(res.json["months"].as_array().unwrap().len(), 12);
    assert_eq!(res.json["months"][0]["month"], "2026-01");

    for q in ["year=1800", "year=abc"] {
        let res = call(
            &s.app,
            &s.viewer,
            "GET",
            &format!("/api/v1/costs?{q}"),
            None,
        )
        .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{q}");
    }

    let res = call(
        &s.app,
        &s.viewer,
        "GET",
        &format!("/api/v1/meters/{}/contract-templates", s.meter),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json[0]["code"], "electricity");
    assert_eq!(res.json[0]["direction"], "import");
    assert_eq!(res.json[0]["positions"][0]["base_interval"], "year");
    assert_eq!(res.json[0]["positions"][0]["factor"], "1");
}

// @test SPEC_API_REST
#[tokio::test]
async fn openapi_lists_contract_methods() {
    let s = setup("gina").await;
    let res = call(&s.app, &s.viewer, "GET", "/api/v1/openapi.json", None).await;
    let paths = &res.json["paths"];
    let item = &paths["/api/v1/contracts/{id}"];
    for m in ["get", "put", "delete"] {
        assert!(item[m].is_object(), "{m}");
    }
    let item = &paths["/api/v1/meters/{id}/contracts"];
    assert!(item["get"].is_object() && item["post"].is_object());
    assert!(paths["/api/v1/costs"]["get"].is_object());
    assert!(res.json["components"]["schemas"]["ContractBody"].is_object());
}
