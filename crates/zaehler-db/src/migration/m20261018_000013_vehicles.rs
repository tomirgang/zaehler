//! Fahrzeuge mit Verbrauchsprofilen, Anschaffung und Ladevorgängen sowie
//! das gemeinsame Kostenbuch (SPEC_CAR_VEHICLES, SPEC_ASSET_COSTS). Beträge
//! als Festkomma 10⁻⁶, Mengen, Kilometer und Prozente 10⁻³
//! (SPEC_STORE_FIXED_POINT). `asset_costs.asset_id` verweist je nach
//! `asset_type` auf ein Fahrzeug (Messstelle) oder ab M8 auf eine PV-Anlage
//! und hat daher keinen Fremdschlüssel; `document_id` ebenso nicht, weil
//! Dokumente nach dem Ablauf ohne Verweis entfernt werden.

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261018_000013_vehicles"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum Vehicles {
    Table,
    MeterId,
    Plate,
    Model,
    DriveType,
    HomeChargingMeterId,
    PvShareOverridePctMilli,
}

#[derive(DeriveIden, Clone, Copy)]
enum VehicleConsumptionProfiles {
    Table,
    Id,
    VehicleId,
    ValidFrom,
    #[sea_orm(iden = "kwh_per_100km_milli")]
    KwhPer100kmMilli,
    ChargingLossPctMilli,
    ElectricSharePctMilli,
}

#[derive(DeriveIden, Clone, Copy)]
enum VehicleAcquisitions {
    Table,
    VehicleId,
    PurchasedOn,
    PriceMicro,
    ResidualMicro,
    PlannedEndOn,
    SoldOn,
    SalePriceMicro,
}

#[derive(DeriveIden, Clone, Copy)]
enum ChargingSessions {
    Table,
    Id,
    VehicleId,
    ChargedAt,
    EnergyMilli,
    AmountMicro,
    OdometerMilli,
    Provider,
    Kind,
    Source,
    DocumentId,
}

#[derive(DeriveIden, Clone, Copy)]
enum AssetCosts {
    Table,
    Id,
    HouseholdId,
    AssetType,
    AssetId,
    BookedOn,
    Category,
    AmountMicro,
    QuantityMilli,
    OdometerMilli,
    FullTank,
    RecurrenceMonths,
    RecurrenceUntil,
    DocumentId,
    Note,
}

