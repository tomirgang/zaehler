use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "meter_devices")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub meter_id: Uuid,
    pub serial_number: Option<String>,
    pub installed_at: DateTimeUtc,
    pub removed_at: Option<DateTimeUtc>,
    pub rollover_digits: Option<i16>,
    pub decimal_places: i16,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::meter::Entity",
        from = "Column::MeterId",
        to = "super::meter::Column::Id"
    )]
    Meter,
}

impl Related<super::meter::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Meter.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
