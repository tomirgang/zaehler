//! Fahrzeugauswertung: Kosten je Kategorie, Kosten pro km, Verbrauch,
//! Energie- und Ladekosten, Prognose der Fahrleistung (SPEC_CAR_REPORT).

use chrono::{Datelike, Months, NaiveDate};
use rust_decimal::Decimal;
use zaehler_domain::{
    assets::{AssetCategory, VehicleCostCategory},
    calendar::Calendar,
    cost::Estimated,
    meter::MeterPurpose,
    period::Period,
    vehicle::{
        ChargingTotals, ConsumptionProfile, CostPerKm, EnergyCost, EnergyInput, FuelFill,
        FuelSegment, cost_per_km, depreciation_in, energy_cost, fuel_average, fuel_segments,
    },
};

use crate::{
    AppError, Role, Tenant,
    ids::MeterId,
    model::{AssetCost, AssetRef, ChargingSession, MeterAggregate, Vehicle},
    usecase::{
        cost::{MeterPricing, calendar_today, load_pricing, parent_of},
        use_case,
        vehicle::load_vehicle,
    },
};

use_case!(
    /// Auswertung eines Fahrzeugs je Kalenderjahr und Haltedauer (Viewer).
    VehicleReport,
);

/// Auswertung eines Zeitraums.
#[derive(Debug, Clone)]
pub struct VehiclePeriod {
    /// Kalenderjahr; `None` für die Haltedauer
    pub year: Option<i32>,
    /// Zeitraum der Kosten (bis heute)
    pub period: Period,
    /// Gefahrene km aus der Messstelle
    pub km: Option<Estimated<Decimal>>,
    /// Kosten je Kategorie; `charging` enthält die Ladekosten
    pub categories: Vec<(VehicleCostCategory, Decimal)>,
    /// Laden extern und zu Hause (nur Fahrzeuge, die laden)
    pub energy: Option<EnergyCost>,
    /// Durchschnitt l/100 km der Vollbetankungsabschnitte
    pub fuel_per_100km: Option<Decimal>,
    pub cost: CostPerKm,
}

impl VehiclePeriod {
    /// Ein Wert des Zeitraums beruht auf einer Schätzung.
    #[must_use]
    pub fn has_estimate(&self) -> bool {
        self.km.is_some_and(|k| k.estimated)
            || self.energy.as_ref().is_some_and(|e| {
                e.home_cost.is_some_and(|c| c.estimated) || e.home_kwh.is_some_and(|k| k.estimated)
            })
    }
}

/// Prognose für das laufende Jahr (SPEC_CAR_REPORT).
#[derive(Debug, Clone)]
pub struct VehicleForecast {
    pub year: i32,
    /// Fahrleistung des Jahres (gemessen und prognostiziert)
    pub km: Estimated<Decimal>,
    /// Fixkosten des Jahres einschließlich offener wiederkehrender
    pub fixed: Decimal,
    /// Variable Kosten je km der letzten 12 Monate
    pub variable_per_km: Option<Decimal>,
    /// Fixkosten + Fahrleistung × variable Kosten je km
    pub expected: Option<Decimal>,
}

/// Heimlade-Messstelle der Auswertung.
#[derive(Debug, Clone)]
pub struct HomeMeter {
    pub id: MeterId,
    pub name: String,
    /// Wallbox-Zwischenzähler (gemessen) statt Hausstrom (Schätzung)
    pub wallbox: bool,
    /// Ladepreis aus dieser Messstelle (bei Wallbox der übergeordneten)
    pub price_meter: Option<String>,
}

/// Ergebnis von [`VehicleReport`].
#[derive(Debug, Clone)]
pub struct VehicleReportView {
    pub meter: MeterAggregate,
    pub vehicle: Vehicle,
    pub today: NaiveDate,
    /// Kalenderjahre, neueste zuerst
    pub years: Vec<VehiclePeriod>,
    pub holding: Option<VehiclePeriod>,
    pub forecast: Option<VehicleForecast>,
    pub fuel_segments: Vec<FuelSegment>,
    pub home: Option<HomeMeter>,
}

/// Daten für die Berechnung je Zeitraum.
struct Inputs {
    cal: Calendar,
    today: NaiveDate,
    vehicle: Vehicle,
    profiles: Vec<ConsumptionProfile>,
    km: MeterPricing,
    costs: Vec<AssetCost>,
    charging: Vec<ChargingSession>,
    segments: Vec<FuelSegment>,
    /// Heimlade-Messstelle und Messstelle des Ladepreises
    home: Option<(MeterPricing, Option<MeterPricing>)>,
}