#[derive(DeriveIden)]
enum Meters {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Households {
    Table,
    Id,
}

fn fk(
    from: (impl IntoIden, impl IntoIden),
    to: (impl IntoIden, impl IntoIden),
    action: ForeignKeyAction,
) -> ForeignKeyCreateStatement {
    ForeignKey::create()
        .from(from.0, from.1)
        .to(to.0, to.1)
        .on_delete(action)
        .to_owned()
}

// @impl SPEC_CAR_VEHICLES, SPEC_ASSET_COSTS
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    #[allow(clippy::too_many_lines)] // eine Tabelle je Abschnitt
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Vehicles::Table)
                .if_not_exists()
                .col(pk_uuid(Vehicles::MeterId))
                .col(string(Vehicles::Plate))
                .col(string(Vehicles::Model))
                .col(string(Vehicles::DriveType))
                .col(uuid_null(Vehicles::HomeChargingMeterId))
                .col(big_integer_null(Vehicles::PvShareOverridePctMilli))
                .foreign_key(&mut fk(
                    (Vehicles::Table, Vehicles::MeterId),
                    (Meters::Table, Meters::Id),
                    ForeignKeyAction::Cascade,
                ))
                .foreign_key(&mut fk(
                    (Vehicles::Table, Vehicles::HomeChargingMeterId),
                    (Meters::Table, Meters::Id),
                    ForeignKeyAction::SetNull,
                ))
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(VehicleConsumptionProfiles::Table)
                .if_not_exists()
                .col(pk_uuid(VehicleConsumptionProfiles::Id))
                .col(uuid(VehicleConsumptionProfiles::VehicleId))
                .col(date(VehicleConsumptionProfiles::ValidFrom))
                .col(big_integer(VehicleConsumptionProfiles::KwhPer100kmMilli))
                .col(big_integer(
                    VehicleConsumptionProfiles::ChargingLossPctMilli,
                ))
                .col(big_integer_null(
                    VehicleConsumptionProfiles::ElectricSharePctMilli,
                ))
                .foreign_key(&mut fk(
                    (
                        VehicleConsumptionProfiles::Table,
                        VehicleConsumptionProfiles::VehicleId,
                    ),
                    (Vehicles::Table, Vehicles::MeterId),
                    ForeignKeyAction::Cascade,
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_vehicle_profiles_vehicle_valid_from")
                .table(VehicleConsumptionProfiles::Table)
                .col(VehicleConsumptionProfiles::VehicleId)
                .col(VehicleConsumptionProfiles::ValidFrom)
                .unique()
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(VehicleAcquisitions::Table)
                .if_not_exists()
                .col(pk_uuid(VehicleAcquisitions::VehicleId))
                .col(date(VehicleAcquisitions::PurchasedOn))
                .col(big_integer(VehicleAcquisitions::PriceMicro))
                .col(big_integer(VehicleAcquisitions::ResidualMicro))
                .col(date(VehicleAcquisitions::PlannedEndOn))
                .col(date_null(VehicleAcquisitions::SoldOn))
                .col(big_integer_null(VehicleAcquisitions::SalePriceMicro))
                .foreign_key(&mut fk(
                    (VehicleAcquisitions::Table, VehicleAcquisitions::VehicleId),
                    (Vehicles::Table, Vehicles::MeterId),
                    ForeignKeyAction::Cascade,
                ))
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(ChargingSessions::Table)
                .if_not_exists()
                .col(pk_uuid(ChargingSessions::Id))
                .col(uuid(ChargingSessions::VehicleId))
                .col(timestamp_with_time_zone(ChargingSessions::ChargedAt))
                .col(big_integer(ChargingSessions::EnergyMilli))
                .col(big_integer(ChargingSessions::AmountMicro))
                .col(big_integer_null(ChargingSessions::OdometerMilli))
                .col(string_null(ChargingSessions::Provider))
                .col(string(ChargingSessions::Kind))
                .col(string(ChargingSessions::Source))
                .col(uuid_null(ChargingSessions::DocumentId))
                .foreign_key(&mut fk(
                    (ChargingSessions::Table, ChargingSessions::VehicleId),
                    (Vehicles::Table, Vehicles::MeterId),
                    ForeignKeyAction::Cascade,
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_charging_sessions_vehicle_charged_at")
                .table(ChargingSessions::Table)
                .col(ChargingSessions::VehicleId)
                .col(ChargingSessions::ChargedAt)
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(AssetCosts::Table)
                .if_not_exists()
                .col(pk_uuid(AssetCosts::Id))
                .col(uuid(AssetCosts::HouseholdId))
                .col(string(AssetCosts::AssetType))
                .col(uuid(AssetCosts::AssetId))
                .col(date(AssetCosts::BookedOn))
                .col(string(AssetCosts::Category))
                .col(big_integer(AssetCosts::AmountMicro))
                .col(big_integer_null(AssetCosts::QuantityMilli))
                .col(big_integer_null(AssetCosts::OdometerMilli))
                .col(boolean_null(AssetCosts::FullTank))
                .col(small_integer_null(AssetCosts::RecurrenceMonths))
                .col(date_null(AssetCosts::RecurrenceUntil))
                .col(uuid_null(AssetCosts::DocumentId))
                .col(text_null(AssetCosts::Note))
                .foreign_key(&mut fk(
                    (AssetCosts::Table, AssetCosts::HouseholdId),
                    (Households::Table, Households::Id),
                    ForeignKeyAction::Cascade,
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_asset_costs_asset_booked_on")
                .table(AssetCosts::Table)
                .col(AssetCosts::HouseholdId)
                .col(AssetCosts::AssetType)
                .col(AssetCosts::AssetId)
                .col(AssetCosts::BookedOn)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            AssetCosts::Table.into_iden(),
            ChargingSessions::Table.into_iden(),
            VehicleAcquisitions::Table.into_iden(),
            VehicleConsumptionProfiles::Table.into_iden(),
            Vehicles::Table.into_iden(),
        ] {
            m.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
