//! Persistenzschicht. Unterstützt PostgreSQL (CNPG) und SQLite über dieselben
//! Entities und Migrationen (ADR_003). Dezimalwerte werden als Festkomma-
//! Integer gespeichert (ADR_009, siehe [`fixed`]).

mod backup;
pub mod entity;
pub mod fixed;
mod jobs;
mod lock;
pub mod migration;
mod session_store;
mod uow;

pub use lock::PgLeaderLock;
pub use session_store::DbSessionStore;
pub use uow::SeaOrmStore;

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, Statement,
    TransactionTrait,
};
use sea_orm_migration::MigratorTrait;

pub use sea_orm::DatabaseConnection as Db;

/// Baut die Verbindung anhand der URL auf (`postgres://…` oder `sqlite://…`).
// @impl SPEC_OPS_DB_BACKENDS
pub async fn connect(url: &str, max_connections: u32) -> Result<DatabaseConnection, DbErr> {
    if let Some(dir) = sqlite_parent_dir(url) {
        std::fs::create_dir_all(dir)
            .map_err(|e| DbErr::Custom(format!("cannot create {}: {e}", dir.display())))?;
    }
    let mut opts = ConnectOptions::new(url.to_owned());
    opts.max_connections(max_connections).sqlx_logging(false);
    let db = Database::connect(opts).await?;
    if url.starts_with("sqlite:") {
        // WAL ist eine Eigenschaft der Datei und gilt damit für alle Verbindungen.
        // `foreign_keys = ON` und `busy_timeout = 5s` setzt sqlx bereits je Verbindung.
        db.execute_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "PRAGMA journal_mode = WAL",
            [],
        ))
        .await?;
    }
    Ok(db)
}

/// Verzeichnis der SQLite-Datei, damit `mode=rwc` die Datei anlegen kann
/// (SQLite erzeugt keine Verzeichnisse). `None` für In-Memory und Postgres.
fn sqlite_parent_dir(url: &str) -> Option<&std::path::Path> {
    let path = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))?;
    let path = path.split('?').next().unwrap_or_default();
    if path.is_empty() || path.starts_with(":memory:") {
        return None;
    }
    std::path::Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
}

/// Stand des Schemas gegenüber den Migrationen dieser Programmversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaStatus {
    /// Angewendete Migrationen, aufsteigend
    pub applied: Vec<String>,
    /// Ausstehende Migrationen dieser Version, in Ausführungsreihenfolge
    pub pending: Vec<String>,
}

/// Namen aller Migrationen dieser Version in Ausführungsreihenfolge.
#[must_use]
pub fn migration_names() -> Vec<String> {
    migration::Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect()
}

/// Vergleicht die angewendeten mit den bekannten Migrationen. Enthält die
/// Datenbank Migrationen, die diese Version nicht kennt, stammt sie von einer
/// neueren Version; dann wird nichts ausgeführt.
// @impl SPEC_OPS_UPGRADE
pub async fn schema_status(db: &DatabaseConnection) -> Result<SchemaStatus, DbErr> {
    use zaehler_app::port::BackupRepository;
    let applied = SeaOrmStore::new(db.clone())
        .schema()
        .await
        .map_err(|e| DbErr::Custom(e.to_string()))?;
    let known = migration_names();
    let unknown: Vec<&str> = applied
        .iter()
        .filter(|a| !known.contains(a))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        return Err(DbErr::Custom(format!(
            "Die Datenbank enthält Migrationen einer neueren Programmversion ({}); \
             ein Downgrade ist nicht möglich",
            unknown.join(", ")
        )));
    }
    Ok(SchemaStatus {
        pending: known.into_iter().filter(|k| !applied.contains(k)).collect(),
        applied,
    })
}

/// Führt alle ausstehenden Migrationen aus; verweigert eine Datenbank einer
/// neueren Programmversion.
// @impl SPEC_OPS_MIGRATIONS, SPEC_OPS_UPGRADE
pub async fn migrate(db: &DatabaseConnection) -> Result<(), DbErr> {
    let status = schema_status(db).await?;
    if status.pending.is_empty() {
        return Ok(());
    }
    tracing::info!(pending = ?status.pending, "running database migrations");
    if db.get_database_backend() != DbBackend::Postgres {
        return migration::Migrator::up(db, None).await;
    }
    // Mehrere Replicas starten ggf. gleichzeitig: Advisory-Lock (transaktions-
    // gebunden) serialisiert die Migration; DDL ist in Postgres transaktional.
    let txn = db.begin().await?;
    txn.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT pg_advisory_xact_lock($1)",
        [MIGRATION_LOCK_KEY.into()],
    ))
    .await?;
    migration::Migrator::up(&txn, None).await?;
    txn.commit().await
}

