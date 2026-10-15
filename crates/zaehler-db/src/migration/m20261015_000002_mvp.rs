//! MVP-Erweiterungen: Abrechnungsjahr je Messstelle, Nachkommastellen je
//! Gerät, letzter Login, Eindeutigkeit von Ablesungen, Sitzungen.
//! SQLite erlaubt nur eine Spaltenänderung je `ALTER TABLE`.

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261015_000002_mvp"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum Meters {
    Table,
    BillingStartMonth,
    BillingStartDay,
}

#[derive(DeriveIden, Clone, Copy)]
enum MeterDevices {
    Table,
    SerialNumber,
    DecimalPlaces,
}

#[derive(DeriveIden, Clone, Copy)]
enum Users {
    Table,
    LastLoginAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum Readings {
    Table,
    RegisterId,
    DeviceId,
    ReadAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum Sessions {
    Table,
    Id,
    Data,
    ExpiresAt,
}

async fn add_column(
    m: &SchemaManager<'_>,
    table: impl IntoIden,
    col: &mut ColumnDef,
) -> Result<(), DbErr> {
    m.alter_table(Table::alter().table(table).add_column(col).to_owned())
        .await
}

// @impl SPEC_DM_CORE, SPEC_SEC_SESSION_CSRF
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        add_column(
            m,
            Meters::Table,
            &mut small_integer_null(Meters::BillingStartMonth),
        )
        .await?;
        add_column(
            m,
            Meters::Table,
            &mut small_integer_null(Meters::BillingStartDay),
        )
        .await?;
        add_column(
            m,
            MeterDevices::Table,
            small_integer(MeterDevices::DecimalPlaces).default(0),
        )
        .await?;
        add_column(
            m,
            Users::Table,
            &mut timestamp_with_time_zone_null(Users::LastLoginAt),
        )
        .await?;

        m.create_index(
            Index::create()
                .name("uq_readings_register_device_read_at")
                .table(Readings::Table)
                .col(Readings::RegisterId)
                .col(Readings::DeviceId)
                .col(Readings::ReadAt)
                .unique()
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_meter_devices_serial_number")
                .table(MeterDevices::Table)
                .col(MeterDevices::SerialNumber)
                .to_owned(),
        )
        .await?;

        m.create_table(
            Table::create()
                .table(Sessions::Table)
                .if_not_exists()
                .col(string(Sessions::Id).primary_key())
                .col(blob(Sessions::Data))
                .col(timestamp_with_time_zone(Sessions::ExpiresAt))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_sessions_expires_at")
                .table(Sessions::Table)
                .col(Sessions::ExpiresAt)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(Sessions::Table).to_owned())
            .await?;
        m.drop_index(
            Index::drop()
                .name("idx_meter_devices_serial_number")
                .table(MeterDevices::Table)
                .to_owned(),
        )
        .await?;
        m.drop_index(
            Index::drop()
                .name("uq_readings_register_device_read_at")
                .table(Readings::Table)
                .to_owned(),
        )
        .await?;
        for (table, col) in [
            (Users::Table.into_iden(), Users::LastLoginAt.into_iden()),
            (
                MeterDevices::Table.into_iden(),
                MeterDevices::DecimalPlaces.into_iden(),
            ),
            (
                Meters::Table.into_iden(),
                Meters::BillingStartDay.into_iden(),
            ),
            (
                Meters::Table.into_iden(),
                Meters::BillingStartMonth.into_iden(),
            ),
        ] {
            m.alter_table(Table::alter().table(table).drop_column(col).to_owned())
                .await?;
        }
        Ok(())
    }
}
