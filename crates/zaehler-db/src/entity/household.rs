use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "households")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub name: String,
    pub timezone: String,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::meter::Entity")]
    Meter,
}

impl Related<super::meter::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Meter.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