impl Inputs {
    /// Bis einschließlich heute.
    fn until_today(&self, p: Period) -> Option<Period> {
        let end = p.end.min(self.today.succ_opt().unwrap_or(self.today));
        (p.start < end).then(|| Period::new(p.start, end))
    }

    /// Fahrleistung im Zeitraum, nur ab der ersten Ablesung.
    fn km(&self, p: Period) -> Option<Estimated<Decimal>> {
        let first = self.km.first_day(self.cal)?;
        let start = p.start.max(first);
        (start < p.end)
            .then(|| self.km.import_quantity(self.cal, Period::new(start, p.end)))
            .flatten()
    }

    fn energy(&self, p: Period, km: Option<Decimal>) -> Option<EnergyCost> {
        if !self.vehicle.data.drive_type.charges() {
            return None;
        }
        let mut external = ChargingTotals::default();
        for c in &self.charging {
            let d = self.cal.local_date(c.charged_at);
            if d >= p.start && d < p.end {
                external.kwh += c.energy;
                external.cost += c.amount;
            }
        }
        let (home_measured, price) = match &self.home {
            Some((home, price_meter)) => {
                let wallbox = home.agg.meter.data.purpose == MeterPurpose::Wallbox;
                let measured = if wallbox {
                    home.first_day(self.cal).and_then(|first| {
                        let start = p.start.max(first);
                        (start < p.end)
                            .then(|| home.import_quantity(self.cal, Period::new(start, p.end)))
                            .flatten()
                    })
                } else {
                    None
                };
                (measured, price_meter.as_ref().and_then(|m| m.unit_price(p)))
            }
            None => (None, None),
        };
        Some(energy_cost(&EnergyInput {
            drive: self.vehicle.data.drive_type,
            period: p,
            km,
            external,
            home_measured,
            price,
            profiles: &self.profiles,
        }))
    }

    /// Auswertung des Zeitraums; Kosten bis heute.
    // @impl SPEC_CAR_REPORT, SPEC_CAR_COST_PER_KM, SPEC_CAR_ENERGY_COST
    fn period(&self, year: Option<i32>, range: Period) -> Option<VehiclePeriod> {
        let p = self.until_today(range)?;
        let km = self.km(p);
        let energy = self.energy(p, km.map(|k| k.value));
        let mut categories: Vec<(VehicleCostCategory, Decimal)> = VehicleCostCategory::ALL
            .iter()
            .map(|c| {
                let sum: Decimal = self
                    .costs
                    .iter()
                    .filter(|e| e.category == AssetCategory::Vehicle(*c))
                    .map(|e| e.amount_in(p))
                    .sum();
                (*c, sum)
            })
            .collect();
        if let Some(e) = &energy
            && let Some(x) = categories
                .iter_mut()
                .find(|(c, _)| *c == VehicleCostCategory::Charging)
        {
            x.1 += e.total();
        }
        let variable = categories
            .iter()
            .filter(|(c, _)| c.is_variable())
            .map(|(_, v)| *v)
            .sum();
        let fixed = categories
            .iter()
            .filter(|(c, _)| !c.is_variable())
            .map(|(_, v)| *v)
            .sum();
        let depreciation = self
            .vehicle
            .holding
            .as_ref()
            .map_or(Decimal::ZERO, |h| depreciation_in(&h.acquisition(), p));
        categories.retain(|(_, v)| !v.is_zero());
        Some(VehiclePeriod {
            year,
            period: p,
            cost: cost_per_km(
                variable,
                fixed,
                depreciation.round_dp(2),
                km.map_or(Decimal::ZERO, |k| k.value),
            ),
            fuel_per_100km: fuel_average(&self.segments, p),
            km,
            categories,
            energy,
        })
    }

