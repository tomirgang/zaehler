use sea_orm::entity::prelude::*;

/// Externer Ladevorgang (`m20261018_000013_vehicles`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "charging_sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub vehicle_id: Uuid,
    pub charged_at: DateTimeUtc,
    pub energy_milli: i64,
    pub amount_micro: i64,
    pub odometer_milli: Option<i64>,
    pub provider: Option<String>,
    /// `ac` oder `dc`
    pub kind: String,
    /// `form` oder `receipt`
    pub source: String,
    pub document_id: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
