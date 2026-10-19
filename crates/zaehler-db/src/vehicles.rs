//! Fahrzeuge und Kostenbuch auf SeaORM (SPEC_CAR_VEHICLES,
//! SPEC_ASSET_COSTS). Fahrzeuge gehören über ihre Messstelle, Einträge des
//! Kostenbuchs direkt zum Haushalt (SPEC_SEC_TENANT_SCOPE).

use async_trait::async_trait;
use rust_decimal::Decimal;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, sea_query::OnConflict,
};
use uuid::Uuid;
use zaehler_app::{
    PortError, Tenant,
    ids::{AssetCostId, ChargingId, DocumentId, MeterId, ProfileId},
    model::{AssetCost, AssetRef, ChargingSession, ProfileRow, Vehicle, VehicleData},
    port::{AssetCostRepository, PortResult, VehicleRepository},
};
use zaehler_domain::{
    assets::{AssetCategory, Recurrence},
    meter::MeterKind,
    vehicle::{ConsumptionProfile, Holding},
};

use super::{SeaOrmUow, data_err, db_err};
use crate::{
    entity::{
        asset_cost, charging_session, meter, vehicle, vehicle_acquisition,
        vehicle_consumption_profile,
    },
    fixed,
};

fn money(v: Decimal) -> PortResult<i64> {
    fixed::money_to_db(v).map_err(data_err)
}

fn quantity(v: Decimal) -> PortResult<i64> {
    fixed::quantity_to_db(v).map_err(data_err)
}

fn charging_from(c: charging_session::Model) -> PortResult<ChargingSession> {
    Ok(ChargingSession {
        id: ChargingId(c.id),
        vehicle: MeterId(c.vehicle_id),
        charged_at: c.charged_at,
        energy: fixed::quantity_from_db(c.energy_milli),
        amount: fixed::money_from_db(c.amount_micro),
        odometer: c.odometer_milli.map(fixed::quantity_from_db),
        provider: c.provider,
        kind: c.kind.parse().map_err(data_err)?,
        source: c.source.parse().map_err(data_err)?,
        document_id: c.document_id.map(DocumentId),
    })
}

fn holding_from(a: &vehicle_acquisition::Model) -> PortResult<Holding> {
    Ok(Holding {
        purchased_on: a.purchased_on,
        price: fixed::money_from_db(a.price_micro),
        mode: a.depreciation_mode.parse().map_err(data_err)?,
        residual_value: a.residual_micro.map(fixed::money_from_db),
        planned_end: a.planned_end_on,
        odometer_at_purchase: a.odometer_at_purchase_milli.map(fixed::quantity_from_db),
        lifetime_km: a.lifetime_km_milli.map(fixed::quantity_from_db),
        end_value: a.end_value_micro.map(fixed::money_from_db),
        sold_on: a.sold_on,
        sale_price: a.sale_price_micro.map(fixed::money_from_db),
    })
}

fn cost_from(c: asset_cost::Model) -> PortResult<AssetCost> {
    let asset_type = c.asset_type.parse().map_err(data_err)?;
    Ok(AssetCost {
        id: AssetCostId(c.id),
        asset: AssetRef {
            asset_type,
            id: c.asset_id,
        },
        booked_on: c.booked_on,
        category: AssetCategory::parse(asset_type, &c.category)
            .ok_or_else(|| data_err("asset_costs.category"))?,
        amount: fixed::money_from_db(c.amount_micro),
        quantity: c.quantity_milli.map(fixed::quantity_from_db),
        unit_price: c.unit_price_micro.map(fixed::money_from_db),
        odometer: c.odometer_milli.map(fixed::quantity_from_db),
        full_tank: c.full_tank,
        recurrence: c
            .recurrence_months
            .map(|m| {
                Ok::<_, PortError>(Recurrence {
                    every_months: u8::try_from(m).map_err(data_err)?,
                    until: c.recurrence_until,
                })
            })
            .transpose()?,
        document_id: c.document_id.map(DocumentId),
        note: c.note,
    })
}

fn cost_model(household: Uuid, c: &AssetCost) -> PortResult<asset_cost::ActiveModel> {
    Ok(asset_cost::ActiveModel {
        id: Set(c.id.0),
        household_id: Set(household),
        asset_type: Set(c.asset.asset_type.as_str().to_owned()),
        asset_id: Set(c.asset.id),
        booked_on: Set(c.booked_on),
        category: Set(c.category.as_str().to_owned()),
        amount_micro: Set(money(c.amount)?),
        quantity_milli: Set(c.quantity.map(quantity).transpose()?),
        unit_price_micro: Set(c.unit_price.map(money).transpose()?),
        odometer_milli: Set(c.odometer.map(quantity).transpose()?),
        full_tank: Set(c.full_tank),
        recurrence_months: Set(c.recurrence.map(|r| i16::from(r.every_months))),
        recurrence_until: Set(c.recurrence.and_then(|r| r.until)),
        document_id: Set(c.document_id.map(|d| d.0)),
        note: Set(c.note.clone()),
    })
}

