use sea_orm::entity::prelude::*;

/// Anschaffung eines Fahrzeugs (`m20261018_000013_vehicles`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "vehicle_acquisitions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub vehicle_id: Uuid,
    pub purchased_on: Date,
    pub price_micro: i64,
    pub residual_micro: i64,
    pub planned_end_on: Date,
    pub sold_on: Option<Date>,
    pub sale_price_micro: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
