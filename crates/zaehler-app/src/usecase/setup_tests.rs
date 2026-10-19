use std::sync::Arc;

use chrono::Duration;
use rust_decimal_macros::dec;
use serde_json::json;

use crate::{
    AppError, Role, Tenant,
    ids::BlobKey,
    model::{Identity, Recognition},
    port::BlobStore,
    testing::{StubRecognizer, memory_ports, memory_ports_with_blobs},
    usecase::{UseCases, housekeeping::Housekeeping},
};

fn identity(subject: &str) -> Identity {
    Identity {
        issuer: "dev".into(),
        subject: subject.into(),
        email: None,
        display_name: subject.into(),
        locale: "en".into(),
    }
}

async fn owner(uc: &UseCases) -> Tenant {
    let signed_in = uc.sign_in.execute(&identity("setup-owner")).await.unwrap();
    uc.resolve_tenant
        .execute(signed_in.user.id, None)
        .await
        .unwrap()
        .tenant
}

#[tokio::test]
async fn setup_draft_resumes_is_owner_only_and_completes_atomically() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    let member = Tenant::new(owner.user_id(), owner.household_id(), Role::Member);

    assert!(matches!(
        uc.start_setup.execute(&member).await.unwrap_err(),
        AppError::Forbidden
    ));
    let initial = uc.start_setup.execute(&owner).await.unwrap();
    let resumed = uc.get_setup_draft.execute(&owner).await.unwrap().unwrap();
    assert_eq!(resumed.created_at, initial.created_at);
    uc.save_setup_draft
        .execute_objects(
            &owner,
            "grid",
            true,
            vec![("grid".into(), json!({"name": "Grid"}))],
        )
        .await
        .unwrap();
    assert!(matches!(
        uc.complete_setup.execute(&member).await.unwrap_err(),
        AppError::Forbidden
    ));

    let created = uc.complete_setup.execute(&owner).await.unwrap();
    assert!(created.contains_key("grid"));
    assert_eq!(uc.list_meters.execute(&owner).await.unwrap().len(), 1);
    assert!(uc.get_setup_draft.execute(&owner).await.unwrap().is_none());
}

#[tokio::test]
async fn invalid_pv_rolls_back_every_setup_meter() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    uc.start_setup.execute(&owner).await.unwrap();
    uc.save_setup_draft
        .execute_objects(
            &owner,
            "grid",
            true,
            vec![("grid".into(), json!({"name": "Grid"}))],
        )
        .await
        .unwrap();
    uc.save_setup_draft
        .execute_objects(
            &owner,
            "pv",
            true,
            vec![
                ("pv".into(), json!({"name": "Roof", "kind": "invalid"})),
                ("pv_generation".into(), json!({"name": "Generation"})),
            ],
        )
        .await
        .unwrap();

    assert!(uc.complete_setup.execute(&owner).await.is_err());
    assert!(uc.list_meters.execute(&owner).await.unwrap().is_empty());
    assert!(uc.get_setup_draft.execute(&owner).await.unwrap().is_some());
}

#[tokio::test]
async fn complete_setup_creates_linked_meters_pv_vehicle_and_contract() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    uc.start_setup.execute(&owner).await.unwrap();
    save_grid_and_heat_pump(&uc, &owner).await;
    save_pv(&uc, &owner).await;
    save_vehicle(&uc, &owner).await;
    save_contract(&uc, &owner).await;

    let created = uc.complete_setup.execute(&owner).await.unwrap();
    for key in [
        "grid",
        "heat_pump",
        "pv_generation",
        "pv_battery",
        "wallbox",
        "vehicle",
    ] {
        assert!(created.contains_key(key), "missing local key {key}");
    }
    assert_eq!(uc.list_meters.execute(&owner).await.unwrap().len(), 6);
    assert_eq!(uc.list_vehicles.execute(&owner).await.unwrap().len(), 1);
    assert_eq!(uc.list_contracts.execute(&owner).await.unwrap().len(), 1);
}

