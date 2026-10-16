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
    pub billing_start_month: Option<i16>,
    pub billing_start_day: Option<i16>,
    pub archived: bool,
    pub created_at: DateTimeUtc,
    /// `zaehler_domain::meter::MeterPurpose` (`m20261016_000008_meter_v1`)
    pub purpose: String,
    /// `every_days` oder `monthly`; ohne Wert gilt `reading_interval_days`
    pub schedule_kind: Option<String>,
    pub schedule_value: Option<i16>,
    pub archived_at: Option<DateTimeUtc>,
    /// 12 Monatsgewichte als JSON (`m20261016_000010_expectation`)
    #[sea_orm(column_type = "Text", nullable)]
    pub season_weights: Option<String>,
    /// Festkomma, Skala 10⁻³ (siehe [`crate::fixed`]).
    pub annual_target_milli: Option<i64>,
    pub target_tolerance_pct: Option<i16>,
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
