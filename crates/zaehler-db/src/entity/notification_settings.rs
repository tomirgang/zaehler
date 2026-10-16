use sea_orm::entity::prelude::*;

/// Benachrichtigungseinstellungen je Nutzer (`m20261016_000009_jobs`); ohne
/// Zeile gelten die Vorgaben.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "notification_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: Uuid,
    /// abweichende Adresse; ohne Wert die aus der Anmeldung
    pub email: Option<String>,
    pub reminder: bool,
    pub alarm: bool,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
