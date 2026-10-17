use sea_orm::entity::prelude::*;

/// Verbrauchsprofil eines Fahrzeugs (`m20261018_000013_vehicles`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "vehicle_consumption_profiles")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub vehicle_id: Uuid,
    pub valid_from: Date,
    pub kwh_per_100km_milli: i64,
    pub charging_loss_pct_milli: i64,
    pub electric_share_pct_milli: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
