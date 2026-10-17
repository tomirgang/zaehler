//! Verträge auf SeaORM (SPEC_TAR_CONTRACTS). Der Haushalt eines Vertrags
//! ergibt sich aus seiner Messstelle (SPEC_SEC_TENANT_SCOPE); abhängige
//! Zeilen werden über `ON DELETE CASCADE` gelöscht.

use async_trait::async_trait;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::Query,
};
use uuid::Uuid;
use zaehler_app::{
    PortError, Tenant,
    ids::{
        AdjustmentId, ContractId, InstallmentId, MeterId, PositionId, PriceComponentId, RegisterId,
        SettlementId,
    },
    model::{
        AdjustmentRow, Contract, ContractData, ContractEntry, InstallmentRow, MonthDay, Position,
        PriceComponentRow, SettlementRow,
    },
    port::{ContractRepository, PortResult},
};
use zaehler_domain::tariff::{Adjustment, Installment, PriceComponent};

use super::{SeaOrmUow, data_err, db_err};
use crate::{
    entity::{
        adjustment, contract, cost_position, installment, meter, price_component, settlement,
    },
    fixed,
};

fn small(v: i16) -> PortResult<u8> {
    u8::try_from(v).map_err(data_err)
}

fn money(v: rust_decimal::Decimal) -> PortResult<i64> {
    fixed::money_to_db(v).map_err(data_err)
}

fn component_model(
    position: Uuid,
    c: &PriceComponentRow,
) -> PortResult<price_component::ActiveModel> {
    let x = &c.component;
    Ok(price_component::ActiveModel {
        id: Set(c.id.0),
        position_id: Set(position),
        valid_from: Set(x.valid_from),
        base_price_micro: Set(money(x.base_price)?),
        base_interval: Set(x.base_interval.as_str().to_owned()),
        unit_pricing: Set(serde_json::to_string(&x.unit_pricing).map_err(data_err)?),
        factor_micro: Set(x.factor.map(money).transpose()?),
    })
}