impl SeaOrmUow {
    /// Messstelle der Art `vehicle` im Haushalt des Tenants.
    async fn require_vehicle_meter(&self, t: &Tenant, id: MeterId) -> PortResult<()> {
        meter::Entity::find_by_id(id.0)
            .filter(meter::Column::HouseholdId.eq(t.household_id().0))
            .filter(meter::Column::Kind.eq(MeterKind::Vehicle.as_str()))
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|_| ())
            .ok_or(PortError::NotFound)
    }

    /// Fahrzeug-Datensatz vorhanden.
    async fn require_vehicle(&self, t: &Tenant, id: MeterId) -> PortResult<()> {
        self.require_vehicle_meter(t, id).await?;
        vehicle::Entity::find_by_id(id.0)
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|_| ())
            .ok_or(PortError::NotFound)
    }

    async fn load_vehicles(&self, ids: Vec<Uuid>) -> PortResult<Vec<Vehicle>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = vehicle::Entity::find()
            .filter(vehicle::Column::MeterId.is_in(ids.iter().copied()))
            .order_by_asc(vehicle::Column::MeterId)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let profiles = vehicle_consumption_profile::Entity::find()
            .filter(vehicle_consumption_profile::Column::VehicleId.is_in(ids.iter().copied()))
            .order_by_asc(vehicle_consumption_profile::Column::ValidFrom)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let acquisitions = vehicle_acquisition::Entity::find()
            .filter(vehicle_acquisition::Column::VehicleId.is_in(ids))
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        rows.into_iter()
            .map(|v| {
                Ok(Vehicle {
                    meter_id: MeterId(v.meter_id),
                    data: VehicleData {
                        plate: v.plate,
                        model: v.model,
                        drive_type: v.drive_type.parse().map_err(data_err)?,
                        home_charging_meter_id: v.home_charging_meter_id.map(MeterId),
                        pv_share_pct: v.pv_share_override_pct_milli.map(fixed::quantity_from_db),
                    },
                    profiles: profiles
                        .iter()
                        .filter(|p| p.vehicle_id == v.meter_id)
                        .map(|p| {
                            Ok(ProfileRow {
                                id: ProfileId(p.id),
                                profile: ConsumptionProfile {
                                    valid_from: p.valid_from,
                                    kwh_per_100km: fixed::quantity_from_db(p.kwh_per_100km_milli),
                                    charging_loss_pct: fixed::quantity_from_db(
                                        p.charging_loss_pct_milli,
                                    ),
                                    electric_share_pct: p
                                        .electric_share_pct_milli
                                        .map(fixed::quantity_from_db),
                                    energy: p.energy.parse().map_err(data_err)?,
                                },
                            })
                        })
                        .collect::<PortResult<_>>()?,
                    holding: acquisitions
                        .iter()
                        .find(|a| a.vehicle_id == v.meter_id)
                        .map(holding_from)
                        .transpose()?,
                })
            })
            .collect()
    }
}

// @impl SPEC_CAR_VEHICLES, SPEC_SEC_TENANT_SCOPE
#[async_trait]
impl VehicleRepository for SeaOrmUow {
    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<Vehicle>> {
        let ids = self.household_meters(t).await?;
        self.load_vehicles(ids).await
    }

    async fn get(&mut self, t: &Tenant, id: MeterId) -> PortResult<Option<Vehicle>> {
        if !self.owns_meter(t, id.0).await? {
            return Ok(None);
        }
        Ok(self.load_vehicles(vec![id.0]).await?.pop())
    }

