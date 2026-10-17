use sea_orm::entity::prelude::*;

/// Kostenposition eines Vertrags (`m20261017_000011_tariffs`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "cost_positions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub contract_id: Uuid,
    pub name: String,
    /// `None`: alle Register der Richtung des Vertrags
    pub register_id: Option<Uuid>,
    /// Festkomma, Skala 10⁻⁶ (siehe [`crate::fixed`]).
    pub factor_micro: i64,
    /// MwSt-Satz in Prozent, Skala 10⁻³
    pub vat_pct_milli: Option<i64>,
    pub sort: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
