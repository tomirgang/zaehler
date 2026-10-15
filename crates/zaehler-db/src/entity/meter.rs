use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "meters")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub household_id: Uuid,
    pub name: String,
    /// `zaehler_domain::meter::MeterKind` in `snake_case`.
    pub kind: String,
    pub unit: String,
    pub reading_interval_days: Option<i32>,
    /// Festkomma, Skala 10⁻³ (siehe [`crate::fixed`]).
    pub max_daily_rate_milli: Option<i64>,
    pub archived: bool,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::household::Entity",
        from = "Column::HouseholdId",
        to = "super::household::Column::Id"
    )]
    Household,
    #[sea_orm(has_many = "super::meter_device::Entity")]
    MeterDevice,
    #[sea_orm(has_many = "super::register::Entity")]
    Register,
}

impl Related<super::household::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Household.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