    /// Frühester Tag mit Daten.
    fn first_day(&self) -> Option<NaiveDate> {
        [
            self.km.first_day(self.cal),
            self.vehicle.holding.as_ref().map(|h| h.purchased_on),
            self.costs.iter().map(|c| c.booked_on).min(),
            self.charging
                .iter()
                .map(|c| self.cal.local_date(c.charged_at))
                .min(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Fahrleistung, Fixkosten und erwartete Kosten des laufenden Jahres.
    // @impl SPEC_CAR_REPORT, SPEC_FC_PERIOD_FORECAST
    fn forecast(&self) -> Option<VehicleForecast> {
        if self
            .vehicle
            .holding
            .as_ref()
            .is_some_and(|h| h.sold_on.is_some())
        {
            return None;
        }
        let year = Period::year(self.today.year());
        let first = self.km.first_day(self.cal)?;
        let km = self
            .km
            .import_quantity(self.cal, Period::new(year.start.max(first), year.end))?;
        let fixed: Decimal = self
            .costs
            .iter()
            .filter(|c| matches!(c.category, AssetCategory::Vehicle(v) if !v.is_variable()))
            .map(|c| c.amount_in(year))
            .sum();
        let last12 = Period::new(
            self.today
                .checked_sub_months(Months::new(12))
                .unwrap_or(self.today)
                .succ_opt()
                .unwrap_or(self.today),
            self.today.succ_opt().unwrap_or(self.today),
        );
        let variable_per_km = self
            .period(None, last12)
            .and_then(|p| p.cost.variable_per_km);
        Some(VehicleForecast {
            year: self.today.year(),
            expected: variable_per_km.map(|v| (fixed + km.value * v).round_dp(2)),
            km,
            fixed,
            variable_per_km,
        })
    }
}

impl VehicleReport {
    /// Kalenderjahre ab dem ersten Eintrag, die Haltedauer (Kauf bis heute
    /// bzw. Verkauf) und die Prognose des laufenden Jahres.
    // @impl SPEC_CAR_REPORT
    #[tracing::instrument(name = "VehicleReport", skip_all)]
    pub async fn execute(&self, t: &Tenant, meter: MeterId) -> Result<VehicleReportView, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let (cal, today) = calendar_today(&self.p, uow.as_mut(), t).await?;
        let vehicle = load_vehicle(uow.as_mut(), t, meter).await?;
        let km = load_pricing(uow.as_mut(), t, meter, cal, today).await?;
        let costs = uow.asset_costs().list(t, AssetRef::vehicle(meter)).await?;
        let charging = uow.vehicles().charging(t, meter).await?;
        let mut home = None;
        let mut home_view = None;
        if let Some(id) = vehicle.data.home_charging_meter_id {
            let hm = load_pricing(uow.as_mut(), t, id, cal, today).await?;
            let wallbox = hm.agg.meter.data.purpose == MeterPurpose::Wallbox;
            let price_meter = if wallbox {
                match parent_of(&hm.agg) {
                    Some(parent) => Some(load_pricing(uow.as_mut(), t, parent, cal, today).await?),
                    None => None,
                }
            } else {
                None
            };
            home_view = Some(HomeMeter {
                id,
                name: hm.agg.meter.data.name.clone(),
                wallbox,
                price_meter: if wallbox {
                    price_meter.as_ref().map(|m| m.agg.meter.data.name.clone())
                } else {
                    Some(hm.agg.meter.data.name.clone())
                },
            });
            home = Some(if wallbox {
                (hm, price_meter)
            } else {
                // Hausstrom: Preis aus derselben Messstelle, keine Messung
                let price = load_pricing(uow.as_mut(), t, id, cal, today).await?;
                (hm, Some(price))
            });
        }
        drop(uow);

        let fills: Vec<FuelFill> = costs
            .iter()
            .filter(|c| c.category == AssetCategory::Vehicle(VehicleCostCategory::Fuel))
            .filter_map(|c| {
                Some(FuelFill {
                    date: c.booked_on,
                    quantity: c.quantity?,
                    odometer: c.odometer,
                    full: c.full_tank.unwrap_or(false),
                })
            })
            .collect();
        let inputs = Inputs {
            cal,
            today,
            profiles: vehicle.consumption_profiles(),
            segments: fuel_segments(&fills),
            vehicle,
            km,
            costs,
            charging,
            home,
        };
        let last_day = inputs
            .vehicle
            .holding
            .as_ref()
            .and_then(|h| h.sold_on)
            .map_or(today, |s| s.min(today));
        let years = inputs.first_day().map_or_else(Vec::new, |first| {
            (first.year()..=last_day.year())
                .rev()
                .filter_map(|y| inputs.period(Some(y), Period::year(y)))
                .collect()
        });
        let holding = match (&inputs.vehicle.holding, inputs.first_day()) {
            (Some(h), _) => inputs.period(None, h.held(today.succ_opt().unwrap_or(today))),
            (None, Some(first)) => {
                inputs.period(None, Period::new(first, today.succ_opt().unwrap_or(today)))
            }
            (None, None) => None,
        };
        Ok(VehicleReportView {
            forecast: inputs.forecast(),
            meter: inputs.km.agg.clone(),
            fuel_segments: inputs.segments.clone(),
            vehicle: inputs.vehicle.clone(),
            today,
            years,
            holding,
            home: home_view,
        })
    }
}