    async fn save(&mut self, t: &Tenant, id: MeterId, data: &VehicleData) -> PortResult<()> {
        self.require_vehicle_meter(t, id).await?;
        if let Some(h) = data.home_charging_meter_id {
            self.require_meter(t, h.0).await?;
        }
        vehicle::Entity::insert(vehicle::ActiveModel {
            meter_id: Set(id.0),
            plate: Set(data.plate.clone()),
            model: Set(data.model.clone()),
            drive_type: Set(data.drive_type.as_str().to_owned()),
            home_charging_meter_id: Set(data.home_charging_meter_id.map(|m| m.0)),
            pv_share_override_pct_milli: Set(data.pv_share_pct.map(quantity).transpose()?),
        })
        .on_conflict(
            OnConflict::column(vehicle::Column::MeterId)
                .update_columns([
                    vehicle::Column::Plate,
                    vehicle::Column::Model,
                    vehicle::Column::DriveType,
                    vehicle::Column::HomeChargingMeterId,
                    vehicle::Column::PvShareOverridePctMilli,
                ])
                .to_owned(),
        )
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn add_profile(&mut self, t: &Tenant, id: MeterId, p: &ProfileRow) -> PortResult<()> {
        self.require_vehicle(t, id).await?;
        let x = &p.profile;
        vehicle_consumption_profile::Entity::insert(vehicle_consumption_profile::ActiveModel {
            id: Set(p.id.0),
            vehicle_id: Set(id.0),
            valid_from: Set(x.valid_from),
            kwh_per_100km_milli: Set(quantity(x.kwh_per_100km)?),
            charging_loss_pct_milli: Set(quantity(x.charging_loss_pct)?),
            electric_share_pct_milli: Set(x.electric_share_pct.map(quantity).transpose()?),
            energy: Set(x.energy.as_str().to_owned()),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete_profile(
        &mut self,
        t: &Tenant,
        id: MeterId,
        profile: ProfileId,
    ) -> PortResult<()> {
        self.require_vehicle(t, id).await?;
        let res = vehicle_consumption_profile::Entity::delete_many()
            .filter(vehicle_consumption_profile::Column::Id.eq(profile.0))
            .filter(vehicle_consumption_profile::Column::VehicleId.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }

    async fn set_holding(
        &mut self,
        t: &Tenant,
        id: MeterId,
        h: Option<&Holding>,
    ) -> PortResult<()> {
        self.require_vehicle(t, id).await?;
        vehicle_acquisition::Entity::delete_by_id(id.0)
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        let Some(h) = h else {
            return Ok(());
        };
        vehicle_acquisition::Entity::insert(vehicle_acquisition::ActiveModel {
            vehicle_id: Set(id.0),
            purchased_on: Set(h.purchased_on),
            price_micro: Set(money(h.price)?),
            residual_micro: Set(h.residual_value.map(money).transpose()?),
            planned_end_on: Set(h.planned_end),
            sold_on: Set(h.sold_on),
            sale_price_micro: Set(h.sale_price.map(money).transpose()?),
            depreciation_mode: Set(h.mode.as_str().to_owned()),
            odometer_at_purchase_milli: Set(h.odometer_at_purchase.map(quantity).transpose()?),
            lifetime_km_milli: Set(h.lifetime_km.map(quantity).transpose()?),
            end_value_micro: Set(h.end_value.map(money).transpose()?),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn charging(&mut self, t: &Tenant, id: MeterId) -> PortResult<Vec<ChargingSession>> {
        if !self.owns_meter(t, id.0).await? {
            return Ok(Vec::new());
        }
        charging_session::Entity::find()
            .filter(charging_session::Column::VehicleId.eq(id.0))
            .order_by_asc(charging_session::Column::ChargedAt)
            .order_by_asc(charging_session::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(charging_from)
            .collect()
    }

    async fn add_charging(&mut self, t: &Tenant, c: &ChargingSession) -> PortResult<()> {
        self.require_vehicle(t, c.vehicle).await?;
        charging_session::Entity::insert(charging_session::ActiveModel {
            id: Set(c.id.0),
            vehicle_id: Set(c.vehicle.0),
            charged_at: Set(c.charged_at),
            energy_milli: Set(quantity(c.energy)?),
            amount_micro: Set(money(c.amount)?),
            odometer_milli: Set(c.odometer.map(quantity).transpose()?),
            provider: Set(c.provider.clone()),
            kind: Set(c.kind.as_str().to_owned()),
            source: Set(c.source.as_str().to_owned()),
            document_id: Set(c.document_id.map(|d| d.0)),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete_charging(&mut self, t: &Tenant, id: MeterId, c: ChargingId) -> PortResult<()> {
        self.require_vehicle_meter(t, id).await?;
        let res = charging_session::Entity::delete_many()
            .filter(charging_session::Column::Id.eq(c.0))
            .filter(charging_session::Column::VehicleId.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }
}

// @impl SPEC_ASSET_COSTS, SPEC_SEC_TENANT_SCOPE
#[async_trait]
impl AssetCostRepository for SeaOrmUow {
    async fn list(&mut self, t: &Tenant, asset: AssetRef) -> PortResult<Vec<AssetCost>> {
        asset_cost::Entity::find()
            .filter(asset_cost::Column::HouseholdId.eq(t.household_id().0))
            .filter(asset_cost::Column::AssetType.eq(asset.asset_type.as_str()))
            .filter(asset_cost::Column::AssetId.eq(asset.id))
            .order_by_asc(asset_cost::Column::BookedOn)
            .order_by_asc(asset_cost::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(cost_from)
            .collect()
    }

    async fn get(&mut self, t: &Tenant, id: AssetCostId) -> PortResult<Option<AssetCost>> {
        asset_cost::Entity::find_by_id(id.0)
            .filter(asset_cost::Column::HouseholdId.eq(t.household_id().0))
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(cost_from)
            .transpose()
    }

    async fn create(&mut self, t: &Tenant, c: &AssetCost) -> PortResult<()> {
        asset_cost::Entity::insert(cost_model(t.household_id().0, c)?)
            .exec_without_returning(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn update(&mut self, t: &Tenant, c: &AssetCost) -> PortResult<()> {
        let current = AssetCostRepository::get(self, t, c.id)
            .await?
            .ok_or(PortError::NotFound)?;
        let model = cost_model(
            t.household_id().0,
            &AssetCost {
                asset: current.asset,
                ..c.clone()
            },
        )?;
        asset_cost::Entity::update(model)
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn delete(&mut self, t: &Tenant, id: AssetCostId) -> PortResult<()> {
        let res = asset_cost::Entity::delete_many()
            .filter(asset_cost::Column::Id.eq(id.0))
            .filter(asset_cost::Column::HouseholdId.eq(t.household_id().0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }
}
