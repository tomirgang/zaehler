use sea_orm::entity::prelude::*;

/// Preisstand einer Kostenposition (`m20261017_000011_tariffs`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "price_components")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub position_id: Uuid,
    pub valid_from: Date,
    /// Festkomma, Skala 10⁻⁶ (siehe [`crate::fixed`]).
    pub base_price_micro: i64,
    /// `day`, `month` oder `year`
    pub base_interval: String,
    /// `zaehler_domain::tariff::UnitPricing` als JSON
    #[sea_orm(column_type = "Text")]
    pub unit_pricing: String,
    /// Überschreibt den Faktor der Position; Skala 10⁻⁶
    pub factor_micro: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
