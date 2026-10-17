//! Fahrzeuge und Kostenbuch im In-Memory-Adapter (SPEC_CAR_VEHICLES,
//! SPEC_ASSET_COSTS) und ihre Abbildung auf Archiv-Datensätze
//! (SPEC_BACKUP_DATA).

use rust_decimal::{Decimal, prelude::ToPrimitive};
use zaehler_domain::{
    assets::{AssetCategory, Recurrence},
    meter::MeterKind,
    vehicle::{ConsumptionProfile, Holding},
};

use super::{MemoryUow, PortError, PortResult, State, Tenant, async_trait};
use crate::{
    backup::{
        AcquisitionRecord, AssetCostRecord, BackupData, ChargingSessionRecord,
        ConsumptionProfileRecord, VehicleRecord,
    },
    ids::{AssetCostId, ChargingId, DocumentId, HouseholdId, MeterId, ProfileId},
    model::{AssetCost, AssetRef, ChargingSession, ProfileRow, Vehicle, VehicleData},
    port::{AssetCostRepository, VehicleRepository},
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

/// Dokumente, auf die Ladevorgänge oder Kostenbuch-Einträge verweisen.
pub(super) fn referenced_documents(s: &State) -> Vec<uuid::Uuid> {
    s.charging
        .iter()
        .filter_map(|c| c.document_id.map(|d| d.0))
        .chain(
            s.asset_costs
                .iter()
                .filter_map(|(_, c)| c.document_id.map(|d| d.0)),
        )
        .collect()
}

/// Entfernt Fahrzeuge und Einträge eines gelöschten Haushalts.
pub(super) fn delete_household(s: &mut State, hid: HouseholdId, meters: &[MeterId]) {
    s.vehicles.retain(|v| !meters.contains(&v.meter_id));
    s.charging.retain(|c| !meters.contains(&c.vehicle));
    s.asset_costs.retain(|(h, _)| *h != hid);
}

pub(super) fn export(s: &State, data: &mut BackupData) {
    for v in &s.vehicles {
        data.vehicles.push(VehicleRecord {
            meter_id: v.meter_id.0,
            plate: v.data.plate.clone(),
            model: v.data.model.clone(),
            drive_type: v.data.drive_type.as_str().into(),
            home_charging_meter_id: v.data.home_charging_meter_id.map(|m| m.0),
            pv_share_override_pct_milli: v.data.pv_share_pct.map(|p| scaled(p, 3)),
        });
        for p in &v.profiles {
            data.consumption_profiles.push(ConsumptionProfileRecord {
                id: p.id.0,
                vehicle_id: v.meter_id.0,
                valid_from: p.profile.valid_from,
                kwh_per_100km_milli: scaled(p.profile.kwh_per_100km, 3),
                charging_loss_pct_milli: scaled(p.profile.charging_loss_pct, 3),
                electric_share_pct_milli: p.profile.electric_share_pct.map(|x| scaled(x, 3)),
            });
        }
        if let Some(h) = &v.holding {
            data.acquisitions.push(AcquisitionRecord {
                vehicle_id: v.meter_id.0,
                purchased_on: h.purchased_on,
                price_micro: scaled(h.price, 6),
                residual_micro: scaled(h.residual_value, 6),
                planned_end_on: h.planned_end,
                sold_on: h.sold_on,
                sale_price_micro: h.sale_price.map(|p| scaled(p, 6)),
            });
        }
    }
    for c in &s.charging {
        data.charging_sessions.push(ChargingSessionRecord {
            id: c.id.0,
            vehicle_id: c.vehicle.0,
            charged_at: c.charged_at,
            energy_milli: scaled(c.energy, 3),
            amount_micro: scaled(c.amount, 6),
            odometer_milli: c.odometer.map(|o| scaled(o, 3)),
            provider: c.provider.clone(),
            kind: c.kind.as_str().into(),
            source: c.source.as_str().into(),
            document_id: c.document_id.map(|d| d.0),
        });
    }
    for (h, c) in &s.asset_costs {
        data.asset_costs.push(AssetCostRecord {
            id: c.id.0,
            household_id: h.0,
            asset_type: c.asset.asset_type.as_str().into(),
            asset_id: c.asset.id,
            booked_on: c.booked_on,
            category: c.category.as_str().into(),
            amount_micro: scaled(c.amount, 6),
            quantity_milli: c.quantity.map(|q| scaled(q, 3)),
            odometer_milli: c.odometer.map(|o| scaled(o, 3)),
            full_tank: c.full_tank,
            recurrence_months: c.recurrence.map(|r| i16::from(r.every_months)),
            recurrence_until: c.recurrence.and_then(|r| r.until),
            document_id: c.document_id.map(|d| d.0),
            note: c.note.clone(),
        });
    }
}

pub(super) fn import(s: &mut State, d: &BackupData) -> PortResult<()> {
    for v in &d.vehicles {
        let mut profiles: Vec<ProfileRow> = d
            .consumption_profiles
            .iter()
            .filter(|p| p.vehicle_id == v.meter_id)
            .map(|p| ProfileRow {
                id: ProfileId(p.id),
                profile: ConsumptionProfile {
                    valid_from: p.valid_from,
                    kwh_per_100km: Decimal::new(p.kwh_per_100km_milli, 3),
                    charging_loss_pct: Decimal::new(p.charging_loss_pct_milli, 3),
                    electric_share_pct: p.electric_share_pct_milli.map(|x| Decimal::new(x, 3)),
                },
            })
            .collect();
        profiles.sort_by_key(|p| p.profile.valid_from);
        let holding = d
            .acquisitions
            .iter()
            .find(|a| a.vehicle_id == v.meter_id)
            .map(|a| Holding {
                purchased_on: a.purchased_on,
                price: Decimal::new(a.price_micro, 6),
                residual_value: Decimal::new(a.residual_micro, 6),
                planned_end: a.planned_end_on,
                sold_on: a.sold_on,
                sale_price: a.sale_price_micro.map(|p| Decimal::new(p, 6)),
            });
        s.vehicles.push(Vehicle {
            meter_id: MeterId(v.meter_id),
            data: VehicleData {
                plate: v.plate.clone(),
                model: v.model.clone(),
                drive_type: v.drive_type.parse().map_err(invalid)?,
                home_charging_meter_id: v.home_charging_meter_id.map(MeterId),
                pv_share_pct: v.pv_share_override_pct_milli.map(|p| Decimal::new(p, 3)),
            },
            profiles,
            holding,
        });
    }
    for c in &d.charging_sessions {
        s.charging.push(ChargingSession {
            id: ChargingId(c.id),
            vehicle: MeterId(c.vehicle_id),
            charged_at: c.charged_at,
            energy: Decimal::new(c.energy_milli, 3),
            amount: Decimal::new(c.amount_micro, 6),
            odometer: c.odometer_milli.map(|o| Decimal::new(o, 3)),
            provider: c.provider.clone(),
            kind: c.kind.parse().map_err(invalid)?,
            source: c.source.parse().map_err(invalid)?,
            document_id: c.document_id.map(DocumentId),
        });
    }
    for c in &d.asset_costs {
        let asset_type = c.asset_type.parse().map_err(invalid)?;
        s.asset_costs.push((
            HouseholdId(c.household_id),
            AssetCost {
                id: AssetCostId(c.id),
                asset: AssetRef {
                    asset_type,
                    id: c.asset_id,
                },
                booked_on: c.booked_on,
                category: AssetCategory::parse(asset_type, &c.category)
                    .ok_or(PortError::Invalid("backup.invalid"))?,
                amount: Decimal::new(c.amount_micro, 6),
                quantity: c.quantity_milli.map(|q| Decimal::new(q, 3)),
                odometer: c.odometer_milli.map(|o| Decimal::new(o, 3)),
                full_tank: c.full_tank,
                recurrence: c
                    .recurrence_months
                    .map(|m| {
                        Ok::<_, PortError>(Recurrence {
                            every_months: u8::try_from(m).map_err(invalid)?,
                            until: c.recurrence_until,
                        })
                    })
                    .transpose()?,
                document_id: c.document_id.map(DocumentId),
                note: c.note.clone(),
            },
        ));
    }
    Ok(())
}

impl MemoryUow {
    /// Messstelle der Art `vehicle` im Haushalt des Tenants.
    fn vehicle_meter(&self, t: &Tenant, meter: MeterId) -> PortResult<()> {
        let ok = self.state.meters.iter().any(|(h, m)| {
            *h == t.household_id() && m.id == meter && m.data.kind == MeterKind::Vehicle
        });
        if ok { Ok(()) } else { Err(PortError::NotFound) }
    }

    fn vehicle_mut(&mut self, t: &Tenant, meter: MeterId) -> PortResult<&mut Vehicle> {
        self.vehicle_meter(t, meter)?;
        self.state
            .vehicles
            .iter_mut()
            .find(|v| v.meter_id == meter)
            .ok_or(PortError::NotFound)
    }
}

#[async_trait]
impl VehicleRepository for MemoryUow {
    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<Vehicle>> {
        let mut out: Vec<Vehicle> = self
            .state
            .vehicles
            .iter()
            .filter(|v| self.state.owns_meter(t, v.meter_id))
            .cloned()
            .collect();
        out.sort_by_key(|v| v.meter_id);
        Ok(out)
    }

    async fn get(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Option<Vehicle>> {
        Ok(VehicleRepository::list(self, t)
            .await?
            .into_iter()
            .find(|v| v.meter_id == meter))
    }

    async fn save(&mut self, t: &Tenant, meter: MeterId, data: &VehicleData) -> PortResult<()> {
        self.vehicle_meter(t, meter)?;
        if let Some(h) = data.home_charging_meter_id
            && !self.state.owns_meter(t, h)
        {
            return Err(PortError::NotFound);
        }
        match self.state.vehicles.iter_mut().find(|v| v.meter_id == meter) {
            Some(v) => v.data = data.clone(),
            None => self.state.vehicles.push(Vehicle {
                meter_id: meter,
                data: data.clone(),
                profiles: Vec::new(),
                holding: None,
            }),
        }
        Ok(())
    }

    async fn add_profile(&mut self, t: &Tenant, meter: MeterId, p: &ProfileRow) -> PortResult<()> {
        let v = self.vehicle_mut(t, meter)?;
        if v.profiles
            .iter()
            .any(|x| x.profile.valid_from == p.profile.valid_from)
        {
            return Err(PortError::Conflict(
                "vehicle_consumption_profiles.valid_from".into(),
            ));
        }
        v.profiles.push(p.clone());
        v.profiles.sort_by_key(|x| x.profile.valid_from);
        Ok(())
    }

    async fn delete_profile(
        &mut self,
        t: &Tenant,
        meter: MeterId,
        id: ProfileId,
    ) -> PortResult<()> {
        let v = self.vehicle_mut(t, meter)?;
        let before = v.profiles.len();
        v.profiles.retain(|p| p.id != id);
        if v.profiles.len() == before {
            return Err(PortError::NotFound);
        }
        Ok(())
    }

    async fn set_holding(
        &mut self,
        t: &Tenant,
        meter: MeterId,
        h: Option<&Holding>,
    ) -> PortResult<()> {
        self.vehicle_mut(t, meter)?.holding = h.cloned();
        Ok(())
    }

    async fn charging(&mut self, t: &Tenant, meter: MeterId) -> PortResult<Vec<ChargingSession>> {
        if !self.state.owns_meter(t, meter) {
            return Ok(Vec::new());
        }
        let mut out: Vec<ChargingSession> = self
            .state
            .charging
            .iter()
            .filter(|c| c.vehicle == meter)
            .cloned()
            .collect();
        out.sort_by_key(|c| (c.charged_at, c.id));
        Ok(out)
    }

    async fn add_charging(&mut self, t: &Tenant, c: &ChargingSession) -> PortResult<()> {
        self.vehicle_mut(t, c.vehicle)?;
        self.state.charging.push(c.clone());
        Ok(())
    }

    async fn delete_charging(
        &mut self,
        t: &Tenant,
        meter: MeterId,
        id: ChargingId,
    ) -> PortResult<()> {
        self.vehicle_meter(t, meter)?;
        let before = self.state.charging.len();
        self.state
            .charging
            .retain(|c| !(c.id == id && c.vehicle == meter));
        if self.state.charging.len() == before {
            return Err(PortError::NotFound);
        }
        Ok(())
    }
}

#[async_trait]
impl AssetCostRepository for MemoryUow {
    async fn list(&mut self, t: &Tenant, asset: AssetRef) -> PortResult<Vec<AssetCost>> {
        let mut out: Vec<AssetCost> = self
            .state
            .asset_costs
            .iter()
            .filter(|(h, c)| *h == t.household_id() && c.asset == asset)
            .map(|(_, c)| c.clone())
            .collect();
        out.sort_by_key(|c| (c.booked_on, c.id));
        Ok(out)
    }

    async fn get(&mut self, t: &Tenant, id: AssetCostId) -> PortResult<Option<AssetCost>> {
        Ok(self
            .state
            .asset_costs
            .iter()
            .find(|(h, c)| *h == t.household_id() && c.id == id)
            .map(|(_, c)| c.clone()))
    }

    async fn create(&mut self, t: &Tenant, c: &AssetCost) -> PortResult<()> {
        if self.state.asset_costs.iter().any(|(_, x)| x.id == c.id) {
            return Err(PortError::Conflict("asset_costs.id".into()));
        }
        self.state.asset_costs.push((t.household_id(), c.clone()));
        Ok(())
    }

    async fn update(&mut self, t: &Tenant, c: &AssetCost) -> PortResult<()> {
        let (_, x) = self
            .state
            .asset_costs
            .iter_mut()
            .find(|(h, x)| *h == t.household_id() && x.id == c.id)
            .ok_or(PortError::NotFound)?;
        *x = AssetCost {
            asset: x.asset,
            ..c.clone()
        };
        Ok(())
    }

    async fn delete(&mut self, t: &Tenant, id: AssetCostId) -> PortResult<()> {
        let before = self.state.asset_costs.len();
        self.state
            .asset_costs
            .retain(|(h, c)| !(*h == t.household_id() && c.id == id));
        if self.state.asset_costs.len() == before {
            return Err(PortError::NotFound);
        }
        Ok(())
    }
}
