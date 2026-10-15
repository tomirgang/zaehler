use sea_orm::entity::prelude::*;

/// Rolle eines Nutzers im Haushalt: `owner`, `member` oder `viewer`.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "memberships")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub household_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: Uuid,
    pub role: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::household::Entity",
        from = "Column::HouseholdId",
        to = "super::household::Column::Id"
    )]
    Household,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
}

impl ActiveModelBehavior for ActiveModel {}
