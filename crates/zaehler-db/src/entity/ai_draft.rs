use sea_orm::entity::prelude::*;

/// Entwurf der Foto-Erfassung (`m20261015_000003_ai_drafts`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "ai_drafts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub household_id: Uuid,
    pub created_by: Uuid,
    pub photo_key: String,
    pub model: Option<String>,
    pub duration_ms: Option<i64>,
    /// `zaehler_app::model::Recognition` als JSON
    #[sea_orm(column_type = "Text", nullable)]
    pub suggestion: Option<String>,
    pub matched_meter_id: Option<Uuid>,
    /// `zaehler_app::model::DraftStatus`
    pub status: String,
    pub confirmed_reading_id: Option<Uuid>,
    pub corrected: Option<bool>,
    pub created_at: DateTimeUtc,
    pub finished_at: Option<DateTimeUtc>,
    /// Aufnahmezeit aus den Bildmetadaten (`m20261016_000004_draft_taken_at`)
    pub taken_at: Option<DateTimeUtc>,
    /// Beginn der laufenden Analyse (`m20261016_000005_draft_analysis_started_at`)
    pub analysis_started_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
