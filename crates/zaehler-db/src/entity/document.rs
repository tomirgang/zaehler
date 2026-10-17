use sea_orm::entity::prelude::*;

/// Hochgeladenes Dokument (`m20261017_000012_documents`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "documents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub household_id: Uuid,
    /// `invoice`
    pub kind: String,
    pub blob_key: String,
    /// `application/pdf` oder `image/jpeg`
    pub media_type: String,
    pub bytes: i64,
    pub original_name: String,
    pub created_by: Uuid,
    pub created_at: DateTimeUtc,
    /// `uploaded`, `analysed`, `failed`, `confirmed` oder `discarded`
    pub status: String,
    pub model: Option<String>,
    /// Vorschlag der Erkennung als JSON
    #[sea_orm(column_type = "Text", nullable)]
    pub extraction: Option<String>,
    pub meter_id: Option<Uuid>,
    pub analysed_at: Option<DateTimeUtc>,
    pub finished_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
