//! Verträge im In-Memory-Adapter (SPEC_TAR_CONTRACTS) und ihre Abbildung
//! auf Archiv-Datensätze (SPEC_BACKUP_DATA).

use rust_decimal::{Decimal, prelude::ToPrimitive};
use zaehler_domain::tariff::{Adjustment, Installment, PriceComponent};

use super::{MemoryUow, PortError, PortResult, State, Tenant, async_trait};
use crate::{
    backup::{
        AdjustmentRecord, BackupData, ContractRecord, CostPositionRecord, InstallmentRecord,
        PriceComponentRecord, SettlementRecord,
    },
    ids::{
        AdjustmentId, ContractId, InstallmentId, MeterId, PositionId, PriceComponentId, RegisterId,
        SettlementId,
    },
    model::{
        AdjustmentRow, Contract, ContractData, ContractEntry, InstallmentRow, MonthDay, Position,
        PriceComponentRow, SettlementRow,
    },
    port::ContractRepository,
};

fn invalid<E>(_: E) -> PortError {
    PortError::Invalid("backup.invalid")
}

fn scaled(d: Decimal, scale: u32) -> i64 {
    (d * Decimal::from(10_i64.pow(scale)))
        .trunc()
        .to_i64()
        .unwrap_or(0)
}

/// Verträge des Zustands als Archiv-Datensätze.
pub(super) fn export(s: &State, data: &mut BackupData) {
    for c in &s.contracts {
        data.contracts.push(ContractRecord {
            id: c.id.0,
            meter_id: c.meter_id.0,
            direction: c.data.direction.as_str().into(),
            provider: c.data.provider.clone(),
            contract_number: c.data.contract_number.clone(),
            starts_on: c.data.starts_on,
            ends_on: c.data.ends_on,
            billing_start_month: i16::from(c.data.billing_start.month),
            billing_start_day: i16::from(c.data.billing_start.day),
            note: c.data.note.clone(),
        });
        for (sort, p) in (0_i32..).zip(&c.data.positions) {
            data.cost_positions.push(CostPositionRecord {
                id: p.id.0,
                contract_id: c.id.0,
                name: p.name.clone(),
                register_id: p.register_id.map(|r| r.0),
                factor_micro: scaled(p.factor, 6),
                vat_pct_milli: p.vat_pct.map(|v| scaled(v, 3)),
                sort,
            });
            for x in &p.components {
                data.price_components.push(PriceComponentRecord {
                    id: x.id.0,
                    position_id: p.id.0,
                    valid_from: x.component.valid_from,
                    base_price_micro: scaled(x.component.base_price, 6),
                    base_interval: x.component.base_interval.as_str().into(),
                    unit_pricing: serde_json::to_string(&x.component.unit_pricing)
                        .unwrap_or_default(),
                    factor_micro: x.component.factor.map(|f| scaled(f, 6)),
                });
            }
        }
        for x in &c.installments {
            data.installments.push(InstallmentRecord {
                id: x.id.0,
                contract_id: c.id.0,
                valid_from: x.installment.valid_from,
                amount_micro: scaled(x.installment.amount, 6),
                interval_months: i16::from(x.installment.every_months),
            });
        }
        for x in &c.adjustments {
            data.adjustments.push(AdjustmentRecord {
                id: x.id.0,
                contract_id: c.id.0,
                kind: x.adjustment.kind.as_str().into(),
                booked_on: x.adjustment.booked_on,
                amount_micro: scaled(x.adjustment.amount, 6),
                note: x.note.clone(),
            });
        }
        for x in &c.settlements {
            data.settlements.push(SettlementRecord {
                id: x.id.0,
                contract_id: c.id.0,
                period_start: x.period_start,
                period_end: x.period_end,
                amount_micro: scaled(x.amount, 6),
                document_id: x.document_id,
            });
        }
    }
}

