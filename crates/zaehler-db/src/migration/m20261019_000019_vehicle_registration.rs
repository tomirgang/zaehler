//! Erstzulassung für die Restwertkurve (SPEC_CAR_DEPRECIATION).

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261019_000019_vehicle_registration"
    }
}

#[derive(DeriveIden)]
enum VehicleAcquisitions {
    Table,
    FirstRegistrationOn,
}

// @impl SPEC_CAR_DEPRECIATION
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(VehicleAcquisitions::Table)
                .add_column(date_null(VehicleAcquisitions::FirstRegistrationOn))
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(VehicleAcquisitions::Table)
                .drop_column(VehicleAcquisitions::FirstRegistrationOn)
                .to_owned(),
        )
        .await
    }
}
