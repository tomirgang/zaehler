//! Upgrade ohne Datenverlust und verlustfreier Export (SPEC_OPS_UPGRADE,
//! SPEC_BACKUP_DATA).

use chrono::{DateTime, TimeZone, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, Value};
use sea_orm_migration::MigratorTrait;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zaehler_app::{
    backup::{
        AcquisitionRecord, AdjustmentRecord, AssetCostRecord, BackupData, ChargingSessionRecord,
        ConsumptionProfileRecord, ContractRecord, CostPositionRecord, DeviceRecord, DocumentRecord,
        DraftRecord, HouseholdRecord, InstallmentRecord, InvitationRecord, MembershipRecord,
        MeterRecord, PriceComponentRecord, ReadingRecord, RegisterRecord, RevisionRecord,
        SettlementRecord, UserRecord, VehicleRecord,
    },
    port::BackupRepository,
};

use crate::{SeaOrmStore, connect, migrate, migration::Migrator, migration_names, schema_status};

/// Freigegebene Migrationen mit SHA-256 des Quelltexts. Neue Migrationen
/// werden angehängt; eine freigegebene Migration wird nie geändert, sonst
/// wären Datenbanken älterer Installationen nicht mehr sicher aktualisierbar.
const RELEASED: [(&str, &str, &str); 5] = [
    (
        "m20261015_000001_core",
        include_str!("migration/m20261015_000001_core.rs"),
        "6656b985449dbad8f5d1a20fee8b6061278d6a61cdd6790896f9b9dba4375e18",
    ),
    (
        "m20261015_000002_mvp",
        include_str!("migration/m20261015_000002_mvp.rs"),
        "7fa47f22b97ce196b604fbc2d31d7e1134f2367c80627b126aceef172dcbc191",
    ),
    (
        "m20261015_000003_ai_drafts",
        include_str!("migration/m20261015_000003_ai_drafts.rs"),
        "e888db576689367082afec5711eb97b6e2f595973c6eeaf99c0b9d970fe5b31b",
    ),
    (
        "m20261016_000004_draft_taken_at",
        include_str!("migration/m20261016_000004_draft_taken_at.rs"),
        "a6d0a339a2a2fd6d3a4b23698cba232bbc1035df6efa7b4b611b5e25fad0dda5",
    ),
    (
        "m20261016_000005_draft_analysis_started_at",
        include_str!("migration/m20261016_000005_draft_analysis_started_at.rs"),
        "fcb74815a37aed518a3d6e81b606d2ed88089d7753ddabe116dabed0f1ac4a7a",
    ),
];

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn at(day: u32, micros: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, day, 7, 30, 15).unwrap()
        + chrono::Duration::microseconds(i64::from(micros))
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

async fn temp_db() -> (DatabaseConnection, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("zaehler-upgrade-{}", Uuid::now_v7()));
    let url = format!("sqlite://{}/test.db?mode=rwc", dir.display());
    (connect(&url, 1).await.unwrap(), dir)
}

// @test SPEC_OPS_UPGRADE
#[test]
fn released_migrations_are_append_only_and_unchanged() {
    let names = migration_names();
    assert!(
        names.len() >= RELEASED.len(),
        "freigegebene Migration entfernt"
    );
    for (i, (name, source, hash)) in RELEASED.iter().enumerate() {
        assert_eq!(&names[i], name, "Reihenfolge der freigegebenen Migrationen");
        assert_eq!(
            hex(source.as_bytes()),
            *hash,
            "Migration {name} wurde nach der Freigabe geändert; Änderungen gehören in eine neue Migration"
        );
    }
}

