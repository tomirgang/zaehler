//! Persistenzschicht. Unterstützt PostgreSQL (CNPG) und SQLite über dieselben
//! Entities und Migrationen (ADR_003). Dezimalwerte werden als Festkomma-
//! Integer gespeichert (ADR_009, siehe [`fixed`]).

pub mod entity;
pub mod fixed;
pub mod migration;
mod session_store;
mod uow;

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

/// Führt alle ausstehenden Migrationen aus.
// @impl SPEC_OPS_MIGRATIONS
pub async fn migrate(db: &DatabaseConnection) -> Result<(), DbErr> {
    tracing::info!("running database migrations");
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
    // @test SPEC_OPS_MIGRATIONS, SPEC_OPS_DB_BACKENDS
    #[tokio::test]
    async fn migrations_run_on_postgres() {
        let Ok(url) = std::env::var("ZAEHLER_TEST_DATABASE_URL") else {
            eprintln!("ZAEHLER_TEST_DATABASE_URL nicht gesetzt, Test übersprungen");
            return;
        };
        let db = super::connect(&url, 2).await.unwrap();
        super::migrate(&db).await.unwrap();
        // Idempotent: zweiter Lauf darf nichts tun.
        super::migrate(&db).await.unwrap();
        zaehler_app::testing::suite::run(&super::SeaOrmStore::new(db)).await;
    }

    // @test SPEC_ARCH_UOW, SPEC_SEC_TENANT_SCOPE, SPEC_OPS_DB_BACKENDS, SPEC_DM_CORE
    #[tokio::test]
    async fn sqlite_adapter_passes_port_suite() {
        let dir = std::env::temp_dir().join(format!("zaehler-test-{}", uuid::Uuid::now_v7()));
        let url = format!("sqlite://{}/test.db?mode=rwc", dir.display());
        let db = super::connect(&url, 4).await.unwrap();
        super::migrate(&db).await.unwrap();
        zaehler_app::testing::suite::run(&super::SeaOrmStore::new(db)).await;
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
}
