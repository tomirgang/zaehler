//! Upgrade ohne Datenverlust und verlustfreier Export (SPEC_OPS_UPGRADE,
//! SPEC_BACKUP_DATA).

use chrono::{DateTime, TimeZone, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, Value};
use sea_orm_migration::MigratorTrait;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zaehler_app::{
    backup::{
        BackupData, DeviceRecord, DraftRecord, HouseholdRecord, MembershipRecord, MeterRecord,
        ReadingRecord, RegisterRecord, UserRecord,
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
        }],
        devices: vec![DeviceRecord {
            id: id(4),
            meter_id: id(3),
            serial_number: Some("1EMH0012".into()),
            installed_at: at(1, 4),
            removed_at: None,
            rollover_digits: Some(6),
            decimal_places: 0,
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
        }],
        drafts: Vec::new(),
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
    data
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