impl SeaOrmUow {
    /// Verträge, die `cond` erfüllen, mit allen Einträgen.
    #[allow(clippy::too_many_lines)] // Abbildung aller abhängigen Tabellen
    async fn load_contracts(&self, cond: Condition) -> PortResult<Vec<Contract>> {
        let rows = contract::Entity::find()
            .filter(cond)
            .order_by_asc(contract::Column::MeterId)
            .order_by_asc(contract::Column::StartsOn)
            .order_by_asc(contract::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = rows.iter().map(|c| c.id).collect();
        let positions = cost_position::Entity::find()
            .filter(cost_position::Column::ContractId.is_in(ids.iter().copied()))
            .order_by_asc(cost_position::Column::Sort)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let components = price_component::Entity::find()
            .filter(price_component::Column::PositionId.is_in(positions.iter().map(|p| p.id)))
            .order_by_asc(price_component::Column::ValidFrom)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let installments = installment::Entity::find()
            .filter(installment::Column::ContractId.is_in(ids.iter().copied()))
            .order_by_asc(installment::Column::ValidFrom)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let adjustments = adjustment::Entity::find()
            .filter(adjustment::Column::ContractId.is_in(ids.iter().copied()))
            .order_by_asc(adjustment::Column::BookedOn)
            .order_by_asc(adjustment::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let settlements = settlement::Entity::find()
            .filter(settlement::Column::ContractId.is_in(ids.iter().copied()))
            .order_by_asc(settlement::Column::PeriodStart)
            .order_by_asc(settlement::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        rows.into_iter()
            .map(|c| {
                let mut ps = Vec::new();
                for p in positions.iter().filter(|p| p.contract_id == c.id) {
                    let mut cs = Vec::new();
                    for x in components.iter().filter(|x| x.position_id == p.id) {
                        cs.push(PriceComponentRow {
                            id: PriceComponentId(x.id),
                            component: PriceComponent {
                                valid_from: x.valid_from,
                                base_price: fixed::money_from_db(x.base_price_micro),
                                base_interval: x.base_interval.parse().map_err(data_err)?,
                                unit_pricing: serde_json::from_str(&x.unit_pricing)
                                    .map_err(data_err)?,
                                factor: x.factor_micro.map(fixed::money_from_db),
                            },
                        });
                    }
                    ps.push(Position {
                        id: PositionId(p.id),
                        name: p.name.clone(),
                        register_id: p.register_id.map(RegisterId),
                        factor: fixed::money_from_db(p.factor_micro),
                        vat_pct: p.vat_pct_milli.map(fixed::quantity_from_db),
                        components: cs,
                    });
                }
                let mut inst = Vec::new();
                for x in installments.iter().filter(|x| x.contract_id == c.id) {
                    inst.push(InstallmentRow {
                        id: InstallmentId(x.id),
                        installment: Installment {
                            amount: fixed::money_from_db(x.amount_micro),
                            valid_from: x.valid_from,
                            every_months: small(x.interval_months)?,
                        },
                    });
                }
                let mut adj = Vec::new();
                for x in adjustments.iter().filter(|x| x.contract_id == c.id) {
                    adj.push(AdjustmentRow {
                        id: AdjustmentId(x.id),
                        adjustment: Adjustment {
                            kind: x.kind.parse().map_err(data_err)?,
                            booked_on: x.booked_on,
                            amount: fixed::money_from_db(x.amount_micro),
                        },
                        note: x.note.clone(),
                    });
                }
                Ok(Contract {
                    id: ContractId(c.id),
                    meter_id: MeterId(c.meter_id),
                    data: ContractData {
                        direction: c.direction.parse().map_err(data_err)?,
                        provider: c.provider,
                        contract_number: c.contract_number,
                        starts_on: c.starts_on,
                        ends_on: c.ends_on,
                        billing_start: MonthDay {
                            month: small(c.billing_start_month)?,
                            day: small(c.billing_start_day)?,
                        },
                        note: c.note,
                        positions: ps,
                    },
                    installments: inst,
                    adjustments: adj,
                    settlements: settlements
                        .iter()
                        .filter(|x| x.contract_id == c.id)
                        .map(|x| SettlementRow {
                            id: SettlementId(x.id),
                            period_start: x.period_start,
                            period_end: x.period_end,
                            amount: fixed::money_from_db(x.amount_micro),
                            document_id: x.document_id,
                        })
                        .collect(),
                })
            })
            .collect()
    }

    async fn household_meters(&self, t: &Tenant) -> PortResult<Vec<Uuid>> {
        meter::Entity::find()
            .select_only()
            .column(meter::Column::Id)
            .filter(meter::Column::HouseholdId.eq(t.household_id().0))
            .into_tuple::<Uuid>()
            .all(&self.txn)
            .await
            .map_err(db_err)
    }

    /// Messstelle eines Vertrags des Tenants.
    async fn contract_meter(&self, t: &Tenant, id: ContractId) -> PortResult<Uuid> {
        let c = contract::Entity::find_by_id(id.0)
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .ok_or(PortError::NotFound)?;
        self.require_meter(t, c.meter_id).await?;
        Ok(c.meter_id)
    }

    /// Register der Positionen gehören zur Messstelle.
    async fn check_position_registers(&self, meter: Uuid, data: &ContractData) -> PortResult<()> {
        for p in &data.positions {
            if let Some(r) = p.register_id
                && self.meter_of_register(r.0).await? != Some(meter)
            {
                return Err(PortError::NotFound);
            }
        }
        Ok(())
    }

    async fn insert_positions(&self, contract: Uuid, data: &ContractData) -> PortResult<()> {
        for (sort, p) in (0_i32..).zip(&data.positions) {
            cost_position::Entity::insert(cost_position::ActiveModel {
                id: Set(p.id.0),
                contract_id: Set(contract),
                name: Set(p.name.clone()),
                register_id: Set(p.register_id.map(|r| r.0)),
                factor_micro: Set(money(p.factor)?),
                vat_pct_milli: Set(p
                    .vat_pct
                    .map(|v| fixed::quantity_to_db(v).map_err(data_err))
                    .transpose()?),
                sort: Set(sort),
            })
            .exec_without_returning(&self.txn)
            .await
            .map_err(db_err)?;
            for c in &p.components {
                price_component::Entity::insert(component_model(p.id.0, c)?)
                    .exec_without_returning(&self.txn)
                    .await
                    .map_err(db_err)?;
            }
        }
        Ok(())
    }
}

fn contract_model(id: Uuid, meter: Uuid, d: &ContractData) -> contract::ActiveModel {
    contract::ActiveModel {
        id: Set(id),
        meter_id: Set(meter),
        direction: Set(d.direction.as_str().to_owned()),
        provider: Set(d.provider.clone()),
        contract_number: Set(d.contract_number.clone()),
        starts_on: Set(d.starts_on),
        ends_on: Set(d.ends_on),
        billing_start_month: Set(i16::from(d.billing_start.month)),
        billing_start_day: Set(i16::from(d.billing_start.day)),
        note: Set(d.note.clone()),
    }
}

// @impl SPEC_TAR_CONTRACTS, SPEC_SEC_TENANT_SCOPE
#[async_trait]
impl ContractRepository for SeaOrmUow {
    async fn any(&mut self, t: &Tenant) -> PortResult<bool> {
        let meters = Query::select()
            .column(meter::Column::Id)
            .from(meter::Entity)
            .and_where(meter::Column::HouseholdId.eq(t.household_id().0))
            .to_owned();
        Ok(contract::Entity::find()
            .select_only()
            .column(contract::Column::Id)
            .filter(contract::Column::MeterId.in_subquery(meters))
            .into_tuple::<Uuid>()
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .is_some())
    }

    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<Contract>> {
        let meters = self.household_meters(t).await?;
        if meters.is_empty() {
            return Ok(Vec::new());
        }
        self.load_contracts(Condition::all().add(contract::Column::MeterId.is_in(meters)))
            .await
    }

    async fn for_meter(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Vec<Contract>> {
        if !self.owns_meter(t, meter.0).await? {
            return Ok(Vec::new());
        }
        self.load_contracts(Condition::all().add(contract::Column::MeterId.eq(meter.0)))
            .await
    }

    async fn get(&mut self, t: &Tenant, id: ContractId) -> PortResult<Option<Contract>> {
        let Some(c) = self
            .load_contracts(Condition::all().add(contract::Column::Id.eq(id.0)))
            .await?
            .pop()
        else {
            return Ok(None);
        };
        Ok(self.owns_meter(t, c.meter_id.0).await?.then_some(c))
    }

    async fn create(
        &mut self,
        t: &Tenant,
        id: ContractId,
        meter: MeterId,
        data: &ContractData,
    ) -> PortResult<()> {
        self.require_meter(t, meter.0).await?;
        self.check_position_registers(meter.0, data).await?;
        contract::Entity::insert(contract_model(id.0, meter.0, data))
            .exec_without_returning(&self.txn)
            .await
            .map_err(db_err)?;
        self.insert_positions(id.0, data).await
    }

    async fn update(&mut self, t: &Tenant, id: ContractId, data: &ContractData) -> PortResult<()> {
        let meter = self.contract_meter(t, id).await?;
        self.check_position_registers(meter, data).await?;
        contract::Entity::update(contract_model(id.0, meter, data))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        cost_position::Entity::delete_many()
            .filter(cost_position::Column::ContractId.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        self.insert_positions(id.0, data).await
    }

    async fn delete(&mut self, t: &Tenant, id: ContractId) -> PortResult<()> {
        self.contract_meter(t, id).await?;
        contract::Entity::delete_by_id(id.0)
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn add_component(
        &mut self,
        t: &Tenant,
        position: PositionId,
        c: &PriceComponentRow,
    ) -> PortResult<()> {
        let p = cost_position::Entity::find_by_id(position.0)
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .ok_or(PortError::NotFound)?;
        self.contract_meter(t, ContractId(p.contract_id)).await?;
        price_component::Entity::insert(component_model(p.id, c)?)
            .exec_without_returning(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn add_installment(
        &mut self,
        t: &Tenant,
        contract: ContractId,
        i: &InstallmentRow,
    ) -> PortResult<()> {
        self.contract_meter(t, contract).await?;
        installment::Entity::insert(installment::ActiveModel {
            id: Set(i.id.0),
            contract_id: Set(contract.0),
            valid_from: Set(i.installment.valid_from),
            amount_micro: Set(money(i.installment.amount)?),
            interval_months: Set(i16::from(i.installment.every_months)),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn add_adjustment(
        &mut self,
        t: &Tenant,
        contract: ContractId,
        a: &AdjustmentRow,
    ) -> PortResult<()> {
        self.contract_meter(t, contract).await?;
        adjustment::Entity::insert(adjustment::ActiveModel {
            id: Set(a.id.0),
            contract_id: Set(contract.0),
            kind: Set(a.adjustment.kind.as_str().to_owned()),
            booked_on: Set(a.adjustment.booked_on),
            amount_micro: Set(money(a.adjustment.amount)?),
            note: Set(a.note.clone()),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn add_settlement(
        &mut self,
        t: &Tenant,
        contract: ContractId,
        s: &SettlementRow,
    ) -> PortResult<()> {
        self.contract_meter(t, contract).await?;
        settlement::Entity::insert(settlement::ActiveModel {
            id: Set(s.id.0),
            contract_id: Set(contract.0),
            period_start: Set(s.period_start),
            period_end: Set(s.period_end),
            amount_micro: Set(money(s.amount)?),
            document_id: Set(s.document_id),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete_entry(
        &mut self,
        t: &Tenant,
        contract: ContractId,
        entry: ContractEntry,
    ) -> PortResult<()> {
        self.contract_meter(t, contract).await?;
        let affected = match entry {
            ContractEntry::Installment(id) => {
                installment::Entity::delete_many()
                    .filter(installment::Column::Id.eq(id.0))
                    .filter(installment::Column::ContractId.eq(contract.0))
                    .exec(&self.txn)
                    .await
            }
            ContractEntry::Adjustment(id) => {
                adjustment::Entity::delete_many()
                    .filter(adjustment::Column::Id.eq(id.0))
                    .filter(adjustment::Column::ContractId.eq(contract.0))
                    .exec(&self.txn)
                    .await
            }
            ContractEntry::Settlement(id) => {
                settlement::Entity::delete_many()
                    .filter(settlement::Column::Id.eq(id.0))
                    .filter(settlement::Column::ContractId.eq(contract.0))
                    .exec(&self.txn)
                    .await
            }
        }
        .map_err(db_err)?
        .rows_affected;
        if affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }
}
