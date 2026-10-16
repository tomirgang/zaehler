//! Leader-Lock für Jobs bei mehreren Replicas (SPEC_JOB_SCHEDULER,
//! SPEC_OPS_REPLICAS).
//!
//! PostgreSQL: Session-Advisory-Lock auf einer eigenen Verbindung außerhalb
//! des Pools. Bricht die Verbindung ab, gibt der Server den Lock frei; eine
//! andere Instanz übernimmt im nächsten Takt.

use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, Statement,
};
use zaehler_app::port::LeaderLock;

/// Schlüssel des Advisory-Locks (zwei `int4`): ASCII „zaeh“ und „jobs“.
const LOCK_KEY: (i32, i32) = (0x7A61_6568, 0x6A6F_6273);

/// Leader-Lock über `pg_try_advisory_lock` auf einer eigenen Verbindung.
pub struct PgLeaderLock {
    conn: DatabaseConnection,
    held: AtomicBool,
}

impl PgLeaderLock {
    /// Eigene Verbindung direkt zur Datenbank (nicht über einen Pooler im
    /// Transaktionsmodus, `database.lock_url`).
    pub async fn connect(url: &str) -> Result<Self, DbErr> {
        let mut opts = ConnectOptions::new(url.to_owned());
        opts.max_connections(1)
            .min_connections(1)
            .idle_timeout(None::<std::time::Duration>)
            .max_lifetime(None::<std::time::Duration>)
            .sqlx_logging(false);
        Ok(Self {
            conn: Database::connect(opts).await?,
            held: AtomicBool::new(false),
        })
    }

    async fn scalar(&self, sql: &str) -> Result<bool, DbErr> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql,
                [LOCK_KEY.0.into(), LOCK_KEY.1.into()],
            ))
            .await?
            .ok_or_else(|| DbErr::Custom("keine Antwort".into()))?;
        row.try_get_by_index::<bool>(0)
    }

    /// Hält diese Sitzung den Lock (nach einem Verbindungsabbruch nicht mehr)?
    async fn still_held(&self) -> Result<bool, DbErr> {
        self.scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' \
             AND pid = pg_backend_pid() AND granted AND objsubid = 2 \
             AND classid = $1::int4::oid AND objid = $2::int4::oid)",
        )
        .await
    }

    async fn check(&self) -> Result<bool, DbErr> {
        if self.held.load(Ordering::SeqCst) && self.still_held().await? {
            return Ok(true);
        }
        let got = self.scalar("SELECT pg_try_advisory_lock($1, $2)").await?;
        Ok(got)
    }

    /// Gibt den Lock frei und schließt die Verbindung (Herunterfahren, Tests).
    pub async fn release(self) -> Result<(), DbErr> {
        self.conn.close().await
    }
}

#[async_trait]
impl LeaderLock for PgLeaderLock {
    // @impl SPEC_JOB_SCHEDULER, SPEC_OPS_REPLICAS
    async fn is_leader(&self) -> bool {
        let leader = match self.check().await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, "leader lock check failed");
                false
            }
        };
        let was = self.held.swap(leader, Ordering::SeqCst);
        if was != leader {
            tracing::info!(leader, "leader lock changed");
        }
        leader
    }
}
