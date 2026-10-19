//! Preis pro Liter im Kostenbuch und Energieart der Verbrauchsprofile
//! (SPEC_CAR_FUEL_PRICE_UI).

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261019_000017_fuel_price"
    }
}

#[derive(DeriveIden)]
enum AssetCosts {
    Table,
    UnitPriceMicro,
}

#[derive(DeriveIden)]
enum VehicleConsumptionProfiles {
    Table,
    VehicleId,
    ValidFrom,
    Energy,
}

const OLD_INDEX: &str = "idx_vehicle_profiles_vehicle_valid_from";
const NEW_INDEX: &str = "idx_vehicle_profiles_vehicle_energy_valid_from";

/// Eindeutiger Beginn je Fahrzeug, mit `energy` je Energieart.
fn profile_index(name: &str, with_energy: bool) -> IndexCreateStatement {
    let mut i = Index::create();
    i.name(name)
        .table(VehicleConsumptionProfiles::Table)
        .col(VehicleConsumptionProfiles::VehicleId);
    if with_energy {
        i.col(VehicleConsumptionProfiles::Energy);
    }
    i.col(VehicleConsumptionProfiles::ValidFrom)
        .unique()
        .to_owned()
}

// @impl SPEC_CAR_FUEL_PRICE_UI
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(AssetCosts::Table)
                .add_column(big_integer_null(AssetCosts::UnitPriceMicro))
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(VehicleConsumptionProfiles::Table)
                .add_column(string(VehicleConsumptionProfiles::Energy).default("electric"))
                .to_owned(),
        )
        .await?;
        m.drop_index(
            Index::drop()
                .name(OLD_INDEX)
                .table(VehicleConsumptionProfiles::Table)
                .to_owned(),
        )
        .await?;
        m.create_index(profile_index(NEW_INDEX, true)).await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_index(
            Index::drop()
                .name(NEW_INDEX)
                .table(VehicleConsumptionProfiles::Table)
                .to_owned(),
        )
        .await?;
        m.create_index(profile_index(OLD_INDEX, false)).await?;
        m.alter_table(
            Table::alter()
                .table(VehicleConsumptionProfiles::Table)
                .drop_column(VehicleConsumptionProfiles::Energy)
                .to_owned(),
        )
        .await?;
        m.alter_table(
            Table::alter()
                .table(AssetCosts::Table)
                .drop_column(AssetCosts::UnitPriceMicro)
                .to_owned(),
        )
        .await
    }
}
