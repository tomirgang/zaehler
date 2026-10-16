//! Beginn der Analyse an Entwürfen für die Warteschlange der
//! Mehrfach-Foto-Erfassung (SPEC_AI_MULTI).

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261016_000005_draft_analysis_started_at"
    }
}

#[derive(DeriveIden)]
enum AiDrafts {
    Table,
    AnalysisStartedAt,
}

// @impl SPEC_AI_MULTI
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(AiDrafts::Table)
                .add_column(timestamp_with_time_zone_null(AiDrafts::AnalysisStartedAt))
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(AiDrafts::Table)
                .drop_column(AiDrafts::AnalysisStartedAt)
                .to_owned(),
        )
        .await
    }
}
