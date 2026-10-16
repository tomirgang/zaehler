use sea_orm::entity::prelude::*;

/// Letzter Lauf je Job (`m20261016_000009_jobs`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "job_runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub job: String,
    pub last_started_at: DateTimeUtc,
    pub last_finished_at: DateTimeUtc,
    /// `ok` oder `error`
    pub last_outcome: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
