use sea_orm::entity::prelude::*;

/// Kostenbuch-Eintrag (`m20261018_000013_vehicles`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "asset_costs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub household_id: Uuid,
    /// `vehicle` oder `pv_system`
    pub asset_type: String,
    pub asset_id: Uuid,
    pub booked_on: Date,
    pub category: String,
    pub amount_micro: i64,
    pub quantity_milli: Option<i64>,
    /// Preis pro Liter (`m20261019_000017_fuel_price`)
    pub unit_price_micro: Option<i64>,
    pub odometer_milli: Option<i64>,
    pub full_tank: Option<bool>,
    pub recurrence_months: Option<i16>,
    pub recurrence_until: Option<Date>,
    pub document_id: Option<Uuid>,
    #[sea_orm(column_type = "Text", nullable)]
    pub note: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