/// Daten im Schema der ersten Migration, eingefügt per SQL.
async fn insert_core(db: &DatabaseConnection) {
    let sql = |q: &str, v: Vec<Value>| Statement::from_sql_and_values(DbBackend::Sqlite, q, v);
    for stmt in [
        sql(
            "INSERT INTO users (id, oidc_issuer, oidc_subject, email, display_name, locale, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                id(1).into(),
                "dev".into(),
                "anna".into(),
                Some("anna@example.org".to_owned()).into(),
                "Anna Müller".into(),
                "de".into(),
                at(1, 1).into(),
            ],
        ),
        sql(
            "INSERT INTO households (id, name, timezone, created_at) VALUES (?, ?, ?, ?)",
            vec![
                id(2).into(),
                "Haus".into(),
                "Europe/Berlin".into(),
                at(1, 2).into(),
            ],
        ),
        sql(
            "INSERT INTO memberships (household_id, user_id, role) VALUES (?, ?, ?)",
            vec![id(2).into(), id(1).into(), "owner".into()],
        ),
        sql(
            "INSERT INTO meters (id, household_id, name, kind, unit, reading_interval_days, max_daily_rate_milli, archived, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                id(3).into(),
                id(2).into(),
                "Strom".into(),
                "electricity".into(),
                "kWh".into(),
                Some(30_i32).into(),
                Some(40_000_i64).into(),
                false.into(),
                at(1, 3).into(),
            ],
        ),
        sql(
            "INSERT INTO meter_devices (id, meter_id, serial_number, installed_at, removed_at, rollover_digits) VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                id(4).into(),
                id(3).into(),
                Some("1EMH0012".to_owned()).into(),
                at(1, 4).into(),
                Option::<DateTime<Utc>>::None.into(),
                Some(6_i16).into(),
            ],
        ),
        sql(
            "INSERT INTO registers (id, meter_id, code, direction) VALUES (?, ?, ?, ?)",
            vec![id(5).into(), id(3).into(), "1.8.0".into(), "import".into()],
        ),
        sql(
            "INSERT INTO readings (id, register_id, device_id, read_at, value_milli, source, photo_key, ai_confidence, note, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                id(6).into(),
                id(5).into(),
                id(4).into(),
                at(2, 6).into(),
                26_415_250_i64.into(),
                "photo".into(),
                Some(format!("households/{}/readings/{}.jpg", id(2), id(6))).into(),
                Some(0.873_f64).into(),
                Some("Ablesung im Keller".to_owned()).into(),
                Some(id(1)).into(),
                at(2, 7).into(),
            ],
        ),
    ] {
        db.execute_raw(stmt).await.unwrap();
    }
}

/// Erwarteter Export nach jeder Migration: Werte aus dem ersten Schema,
/// später ergänzte Spalten mit ihren Vorgaben.
fn expected_core() -> BackupData {
    BackupData {
        users: vec![UserRecord {
            id: id(1),
            oidc_issuer: "dev".into(),
            oidc_subject: "anna".into(),
            email: Some("anna@example.org".into()),
            display_name: "Anna Müller".into(),
            locale: "de".into(),
            created_at: at(1, 1),
            last_login_at: None,
        }],
        households: vec![HouseholdRecord {
            id: id(2),
            name: "Haus".into(),
            timezone: "Europe/Berlin".into(),
            created_at: at(1, 2),
        }],
        memberships: vec![MembershipRecord {
            household_id: id(2),
            user_id: id(1),
            role: "owner".into(),
        }],
        meters: vec![MeterRecord {
            id: id(3),
            household_id: id(2),
            name: "Strom".into(),
            kind: "electricity".into(),
            unit: "kWh".into(),
            reading_interval_days: Some(30),
            max_daily_rate_milli: Some(40_000),
            billing_start_month: None,
            billing_start_day: None,
            archived: false,
            created_at: at(1, 3),
            purpose: "general".into(),
            schedule_kind: None,
            schedule_value: None,
            archived_at: None,
            season_weights: None,
            annual_target_milli: None,
            target_tolerance_pct: None,
        }],
        devices: vec![DeviceRecord {
            id: id(4),
            meter_id: id(3),
            serial_number: Some("1EMH0012".into()),
            installed_at: at(1, 4),
            removed_at: None,
            rollover_digits: Some(6),
            decimal_places: 0,
            parent_meter_id: None,
        }],
        registers: vec![RegisterRecord {
            id: id(5),
            meter_id: id(3),
            code: "1.8.0".into(),
            direction: "import".into(),
        }],
        readings: vec![ReadingRecord {
            id: id(6),
            register_id: id(5),
            device_id: id(4),
            read_at: at(2, 6),
            value_milli: 26_415_250,
            source: "photo".into(),
            photo_key: Some(format!("households/{}/readings/{}.jpg", id(2), id(6))),
            ai_confidence: Some(0.873),
            note: Some("Ablesung im Keller".into()),
            created_by: Some(id(1)),
            created_at: at(2, 7),
            updated_at: None,
        }],
        ..BackupData::default()
    }
}

