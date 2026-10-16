//! Aufnahmezeit des Fotos an Entwürfen (SPEC_IMG_CAPTURE_TIME).

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261016_000004_draft_taken_at"
    }
}

#[derive(DeriveIden)]
enum AiDrafts {
    Table,
    TakenAt,
}

// @impl SPEC_IMG_CAPTURE_TIME
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(AiDrafts::Table)
                .add_column(timestamp_with_time_zone_null(AiDrafts::TakenAt))
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(AiDrafts::Table)
                .drop_column(AiDrafts::TakenAt)
                .to_owned(),
        )
        .await
    }
}