/// Beliebiger, aber fester Schlüssel für `pg_advisory_xact_lock`.
const MIGRATION_LOCK_KEY: i64 = 0x007A_6165_686C_6572; // ASCII "zaehler"

/// Erreichbarkeitsprüfung (Ping) für `/readyz`.
pub async fn ping(db: &DatabaseConnection) -> Result<(), DbErr> {
    db.ping().await
}

#[cfg(test)]
mod upgrade_tests;

#[cfg(test)]
mod tests {
    // @test SPEC_OPS_MIGRATIONS, SPEC_OPS_DB_BACKENDS
    #[tokio::test]
    async fn migrations_run_on_sqlite() {
        let db = super::connect("sqlite::memory:", 1).await.unwrap();
        super::migrate(&db).await.unwrap();
        super::ping(&db).await.unwrap();
    }

    // @test SPEC_OPS_DB_BACKENDS
    #[test]
    fn sqlite_parent_dir_is_derived_from_url() {
        use std::path::Path;
        let dir = super::sqlite_parent_dir;
        assert_eq!(
            dir("sqlite://data/zaehler.db?mode=rwc"),
            Some(Path::new("data"))
        );
        assert_eq!(
            dir("sqlite:///var/lib/z/data/z.db"),
            Some(Path::new("/var/lib/z/data"))
        );
        assert_eq!(dir("sqlite::memory:"), None);
        assert_eq!(dir("sqlite://z.db"), None);
        assert_eq!(dir("postgres://u@h/db"), None);
    }

    /// Läuft nur, wenn `ZAEHLER_TEST_DATABASE_URL` gesetzt ist (CI-Service-Container).
    // @test SPEC_OPS_MIGRATIONS, SPEC_OPS_DB_BACKENDS, SPEC_BACKUP_DATA
    #[tokio::test]
    async fn migrations_run_on_postgres() {
        use zaehler_app::port::BackupRepository;
        let Ok(url) = std::env::var("ZAEHLER_TEST_DATABASE_URL") else {
            eprintln!("ZAEHLER_TEST_DATABASE_URL nicht gesetzt, Test übersprungen");
            return;
        };
        let db = super::connect(&url, 2).await.unwrap();
        super::migrate(&db).await.unwrap();
        // Idempotent: zweiter Lauf darf nichts tun.
        super::migrate(&db).await.unwrap();
        let store = super::SeaOrmStore::new(db);
        zaehler_app::testing::suite::run(&store).await;
        zaehler_app::testing::suite::drafts(&store, &store).await;
        zaehler_app::testing::suite::jobs(&store, &store, &store).await;

        // Export und Import verlustfrei auch auf PostgreSQL (SPEC_BACKUP_DATA)
        let fixture = crate::upgrade_tests::fixture();
        let household = uuid::Uuid::from_u128(2);
        store.import(&fixture).await.unwrap();
        assert_eq!(
            store.export().await.unwrap().household(household),
            fixture.household(household)
        );
    }

    /// Zwei Instanzen gegen dieselbe Datenbank: genau eine ist Leader; nach
    /// dem Beenden übernimmt die andere im nächsten Takt. Nur mit
    /// `ZAEHLER_TEST_DATABASE_URL`.
    // @test SPEC_JOB_SCHEDULER, SPEC_OPS_REPLICAS
    #[tokio::test]
    async fn leader_lock_is_held_by_exactly_one_instance() {
        use zaehler_app::port::LeaderLock;
        let Ok(url) = std::env::var("ZAEHLER_TEST_DATABASE_URL") else {
            eprintln!("ZAEHLER_TEST_DATABASE_URL nicht gesetzt, Test übersprungen");
            return;
        };
        let a = super::PgLeaderLock::connect(&url).await.unwrap();
        let b = super::PgLeaderLock::connect(&url).await.unwrap();
        assert!(a.is_leader().await);
        assert!(!b.is_leader().await);
        // mehrere Takte: Leader bleibt, ohne den Lock zu stapeln
        for _ in 0..3 {
            assert!(a.is_leader().await);
            assert!(!b.is_leader().await);
        }
        a.release().await.unwrap();
        let mut took_over = false;
        for _ in 0..2 {
            if b.is_leader().await {
                took_over = true;
                break;
            }
        }
        assert!(took_over, "Übernahme innerhalb von zwei Takten");
        b.release().await.unwrap();
    }

