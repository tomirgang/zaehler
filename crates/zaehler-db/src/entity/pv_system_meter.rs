use sea_orm::entity::prelude::*;

/// Zuordnung einer Messstelle zu einer PV-Anlage (`m20261018_000014_pv`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "pv_system_meters")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub pv_system_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub meter_id: Uuid,
    /// `grid`, `generation` oder `battery`
    pub role: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
