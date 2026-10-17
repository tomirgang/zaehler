//! Verträge mit Kostenpositionen, Preisständen, Abschlägen, Anpassungen und
//! Abrechnungen (SPEC_TAR_CONTRACTS, SPEC_TAR_ACTUAL_SETTLEMENT). Beträge
//! und Preise als Festkomma 10⁻⁶, Mengen 10⁻³ (SPEC_STORE_FIXED_POINT).
//! `settlements.document_id` hat keinen Fremdschlüssel, weil die Tabelle
//! `documents` erst mit `…_000012_documents` entsteht und SQLite ihn nicht
//! nachträglich anlegen kann.

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261017_000011_tariffs"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum Contracts {
    Table,
    Id,
    MeterId,
    Direction,
    Provider,
    ContractNumber,
    StartsOn,
    EndsOn,
    BillingStartMonth,
    BillingStartDay,
    Note,
}

#[derive(DeriveIden, Clone, Copy)]
enum CostPositions {
    Table,
    Id,
    ContractId,
    Name,
    RegisterId,
    FactorMicro,
    VatPctMilli,
    Sort,
}

#[derive(DeriveIden, Clone, Copy)]
enum PriceComponents {
    Table,
    Id,
    PositionId,
    ValidFrom,
    BasePriceMicro,
    BaseInterval,
    UnitPricing,
    FactorMicro,
}

#[derive(DeriveIden, Clone, Copy)]
enum Installments {
    Table,
    Id,
    ContractId,
    ValidFrom,
    AmountMicro,
    IntervalMonths,
}

#[derive(DeriveIden, Clone, Copy)]
enum Adjustments {
    Table,
    Id,
    ContractId,
    Kind,
    BookedOn,
    AmountMicro,
    Note,
}

#[derive(DeriveIden, Clone, Copy)]
enum Settlements {
    Table,
    Id,
    ContractId,
    PeriodStart,
    PeriodEnd,
    AmountMicro,
    DocumentId,
}

#[derive(DeriveIden)]
enum Meters {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Registers {
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

// @impl SPEC_TAR_CONTRACTS, SPEC_TAR_ACTUAL_SETTLEMENT
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    #[allow(clippy::too_many_lines)] // eine Tabelle je Abschnitt
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Contracts::Table)
                .if_not_exists()
                .col(pk_uuid(Contracts::Id))
                .col(uuid(Contracts::MeterId))
                .col(string(Contracts::Direction))
                .col(string(Contracts::Provider))
                .col(string_null(Contracts::ContractNumber))
                .col(date(Contracts::StartsOn))
                .col(date_null(Contracts::EndsOn))
                .col(small_integer(Contracts::BillingStartMonth))
                .col(small_integer(Contracts::BillingStartDay))
                .col(text_null(Contracts::Note))
                .foreign_key(&mut cascade(
                    (Contracts::Table, Contracts::MeterId),
                    (Meters::Table, Meters::Id),
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_contracts_meter_starts_on")
                .table(Contracts::Table)
                .col(Contracts::MeterId)
                .col(Contracts::StartsOn)
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(CostPositions::Table)
                .if_not_exists()
                .col(pk_uuid(CostPositions::Id))
                .col(uuid(CostPositions::ContractId))
                .col(string(CostPositions::Name))
                .col(uuid_null(CostPositions::RegisterId))
                .col(big_integer(CostPositions::FactorMicro))
                .col(big_integer_null(CostPositions::VatPctMilli))
                .col(integer(CostPositions::Sort))
                .foreign_key(&mut cascade(
                    (CostPositions::Table, CostPositions::ContractId),
                    (Contracts::Table, Contracts::Id),
                ))
                .foreign_key(&mut cascade(
                    (CostPositions::Table, CostPositions::RegisterId),
                    (Registers::Table, Registers::Id),
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_cost_positions_contract")
                .table(CostPositions::Table)
                .col(CostPositions::ContractId)
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(PriceComponents::Table)
                .if_not_exists()
                .col(pk_uuid(PriceComponents::Id))
                .col(uuid(PriceComponents::PositionId))
                .col(date(PriceComponents::ValidFrom))
                .col(big_integer(PriceComponents::BasePriceMicro))
                .col(string(PriceComponents::BaseInterval))
                .col(text(PriceComponents::UnitPricing))
                .col(big_integer_null(PriceComponents::FactorMicro))
                .foreign_key(&mut cascade(
                    (PriceComponents::Table, PriceComponents::PositionId),
                    (CostPositions::Table, CostPositions::Id),
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_price_components_position_valid_from")
                .table(PriceComponents::Table)
                .col(PriceComponents::PositionId)
                .col(PriceComponents::ValidFrom)
                .unique()
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(Installments::Table)
                .if_not_exists()
                .col(pk_uuid(Installments::Id))
                .col(uuid(Installments::ContractId))
                .col(date(Installments::ValidFrom))
                .col(big_integer(Installments::AmountMicro))
                .col(small_integer(Installments::IntervalMonths))
                .foreign_key(&mut cascade(
                    (Installments::Table, Installments::ContractId),
                    (Contracts::Table, Contracts::Id),
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_installments_contract_valid_from")
                .table(Installments::Table)
                .col(Installments::ContractId)
                .col(Installments::ValidFrom)
                .unique()
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(Adjustments::Table)
                .if_not_exists()
                .col(pk_uuid(Adjustments::Id))
                .col(uuid(Adjustments::ContractId))
                .col(string(Adjustments::Kind))
                .col(date(Adjustments::BookedOn))
                .col(big_integer(Adjustments::AmountMicro))
                .col(text_null(Adjustments::Note))
                .foreign_key(&mut cascade(
                    (Adjustments::Table, Adjustments::ContractId),
                    (Contracts::Table, Contracts::Id),
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_adjustments_contract")
                .table(Adjustments::Table)
                .col(Adjustments::ContractId)
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(Settlements::Table)
                .if_not_exists()
                .col(pk_uuid(Settlements::Id))
                .col(uuid(Settlements::ContractId))
                .col(date(Settlements::PeriodStart))
                .col(date(Settlements::PeriodEnd))
                .col(big_integer(Settlements::AmountMicro))
                .col(uuid_null(Settlements::DocumentId))
                .foreign_key(&mut cascade(
                    (Settlements::Table, Settlements::ContractId),
                    (Contracts::Table, Contracts::Id),
                ))
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_settlements_contract")
                .table(Settlements::Table)
                .col(Settlements::ContractId)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            Settlements::Table.into_iden(),
            Adjustments::Table.into_iden(),
            Installments::Table.into_iden(),
            PriceComponents::Table.into_iden(),
            CostPositions::Table.into_iden(),
            Contracts::Table.into_iden(),
        ] {
            m.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
