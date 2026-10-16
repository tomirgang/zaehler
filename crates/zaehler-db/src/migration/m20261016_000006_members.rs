//! Einladungen in einen Haushalt (SPEC_TEN_INVITES).

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261016_000006_members"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum Invitations {
    Table,
    Id,
    HouseholdId,
    Role,
    TokenHash,
    Email,
    CreatedBy,
    CreatedAt,
    ExpiresAt,
    AcceptedBy,
    AcceptedAt,
    RevokedAt,
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

// @impl SPEC_TEN_INVITES
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Invitations::Table)
                .if_not_exists()
                .col(pk_uuid(Invitations::Id))
                .col(uuid(Invitations::HouseholdId))
                .col(string(Invitations::Role))
                .col(string(Invitations::TokenHash))
                .col(string_null(Invitations::Email))
                .col(uuid(Invitations::CreatedBy))
                .col(timestamp_with_time_zone(Invitations::CreatedAt))
                .col(timestamp_with_time_zone(Invitations::ExpiresAt))
                .col(uuid_null(Invitations::AcceptedBy))
                .col(timestamp_with_time_zone_null(Invitations::AcceptedAt))
                .col(timestamp_with_time_zone_null(Invitations::RevokedAt))
                .foreign_key(
                    ForeignKey::create()
                        .from(Invitations::Table, Invitations::HouseholdId)
                        .to(Households::Table, Households::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(Invitations::Table, Invitations::CreatedBy)
                        .to(Users::Table, Users::Id),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(Invitations::Table, Invitations::AcceptedBy)
                        .to(Users::Table, Users::Id),
                )
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("uq_invitations_token_hash")
                .table(Invitations::Table)
                .col(Invitations::TokenHash)
                .unique()
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_invitations_household_id")
                .table(Invitations::Table)
                .col(Invitations::HouseholdId)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(Invitations::Table).to_owned())
            .await
    }
}
