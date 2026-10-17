//! PV-Anlagen auf SeaORM (SPEC_PV_ASSIGNMENT). Anlagen gehören zum
//! Haushalt; zugeordnete Messstellen müssen zum selben Haushalt gehören
//! (SPEC_SEC_TENANT_SCOPE).

use async_trait::async_trait;
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use zaehler_app::{
    PortError, Tenant,
    ids::{MeterId, PvSystemId},
    model::PvSystem,
    port::{PortResult, PvRepository},
};
use zaehler_domain::pv::PvSystemData;

use super::{SeaOrmUow, data_err, db_err};
use crate::{
    entity::{pv_system, pv_system_meter},
    fixed,
};

fn quantity(v: rust_decimal::Decimal) -> PortResult<i64> {
    fixed::quantity_to_db(v).map_err(data_err)
}

fn system_model(household: uuid::Uuid, s: &PvSystem) -> PortResult<pv_system::ActiveModel> {
    let d = &s.data;
    Ok(pv_system::ActiveModel {
        id: Set(s.id.0),
        household_id: Set(household),
        name: Set(d.name.clone()),
        kind: Set(d.kind.as_str().to_owned()),
        kwp_milli: Set(quantity(d.kwp)?),
        commissioned_on: Set(d.commissioned_on),
        azimuth: Set(d.azimuth),
        tilt: Set(d.tilt),
        inverter: Set(d.inverter.clone()),
        battery_kwh_milli: Set(d.battery_kwh.map(quantity).transpose()?),
        specific_yield_milli: Set(quantity(d.specific_yield)?),
        degradation_pct_milli: Set(quantity(d.degradation_pct)?),
        month_shares: Set(d
            .month_shares
            .map(|m| serde_json::to_string(&m))
            .transpose()
            .map_err(data_err)?),
        alarm_threshold_pct: Set(i16::from(d.alarm_threshold_pct)),
    })
}

impl SeaOrmUow {
    async fn require_pv_meters(&self, t: &Tenant, s: &PvSystem) -> PortResult<()> {
        for (m, _) in &s.meters {
            self.require_meter(t, m.0).await?;
        }
        Ok(())
    }

    async fn insert_pv_meters(&self, s: &PvSystem) -> PortResult<()> {
        for (m, role) in &s.meters {
            pv_system_meter::Entity::insert(pv_system_meter::ActiveModel {
                pv_system_id: Set(s.id.0),
                meter_id: Set(m.0),
                role: Set(role.as_str().to_owned()),
            })
            .exec_without_returning(&self.txn)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }
}

// @impl SPEC_PV_ASSIGNMENT, SPEC_SEC_TENANT_SCOPE
#[async_trait]
impl PvRepository for SeaOrmUow {
    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<PvSystem>> {
        let rows = pv_system::Entity::find()
            .filter(pv_system::Column::HouseholdId.eq(t.household_id().0))
            .order_by_asc(pv_system::Column::Name)
            .order_by_asc(pv_system::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let meters = pv_system_meter::Entity::find()
            .filter(pv_system_meter::Column::PvSystemId.is_in(rows.iter().map(|r| r.id)))
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        rows.into_iter()
            .map(|r| {
                let mut assigned = Vec::new();
                for m in meters.iter().filter(|m| m.pv_system_id == r.id) {
                    assigned.push((MeterId(m.meter_id), m.role.parse().map_err(data_err)?));
                }
                assigned
                    .sort_by_key(|(m, role): &(MeterId, zaehler_domain::pv::PvRole)| (*role, *m));
                Ok(PvSystem {
                    id: PvSystemId(r.id),
                    data: PvSystemData {
                        name: r.name,
                        kind: r.kind.parse().map_err(data_err)?,
                        kwp: fixed::quantity_from_db(r.kwp_milli),
                        commissioned_on: r.commissioned_on,
                        azimuth: r.azimuth,
                        tilt: r.tilt,
                        inverter: r.inverter,
                        battery_kwh: r.battery_kwh_milli.map(fixed::quantity_from_db),
                        specific_yield: fixed::quantity_from_db(r.specific_yield_milli),
                        degradation_pct: fixed::quantity_from_db(r.degradation_pct_milli),
                        month_shares: r
                            .month_shares
                            .as_deref()
                            .map(serde_json::from_str)
                            .transpose()
                            .map_err(data_err)?,
                        alarm_threshold_pct: u8::try_from(r.alarm_threshold_pct)
                            .map_err(data_err)?,
                    },
                    meters: assigned,
                })
            })
            .collect()
    }

    async fn get(&mut self, t: &Tenant, id: PvSystemId) -> PortResult<Option<PvSystem>> {
        Ok(PvRepository::list(self, t)
            .await?
            .into_iter()
            .find(|s| s.id == id))
    }

    async fn create(&mut self, t: &Tenant, s: &PvSystem) -> PortResult<()> {
        self.require_pv_meters(t, s).await?;
        pv_system::Entity::insert(system_model(t.household_id().0, s)?)
            .exec_without_returning(&self.txn)
            .await
            .map_err(db_err)?;
        self.insert_pv_meters(s).await
    }

    async fn update(&mut self, t: &Tenant, s: &PvSystem) -> PortResult<()> {
        self.require_pv_meters(t, s).await?;
        let res = pv_system::Entity::update_many()
            .set(system_model(t.household_id().0, s)?)
            .filter(pv_system::Column::Id.eq(s.id.0))
            .filter(pv_system::Column::HouseholdId.eq(t.household_id().0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        pv_system_meter::Entity::delete_many()
            .filter(pv_system_meter::Column::PvSystemId.eq(s.id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        self.insert_pv_meters(s).await
    }

    async fn delete(&mut self, t: &Tenant, id: PvSystemId) -> PortResult<()> {
        let res = pv_system::Entity::delete_many()
            .filter(pv_system::Column::Id.eq(id.0))
            .filter(pv_system::Column::HouseholdId.eq(t.household_id().0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }
}
