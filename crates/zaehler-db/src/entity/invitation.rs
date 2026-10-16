use sea_orm::entity::prelude::*;

/// Einladung in einen Haushalt (`m20261016_000006_members`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invitations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub household_id: Uuid,
    /// `owner`, `member` oder `viewer`
    pub role: String,
    /// SHA-256 des Tokens, hexadezimal
    pub token_hash: String,
    pub email: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTimeUtc,
    pub expires_at: DateTimeUtc,
    pub accepted_by: Option<Uuid>,
    pub accepted_at: Option<DateTimeUtc>,
    pub revoked_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