async fn save_grid_and_heat_pump(uc: &UseCases, owner: &Tenant) {
    save_objects(
        uc,
        owner,
        "grid",
        vec![(
            "grid",
            json!({
                "name": "Grid",
                "start_value": "100.1",
                "has_export": true,
                "export_start": "4.2"
            }),
        )],
    )
    .await;
    save_objects(
        uc,
        owner,
        "heat_pump",
        vec![(
            "heat_pump",
            json!({"name": "Heat pump", "start_value": "12.3"}),
        )],
    )
    .await;
}

async fn save_pv(uc: &UseCases, owner: &Tenant) {
    save_objects(
        uc,
        owner,
        "pv",
        vec![
            (
                "pv",
                json!({
                    "name": "Roof",
                    "kind": "surplus",
                    "kwp": "8",
                    "commissioned_on": "2025-06-01",
                    "grid": "grid",
                    "generation": "pv_generation"
                }),
            ),
            (
                "pv_generation",
                json!({"name": "Generation", "start_value": "20.5"}),
            ),
            (
                "pv_battery",
                json!({
                    "name": "Battery",
                    "charge_start": "3.1",
                    "discharge_start": "1.2"
                }),
            ),
        ],
    )
    .await;
}

async fn save_vehicle(uc: &UseCases, owner: &Tenant) {
    save_objects(
        uc,
        owner,
        "vehicle",
        vec![
            (
                "vehicle",
                json!({
                    "name": "EV",
                    "model": "Example",
                    "plate": "E-1",
                    "drive_type": "electric",
                    "odometer": "500",
                    "home_meter": "wallbox"
                }),
            ),
            ("wallbox", json!({"name": "Wallbox", "start_value": "5"})),
        ],
    )
    .await;
}

async fn save_contract(uc: &UseCases, owner: &Tenant) {
    save_objects(
        uc,
        owner,
        "contracts",
        vec![(
            "contracts",
            json!({
                "meter": "grid",
                "contract": {
                    "direction": "import",
                    "provider": "Utility",
                    "starts_on": "2026-01-01",
                    "positions": [{
                        "name": "Energy",
                        "factor": "1",
                        "components": [{
                            "valid_from": "2026-01-01",
                            "base_price": "10",
                            "base_interval": "month",
                            "unit_pricing": {"kind": "flat", "price": "0.3"}
                        }]
                    }]
                }
            }),
        )],
    )
    .await;
}

