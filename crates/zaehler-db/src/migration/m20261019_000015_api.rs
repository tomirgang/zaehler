//! API-Tokens und Token automatisch gelieferter Ablesungen (SPEC_API_AUTH,
//! SPEC_API_DAILY_READINGS). `readings.api_token_id` hat keinen
//! Fremdschlüssel, da SQLite beim Hinzufügen einer Spalte keinen anlegen
//! kann; Tokens werden nur widerrufen, nie gelöscht, außer mit dem Haushalt.

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261019_000015_api"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum ApiTokens {
    Table,
    Id,
    HouseholdId,
    UserId,
    Name,
    Role,
    TokenHash,
    Prefix,
    CreatedAt,
    ExpiresAt,
    RevokedAt,
    LastUsedAt,
}

#[derive(DeriveIden)]
enum Readings {
    Table,
    RegisterId,
    ReadAt,
    ApiTokenId,
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

fn cascade(
    from: (impl IntoIden, impl IntoIden),
    to: (impl IntoIden, impl IntoIden),
) -> ForeignKeyCreateStatement {
    ForeignKey::create()
        .from(from.0, from.1)
        .to(to.0, to.1)
        .on_delete(ForeignKeyAction::Cascade)
        .to_owned()
}

// @impl SPEC_API_AUTH, SPEC_API_DAILY_READINGS
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(ApiTokens::Table)
                .if_not_exists()
                .col(pk_uuid(ApiTokens::Id))
                .col(uuid(ApiTokens::HouseholdId))
                .col(uuid(ApiTokens::UserId))
                .col(string(ApiTokens::Name))
                .col(string(ApiTokens::Role))
                .col(string_uniq(ApiTokens::TokenHash))
                .col(string(ApiTokens::Prefix))
                .col(timestamp_with_time_zone(ApiTokens::CreatedAt))
                .col(timestamp_with_time_zone_null(ApiTokens::ExpiresAt))
                .col(timestamp_with_time_zone_null(ApiTokens::RevokedAt))
                .col(timestamp_with_time_zone_null(ApiTokens::LastUsedAt))
                .foreign_key(&mut cascade(
                    (ApiTokens::Table, ApiTokens::HouseholdId),
                    (Households::Table, Households::Id),
                ))
                .foreign_key(&mut cascade(
                    (ApiTokens::Table, ApiTokens::UserId),
                    (Users::Table, Users::Id),
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_api_tokens_household")
                .table(ApiTokens::Table)
                .col(ApiTokens::HouseholdId)
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Readings::Table)
                .add_column(uuid_null(Readings::ApiTokenId))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_readings_api_token_day")
                .table(Readings::Table)
                .col(Readings::ApiTokenId)
                .col(Readings::RegisterId)
                .col(Readings::ReadAt)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_index(
            Index::drop()
                .name("idx_readings_api_token_day")
                .table(Readings::Table)
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(Readings::Table)
                .drop_column(Readings::ApiTokenId)
                .to_owned(),
        )
        .await?;
        m.drop_table(Table::drop().table(ApiTokens::Table).to_owned())
            .await
    }
}
