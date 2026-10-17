//! PV-Anlagen im In-Memory-Adapter (SPEC_PV_ASSIGNMENT) und ihre Abbildung
//! auf Archiv-Datensätze (SPEC_BACKUP_DATA).

use rust_decimal::{Decimal, prelude::ToPrimitive};
use zaehler_domain::pv::PvSystemData;

use super::{MemoryUow, PortError, PortResult, State, Tenant, async_trait};
use crate::{
    backup::{BackupData, PvSystemMeterRecord, PvSystemRecord},
    ids::{HouseholdId, MeterId, PvSystemId},
    model::PvSystem,
    port::PvRepository,
};

fn invalid<E>(_: E) -> PortError {
    PortError::Invalid("backup.invalid")
}

fn milli(d: Decimal) -> i64 {
    (d * Decimal::from(1000)).trunc().to_i64().unwrap_or(0)
}

pub(super) fn delete_household(s: &mut State, hid: HouseholdId) {
    s.pv_systems.retain(|(h, _)| *h != hid);
}

pub(super) fn export(s: &State, data: &mut BackupData) {
    for (h, p) in &s.pv_systems {
        let d = &p.data;
        data.pv_systems.push(PvSystemRecord {
            id: p.id.0,
            household_id: h.0,
            name: d.name.clone(),
            kind: d.kind.as_str().into(),
            kwp_milli: milli(d.kwp),
            commissioned_on: d.commissioned_on,
            azimuth: d.azimuth,
            tilt: d.tilt,
            inverter: d.inverter.clone(),
            battery_kwh_milli: d.battery_kwh.map(milli),
            specific_yield_milli: milli(d.specific_yield),
            degradation_pct_milli: milli(d.degradation_pct),
            month_shares: d.month_shares.and_then(|m| serde_json::to_string(&m).ok()),
            alarm_threshold_pct: i16::from(d.alarm_threshold_pct),
        });
        for (meter, role) in &p.meters {
            data.pv_system_meters.push(PvSystemMeterRecord {
                pv_system_id: p.id.0,
                meter_id: meter.0,
                role: role.as_str().into(),
            });
        }
    }
}

pub(super) fn import(s: &mut State, d: &BackupData) -> PortResult<()> {
    for r in &d.pv_systems {
        let mut meters = Vec::new();
        for m in d.pv_system_meters.iter().filter(|m| m.pv_system_id == r.id) {
            meters.push((MeterId(m.meter_id), m.role.parse().map_err(invalid)?));
        }
        meters.sort_by_key(|(m, role)| (*role, *m));
        s.pv_systems.push((
            HouseholdId(r.household_id),
            PvSystem {
                id: PvSystemId(r.id),
                data: PvSystemData {
                    name: r.name.clone(),
                    kind: r.kind.parse().map_err(invalid)?,
                    kwp: Decimal::new(r.kwp_milli, 3),
                    commissioned_on: r.commissioned_on,
                    azimuth: r.azimuth,
                    tilt: r.tilt,
                    inverter: r.inverter.clone(),
                    battery_kwh: r.battery_kwh_milli.map(|v| Decimal::new(v, 3)),
                    specific_yield: Decimal::new(r.specific_yield_milli, 3),
                    degradation_pct: Decimal::new(r.degradation_pct_milli, 3),
                    month_shares: r
                        .month_shares
                        .as_deref()
                        .map(serde_json::from_str)
                        .transpose()
                        .map_err(invalid)?,
                    alarm_threshold_pct: u8::try_from(r.alarm_threshold_pct).map_err(invalid)?,
                },
                meters,
            },
        ));
    }
    Ok(())
}

impl MemoryUow {
    fn check_pv_meters(&self, t: &Tenant, s: &PvSystem) -> PortResult<()> {
        if s.meters.iter().all(|(m, _)| self.state.owns_meter(t, *m)) {
            Ok(())
        } else {
            Err(PortError::NotFound)
        }
    }
}

fn sorted(mut s: PvSystem) -> PvSystem {
    s.meters.sort_by_key(|(m, role)| (*role, *m));
    s
}

#[async_trait]
impl PvRepository for MemoryUow {
    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<PvSystem>> {
        let mut out: Vec<PvSystem> = self
            .state
            .pv_systems
            .iter()
            .filter(|(h, _)| *h == t.household_id())
            .map(|(_, s)| sorted(s.clone()))
            .collect();
        out.sort_by(|a, b| a.data.name.cmp(&b.data.name).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    async fn get(&mut self, t: &Tenant, id: PvSystemId) -> PortResult<Option<PvSystem>> {
        Ok(PvRepository::list(self, t)
            .await?
            .into_iter()
            .find(|s| s.id == id))
    }

    async fn create(&mut self, t: &Tenant, s: &PvSystem) -> PortResult<()> {
        self.check_pv_meters(t, s)?;
        if self.state.pv_systems.iter().any(|(_, x)| x.id == s.id) {
            return Err(PortError::Conflict("pv_systems.id".into()));
        }
        self.state
            .pv_systems
            .push((t.household_id(), sorted(s.clone())));
        Ok(())
    }

    async fn update(&mut self, t: &Tenant, s: &PvSystem) -> PortResult<()> {
        self.check_pv_meters(t, s)?;
        let (_, x) = self
            .state
            .pv_systems
            .iter_mut()
            .find(|(h, x)| *h == t.household_id() && x.id == s.id)
            .ok_or(PortError::NotFound)?;
        *x = sorted(s.clone());
        Ok(())
    }

    async fn delete(&mut self, t: &Tenant, id: PvSystemId) -> PortResult<()> {
        let before = self.state.pv_systems.len();
        self.state
            .pv_systems
            .retain(|(h, x)| !(*h == t.household_id() && x.id == id));
        if self.state.pv_systems.len() == before {
            return Err(PortError::NotFound);
        }
        Ok(())
    }
}
