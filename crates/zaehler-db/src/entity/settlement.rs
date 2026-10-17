use sea_orm::entity::prelude::*;

/// Erfasste Jahresabrechnung eines Vertrags (`m20261017_000011_tariffs`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "settlements")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub contract_id: Uuid,
    pub period_start: Date,
    /// Letzter Tag (einschließlich)
    pub period_end: Date,
    /// Festkomma, Skala 10⁻⁶ (siehe [`crate::fixed`]).
    pub amount_micro: i64,
    pub document_id: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
