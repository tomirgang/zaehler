use sea_orm::entity::prelude::*;

/// Abschlagsstand eines Vertrags (`m20261017_000011_tariffs`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "installments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub contract_id: Uuid,
    pub valid_from: Date,
    /// Festkomma, Skala 10⁻⁶ (siehe [`crate::fixed`]).
    pub amount_micro: i64,
    pub interval_months: i16,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
