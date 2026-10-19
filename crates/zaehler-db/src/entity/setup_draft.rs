use sea_orm::entity::prelude::*;

/// Setup-Assistent, mit einem Entwurf pro Haushalt.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "setup_drafts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub household_id: Uuid,
    #[sea_orm(column_type = "Text")]
    pub payload: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
