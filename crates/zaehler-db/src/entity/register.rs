use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "registers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub meter_id: Uuid,
    /// z.B. `main`, `HT`, `NT`, `export`.
    pub code: String,
    /// `import` oder `export`.
    pub direction: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::meter::Entity",
        from = "Column::MeterId",
        to = "super::meter::Column::Id"
    )]
    Meter,
    #[sea_orm(has_many = "super::reading::Entity")]
    Reading,
}

impl Related<super::meter::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Meter.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
