//! Messstellen v1: Verwendungszweck, Ableseplan, Zeitpunkt der Archivierung
//! und übergeordnete Messstelle eines Geräts (SPEC_RDG_SCHEDULE,
//! SPEC_MTR_ARCHIVE, SPEC_MTR_SUBMETER). Ohne `schedule_kind` gilt weiter
//! `reading_interval_days`; ältere Programmversionen lesen die Spalte
//! unverändert. SQLite erlaubt nur eine Spaltenänderung je `ALTER TABLE`.

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261016_000008_meter_v1"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum Meters {
    Table,
    Purpose,
    ScheduleKind,
    ScheduleValue,
    ArchivedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum MeterDevices {
    Table,
    ParentMeterId,
}

async fn add_column(
    m: &SchemaManager<'_>,
    table: impl IntoIden,
    col: &mut ColumnDef,
) -> Result<(), DbErr> {
    m.alter_table(Table::alter().table(table).add_column(col).to_owned())
        .await
}

// @impl SPEC_RDG_SCHEDULE, SPEC_MTR_ARCHIVE, SPEC_MTR_SUBMETER
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        add_column(m, Meters::Table, string(Meters::Purpose).default("general")).await?;
        add_column(m, Meters::Table, &mut string_null(Meters::ScheduleKind)).await?;
        add_column(
            m,
            Meters::Table,
            &mut small_integer_null(Meters::ScheduleValue),
        )
        .await?;
        add_column(
            m,
            Meters::Table,
            &mut timestamp_with_time_zone_null(Meters::ArchivedAt),
        )
        .await?;
        // ohne Fremdschlüssel: SQLite kann ihn nicht nachträglich anlegen;
        // die Zuordnung prüft der Anwendungsfall
        add_column(
            m,
            MeterDevices::Table,
            &mut uuid_null(MeterDevices::ParentMeterId),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_meter_devices_parent_meter_id")
                .table(MeterDevices::Table)
                .col(MeterDevices::ParentMeterId)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_index(
            Index::drop()
                .name("idx_meter_devices_parent_meter_id")
                .table(MeterDevices::Table)
                .to_owned(),
        )
        .await?;
        for (table, col) in [
            (
                MeterDevices::Table.into_iden(),
                MeterDevices::ParentMeterId.into_iden(),
            ),
            (Meters::Table.into_iden(), Meters::ArchivedAt.into_iden()),
            (Meters::Table.into_iden(), Meters::ScheduleValue.into_iden()),
            (Meters::Table.into_iden(), Meters::ScheduleKind.into_iden()),
            (Meters::Table.into_iden(), Meters::Purpose.into_iden()),
        ] {
            m.alter_table(Table::alter().table(table).drop_column(col).to_owned())
                .await?;
        }
        Ok(())
    }
}
