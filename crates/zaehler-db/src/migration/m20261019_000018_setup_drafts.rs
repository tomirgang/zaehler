//! Ein fortsetzbarer Setup-Entwurf je Haushalt (SPEC_SETUP_FLOW).

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261019_000018_setup_drafts"
    }
}

#[derive(DeriveIden)]
enum SetupDrafts {
    Table,
    HouseholdId,
    Payload,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Households {
    Table,
    Id,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(SetupDrafts::Table)
                .if_not_exists()
                .col(pk_uuid(SetupDrafts::HouseholdId))
                .col(text(SetupDrafts::Payload))
                .col(timestamp_with_time_zone(SetupDrafts::CreatedAt))
                .col(timestamp_with_time_zone(SetupDrafts::UpdatedAt))
                .foreign_key(
                    ForeignKey::create()
                        .from(SetupDrafts::Table, SetupDrafts::HouseholdId)
                        .to(Households::Table, Households::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(SetupDrafts::Table).to_owned())
            .await
    }
}
