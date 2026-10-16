use sea_orm::entity::prelude::*;

/// Revision einer Ablesung (`m20261016_000007_reading_revisions`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "reading_revisions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub household_id: Uuid,
    pub reading_id: Uuid,
    pub meter_id: Uuid,
    pub register_id: Uuid,
    /// `update` oder `delete`
    pub action: String,
    pub changed_by: Uuid,
    pub changed_at: DateTimeUtc,
    pub old_read_at: DateTimeUtc,
    /// Festkomma, Skala 10⁻³ (siehe [`crate::fixed`]).
    pub old_value_milli: i64,
    #[sea_orm(column_type = "Text", nullable)]
    pub old_note: Option<String>,
    pub new_read_at: Option<DateTimeUtc>,
    pub new_value_milli: Option<i64>,
    #[sea_orm(column_type = "Text", nullable)]
    pub new_note: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