// @test SPEC_OPS_UPGRADE, SPEC_BACKUP_DATA
#[tokio::test]
async fn upgrade_from_the_first_schema_keeps_all_data() {
    let (db, dir) = temp_db().await;
    Migrator::up(&db, Some(1)).await.unwrap();
    insert_core(&db).await;
    let store = SeaOrmStore::new(db.clone());
    let expected = expected_core();
    // Export auf dem ältesten Schema (Sicherung vor der Migration)
    assert_eq!(store.export().await.unwrap(), expected);
    let total = migration_names().len();
    for step in 2..=total {
        Migrator::up(&db, Some(1)).await.unwrap();
        assert_eq!(
            store.export().await.unwrap(),
            expected,
            "Daten nach Migration {step} von {total}"
        );
        assert_eq!(store.schema().await.unwrap().len(), step);
    }
    assert!(schema_status(&db).await.unwrap().pending.is_empty());
    // erneuter Lauf ändert nichts
    migrate(&db).await.unwrap();
    assert_eq!(store.export().await.unwrap(), expected);
    std::fs::remove_dir_all(dir).ok();
}

// @test SPEC_OPS_UPGRADE
#[tokio::test]
async fn database_of_a_newer_version_is_refused() {
    let (db, dir) = temp_db().await;
    migrate(&db).await.unwrap();
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "INSERT INTO seaql_migrations (version, applied_at) VALUES (?, ?)",
        ["m20991231_000001_future".into(), 0_i64.into()],
    ))
    .await
    .unwrap();
    let err = schema_status(&db).await.unwrap_err().to_string();
    assert!(err.contains("neueren Programmversion"), "{err}");
    assert!(migrate(&db).await.is_err());
    std::fs::remove_dir_all(dir).ok();
}

