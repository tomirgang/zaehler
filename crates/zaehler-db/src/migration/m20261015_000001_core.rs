//! Kernschema: Haushalte, Nutzer, Mitgliedschaften, Messstellen, Geräte,
//! Register und Zählerstände.

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261015_000001_core"
    }
}

// @impl SPEC_DM_CORE
#[allow(clippy::too_many_lines)]
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Households::Table)
                .if_not_exists()
                .col(pk_uuid(Households::Id))
                .col(string(Households::Name))
                .col(string(Households::Timezone).default("Europe/Berlin"))
                .col(timestamp_with_time_zone(Households::CreatedAt))
                .to_owned(),
        )
        .await?;

        m.create_table(
            Table::create()
                .table(Users::Table)
                .if_not_exists()
                .col(pk_uuid(Users::Id))
                .col(string(Users::OidcIssuer))
                .col(string(Users::OidcSubject))
                .col(string_null(Users::Email))
                .col(string(Users::DisplayName))
                .col(string(Users::Locale).default("de"))
                .col(timestamp_with_time_zone(Users::CreatedAt))
                .index(
                    Index::create()
                        .unique()
                        .col(Users::OidcIssuer)
                        .col(Users::OidcSubject),
                )
                .to_owned(),
        )
        .await?;

        m.create_table(
            Table::create()
                .table(Memberships::Table)
                .if_not_exists()
                .col(uuid(Memberships::HouseholdId))
                .col(uuid(Memberships::UserId))
                .col(string(Memberships::Role))
                .primary_key(
                    Index::create()
                        .col(Memberships::HouseholdId)
                        .col(Memberships::UserId),
                )
                .foreign_key(&mut fk(
                    Memberships::Table,
                    Memberships::HouseholdId,
                    Households::Table,
                    Households::Id,
                ))
                .foreign_key(&mut fk(
                    Memberships::Table,
                    Memberships::UserId,
                    Users::Table,
                    Users::Id,
                ))
                .to_owned(),
        )
        .await?;

        m.create_table(
            Table::create()
                .table(Meters::Table)
                .if_not_exists()
                .col(pk_uuid(Meters::Id))
                .col(uuid(Meters::HouseholdId))
                .col(string(Meters::Name))
                .col(string(Meters::Kind))
                .col(string(Meters::Unit))
                .col(integer_null(Meters::ReadingIntervalDays))
                .col(big_integer_null(Meters::MaxDailyRateMilli))
                .col(boolean(Meters::Archived).default(false))
                .col(timestamp_with_time_zone(Meters::CreatedAt))
                .foreign_key(&mut fk(
                    Meters::Table,
                    Meters::HouseholdId,
                    Households::Table,
                    Households::Id,
                ))
                .to_owned(),
        )
        .await?;

        m.create_table(
            Table::create()
                .table(MeterDevices::Table)
                .if_not_exists()
                .col(pk_uuid(MeterDevices::Id))
                .col(uuid(MeterDevices::MeterId))
                .col(string_null(MeterDevices::SerialNumber))
                .col(timestamp_with_time_zone(MeterDevices::InstalledAt))
                .col(timestamp_with_time_zone_null(MeterDevices::RemovedAt))
                .col(small_integer_null(MeterDevices::RolloverDigits))
                .foreign_key(&mut fk(
                    MeterDevices::Table,
                    MeterDevices::MeterId,
                    Meters::Table,
                    Meters::Id,
                ))
                .to_owned(),
        )
        .await?;

        m.create_table(
            Table::create()
                .table(Registers::Table)
                .if_not_exists()
                .col(pk_uuid(Registers::Id))
                .col(uuid(Registers::MeterId))
                .col(string(Registers::Code))
                .col(string(Registers::Direction))
                .index(
                    Index::create()
                        .unique()
                        .col(Registers::MeterId)
                        .col(Registers::Code),
                )
                .foreign_key(&mut fk(
                    Registers::Table,
                    Registers::MeterId,
                    Meters::Table,
                    Meters::Id,
                ))
                .to_owned(),
        )
        .await?;

        m.create_table(
            Table::create()
                .table(Readings::Table)
                .if_not_exists()
                .col(pk_uuid(Readings::Id))
                .col(uuid(Readings::RegisterId))
                .col(uuid(Readings::DeviceId))
                .col(timestamp_with_time_zone(Readings::ReadAt))
                .col(big_integer(Readings::ValueMilli))
                .col(string(Readings::Source))
                .col(string_null(Readings::PhotoKey))
                .col(double_null(Readings::AiConfidence))
                .col(string_null(Readings::Note))
                .col(uuid_null(Readings::CreatedBy))
                .col(timestamp_with_time_zone(Readings::CreatedAt))
                .foreign_key(&mut fk(
                    Readings::Table,
                    Readings::RegisterId,
                    Registers::Table,
                    Registers::Id,
                ))
                .foreign_key(&mut fk(
                    Readings::Table,
                    Readings::DeviceId,
                    MeterDevices::Table,
                    MeterDevices::Id,
                ))
                .foreign_key(&mut fk(
                    Readings::Table,
                    Readings::CreatedBy,
                    Users::Table,
                    Users::Id,
                ))
                .to_owned(),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("idx_readings_register_read_at")
                .table(Readings::Table)
                .col(Readings::RegisterId)
                .col(Readings::ReadAt)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            Readings::Table.into_iden(),
            Registers::Table.into_iden(),
            MeterDevices::Table.into_iden(),
            Meters::Table.into_iden(),
            Memberships::Table.into_iden(),
            Users::Table.into_iden(),
            Households::Table.into_iden(),
        ] {
            m.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}

fn fk<T: IntoIden + Copy, C: IntoIden, RT: IntoIden, RC: IntoIden>(
    table: T,
    col: C,
    ref_table: RT,
    ref_col: RC,
) -> ForeignKeyCreateStatement {
    ForeignKey::create()
        .from(table, col)
        .to(ref_table, ref_col)
        .on_delete(ForeignKeyAction::Cascade)
        .to_owned()
}

#[derive(DeriveIden, Clone, Copy)]
enum Households {
    Table,
    Id,
    Name,
    Timezone,
    CreatedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum Users {
    Table,
    Id,
    OidcIssuer,
    OidcSubject,
    Email,
    DisplayName,
    Locale,
    CreatedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum Memberships {
    Table,
    HouseholdId,
    UserId,
    Role,
}

#[derive(DeriveIden, Clone, Copy)]
enum Meters {
    Table,
    Id,
    HouseholdId,
    Name,
    Kind,
    Unit,
    ReadingIntervalDays,
    MaxDailyRateMilli,
    Archived,
    CreatedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum MeterDevices {
    Table,
    Id,
    MeterId,
    SerialNumber,
    InstalledAt,
    RemovedAt,
    RolloverDigits,
}

#[derive(DeriveIden, Clone, Copy)]
enum Registers {
    Table,
    Id,
    MeterId,
    Code,
    Direction,
}

#[derive(DeriveIden, Clone, Copy)]
enum Readings {
    Table,
    Id,
    RegisterId,
    DeviceId,
    ReadAt,
    ValueMilli,
    Source,
    PhotoKey,
    AiConfidence,
    Note,
    CreatedBy,
    CreatedAt,
}