async fn save_objects(
    uc: &UseCases,
    owner: &Tenant,
    step: &str,
    objects: Vec<(&str, serde_json::Value)>,
) {
    uc.save_setup_draft
        .execute_objects(
            owner,
            step,
            true,
            objects
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn setup_photo_is_saved_and_recognition_is_optional() {
    let (ports, _, _, _) = memory_ports_with_blobs();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    uc.start_setup.execute(&owner).await.unwrap();
    let photo = uc
        .analyse_setup_photo
        .execute(
            &owner,
            "grid",
            bytes::Bytes::from_static(&[0xff, 0xd8, 0xff]),
        )
        .await
        .unwrap();
    assert!(photo.recognition.is_none());
    let draft = uc.get_setup_draft.execute(&owner).await.unwrap().unwrap();
    assert_eq!(
        draft.photos.get("grid").unwrap(),
        &photo.photo_key.to_string()
    );
}

#[tokio::test]
async fn setup_photo_recognition_is_attached_to_object() {
    let (mut ports, _, _, _) = memory_ports_with_blobs();
    ports.recognizer = Some(Arc::new(StubRecognizer(Ok(Recognition {
        value: Some(dec!(123.4)),
        serial_number: Some("M-42".into()),
        register: Some("1.8.0".into()),
        unit: Some("kWh".into()),
        meter_kind: Some("electricity".parse().unwrap()),
        confidence: dec!(0.98),
    }))));
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    uc.start_setup.execute(&owner).await.unwrap();
    uc.analyse_setup_photo
        .execute(
            &owner,
            "grid",
            bytes::Bytes::from_static(&[0xff, 0xd8, 0xff]),
        )
        .await
        .unwrap();
    let draft = uc.get_setup_draft.execute(&owner).await.unwrap().unwrap();
    let recognition = &draft.objects["grid"]["recognition"];
    assert_eq!(recognition["serial_number"], "M-42");
    assert_eq!(recognition["register"], "1.8.0");
    assert_eq!(recognition["value"].to_string().trim_matches('"'), "123.4");
}

#[tokio::test]
async fn setup_start_reading_uses_a_copy_of_its_setup_photo() {
    let (ports, _, _, blobs) = memory_ports_with_blobs();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    uc.start_setup.execute(&owner).await.unwrap();
    let photo = uc
        .analyse_setup_photo
        .execute(
            &owner,
            "grid",
            bytes::Bytes::from_static(&[0xff, 0xd8, 0xff, 0x01]),
        )
        .await
        .unwrap();
    uc.save_setup_draft
        .execute_objects(
            &owner,
            "grid",
            true,
            vec![(
                "grid".into(),
                json!({"name": "Grid", "start_value": "123.4"}),
            )],
        )
        .await
        .unwrap();

    let created = uc.complete_setup.execute(&owner).await.unwrap();
    let meter = *created.get("grid").unwrap();
    let reading = &uc.get_meter.execute(&owner, meter).await.unwrap().readings[0];
    let target = BlobKey::reading_photo(owner.household_id(), reading.id);
    assert_eq!(
        reading.source,
        zaehler_domain::reading::ReadingSource::Photo
    );
    assert_eq!(reading.photo_key.as_ref(), Some(&target));
    assert_eq!(
        blobs.get(&target).await.unwrap(),
        bytes::Bytes::from_static(&[0xff, 0xd8, 0xff, 0x01])
    );
    assert!(blobs.get(&photo.photo_key).await.is_err());
}

#[tokio::test]
async fn housekeeping_expires_setup_drafts_and_deletes_their_photos() {
    let (ports, store, clock, blobs) = memory_ports_with_blobs();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    uc.start_setup.execute(&owner).await.unwrap();
    let photo = uc
        .analyse_setup_photo
        .execute(
            &owner,
            "grid",
            bytes::Bytes::from_static(&[0xff, 0xd8, 0xff]),
        )
        .await
        .unwrap();
    clock.advance(Duration::days(2));

    let housekeeping = Housekeeping::new(store, blobs.clone(), clock, Duration::hours(24))
        .with_setup_draft_ttl(Duration::days(1));
    let report = housekeeping.execute().await.unwrap();
    assert_eq!(report.setup_drafts, 1);
    assert!(uc.get_setup_draft.execute(&owner).await.unwrap().is_none());
    assert!(blobs.get(&photo.photo_key).await.is_err());
}

async fn other_owner(uc: &UseCases) -> Tenant {
    let signed_in = uc.sign_in.execute(&identity("other-owner")).await.unwrap();
    uc.resolve_tenant
        .execute(signed_in.user.id, None)
        .await
        .unwrap()
        .tenant
}

async fn first_grid(uc: &UseCases, owner: &Tenant) -> crate::ids::MeterId {
    uc.start_setup.execute(owner).await.unwrap();
    save_objects(
        uc,
        owner,
        "grid",
        vec![("grid", json!({"name": "Grid", "start_value": "10"}))],
    )
    .await;
    uc.complete_setup.execute(owner).await.unwrap()["grid"]
}

#[tokio::test]
async fn setup_extends_an_existing_household_with_existing_ids_and_acquisition() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    let grid = first_grid(&uc, &owner).await;

    uc.start_setup.execute(&owner).await.unwrap();
    save_objects(
        &uc,
        &owner,
        "grid",
        vec![("grid", json!({"existing_id": grid.to_string()}))],
    )
    .await;
    save_objects(
        &uc,
        &owner,
        "heat_pump",
        vec![("heat_pump", json!({"name": "HP", "start_value": "1"}))],
    )
    .await;
    save_objects(
        &uc,
        &owner,
        "media",
        vec![(
            "media_general",
            json!({"name": "Water", "kind": "water", "start_value": "5"}),
        )],
    )
    .await;
    save_objects(
        &uc,
        &owner,
        "vehicle",
        vec![(
            "vehicle",
            json!({
                "name": "EV", "model": "M", "drive_type": "electric", "odometer": "10",
                "home_meter": format!("id:{grid}"),
                "acquisition_date": "2025-01-01", "acquisition_price": "30000",
                "acquisition_residual_value": "10000",
                "acquisition_planned_end": "2030-01-01"
            }),
        )],
    )
    .await;
    save_objects(
        &uc,
        &owner,
        "contracts",
        vec![(
            "contracts",
            json!({
                "meter": format!("id:{grid}"), "second_meter": "media_general",
                "contract": {
                    "direction": "import", "provider": "U", "starts_on": "2026-01-01",
                    "positions": [{"name": "E", "factor": "1", "components": [{
                        "valid_from": "2026-01-01", "base_price": "1", "base_interval": "month",
                        "unit_pricing": {"kind": "flat", "price": "0.3"}}]}]
                }
            }),
        )],
    )
    .await;
    let created = uc.complete_setup.execute(&owner).await.unwrap();

    assert_eq!(created["grid"], grid);
    assert_eq!(uc.list_meters.execute(&owner).await.unwrap().len(), 4);
    let hp = uc
        .get_meter
        .execute(&owner, created["heat_pump"])
        .await
        .unwrap();
    assert_eq!(
        hp.aggregate.active_device().unwrap().data.parent_meter_id,
        Some(grid)
    );
    let vehicle = uc
        .get_vehicle
        .execute(&owner, created["vehicle"])
        .await
        .unwrap();
    let vehicle = vehicle.vehicle.unwrap();
    assert_eq!(vehicle.data.home_charging_meter_id, Some(grid));
    assert_eq!(vehicle.holding.unwrap().price, dec!(30000));
    assert_eq!(uc.list_contracts.execute(&owner).await.unwrap().len(), 2);
}

// @test SPEC_SETUP_FLOW
#[tokio::test]
async fn every_setup_meter_can_reuse_an_existing_meter() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    uc.start_setup.execute(&owner).await.unwrap();
    save_grid_and_heat_pump(&uc, &owner).await;
    save_pv(&uc, &owner).await;
    save_vehicle(&uc, &owner).await;
    save_objects(
        &uc,
        &owner,
        "media",
        vec![(
            "media_general",
            json!({"name": "Water", "kind": "water", "start_value": "5"}),
        )],
    )
    .await;
    let first = uc.complete_setup.execute(&owner).await.unwrap();
    let meters = uc.list_meters.execute(&owner).await.unwrap().len();
    // Erzeugungs- und Speicherzähler gehören höchstens zu einer Anlage
    for system in uc.list_pv_systems.execute(&owner).await.unwrap() {
        uc.delete_pv_system
            .execute(&owner, system.id)
            .await
            .unwrap();
    }

    uc.start_setup.execute(&owner).await.unwrap();
    let existing = |key: &str| json!({"existing_id": first[key].to_string()});
    save_objects(&uc, &owner, "grid", vec![("grid", existing("grid"))]).await;
    save_objects(
        &uc,
        &owner,
        "heat_pump",
        vec![("heat_pump", existing("heat_pump"))],
    )
    .await;
    save_objects(
        &uc,
        &owner,
        "pv",
        vec![
            (
                "pv",
                json!({"name": "Garage", "kind": "surplus", "kwp": "2",
                       "grid": "grid", "generation": "pv_generation"}),
            ),
            ("pv_generation", existing("pv_generation")),
            ("pv_battery", existing("pv_battery")),
        ],
    )
    .await;
    save_objects(
        &uc,
        &owner,
        "media",
        vec![("media_general", existing("media_general"))],
    )
    .await;
    save_objects(
        &uc,
        &owner,
        "vehicle",
        vec![(
            "vehicle",
            json!({"name": "EV 2", "model": "M", "drive_type": "electric",
                   "home_meter": format!("id:{}", first["wallbox"])}),
        )],
    )
    .await;
    let second = uc.complete_setup.execute(&owner).await.unwrap();

    for key in [
        "grid",
        "heat_pump",
        "pv_generation",
        "pv_battery",
        "media_general",
    ] {
        assert_eq!(second[key], first[key], "{key} reused");
    }
    // nur das zweite Fahrzeug ist neu
    assert_eq!(
        uc.list_meters.execute(&owner).await.unwrap().len(),
        meters + 1
    );
}

