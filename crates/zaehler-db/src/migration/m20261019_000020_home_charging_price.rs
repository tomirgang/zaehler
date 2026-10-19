//! Strompreis beim Heimladen ohne Vertragspreis (SPEC_CAR_ENERGY_COST).

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261019_000020_home_charging_price"
    }
}

#[derive(DeriveIden)]
enum Vehicles {
    Table,
    HomeChargingPriceMicro,
}

// @impl SPEC_CAR_ENERGY_COST
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(Vehicles::Table)
                .add_column(big_integer_null(Vehicles::HomeChargingPriceMicro))
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(Vehicles::Table)
                .drop_column(Vehicles::HomeChargingPriceMicro)
                .to_owned(),
        )
        .await
    }
}
