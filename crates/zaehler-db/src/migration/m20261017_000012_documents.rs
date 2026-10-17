//! Hochgeladene Dokumente mit Erkennungsergebnis (SPEC_DOC_UPLOADS,
//! SPEC_AI_INVOICE). Abweichend vom Entwurf liegen Status und Vorschlag am
//! Dokument statt in `ai_drafts`: Entwürfe der Foto-Erfassung verlangen ein
//! Foto und bilden genau eine Ablesung ab.

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261017_000012_documents"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum Documents {
    Table,
    Id,
    HouseholdId,
    Kind,
    BlobKey,
    MediaType,
    Bytes,
    OriginalName,
    CreatedBy,
    CreatedAt,
    Status,
    Model,
    Extraction,
    MeterId,
    AnalysedAt,
    FinishedAt,
}

#[derive(DeriveIden)]
enum Households {
    Table,
    Id,
}

// @impl SPEC_DOC_UPLOADS
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Documents::Table)
                .if_not_exists()
                .col(pk_uuid(Documents::Id))
                .col(uuid(Documents::HouseholdId))
                .col(string(Documents::Kind))
                .col(string(Documents::BlobKey))
                .col(string(Documents::MediaType))
                .col(big_integer(Documents::Bytes))
                .col(string(Documents::OriginalName))
                .col(uuid(Documents::CreatedBy))
                .col(timestamp_with_time_zone(Documents::CreatedAt))
                .col(string(Documents::Status))
                .col(string_null(Documents::Model))
                .col(text_null(Documents::Extraction))
                .col(uuid_null(Documents::MeterId))
                .col(timestamp_with_time_zone_null(Documents::AnalysedAt))
                .col(timestamp_with_time_zone_null(Documents::FinishedAt))
                .foreign_key(
                    ForeignKey::create()
                        .from(Documents::Table, Documents::HouseholdId)
                        .to(Households::Table, Households::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_documents_household_status")
                .table(Documents::Table)
                .col(Documents::HouseholdId)
                .col(Documents::Status)
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_documents_created_at")
                .table(Documents::Table)
                .col(Documents::CreatedAt)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(Documents::Table).to_owned())
            .await
    }
}