#[tokio::test]
async fn setup_rejects_foreign_meters_and_invalid_acquisition_without_partial_data() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    let other = other_owner(&uc).await;
    let foreign = first_grid(&uc, &other).await;

    uc.start_setup.execute(&owner).await.unwrap();
    save_objects(
        &uc,
        &owner,
        "grid",
        vec![("grid", json!({"existing_id": foreign.to_string()}))],
    )
    .await;
    assert!(matches!(
        uc.complete_setup.execute(&owner).await.unwrap_err(),
        AppError::Validation(_)
    ));

    save_objects(
        &uc,
        &owner,
        "grid",
        vec![("grid", json!({"name": "Grid", "start_value": "1"}))],
    )
    .await;
    save_objects(
        &uc,
        &owner,
        "vehicle",
        vec![(
            "vehicle",
            json!({"name": "EV", "model": "M", "drive_type": "electric",
                   "odometer": "1", "acquisition_price": "100"}),
        )],
    )
    .await;
    assert!(matches!(
        uc.complete_setup.execute(&owner).await.unwrap_err(),
        AppError::Validation(_)
    ));
    assert!(uc.list_meters.execute(&owner).await.unwrap().is_empty());
    assert!(uc.get_setup_draft.execute(&owner).await.unwrap().is_some());

    save_objects(
        &uc,
        &owner,
        "vehicle",
        vec![(
            "vehicle",
            json!({"name": "EV", "model": "M", "drive_type": "electric",
                   "odometer": "1", "home_meter": format!("id:{foreign}")}),
        )],
    )
    .await;
    assert!(uc.complete_setup.execute(&owner).await.is_err());
    assert!(uc.list_meters.execute(&owner).await.unwrap().is_empty());
}

