use sea_orm::entity::prelude::*;

/// Persönliches API-Token (`m20261019_000015_api`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "api_tokens")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub household_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    /// `owner`, `member` oder `viewer`
    pub role: String,
    /// SHA-256 des Tokens, hexadezimal
    pub token_hash: String,
    /// Die ersten 8 Zeichen des Tokens
    pub prefix: String,
    pub created_at: DateTimeUtc,
    pub expires_at: Option<DateTimeUtc>,
    pub revoked_at: Option<DateTimeUtc>,
    pub last_used_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
