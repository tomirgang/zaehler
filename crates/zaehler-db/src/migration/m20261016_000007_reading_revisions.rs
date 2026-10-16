//! Revisionen von Ablesungen und Zeitpunkt der letzten Korrektur
//! (SPEC_RDG_HISTORY). Revisionen haben keinen Fremdschlüssel auf die
//! Ablesung, damit sie nach dem Löschen erhalten bleiben.

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261016_000007_reading_revisions"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum ReadingRevisions {
    Table,
    Id,
    HouseholdId,
    ReadingId,
    MeterId,
    RegisterId,
    Action,
    ChangedBy,
    ChangedAt,
    OldReadAt,
    OldValueMilli,
    OldNote,
    NewReadAt,
    NewValueMilli,
    NewNote,
}

#[derive(DeriveIden)]
enum Households {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Readings {
    Table,
    UpdatedAt,
}

// @impl SPEC_RDG_HISTORY
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(ReadingRevisions::Table)
                .if_not_exists()
                .col(pk_uuid(ReadingRevisions::Id))
                .col(uuid(ReadingRevisions::HouseholdId))
                .col(uuid(ReadingRevisions::ReadingId))
                .col(uuid(ReadingRevisions::MeterId))
                .col(uuid(ReadingRevisions::RegisterId))
                .col(string(ReadingRevisions::Action))
                .col(uuid(ReadingRevisions::ChangedBy))
                .col(timestamp_with_time_zone(ReadingRevisions::ChangedAt))
                .col(timestamp_with_time_zone(ReadingRevisions::OldReadAt))
                .col(big_integer(ReadingRevisions::OldValueMilli))
                .col(text_null(ReadingRevisions::OldNote))
                .col(timestamp_with_time_zone_null(ReadingRevisions::NewReadAt))
                .col(big_integer_null(ReadingRevisions::NewValueMilli))
                .col(text_null(ReadingRevisions::NewNote))
                .foreign_key(
                    ForeignKey::create()
                        .from(ReadingRevisions::Table, ReadingRevisions::HouseholdId)
                        .to(Households::Table, Households::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_reading_revisions_meter_changed_at")
                .table(ReadingRevisions::Table)
                .col(ReadingRevisions::MeterId)
                .col(ReadingRevisions::ChangedAt)
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Readings::Table)
                .add_column(timestamp_with_time_zone_null(Readings::UpdatedAt))
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(Readings::Table)
                .drop_column(Readings::UpdatedAt)
                .to_owned(),
        )
        .await?;
        m.drop_table(Table::drop().table(ReadingRevisions::Table).to_owned())
            .await
    }
}
