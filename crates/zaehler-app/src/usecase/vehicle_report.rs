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
        ChargingTotals, ConsumptionProfile, CostPerKm, DepreciationMode, EnergyCost, EnergyInput,
        FuelFill, FuelSegment, Remaining, cost_per_km, energy_cost, estimated_fuel_price,
        fuel_average, fuel_profile_consumption, fuel_segments, last_fuel_price, remaining_time,
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
    /// Fixkosten + Fahrleistung × variable Kosten je km; mit
    /// Kraftstoffprognose ersetzt diese den Kraftstoffanteil
    pub expected: Option<Decimal>,
    /// Kraftstoffkosten im Rest des Jahres (SPEC_CAR_FUEL_PRICE)
    pub fuel_cost: Option<Estimated<Decimal>>,
}

/// Restwert und Restlebensdauer (SPEC_CAR_MILEAGE_DEPRECIATION).
#[derive(Debug, Clone, Copy)]
pub struct ResidualInfo {
    pub mode: DepreciationMode,
    /// Kaufpreis abzüglich Wertverlust bis heute, bei Verkauf der Erlös
    pub value: Decimal,
    pub remaining: Remaining,
    /// Jahresfahrleistung der letzten 12 Monate (Laufleistungsmodus)
    pub annual_km: Option<Decimal>,
}

/// Preis pro Liter (SPEC_CAR_FUEL_PRICE).
#[derive(Debug, Clone)]
pub struct FuelPriceInfo {
    /// Letzter bekannter Preis mit Datum
    pub last: Option<(NaiveDate, Decimal)>,
    /// Geschätzter Durchschnitt der letzten 3 Monate, nur ohne bekannten Preis
    pub estimate: Option<Decimal>,
    /// Verlauf: Tankvorgänge mit bekanntem Preis, nach Datum
    pub points: Vec<(NaiveDate, Decimal)>,
}

impl FuelPriceInfo {
    /// Preis für die Prognose: letzter bekannter, sonst die Schätzung.
    #[must_use]
    pub fn for_forecast(&self) -> Option<Estimated<Decimal>> {
        self.last
            .map(|(_, p)| Estimated {
                value: p,
                estimated: false,
            })
            .or(self.estimate.map(|p| Estimated {
                value: p,
                estimated: true,
            }))
    }
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
    pub residual: Option<ResidualInfo>,
    /// Nur für Fahrzeuge, die tanken
    pub fuel_price: Option<FuelPriceInfo>,
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
    /// PV am Hausanschluss der Preis-Messstelle (Mischpreis, WP_V1_18)
    pv: Option<super::pv::GridSource>,
}

impl Inputs {
    /// Bis einschließlich heute.
    fn until_today(&self, p: Period) -> Option<Period> {
        let end = p.end.min(self.today.succ_opt().unwrap_or(self.today));
        (p.start < end).then(|| Period::new(p.start, end))
    }

    /// Kilometerstand zu Beginn des Tages.
    fn odometer(&self, d: NaiveDate) -> Option<Decimal> {
        self.km.value_at(self.cal, d)
    }

    /// Morgen, Ende der Zeiträume bis einschließlich heute.
    fn tomorrow(&self) -> NaiveDate {
        self.today.succ_opt().unwrap_or(self.today)
    }

    /// Die letzten `months` Monate bis einschließlich heute.
    fn last_months(&self, months: u32) -> Period {
        let start = self
            .today
            .checked_sub_months(Months::new(months))
            .unwrap_or(self.today)
            .succ_opt()
            .unwrap_or(self.today);
        Period::new(start, self.tomorrow())
    }

    /// Jahresfahrleistung aus den letzten 12 Monaten ab der ersten Ablesung;
    /// mit weniger als 90 Tagen undefiniert.
    fn annual_km(&self) -> Option<Decimal> {
        let last12 = self.last_months(12);
        let first = self.km.first_day(self.cal)?;
        let p = Period::new(last12.start.max(first), last12.end);
        if p.days() < 90 {
            return None;
        }
        let km = self.km.import_quantity(self.cal, p)?.value;
        Some((km * Decimal::from(365) / Decimal::from(p.days())).round_dp(0))
    }

    /// Restwert und Restlebensdauer heute.
    // @impl SPEC_CAR_MILEAGE_DEPRECIATION
    fn residual(&self) -> Option<ResidualInfo> {
        let h = self.vehicle.holding.as_ref()?;
        let sold = h.sold_on.is_some_and(|s| s <= self.today);
        let lost = h.depreciation_in(
            Period::new(h.purchased_on, self.tomorrow().max(h.purchased_on)),
            |d| self.odometer(d),
        );
        let value = match (sold, h.sale_price) {
            (true, Some(p)) => p,
            _ => (h.price - lost).round_dp(2),
        };
        let (remaining, annual_km) = match h.mode {
            DepreciationMode::Time => (
                h.acquisition().map_or(Remaining::InsufficientData, |a| {
                    remaining_time(&a, sold, self.today)
                }),
                None,
            ),
            DepreciationMode::Mileage => {
                let annual = self.annual_km();
                (
                    h.mileage().map_or(Remaining::InsufficientData, |m| {
                        m.remaining(self.today, self.odometer(self.tomorrow()), annual)
                    }),
                    annual,
                )
            }
        };
        Some(ResidualInfo {
            mode: h.mode,
            value,
            remaining,
            annual_km,
        })
    }