    // @test SPEC_ARCH_UOW, SPEC_SEC_TENANT_SCOPE, SPEC_OPS_DB_BACKENDS, SPEC_DM_CORE
    #[tokio::test]
    async fn sqlite_adapter_passes_port_suite() {
        let dir = std::env::temp_dir().join(format!("zaehler-test-{}", uuid::Uuid::now_v7()));
        let url = format!("sqlite://{}/test.db?mode=rwc", dir.display());
        let db = super::connect(&url, 4).await.unwrap();
        super::migrate(&db).await.unwrap();
        let store = super::SeaOrmStore::new(db);
        zaehler_app::testing::suite::run(&store).await;
        zaehler_app::testing::suite::drafts(&store, &store).await;
        zaehler_app::testing::suite::jobs(&store, &store, &store).await;
        std::fs::remove_dir_all(dir).ok();
    }

    // @test SPEC_SEC_SESSION_CSRF
    #[tokio::test]
    async fn session_store_roundtrip_and_expiry() {
        use tower_sessions::{
            SessionStore,
            session::{Id, Record},
        };
        let db = super::connect("sqlite::memory:", 1).await.unwrap();
        super::migrate(&db).await.unwrap();
        let store = super::DbSessionStore::new(db);
        let mut data = std::collections::HashMap::new();
        data.insert("user_id".to_owned(), serde_json::json!("abc"));
        let mut record = Record {
            id: Id::default(),
            data,
            expiry_date: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
        };
        store.create(&mut record).await.unwrap();
        assert_eq!(store.load(&record.id).await.unwrap(), Some(record.clone()));
        record
            .data
            .insert("csrf".to_owned(), serde_json::json!("t"));
        store.save(&record).await.unwrap();
        assert_eq!(store.load(&record.id).await.unwrap().unwrap().data.len(), 2);
        let expired = Record {
            id: Id::default(),
            data: std::collections::HashMap::default(),
            expiry_date: time::OffsetDateTime::now_utc() - time::Duration::hours(1),
        };
        store.save(&expired).await.unwrap();
        assert_eq!(store.load(&expired.id).await.unwrap(), None);
        assert_eq!(store.delete_expired(chrono::Utc::now()).await.unwrap(), 1);
        store.delete(&record.id).await.unwrap();
        assert_eq!(store.load(&record.id).await.unwrap(), None);
    }

    // @test SPEC_OPS_HOUSEKEEPING
    #[tokio::test]
    async fn expired_sessions_are_removed_idempotently_in_parallel() {
        use tower_sessions::{
            SessionStore,
            session::{Id, Record},
        };
        use zaehler_app::port::MaintenanceRepository;
        let dir = std::env::temp_dir().join(format!("zaehler-test-{}", uuid::Uuid::now_v7()));
        let url = format!("sqlite://{}/test.db?mode=rwc", dir.display());
        let db = super::connect(&url, 4).await.unwrap();
        super::migrate(&db).await.unwrap();
        let sessions = super::DbSessionStore::new(db.clone());
        for hours in [-2, -1, 1] {
            sessions
                .save(&Record {
                    id: Id::default(),
                    data: std::collections::HashMap::default(),
                    expiry_date: time::OffsetDateTime::now_utc() + time::Duration::hours(hours),
                })
                .await
                .unwrap();
        }
        let a = super::SeaOrmStore::new(db.clone());
        let b = super::SeaOrmStore::new(db);
        let now = chrono::Utc::now();
        let (x, y) = tokio::join!(
            a.delete_expired_sessions(now),
            b.delete_expired_sessions(now)
        );
        assert_eq!(x.unwrap() + y.unwrap(), 2);
        assert_eq!(a.delete_expired_sessions(now).await.unwrap(), 0);
        std::fs::remove_dir_all(dir).ok();
    }
}
