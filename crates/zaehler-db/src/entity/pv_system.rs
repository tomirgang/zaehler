use sea_orm::entity::prelude::*;

/// PV-Anlage (`m20261018_000014_pv`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "pv_systems")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub household_id: Uuid,
    pub name: String,
    /// `surplus`, `full_feed_in` oder `plug_in`
    pub kind: String,
    pub kwp_milli: i64,
    pub commissioned_on: Date,
    pub azimuth: Option<i16>,
    pub tilt: Option<i16>,
    pub inverter: Option<String>,
    pub battery_kwh_milli: Option<i64>,
    pub specific_yield_milli: i64,
    pub degradation_pct_milli: i64,
    /// Monatsanteile als JSON-Array
    #[sea_orm(column_type = "Text", nullable)]
    pub month_shares: Option<String>,
    pub alarm_threshold_pct: i16,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