/// Datensätze mit allen Feldern, auch NULL, Umlauten und Mikrosekunden.
#[allow(clippy::too_many_lines)] // ein Datensatz je Tabelle
pub(crate) fn fixture() -> BackupData {
    let mut data = expected_core();
    data.users.push(UserRecord {
        id: id(11),
        oidc_issuer: "https://auth.example.org/realms/home".into(),
        oidc_subject: "f4b2c1".into(),
        email: None,
        display_name: "Björn".into(),
        locale: "en".into(),
        created_at: at(3, 999_999),
        last_login_at: Some(at(4, 123_456)),
    });
    data.memberships.push(MembershipRecord {
        household_id: id(2),
        user_id: id(11),
        role: "member".into(),
    });
    data.meters[0].billing_start_month = Some(4);
    data.meters[0].billing_start_day = Some(1);
    data.devices[0].removed_at = Some(at(5, 0));
    data.devices[0].decimal_places = 1;
    data.devices.push(DeviceRecord {
        id: id(14),
        meter_id: id(3),
        serial_number: None,
        installed_at: at(5, 0),
        removed_at: None,
        rollover_digits: None,
        decimal_places: 3,
        parent_meter_id: None,
    });
    // zweite Messstelle: Wärmepumpe als Zwischenzähler, monatlicher Plan, archiviert
    data.meters.push(MeterRecord {
        id: id(21),
        household_id: id(2),
        name: "Wärmepumpe".into(),
        kind: "electricity".into(),
        unit: "kWh".into(),
        reading_interval_days: None,
        max_daily_rate_milli: None,
        billing_start_month: None,
        billing_start_day: None,
        archived: true,
        created_at: at(3, 5),
        purpose: "heat_pump".into(),
        schedule_kind: Some("monthly".into()),
        schedule_value: Some(1),
        archived_at: Some(at(9, 0)),
        season_weights: Some(r#"["10","10","9","8","7","6","6","6","7","9","10","12"]"#.into()),
        annual_target_milli: Some(3_500_000),
        target_tolerance_pct: Some(15),
    });
    data.devices.push(DeviceRecord {
        id: id(22),
        meter_id: id(21),
        serial_number: Some("WP-1".into()),
        installed_at: at(3, 5),
        removed_at: None,
        rollover_digits: None,
        decimal_places: 1,
        parent_meter_id: Some(id(3)),
    });
    data.readings.push(ReadingRecord {
        id: id(16),
        register_id: id(5),
        device_id: id(14),
        read_at: at(6, 1),
        value_milli: 0,
        source: "csv_import".into(),
        photo_key: None,
        ai_confidence: None,
        note: None,
        created_by: None,
        created_at: at(6, 2),
        updated_at: Some(at(7, 3)),
    });
    data.revisions.push(RevisionRecord {
        id: id(19),
        household_id: id(2),
        reading_id: id(16),
        meter_id: id(3),
        register_id: id(5),
        action: "update".into(),
        changed_by: id(11),
        changed_at: at(7, 3),
        old_read_at: at(6, 0),
        old_value_milli: 1_500,
        old_note: Some("Tippfehler".into()),
        new_read_at: Some(at(6, 1)),
        new_value_milli: Some(0),
        new_note: None,
    });
    data.revisions.push(RevisionRecord {
        id: id(20),
        household_id: id(2),
        reading_id: id(99),
        meter_id: id(3),
        register_id: id(5),
        action: "delete".into(),
        changed_by: id(1),
        changed_at: at(8, 0),
        old_read_at: at(7, 0),
        old_value_milli: 26_500_000,
        old_note: None,
        new_read_at: None,
        new_value_milli: None,
        new_note: None,
    });
    data.drafts.push(DraftRecord {
        id: id(17),
        household_id: id(2),
        created_by: id(1),
        photo_key: format!("households/{}/drafts/{}.jpg", id(2), id(17)),
        model: Some("google/gemini-2.5-flash".into()),
        duration_ms: Some(4_321),
        suggestion: Some(r#"{"value":"26415.25","serial_number":null,"register":"1.8.0","unit":"kWh","meter_kind":"electricity","confidence":"0.87"}"#.into()),
        matched_meter_id: Some(id(3)),
        status: "confirmed".into(),
        confirmed_reading_id: Some(id(6)),
        corrected: Some(true),
        created_at: at(2, 5),
        finished_at: Some(at(2, 8)),
        taken_at: Some(at(2, 0)),
        analysis_started_at: Some(at(2, 3)),
    });
    data.invitations.push(InvitationRecord {
        id: id(18),
        household_id: id(2),
        role: "member".into(),
        token_hash: "ab".repeat(32),
        email: Some("björn@example.org".into()),
        created_by: id(1),
        created_at: at(3, 1),
        expires_at: at(10, 1),
        accepted_by: Some(id(11)),
        accepted_at: Some(at(4, 7)),
        revoked_at: None,
    });
    contracts_fixture(&mut data);
    vehicles_fixture(&mut data);
    data
}

/// Vertrag mit zwei Positionen, Staffel, Faktor je Preisstand, Abschlägen,
/// Bonus und Abrechnung (ab `m20261017_000011_tariffs`).
#[allow(clippy::too_many_lines)] // vollständiger Datensatz
fn contracts_fixture(data: &mut BackupData) {
    let day = |m: u32, d: u32| chrono::NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    data.contracts.push(ContractRecord {
        id: id(30),
        meter_id: id(3),
        direction: "import".into(),
        provider: "Stadtwerke Süd".into(),
        contract_number: Some("V-4711".into()),
        starts_on: day(1, 1),
        ends_on: Some(day(12, 31)),
        billing_start_month: 4,
        billing_start_day: 1,
        note: Some("Ökostrom".into()),
    });
    data.contracts.push(ContractRecord {
        id: id(31),
        meter_id: id(3),
        direction: "export".into(),
        provider: "Netzbetreiber".into(),
        contract_number: None,
        starts_on: day(2, 1),
        ends_on: None,
        billing_start_month: 1,
        billing_start_day: 1,
        note: None,
    });
    data.cost_positions.push(CostPositionRecord {
        id: id(32),
        contract_id: id(30),
        name: "Arbeitspreis".into(),
        register_id: Some(id(5)),
        factor_micro: 1_000_000,
        vat_pct_milli: Some(19_000),
        sort: 0,
    });
    data.cost_positions.push(CostPositionRecord {
        id: id(33),
        contract_id: id(30),
        name: "Messstellenbetrieb".into(),
        register_id: None,
        factor_micro: 1_000_000,
        vat_pct_milli: None,
        sort: 1,
    });
    data.price_components.push(PriceComponentRecord {
        id: id(34),
        position_id: id(32),
        valid_from: day(1, 1),
        base_price_micro: 12_950_000,
        base_interval: "month".into(),
        unit_pricing: r#"{"kind":"tiered","tiers":[{"up_to":"1000","price":"0.30"},{"up_to":null,"price":"0.32"}]}"#.into(),
        factor_micro: None,
    });
    data.price_components.push(PriceComponentRecord {
        id: id(35),
        position_id: id(32),
        valid_from: day(7, 1),
        base_price_micro: 13_500_000,
        base_interval: "month".into(),
        unit_pricing: r#"{"kind":"flat","price":"0.3149"}"#.into(),
        factor_micro: Some(1_002_500),
    });
    data.price_components.push(PriceComponentRecord {
        id: id(36),
        position_id: id(33),
        valid_from: day(1, 1),
        base_price_micro: 20_000_000,
        base_interval: "year".into(),
        unit_pricing: r#"{"kind":"flat","price":"0"}"#.into(),
        factor_micro: None,
    });
    data.installments.push(InstallmentRecord {
        id: id(37),
        contract_id: id(30),
        valid_from: day(1, 15),
        amount_micro: 85_000_000,
        interval_months: 1,
    });
    data.installments.push(InstallmentRecord {
        id: id(38),
        contract_id: id(30),
        valid_from: day(7, 15),
        amount_micro: 92_000_000,
        interval_months: 2,
    });
    data.adjustments.push(AdjustmentRecord {
        id: id(39),
        contract_id: id(30),
        kind: "bonus".into(),
        booked_on: day(3, 1),
        amount_micro: 50_000_000,
        note: Some("Neukundenbonus".into()),
    });
    data.settlements.push(SettlementRecord {
        id: id(40),
        contract_id: id(30),
        period_start: day(1, 1),
        period_end: day(3, 31),
        amount_micro: 312_450_000,
        document_id: Some(id(41)),
    });
    // Rechnung als PDF, bestätigt (ab `m20261017_000012_documents`)
    data.documents.push(DocumentRecord {
        id: id(41),
        household_id: id(2),
        kind: "invoice".into(),
        blob_key: format!("households/{}/documents/{}.pdf", id(2), id(41)),
        media_type: "application/pdf".into(),
        bytes: 48_213,
        original_name: "Jahresabrechnung Süd.pdf".into(),
        created_by: id(1),
        created_at: at(4, 1),
        status: "confirmed".into(),
        model: Some("google/gemini-2.5-flash".into()),
        extraction: Some(r#"{"provider":"Stadtwerke Süd","total_amount":"312.45"}"#.into()),
        meter_id: Some(id(3)),
        analysed_at: Some(at(4, 2)),
        finished_at: Some(at(4, 3)),
    });
}

// @test SPEC_BACKUP_DATA
#[tokio::test]
async fn export_and_import_are_lossless() {
    let data = fixture();
    let (a, dir_a) = temp_db().await;
    migrate(&a).await.unwrap();
    let source = SeaOrmStore::new(a);
    assert!(source.is_empty().await.unwrap());
    source.import(&data).await.unwrap();
    assert!(!source.is_empty().await.unwrap());
    let exported = source.export().await.unwrap();
    assert_eq!(exported, data);

    let (b, dir_b) = temp_db().await;
    migrate(&b).await.unwrap();
    let target = SeaOrmStore::new(b);
    target.import(&exported).await.unwrap();
    assert_eq!(target.export().await.unwrap(), data);
    // zweiter Import derselben Daten scheitert vollständig (Transaktion)
    assert!(target.import(&exported).await.is_err());
    assert_eq!(target.export().await.unwrap(), data);
    std::fs::remove_dir_all(dir_a).ok();
    std::fs::remove_dir_all(dir_b).ok();
}

/// Fahrzeug mit Profil, Anschaffung, Ladevorgang und Kostenbuch-Einträgen
/// (ab `m20261018_000013_vehicles`).
fn vehicles_fixture(data: &mut BackupData) {
    let day = |m: u32, d: u32| chrono::NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let mut car = data.meters[0].clone();
    car.id = id(50);
    car.name = "Familienauto".into();
    car.kind = "vehicle".into();
    car.unit = "km".into();
    car.billing_start_month = None;
    car.billing_start_day = None;
    data.meters.push(car);
    data.vehicles.push(VehicleRecord {
        meter_id: id(50),
        plate: "M-ZÄ 42E".into(),
        model: "Kombi Hybrid".into(),
        drive_type: "plugin_hybrid".into(),
        home_charging_meter_id: Some(id(3)),
        pv_share_override_pct_milli: Some(35_500),
    });
    data.consumption_profiles.push(ConsumptionProfileRecord {
        id: id(51),
        vehicle_id: id(50),
        valid_from: day(1, 1),
        kwh_per_100km_milli: 18_750,
        charging_loss_pct_milli: 10_000,
        electric_share_pct_milli: Some(60_000),
    });
    data.acquisitions.push(AcquisitionRecord {
        vehicle_id: id(50),
        purchased_on: day(1, 15),
        price_micro: 32_990_000_000,
        residual_micro: 12_000_000_000,
        planned_end_on: chrono::NaiveDate::from_ymd_opt(2032, 1, 15).unwrap(),
        sold_on: None,
        sale_price_micro: None,
    });
    data.charging_sessions.push(ChargingSessionRecord {
        id: id(52),
        vehicle_id: id(50),
        charged_at: at(4, 654_321),
        energy_milli: 32_125,
        amount_micro: 18_990_000,
        odometer_milli: Some(12_345_000),
        provider: Some("Ladesäule Marktplatz".into()),
        kind: "dc".into(),
        source: "form".into(),
        document_id: None,
    });
    data.asset_costs.push(AssetCostRecord {
        id: id(53),
        household_id: id(2),
        asset_type: "vehicle".into(),
        asset_id: id(50),
        booked_on: day(2, 1),
        category: "insurance".into(),
        amount_micro: 612_400_000,
        quantity_milli: None,
        odometer_milli: None,
        full_tank: None,
        recurrence_months: Some(12),
        recurrence_until: Some(day(12, 31)),
        document_id: None,
        note: Some("Vollkasko".into()),
    });
    data.asset_costs.push(AssetCostRecord {
        id: id(54),
        household_id: id(2),
        asset_type: "vehicle".into(),
        asset_id: id(50),
        booked_on: day(3, 2),
        category: "fuel".into(),
        amount_micro: 71_230_000,
        quantity_milli: Some(41_120),
        odometer_milli: Some(12_800_000),
        full_tank: Some(true),
        recurrence_months: None,
        recurrence_until: None,
        document_id: Some(id(41)),
        note: None,
    });
}
