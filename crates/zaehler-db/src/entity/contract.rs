use sea_orm::entity::prelude::*;

/// Vertrag einer Messstelle (`m20261017_000011_tariffs`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "contracts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub meter_id: Uuid,
    /// `import` oder `export`
    pub direction: String,
    pub provider: String,
    pub contract_number: Option<String>,
    pub starts_on: Date,
    /// Letzter Tag (einschließlich)
    pub ends_on: Option<Date>,
    pub billing_start_month: i16,
    pub billing_start_day: i16,
    #[sea_orm(column_type = "Text", nullable)]
    pub note: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
