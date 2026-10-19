use sea_orm::entity::prelude::*;

/// Anschaffung eines Fahrzeugs (`m20261018_000013_vehicles`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "vehicle_acquisitions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub vehicle_id: Uuid,
    pub purchased_on: Date,
    pub price_micro: i64,
    pub residual_micro: Option<i64>,
    pub planned_end_on: Option<Date>,
    pub sold_on: Option<Date>,
    pub sale_price_micro: Option<i64>,
    /// `time` oder `mileage` (`m20261019_000016_vehicle_mileage`)
    pub depreciation_mode: String,
    pub odometer_at_purchase_milli: Option<i64>,
    pub lifetime_km_milli: Option<i64>,
    pub end_value_micro: Option<i64>,
    /// `m20261019_000019_vehicle_registration`
    pub first_registration_on: Option<Date>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