/// Verträge aus Archiv-Datensätzen.
pub(super) fn import(s: &mut State, d: &BackupData) -> PortResult<()> {
    for c in &d.contracts {
        let mut positions: Vec<(i32, Position)> = Vec::new();
        for p in d.cost_positions.iter().filter(|p| p.contract_id == c.id) {
            let mut components = Vec::new();
            for x in d.price_components.iter().filter(|x| x.position_id == p.id) {
                components.push(PriceComponentRow {
                    id: PriceComponentId(x.id),
                    component: PriceComponent {
                        valid_from: x.valid_from,
                        base_price: Decimal::new(x.base_price_micro, 6),
                        base_interval: x.base_interval.parse().map_err(invalid)?,
                        unit_pricing: serde_json::from_str(&x.unit_pricing).map_err(invalid)?,
                        factor: x.factor_micro.map(|f| Decimal::new(f, 6)),
                    },
                });
            }
            components.sort_by_key(|x| x.component.valid_from);
            positions.push((
                p.sort,
                Position {
                    id: PositionId(p.id),
                    name: p.name.clone(),
                    register_id: p.register_id.map(RegisterId),
                    factor: Decimal::new(p.factor_micro, 6),
                    vat_pct: p.vat_pct_milli.map(|v| Decimal::new(v, 3)),
                    components,
                },
            ));
        }
        positions.sort_by_key(|(sort, _)| *sort);
        let small = |v: i16| u8::try_from(v).map_err(invalid);
        let mut installments = Vec::new();
        for x in d.installments.iter().filter(|x| x.contract_id == c.id) {
            installments.push(InstallmentRow {
                id: InstallmentId(x.id),
                installment: Installment {
                    amount: Decimal::new(x.amount_micro, 6),
                    valid_from: x.valid_from,
                    every_months: small(x.interval_months)?,
                },
            });
        }
        let mut adjustments = Vec::new();
        for x in d.adjustments.iter().filter(|x| x.contract_id == c.id) {
            adjustments.push(AdjustmentRow {
                id: AdjustmentId(x.id),
                adjustment: Adjustment {
                    kind: x.kind.parse().map_err(invalid)?,
                    booked_on: x.booked_on,
                    amount: Decimal::new(x.amount_micro, 6),
                },
                note: x.note.clone(),
            });
        }
        let settlements = d
            .settlements
            .iter()
            .filter(|x| x.contract_id == c.id)
            .map(|x| SettlementRow {
                id: SettlementId(x.id),
                period_start: x.period_start,
                period_end: x.period_end,
                amount: Decimal::new(x.amount_micro, 6),
                document_id: x.document_id,
            })
            .collect();
        s.contracts.push(sorted(Contract {
            id: ContractId(c.id),
            meter_id: MeterId(c.meter_id),
            data: ContractData {
                direction: c.direction.parse().map_err(invalid)?,
                provider: c.provider.clone(),
                contract_number: c.contract_number.clone(),
                starts_on: c.starts_on,
                ends_on: c.ends_on,
                billing_start: MonthDay {
                    month: small(c.billing_start_month)?,
                    day: small(c.billing_start_day)?,
                },
                note: c.note.clone(),
                positions: positions.into_iter().map(|(_, p)| p).collect(),
            },
            installments,
            adjustments,
            settlements,
        }));
    }
    Ok(())
}

impl MemoryUow {
    fn contract_mut(&mut self, t: &Tenant, id: ContractId) -> PortResult<&mut Contract> {
        let meter = self
            .state
            .contracts
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.meter_id)
            .ok_or(PortError::NotFound)?;
        if !self.state.owns_meter(t, meter) {
            return Err(PortError::NotFound);
        }
        self.state
            .contracts
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(PortError::NotFound)
    }

    /// Register der Positionen gehören zur Messstelle.
    fn check_registers(&self, meter: MeterId, data: &ContractData) -> PortResult<()> {
        for p in &data.positions {
            if let Some(r) = p.register_id
                && self.state.meter_of_register(r) != Some(meter)
            {
                return Err(PortError::NotFound);
            }
        }
        Ok(())
    }
}

fn sorted(mut c: Contract) -> Contract {
    for p in &mut c.data.positions {
        p.components.sort_by_key(|x| x.component.valid_from);
    }
    c.installments.sort_by_key(|i| i.installment.valid_from);
    c.adjustments
        .sort_by_key(|a| (a.adjustment.booked_on, a.id));
    c.settlements.sort_by_key(|s| (s.period_start, s.id));
    c
}

#[async_trait]
impl ContractRepository for MemoryUow {
    async fn any(&mut self, t: &Tenant) -> PortResult<bool> {
        Ok(self
            .state
            .contracts
            .iter()
            .any(|c| self.state.owns_meter(t, c.meter_id)))
    }

    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<Contract>> {
        let mut out: Vec<Contract> = self
            .state
            .contracts
            .iter()
            .filter(|c| self.state.owns_meter(t, c.meter_id))
            .cloned()
            .map(sorted)
            .collect();
        out.sort_by_key(|c| (c.meter_id, c.data.starts_on, c.id));
        Ok(out)
    }

