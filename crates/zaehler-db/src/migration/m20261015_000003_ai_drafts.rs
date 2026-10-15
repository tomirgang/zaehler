//! Entwürfe der Foto-Erfassung (SPEC_AI_DRAFTS).

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261015_000003_ai_drafts"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum AiDrafts {
    Table,
    Id,
    HouseholdId,
    CreatedBy,
    PhotoKey,
    Model,
    DurationMs,
    Suggestion,
    MatchedMeterId,
    Status,
    ConfirmedReadingId,
    Corrected,
    CreatedAt,
    FinishedAt,
}

#[derive(DeriveIden)]
enum Households {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Readings {
    Table,
    Id,
}

// @impl SPEC_AI_DRAFTS
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(AiDrafts::Table)
                .if_not_exists()
                .col(pk_uuid(AiDrafts::Id))
                .col(uuid(AiDrafts::HouseholdId))
                .col(uuid(AiDrafts::CreatedBy))
                .col(string(AiDrafts::PhotoKey))
                .col(string_null(AiDrafts::Model))
                .col(big_integer_null(AiDrafts::DurationMs))
                .col(text_null(AiDrafts::Suggestion))
                .col(uuid_null(AiDrafts::MatchedMeterId))
                .col(string(AiDrafts::Status))
                .col(uuid_null(AiDrafts::ConfirmedReadingId))
                .col(boolean_null(AiDrafts::Corrected))
                .col(timestamp_with_time_zone(AiDrafts::CreatedAt))
                .col(timestamp_with_time_zone_null(AiDrafts::FinishedAt))
                .foreign_key(
                    ForeignKey::create()
                        .from(AiDrafts::Table, AiDrafts::HouseholdId)
                        .to(Households::Table, Households::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(AiDrafts::Table, AiDrafts::CreatedBy)
                        .to(Users::Table, Users::Id),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(AiDrafts::Table, AiDrafts::ConfirmedReadingId)
                        .to(Readings::Table, Readings::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_ai_drafts_household_status_created_at")
                .table(AiDrafts::Table)
                .col(AiDrafts::HouseholdId)
                .col(AiDrafts::Status)
                .col(AiDrafts::CreatedAt)
                .to_owned(),
        )
        .await?;
        // Housekeeping sucht haushaltsübergreifend nach offenen Entwürfen
        m.create_index(
            Index::create()
                .name("idx_ai_drafts_status_created_at")
                .table(AiDrafts::Table)
                .col(AiDrafts::Status)
                .col(AiDrafts::CreatedAt)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(AiDrafts::Table).to_owned())
            .await
    }
}