async fn photo(uc: &UseCases, owner: &Tenant, key: &str, n: u8) -> BlobKey {
    uc.analyse_setup_photo
        .execute(owner, key, bytes::Bytes::from(vec![0xff, 0xd8, 0xff, n]))
        .await
        .unwrap()
        .photo_key
}

#[tokio::test]
async fn setup_battery_registers_and_media_get_their_own_photos() {
    let (ports, _, _, blobs) = memory_ports_with_blobs();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    uc.start_setup.execute(&owner).await.unwrap();
    save_grid_and_heat_pump(&uc, &owner).await;
    save_pv(&uc, &owner).await;
    save_objects(
        &uc,
        &owner,
        "media",
        vec![(
            "media_general",
            json!({"name": "Water", "kind": "water", "start_value": "5"}),
        )],
    )
    .await;
    let mut sources = Vec::new();
    for (i, key) in [
        "grid_export",
        "pv_battery_charge",
        "pv_battery_discharge",
        "media_general",
    ]
    .into_iter()
    .enumerate()
    {
        sources.push(photo(&uc, &owner, key, u8::try_from(i).unwrap()).await);
    }
    let created = uc.complete_setup.execute(&owner).await.unwrap();

    for (id, expected) in [(created["pv_battery"], 2), (created["grid"], 2)] {
        let detail = uc.get_meter.execute(&owner, id).await.unwrap();
        assert_eq!(
            detail
                .readings
                .iter()
                .filter(|r| r.photo_key.is_some())
                .count(),
            if id == created["grid"] { 1 } else { expected }
        );
    }
    let detail = uc
        .get_meter
        .execute(&owner, created["pv_battery"])
        .await
        .unwrap();
    for reading in &detail.readings {
        let target = BlobKey::reading_photo(owner.household_id(), reading.id);
        assert_eq!(reading.photo_key.as_ref(), Some(&target));
        assert!(blobs.get(&target).await.is_ok());
    }
    assert!(created.contains_key("media_general"));
    for source in sources {
        assert!(blobs.get(&source).await.is_err());
    }
}

