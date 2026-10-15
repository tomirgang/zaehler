use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "readings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub register_id: Uuid,
    pub device_id: Uuid,
    pub read_at: DateTimeUtc,
    /// Festkomma, Skala 10⁻³ (siehe [`crate::fixed`]).
    pub value_milli: i64,
    /// `zaehler_domain::reading::ReadingSource` in `snake_case`.
    pub source: String,
    /// Objektschlüssel des Belegfotos im Objektspeicher.
    pub photo_key: Option<String>,
    pub ai_confidence: Option<f64>,
    pub note: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::register::Entity",
        from = "Column::RegisterId",
        to = "super::register::Column::Id"
    )]
    Register,
    #[sea_orm(
        belongs_to = "super::meter_device::Entity",
        from = "Column::DeviceId",
        to = "super::meter_device::Column::Id"
    )]
    MeterDevice,
}

impl Related<super::register::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Register.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
