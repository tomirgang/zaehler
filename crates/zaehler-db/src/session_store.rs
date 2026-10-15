//! `tower_sessions::SessionStore` auf der Tabelle `sessions` (ADR_016).
//! Wird in der Composition Root an den Web-Adapter übergeben.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    sea_query::OnConflict,
};
use tower_sessions::{
    SessionStore,
    session::{Id, Record},
    session_store,
};

use crate::entity::session;

#[derive(Debug, Clone)]
pub struct DbSessionStore {
    db: DatabaseConnection,
}

impl DbSessionStore {
    #[must_use]
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Entfernt abgelaufene Sitzungen; liefert die Anzahl.
    pub async fn delete_expired(&self, now: DateTime<Utc>) -> Result<u64, sea_orm::DbErr> {
        Ok(session::Entity::delete_many()
            .filter(session::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await?
            .rows_affected)
    }
}

fn backend(e: impl std::fmt::Display) -> session_store::Error {
    session_store::Error::Backend(e.to_string())
}

fn expiry(record: &Record) -> DateTime<Utc> {
    let odt = record.expiry_date;
    DateTime::from_timestamp(odt.unix_timestamp(), odt.nanosecond()).unwrap_or_default()
}

// @impl SPEC_SEC_SESSION_CSRF
#[async_trait]
impl SessionStore for DbSessionStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        while session::Entity::find_by_id(record.id.to_string())
            .one(&self.db)
            .await
            .map_err(backend)?
            .is_some()
        {
            record.id = Id::default();
        }
        self.save(record).await
    }

    async fn save(&self, record: &Record) -> session_store::Result<()> {
        let data =
            serde_json::to_vec(record).map_err(|e| session_store::Error::Encode(e.to_string()))?;
        session::Entity::insert(session::ActiveModel {
            id: Set(record.id.to_string()),
            data: Set(data),
            expires_at: Set(expiry(record)),
        })
        .on_conflict(
            OnConflict::column(session::Column::Id)
                .update_columns([session::Column::Data, session::Column::ExpiresAt])
                .to_owned(),
        )
        .exec_without_returning(&self.db)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn load(&self, id: &Id) -> session_store::Result<Option<Record>> {
        let Some(row) = session::Entity::find_by_id(id.to_string())
            .filter(session::Column::ExpiresAt.gt(Utc::now()))
            .one(&self.db)
            .await
            .map_err(backend)?
        else {
            return Ok(None);
        };
        serde_json::from_slice(&row.data)
            .map(Some)
            .map_err(|e| session_store::Error::Decode(e.to_string()))
    }

    async fn delete(&self, id: &Id) -> session_store::Result<()> {
        session::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await
            .map_err(backend)?;
        Ok(())
    }
}