#[tokio::test]
async fn skipping_a_step_deletes_its_photos_and_unassigned_photos_block_completion() {
    let (ports, _, _, blobs) = memory_ports_with_blobs();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    uc.start_setup.execute(&owner).await.unwrap();
    save_objects(
        &uc,
        &owner,
        "grid",
        vec![("grid", json!({"name": "Grid", "start_value": "1"}))],
    )
    .await;
    let heat = photo(&uc, &owner, "heat_pump", 1).await;
    let (draft, removed) = uc
        .save_setup_draft
        .execute_objects(&owner, "heat_pump", false, Vec::new())
        .await
        .unwrap();
    assert_eq!(removed, 1);
    assert!(draft.photos.is_empty());
    assert!(blobs.get(&heat).await.is_err());

    // Foto eines nie ausgewählten Schritts darf nicht still verworfen werden
    let stray = photo(&uc, &owner, "wallbox", 2).await;
    assert!(matches!(
        uc.complete_setup.execute(&owner).await.unwrap_err(),
        AppError::Validation(_)
    ));
    assert!(uc.list_meters.execute(&owner).await.unwrap().is_empty());
    assert!(blobs.get(&stray).await.is_ok());

    // Nutzer ordnet zu: ohne Wallbox wird das Foto beim Speichern gelöscht
    let (_, removed) = uc
        .save_setup_draft
        .execute_objects(
            &owner,
            "vehicle",
            true,
            vec![(
                "vehicle".into(),
                json!({"name": "EV", "model": "M", "drive_type": "electric", "odometer": "1"}),
            )],
        )
        .await
        .unwrap();
    assert_eq!(removed, 1);
    assert!(blobs.get(&stray).await.is_err());
    uc.complete_setup.execute(&owner).await.unwrap();
}

#[tokio::test]
async fn own_heat_pump_has_no_parent_and_can_get_its_own_contract() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let owner = owner(&uc).await;
    uc.start_setup.execute(&owner).await.unwrap();
    save_objects(
        &uc,
        &owner,
        "grid",
        vec![("grid", json!({"name": "Grid", "start_value": "1"}))],
    )
    .await;
    save_objects(
        &uc,
        &owner,
        "heat_pump",
        vec![(
            "heat_pump",
            json!({"mode": "own", "name": "Heat pump", "start_value": "2"}),
        )],
    )
    .await;
    save_objects(
        &uc,
        &owner,
        "contracts",
        vec![(
            "contracts",
            json!({
                "meter": "heat_pump",
                "contract": {
                    "direction": "import",
                    "provider": "HP tariff",
                    "starts_on": "2026-01-01",
                    "positions": [{
                        "name": "Energy",
                        "factor": "1",
                        "components": [{
                            "valid_from": "2026-01-01",
                            "base_price": "5",
                            "base_interval": "month",
                            "unit_pricing": {"kind": "flat", "price": "0.2"}
                        }]
                    }]
                }
            }),
        )],
    )
    .await;
    let created = uc.complete_setup.execute(&owner).await.unwrap();
    let heat = uc
        .get_meter
        .execute(&owner, created["heat_pump"])
        .await
        .unwrap();
    assert_eq!(
        heat.aggregate.active_device().unwrap().data.parent_meter_id,
        None
    );
    assert_eq!(uc.list_contracts.execute(&owner).await.unwrap().len(), 1);
}