    async fn for_meter(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Vec<Contract>> {
        Ok(self
            .list(t)
            .await?
            .into_iter()
            .filter(|c| c.meter_id == meter)
            .collect())
    }

    async fn get(&mut self, t: &Tenant, id: ContractId) -> PortResult<Option<Contract>> {
        Ok(self.list(t).await?.into_iter().find(|c| c.id == id))
    }

    async fn create(
        &mut self,
        t: &Tenant,
        id: ContractId,
        meter: MeterId,
        data: &ContractData,
    ) -> PortResult<()> {
        if !self.state.owns_meter(t, meter) {
            return Err(PortError::NotFound);
        }
        self.check_registers(meter, data)?;
        if self.state.contracts.iter().any(|c| c.id == id) {
            return Err(PortError::Conflict("contracts.id".into()));
        }
        self.state.contracts.push(Contract {
            id,
            meter_id: meter,
            data: data.clone(),
            installments: Vec::new(),
            adjustments: Vec::new(),
            settlements: Vec::new(),
        });
        Ok(())
    }

    async fn update(&mut self, t: &Tenant, id: ContractId, data: &ContractData) -> PortResult<()> {
        let meter = self.contract_mut(t, id)?.meter_id;
        self.check_registers(meter, data)?;
        self.contract_mut(t, id)?.data = data.clone();
        Ok(())
    }

    async fn delete(&mut self, t: &Tenant, id: ContractId) -> PortResult<()> {
        self.contract_mut(t, id)?;
        self.state.contracts.retain(|c| c.id != id);
        Ok(())
    }

    async fn add_component(
        &mut self,
        t: &Tenant,
        position: PositionId,
        c: &PriceComponentRow,
    ) -> PortResult<()> {
        let contract = self
            .state
            .contracts
            .iter()
            .find(|x| x.data.positions.iter().any(|p| p.id == position))
            .map(|x| x.id)
            .ok_or(PortError::NotFound)?;
        let contract = self.contract_mut(t, contract)?;
        let p = contract
            .data
            .positions
            .iter_mut()
            .find(|p| p.id == position)
            .ok_or(PortError::NotFound)?;
        if p.components
            .iter()
            .any(|x| x.component.valid_from == c.component.valid_from)
        {
            return Err(PortError::Conflict("price_components.valid_from".into()));
        }
        p.components.push(c.clone());
        p.components.sort_by_key(|x| x.component.valid_from);
        Ok(())
    }

    async fn add_installment(
        &mut self,
        t: &Tenant,
        contract: ContractId,
        i: &InstallmentRow,
    ) -> PortResult<()> {
        let c = self.contract_mut(t, contract)?;
        if c.installments
            .iter()
            .any(|x| x.installment.valid_from == i.installment.valid_from)
        {
            return Err(PortError::Conflict("installments.valid_from".into()));
        }
        c.installments.push(i.clone());
        Ok(())
    }

    async fn add_adjustment(
        &mut self,
        t: &Tenant,
        contract: ContractId,
        a: &AdjustmentRow,
    ) -> PortResult<()> {
        self.contract_mut(t, contract)?.adjustments.push(a.clone());
        Ok(())
    }

    async fn add_settlement(
        &mut self,
        t: &Tenant,
        contract: ContractId,
        s: &SettlementRow,
    ) -> PortResult<()> {
        self.contract_mut(t, contract)?.settlements.push(s.clone());
        Ok(())
    }

    async fn delete_entry(
        &mut self,
        t: &Tenant,
        contract: ContractId,
        entry: ContractEntry,
    ) -> PortResult<()> {
        let c = self.contract_mut(t, contract)?;
        let before = c.installments.len() + c.adjustments.len() + c.settlements.len();
        match entry {
            ContractEntry::Installment(id) => c.installments.retain(|x| x.id != id),
            ContractEntry::Adjustment(id) => c.adjustments.retain(|x| x.id != id),
            ContractEntry::Settlement(id) => c.settlements.retain(|x| x.id != id),
        }
        if c.installments.len() + c.adjustments.len() + c.settlements.len() == before {
            return Err(PortError::NotFound);
        }
        Ok(())
    }
}
