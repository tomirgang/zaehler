//! Jahresziel, Toleranz und angepasstes Saisonprofil je Messstelle
//! (SPEC_ANA_TRAFFIC_LIGHT). Das gelernte Profil wird nicht gespeichert.
//! SQLite erlaubt nur eine Spaltenänderung je `ALTER TABLE`.

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261016_000010_expectation"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum Meters {
    Table,
    SeasonWeights,
    AnnualTargetMilli,
    TargetTolerancePct,
}

// @impl SPEC_ANA_TRAFFIC_LIGHT
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for col in [
            text_null(Meters::SeasonWeights),
            big_integer_null(Meters::AnnualTargetMilli),
            small_integer_null(Meters::TargetTolerancePct),
        ] {
            let mut col = col;
            m.alter_table(
                Table::alter()
                    .table(Meters::Table)
                    .add_column(&mut col)
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for col in [
            Meters::TargetTolerancePct,
            Meters::AnnualTargetMilli,
            Meters::SeasonWeights,
        ] {
            m.alter_table(
                Table::alter()
                    .table(Meters::Table)
                    .drop_column(col)
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
}
