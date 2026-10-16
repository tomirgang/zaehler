use sea_orm::entity::prelude::*;

/// Ereignis für die Folgeverarbeitung (`m20261016_000009_jobs`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "outbox")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub household_id: Uuid,
    pub kind: String,
    /// `zaehler_app::model::OutboxEvent` als JSON
    #[sea_orm(column_type = "Text")]
    pub payload: String,
    /// `pending`, `done` oder `failed`
    pub status: String,
    pub attempts: i32,
    pub next_attempt_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
    pub finished_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
