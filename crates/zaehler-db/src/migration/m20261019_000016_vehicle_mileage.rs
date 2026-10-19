//! Wertverlust nach Lebenslaufleistung (SPEC_CAR_MILEAGE_UI).

#![allow(clippy::wildcard_imports)]

use sea_orm::{DbBackend, Statement, TransactionTrait, Value};
use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261019_000016_vehicle_mileage"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum VehicleAcquisitions {
    Table,
    DepreciationMode,
    OdometerAtPurchaseMilli,
    LifetimeKmMilli,
    EndValueMicro,
}

async fn execute_batch(db: &impl ConnectionTrait, sql: &str) -> Result<(), DbErr> {
    for statement in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            statement,
            std::iter::empty::<Value>(),
        ))
        .await?;
    }
    Ok(())
}

// @impl SPEC_CAR_MILEAGE_UI
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if m.get_database_backend() == DbBackend::Sqlite {
            let tx = m.get_connection().begin().await?;
            execute_batch(&tx,
                "CREATE TABLE vehicle_acquisitions_v11 (
                    vehicle_id uuid NOT NULL PRIMARY KEY REFERENCES vehicles(meter_id) ON DELETE CASCADE,
                    purchased_on date NOT NULL, price_micro bigint NOT NULL,
                    residual_micro bigint, planned_end_on date,
                    sold_on date, sale_price_micro bigint,
                    depreciation_mode text NOT NULL DEFAULT 'time',
                    odometer_at_purchase_milli bigint, lifetime_km_milli bigint, end_value_micro bigint
                );
                INSERT INTO vehicle_acquisitions_v11
                    (vehicle_id, purchased_on, price_micro, residual_micro, planned_end_on, sold_on, sale_price_micro)
                    SELECT vehicle_id, purchased_on, price_micro, residual_micro, planned_end_on, sold_on, sale_price_micro
                    FROM vehicle_acquisitions;
                DROP TABLE vehicle_acquisitions;
                ALTER TABLE vehicle_acquisitions_v11 RENAME TO vehicle_acquisitions;"
            ).await?;
            return tx.commit().await;
        }
        execute_batch(
            m.get_connection(),
            "ALTER TABLE vehicle_acquisitions ALTER COLUMN residual_micro DROP NOT NULL,
             ALTER COLUMN planned_end_on DROP NOT NULL",
        )
        .await?;
        for col in [
            string(VehicleAcquisitions::DepreciationMode)
                .default("time")
                .to_owned(),
            big_integer_null(VehicleAcquisitions::OdometerAtPurchaseMilli),
            big_integer_null(VehicleAcquisitions::LifetimeKmMilli),
            big_integer_null(VehicleAcquisitions::EndValueMicro),
        ] {
            let mut col = col;
            m.alter_table(
                Table::alter()
                    .table(VehicleAcquisitions::Table)
                    .add_column(&mut col)
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if m.get_database_backend() == DbBackend::Sqlite {
            let tx = m.get_connection().begin().await?;
            execute_batch(&tx,
                "CREATE TABLE vehicle_acquisitions_v1 (
                    vehicle_id uuid NOT NULL PRIMARY KEY REFERENCES vehicles(meter_id) ON DELETE CASCADE,
                    purchased_on date NOT NULL, price_micro bigint NOT NULL,
                    residual_micro bigint NOT NULL, planned_end_on date NOT NULL,
                    sold_on date, sale_price_micro bigint
                );
                INSERT INTO vehicle_acquisitions_v1
                    SELECT vehicle_id, purchased_on, price_micro, COALESCE(residual_micro, 0),
                           COALESCE(planned_end_on, purchased_on), sold_on, sale_price_micro
                    FROM vehicle_acquisitions;
                DROP TABLE vehicle_acquisitions;
                ALTER TABLE vehicle_acquisitions_v1 RENAME TO vehicle_acquisitions;"
            ).await?;
            return tx.commit().await;
        }
        execute_batch(
            m.get_connection(),
            "UPDATE vehicle_acquisitions SET residual_micro = COALESCE(residual_micro, 0),
             planned_end_on = COALESCE(planned_end_on, purchased_on);
             ALTER TABLE vehicle_acquisitions ALTER COLUMN residual_micro SET NOT NULL,
             ALTER COLUMN planned_end_on SET NOT NULL",
        )
        .await?;
        for col in [
            VehicleAcquisitions::EndValueMicro,
            VehicleAcquisitions::LifetimeKmMilli,
            VehicleAcquisitions::OdometerAtPurchaseMilli,
            VehicleAcquisitions::DepreciationMode,
        ] {
            m.alter_table(
                Table::alter()
                    .table(VehicleAcquisitions::Table)
                    .drop_column(col)
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
}