    /// Tankvorgänge (Kategorie Kraftstoff).
    fn fuel_entries(&self) -> impl Iterator<Item = &AssetCost> {
        self.costs
            .iter()
            .filter(|c| c.category == AssetCategory::Vehicle(VehicleCostCategory::Fuel))
    }

    /// Ø-Verbrauch l/100 km der letzten 12 Monate, ersatzweise Profil.
    fn fuel_consumption(&self) -> Option<Decimal> {
        let last12 = self.last_months(12);
        fuel_average(&self.segments, last12)
            .or_else(|| fuel_profile_consumption(&self.profiles, last12))
    }

    /// Letzter Preis, Schätzung und Verlauf (SPEC_CAR_FUEL_PRICE).
    // @impl SPEC_CAR_FUEL_PRICE
    fn fuel_price(&self) -> Option<FuelPriceInfo> {
        if !self.vehicle.data.drive_type.fuels() {
            return None;
        }
        let mut points: Vec<(NaiveDate, Decimal)> = self
            .fuel_entries()
            .filter(|c| c.booked_on <= self.today)
            .filter_map(|c| Some((c.booked_on, c.fuel_unit_price()?)))
            .collect();
        points.sort_by_key(|(d, _)| *d);
        let last = last_fuel_price(points.iter().map(|(d, p)| (*d, Some(*p))));
        let estimate = if last.is_none() {
            let p = self.last_months(3);
            let cost: Decimal = self.fuel_entries().map(|c| c.amount_in(p)).sum();
            match (self.km(p), self.fuel_consumption()) {
                (Some(km), Some(v)) => estimated_fuel_price(cost, km.value, v),
                _ => None,
            }
        } else {
            None
        };
        Some(FuelPriceInfo {
            last,
            estimate,
            points,
        })
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
                let price = price_meter.as_ref().and_then(|m| {
                    let house = m.unit_price(p)?;
                    // mit PV: Mischpreis aus Einspeisevergütung und
                    // Arbeitspreis, immer als Schätzung (REQ_CAR_011)
                    Some(match &self.pv {
                        Some(src) => super::pv::pv_charging_price(
                            src,
                            m,
                            p,
                            self.vehicle.data.pv_share_pct,
                            house.value,
                        )
                        .map_or(house, |v| Estimated {
                            value: v,
                            estimated: true,
                        }),
                        None => house,
                    })
                });
                (measured, price)
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
        let depreciation = self.vehicle.holding.as_ref().map_or(Decimal::ZERO, |h| {
            h.depreciation_in(p, |d| self.odometer(d))
        });
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
        let last = self.period(None, last12);
        let variable_per_km = last.as_ref().and_then(|p| p.cost.variable_per_km);
        let mut expected = variable_per_km.map(|v| (fixed + km.value * v).round_dp(2));
        // Kraftstoff im Rest des Jahres aus Literpreis und Verbrauch; ersetzt
        // den Kraftstoffanteil der variablen Kosten (SPEC_CAR_FUEL_PRICE)
        let fuel_cost = self.fuel_forecast(year, km.value);
        if let (Some(fc), Some(last)) = (fuel_cost, &last)
            && last.cost.km > Decimal::ZERO
        {
            let fuel_last: Decimal = last
                .categories
                .iter()
                .filter(|(c, _)| *c == VehicleCostCategory::Fuel)
                .map(|(_, v)| *v)
                .sum();
            let other_per_km = (last.cost.variable - fuel_last) / last.cost.km;
            let fuel_so_far: Decimal = self
                .fuel_entries()
                .map(|c| c.amount_in(Period::new(year.start, self.tomorrow())))
                .sum();
            expected = Some((fixed + km.value * other_per_km + fuel_so_far + fc.value).round_dp(2));
        }
        Some(VehicleForecast {
            year: self.today.year(),
            expected,
            km,
            fixed,
            variable_per_km,
            fuel_cost,
        })
    }

    /// `prognostizierte km Restjahr · v / 100 · Preis` (SPEC_CAR_FUEL_PRICE).
    // @impl SPEC_CAR_FUEL_PRICE
    fn fuel_forecast(&self, year: Period, km_year: Decimal) -> Option<Estimated<Decimal>> {
        let price = self.fuel_price()?.for_forecast()?;
        let v = self.fuel_consumption()?;
        let driven = self
            .km(Period::new(year.start, self.tomorrow()))
            .map_or(Decimal::ZERO, |k| k.value);
        // Der Vollbetankungsverbrauch bezieht sich auf alle km, beim
        // Plug-in-Hybrid also bereits auf den Mix aus elektrisch und Kraftstoff
        let rest = (km_year - driven).max(Decimal::ZERO);
        Some(Estimated {
            value: (rest * v / Decimal::ONE_HUNDRED * price.value).round_dp(2),
            estimated: true,
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
        let pv = match home.as_ref().and_then(|(_, m)| m.as_ref()) {
            Some(m) => super::pv::GridSource::load(uow.as_mut(), t, m.agg.meter.id, cal).await?,
            None => None,
        };
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
            pv,
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
            residual: inputs.residual(),
            fuel_price: inputs.fuel_price(),
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
