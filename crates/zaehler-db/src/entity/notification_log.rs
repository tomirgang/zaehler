use sea_orm::entity::prelude::*;

/// Versandprotokoll (`m20261016_000009_jobs`); der Schlüssel aus Nutzer, Art
/// und `subject_key` verhindert Doppelversand.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "notification_log")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: Uuid,
    /// `reminder`, `alarm` oder `invitation`
    #[sea_orm(primary_key, auto_increment = false)]
    pub kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub subject_key: String,
    pub household_id: Uuid,
    pub sent_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
