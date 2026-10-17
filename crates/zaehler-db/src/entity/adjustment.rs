use sea_orm::entity::prelude::*;

/// Bonus oder Gutschrift eines Vertrags (`m20261017_000011_tariffs`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "adjustments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub contract_id: Uuid,
    /// `bonus` oder `credit`
    pub kind: String,
    pub booked_on: Date,
    /// Festkomma, Skala 10⁻⁶ (siehe [`crate::fixed`]).
    pub amount_micro: i64,
    #[sea_orm(column_type = "Text", nullable)]
    pub note: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
