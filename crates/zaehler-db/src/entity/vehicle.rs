use sea_orm::entity::prelude::*;

/// Fahrzeug einer Messstelle (`m20261018_000013_vehicles`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "vehicles")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub meter_id: Uuid,
    pub plate: String,
    pub model: String,
    /// `combustion`, `electric` oder `plugin_hybrid`
    pub drive_type: String,
    pub home_charging_meter_id: Option<Uuid>,
    pub pv_share_override_pct_milli: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
