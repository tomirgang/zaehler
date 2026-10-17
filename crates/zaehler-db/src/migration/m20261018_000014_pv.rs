//! PV-Anlagen mit Zuordnung der Messstellen (SPEC_PV_ASSIGNMENT, ADR_021).
//! Leistung, Kapazität, Ertrag und Prozente als Festkomma 10⁻³
//! (SPEC_STORE_FIXED_POINT). Dass eine Messstelle mit Rolle `generation`
//! oder `battery` zu genau einer Anlage gehört, prüft der Anwendungsfall; ein
//! Teilindex ist in SQLite und PostgreSQL nicht einheitlich über den
//! Query-Builder anlegbar.

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261018_000014_pv"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum PvSystems {
    Table,
    Id,
    HouseholdId,
    Name,
    Kind,
    KwpMilli,
    CommissionedOn,
    Azimuth,
    Tilt,
    Inverter,
    BatteryKwhMilli,
    SpecificYieldMilli,
    DegradationPctMilli,
    MonthShares,
    AlarmThresholdPct,
}

#[derive(DeriveIden, Clone, Copy)]
enum PvSystemMeters {
    Table,
    PvSystemId,
    MeterId,
    Role,
}

#[derive(DeriveIden)]
enum Households {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Meters {
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

// @impl SPEC_PV_ASSIGNMENT
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(PvSystems::Table)
                .if_not_exists()
                .col(pk_uuid(PvSystems::Id))
                .col(uuid(PvSystems::HouseholdId))
                .col(string(PvSystems::Name))
                .col(string(PvSystems::Kind))
                .col(big_integer(PvSystems::KwpMilli))
                .col(date(PvSystems::CommissionedOn))
                .col(small_integer_null(PvSystems::Azimuth))
                .col(small_integer_null(PvSystems::Tilt))
                .col(string_null(PvSystems::Inverter))
                .col(big_integer_null(PvSystems::BatteryKwhMilli))
                .col(big_integer(PvSystems::SpecificYieldMilli))
                .col(big_integer(PvSystems::DegradationPctMilli))
                .col(text_null(PvSystems::MonthShares))
                .col(small_integer(PvSystems::AlarmThresholdPct))
                .foreign_key(&mut cascade(
                    (PvSystems::Table, PvSystems::HouseholdId),
                    (Households::Table, Households::Id),
                ))
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(PvSystemMeters::Table)
                .if_not_exists()
                .col(uuid(PvSystemMeters::PvSystemId))
                .col(uuid(PvSystemMeters::MeterId))
                .col(string(PvSystemMeters::Role))
                .primary_key(
                    Index::create()
                        .col(PvSystemMeters::PvSystemId)
                        .col(PvSystemMeters::MeterId),
                )
                .foreign_key(&mut cascade(
                    (PvSystemMeters::Table, PvSystemMeters::PvSystemId),
                    (PvSystems::Table, PvSystems::Id),
                ))
                .foreign_key(&mut cascade(
                    (PvSystemMeters::Table, PvSystemMeters::MeterId),
                    (Meters::Table, Meters::Id),
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_pv_system_meters_meter")
                .table(PvSystemMeters::Table)
                .col(PvSystemMeters::MeterId)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(PvSystemMeters::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(PvSystems::Table).to_owned())
            .await
    }
}
