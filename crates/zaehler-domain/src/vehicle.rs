//! Fahrzeuge: Antriebsart, Wertverlust, Kosten pro Kilometer,
//! Kraftstoffverbrauch und Energiekosten beim Laden (SPEC_CAR_COST_PER_KM,
//! SPEC_CAR_DEPRECIATION, SPEC_CAR_FUEL_CONSUMPTION, SPEC_CAR_ENERGY_COST).

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{assets::VehicleCostCategory, cost::Estimated, period::Period};

/// Antriebsart (REQ_CAR_007).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveType {
    Combustion,
    Electric,
    PluginHybrid,
}

impl DriveType {
    pub const ALL: [Self; 3] = [Self::Combustion, Self::Electric, Self::PluginHybrid];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Combustion => "combustion",
            Self::Electric => "electric",
            Self::PluginHybrid => "plugin_hybrid",
        }
    }

    /// Tankt Kraftstoff.
    #[must_use]
    pub fn fuels(self) -> bool {
        self != Self::Electric
    }

    /// Lädt elektrisch.
    #[must_use]
    pub fn charges(self) -> bool {
        self != Self::Combustion
    }

    /// Kategorie im Kostenbuch zulässig: Verbrenner ohne Laden, Elektro
    /// ohne Kraftstoff (SPEC_CAR_VEHICLES).
    // @impl SPEC_CAR_VEHICLES
    #[must_use]
    pub fn allows(self, category: VehicleCostCategory) -> bool {
        match category {
            VehicleCostCategory::Fuel => self.fuels(),
            VehicleCostCategory::Charging => self.charges(),
            _ => true,
        }
    }
}

impl std::str::FromStr for DriveType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|x| x.as_str() == s)
            .ok_or_else(|| format!("unbekannte Antriebsart: {s}"))
    }
}

/// Anschaffung für die Wertverlust-Berechnung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acquisition {
    pub date: NaiveDate,
    pub price: Decimal,
    /// Geschätzter Restwert am Ende der geplanten Haltedauer bzw. Verkaufserlös.
    pub residual_value: Decimal,
    /// Ende der Haltedauer (ausschließlich)
    pub planned_end: NaiveDate,
}

/// Erfasste Anschaffung mit optionalem Verkauf (REQ_CAR_004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holding {
    pub purchased_on: NaiveDate,
    pub price: Decimal,
    pub residual_value: Decimal,
    pub planned_end: NaiveDate,
    pub sold_on: Option<NaiveDate>,
    pub sale_price: Option<Decimal>,
}

impl Holding {
    /// Bei Verkauf ersetzt der Erlös den Restwert, die Haltedauer endet mit
    /// dem Verkauf (SPEC_CAR_DEPRECIATION).
    #[must_use]
    pub fn acquisition(&self) -> Acquisition {
        match (self.sold_on, self.sale_price) {
            (Some(sold), Some(price)) => Acquisition {
                date: self.purchased_on,
                price: self.price,
                residual_value: price,
                planned_end: sold,
            },
            _ => Acquisition {
                date: self.purchased_on,
                price: self.price,
                residual_value: self.residual_value,
                planned_end: self.sold_on.unwrap_or(self.planned_end),
            },
        }
    }

    /// Haltedauer bis `today` (ausschließlich) bzw. bis zum Verkauf.
    #[must_use]
    pub fn held(&self, today: NaiveDate) -> Period {
        let end = self.sold_on.unwrap_or(today).max(self.purchased_on);
        Period::new(self.purchased_on, end)
    }
}

/// Linearer Wertverlust im Zeitraum (tagesgenau).
// @impl SPEC_CAR_DEPRECIATION
#[must_use]
pub fn depreciation_in(acq: &Acquisition, period: Period) -> Decimal {
    let total_days = (acq.planned_end - acq.date).num_days();
    if total_days <= 0 {
        return Decimal::ZERO;
    }
    let start = period.start.max(acq.date);
    let end = period.end.min(acq.planned_end);
    let days = (end - start).num_days().max(0);
    (acq.price - acq.residual_value) * Decimal::from(days) / Decimal::from(total_days)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostPerKm {
    pub km: Decimal,
    pub variable: Decimal,
    pub fixed: Decimal,
    pub depreciation: Decimal,
    /// Variable Kosten pro km.
    pub variable_per_km: Option<Decimal>,
    /// Vollkosten (variabel + fix + Wertverlust) pro km.
    pub total_per_km: Option<Decimal>,
}

/// Kosten pro km aus den Summen eines Zeitraums; ohne gefahrene km
/// undefiniert.
// @impl SPEC_CAR_COST_PER_KM
#[must_use]
pub fn cost_per_km(
    variable: Decimal,
    fixed: Decimal,
    depreciation: Decimal,
    km: Decimal,
) -> CostPerKm {
    let per_km = |v: Decimal| (km > Decimal::ZERO).then(|| (v / km).round_dp(4));
    CostPerKm {
        km,
        variable,
        fixed,
        depreciation,
        variable_per_km: per_km(variable),
        total_per_km: per_km(variable + fixed + depreciation),
    }
}

/// Betankung für die Vollbetankungsmethode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuelFill {
    pub date: NaiveDate,
    /// Liter
    pub quantity: Decimal,
    pub odometer: Option<Decimal>,
    pub full: bool,
}

/// Abschnitt zwischen zwei Vollbetankungen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuelSegment {
    /// Tag der Vollbetankung am Ende des Abschnitts
    pub until: NaiveDate,
    pub km: Decimal,
    pub litres: Decimal,
    pub per_100km: Decimal,
}

/// Abschnitte zwischen aufeinanderfolgenden Vollbetankungen: Summe der
/// Mengen nach der ersten bis einschließlich der zweiten Vollbetankung je
/// gefahrenem km. Fehlt an einer der beiden Vollbetankungen der
/// Kilometerstand oder ist `km_j ≤ km_i`, entfällt der Abschnitt.
// @impl SPEC_CAR_FUEL_CONSUMPTION
#[must_use]
pub fn fuel_segments(fills: &[FuelFill]) -> Vec<FuelSegment> {
    let mut sorted: Vec<&FuelFill> = fills.iter().collect();
    sorted.sort_by_key(|f| (f.date, f.odometer));
    let mut out = Vec::new();
    let mut last_full: Option<&FuelFill> = None;
    let mut litres = Decimal::ZERO;
    for f in sorted {
        litres += f.quantity;
        if !f.full {
            continue;
        }
        if let Some(start) = last_full
            && let (Some(a), Some(b)) = (start.odometer, f.odometer)
            && b > a
        {
            let km = b - a;
            out.push(FuelSegment {
                until: f.date,
                km,
                litres,
                per_100km: (litres / km * Decimal::ONE_HUNDRED).round_dp(2),
            });
        }
        last_full = Some(f);
        litres = Decimal::ZERO;
    }
    out
}

/// Durchschnitt l/100 km der Abschnitte, die im Zeitraum enden.
// @impl SPEC_CAR_FUEL_CONSUMPTION
#[must_use]
pub fn fuel_average(segments: &[FuelSegment], period: Period) -> Option<Decimal> {
    let (km, litres) = segments
        .iter()
        .filter(|s| s.until >= period.start && s.until < period.end)
        .fold((Decimal::ZERO, Decimal::ZERO), |(k, l), s| {
            (k + s.km, l + s.litres)
        });
    (km > Decimal::ZERO).then(|| (litres / km * Decimal::ONE_HUNDRED).round_dp(2))
}

/// Verbrauchsprofil ab `valid_from` (REQ_CAR_009).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumptionProfile {
    pub valid_from: NaiveDate,
    pub kwh_per_100km: Decimal,
    pub charging_loss_pct: Decimal,
    /// Elektrischer Fahranteil in Prozent (Plug-in-Hybrid)
    pub electric_share_pct: Option<Decimal>,
}

/// Tagesanteile der Profile im Zeitraum; Tage vor dem ersten Profil
/// zählen zum ersten.
fn profile_shares(
    profiles: &[ConsumptionProfile],
    period: Period,
) -> Vec<(Decimal, &ConsumptionProfile)> {
    let mut sorted: Vec<&ConsumptionProfile> = profiles.iter().collect();
    sorted.sort_by_key(|p| p.valid_from);
    let total = period.days();
    if total <= 0 || sorted.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, p) in sorted.iter().enumerate() {
        let start = if i == 0 {
            period.start
        } else {
            p.valid_from.max(period.start)
        };
        let end = sorted
            .get(i + 1)
            .map_or(period.end, |n| n.valid_from.min(period.end));
        let days = (end - start).num_days();
        if days > 0 {
            out.push((Decimal::from(days) / Decimal::from(total), *p));
        }
    }
    out
}

/// Hinterlegter Durchschnittsverbrauch im Zeitraum (kWh/100 km,
/// tagesanteilig).
#[must_use]
pub fn profile_consumption(profiles: &[ConsumptionProfile], period: Period) -> Option<Decimal> {
    let shares = profile_shares(profiles, period);
    (!shares.is_empty()).then(|| {
        shares
            .iter()
            .map(|(w, p)| *w * p.kwh_per_100km)
            .sum::<Decimal>()
            .round_dp(2)
    })
}

/// Energiebedarf `km · a · v / 100 · (1 + l / 100)` mit tagesanteiligen
/// Profilen; ohne Profil, bei Verbrennern und bei Plug-in-Hybriden ohne
/// elektrischen Fahranteil `None`.
// @impl SPEC_CAR_ENERGY_COST
#[must_use]
pub fn energy_demand(
    drive: DriveType,
    km: Decimal,
    profiles: &[ConsumptionProfile],
    period: Period,
) -> Option<Decimal> {
    let shares = profile_shares(profiles, period);
    if shares.is_empty() {
        return None;
    }
    let mut demand = Decimal::ZERO;
    for (w, p) in shares {
        let a = match drive {
            DriveType::Combustion => return None,
            DriveType::Electric => Decimal::ONE,
            DriveType::PluginHybrid => p.electric_share_pct? / Decimal::ONE_HUNDRED,
        };
        demand += w * km * a * p.kwh_per_100km / Decimal::ONE_HUNDRED
            * (Decimal::ONE + p.charging_loss_pct / Decimal::ONE_HUNDRED);
    }
    Some(demand)
}

/// Externe Ladevorgänge eines Zeitraums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChargingTotals {
    pub kwh: Decimal,
    pub cost: Decimal,
}

/// Eingaben der Energiekosten eines Zeitraums.
#[derive(Debug, Clone)]
pub struct EnergyInput<'a> {
    pub drive: DriveType,
    pub period: Period,
    /// Gefahrene km im Zeitraum
    pub km: Option<Decimal>,
    pub external: ChargingTotals,
    /// Energie des Heimlade-Zwischenzählers (teilweise geschätzt, wenn
    /// Ablesungen fehlen)
    pub home_measured: Option<Estimated<Decimal>>,
    /// Ladepreis je kWh beim Heimladen
    pub price: Option<Estimated<Decimal>>,
    pub profiles: &'a [ConsumptionProfile],
}

/// Energie und Kosten beim Laden (SPEC_CAR_ENERGY_COST).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnergyCost {
    pub external: ChargingTotals,
    pub home_kwh: Option<Estimated<Decimal>>,
    pub home_cost: Option<Estimated<Decimal>>,
    /// Gemessen nur mit Heimlade-Zwischenzähler, sonst hinterlegt
    pub kwh_per_100km: Option<Estimated<Decimal>>,
}

impl EnergyCost {
    /// Ladekosten gesamt (Kategorie `charging`).
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.external.cost + self.home_cost.map_or(Decimal::ZERO, |c| c.value)
    }
}

/// Heimladen gemessen (Zwischenzähler) oder geschätzt aus Fahrleistung und
/// Profil abzüglich externer Ladung; Kosten mit dem Ladepreis.
// @impl SPEC_CAR_ENERGY_COST
#[must_use]
pub fn energy_cost(input: &EnergyInput<'_>) -> EnergyCost {
    let home_kwh = match input.home_measured {
        Some(kwh) => Some(kwh),
        None => input
            .km
            .and_then(|km| energy_demand(input.drive, km, input.profiles, input.period))
            .map(|demand| Estimated {
                value: (demand - input.external.kwh).max(Decimal::ZERO).round_dp(3),
                estimated: true,
            }),
    };
    let home_cost = match (home_kwh, input.price) {
        (Some(e), Some(p)) => Some(Estimated {
            value: (e.value * p.value).round_dp(2),
            estimated: e.estimated || p.estimated,
        }),
        _ => None,
    };
    let kwh_per_100km = match (input.home_measured, input.km) {
        (Some(home), Some(km)) if km > Decimal::ZERO && !home.estimated => Some(Estimated {
            value: ((home.value + input.external.kwh) / km * Decimal::ONE_HUNDRED).round_dp(2),
            estimated: false,
        }),
        _ if input.drive.charges() => {
            profile_consumption(input.profiles, input.period).map(|v| Estimated {
                value: v,
                estimated: true,
            })
        }
        _ => None,
    };
    EnergyCost {
        external: input.external,
        home_kwh,
        home_cost,
        kwh_per_100km,
    }
}

/// Kalenderjahre von `from` bis einschließlich `to`.
#[must_use]
pub fn years(from: NaiveDate, to: NaiveDate) -> Vec<i32> {
    (from.year()..=to.year().max(from.year())).collect()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    // @test SPEC_CAR_COST_PER_KM, SPEC_CAR_DEPRECIATION
    #[test]
    fn full_cost_per_km() {
        let acq = Acquisition {
            date: d(2026, 1, 1),
            price: dec!(30000),
            residual_value: dec!(10000),
            planned_end: d(2031, 1, 1),
        };
        let dep = depreciation_in(&acq, Period::year(2026));
        // 20000 € Wertverlust über 1826 Tage, davon 365 in 2026
        assert_eq!(dep.round_dp(2), dec!(3997.81));
        let r = cost_per_km(dec!(1000), dec!(500), dep, dec!(10000));
        assert_eq!(r.variable_per_km, Some(dec!(0.1)));
        assert_eq!(r.total_per_km, Some(dec!(0.5498)));
        assert_eq!(
            cost_per_km(dec!(1), dec!(1), dec!(0), dec!(0)).total_per_km,
            None
        );
    }

    // @test SPEC_CAR_DEPRECIATION
    #[test]
    fn depreciation_is_linear_within_the_holding_period() {
        let acq = Acquisition {
            date: d(2026, 7, 1),
            price: dec!(12000),
            residual_value: dec!(2000),
            planned_end: d(2028, 7, 1),
        };
        // 731 Tage Haltedauer, davon 184 in 2026, 365 in 2027, 182 in 2028
        let parts: Vec<Decimal> = [2025, 2026, 2027, 2028, 2029]
            .map(|y| depreciation_in(&acq, Period::year(y)))
            .to_vec();
        assert_eq!(parts[0], Decimal::ZERO, "vor dem Kauf");
        assert_eq!(parts[4], Decimal::ZERO, "nach dem geplanten Ende");
        assert_eq!(parts.iter().sum::<Decimal>().round_dp(6), dec!(10000));
        assert_eq!(parts[1].round_dp(2), dec!(2517.10));
        // Verkauf: Verkaufserlös ersetzt den Restwert, Ende beim Verkauf
        let holding = Holding {
            purchased_on: d(2026, 7, 1),
            price: dec!(12000),
            residual_value: dec!(2000),
            planned_end: d(2028, 7, 1),
            sold_on: Some(d(2027, 7, 1)),
            sale_price: Some(dec!(9000)),
        };
        assert_eq!(
            depreciation_in(
                &holding.acquisition(),
                Period::new(d(2026, 1, 1), d(2030, 1, 1))
            ),
            dec!(3000)
        );
        assert_eq!(holding.held(d(2030, 1, 1)).end, d(2027, 7, 1));
        // Haltedauer ohne Tage
        let empty = Acquisition {
            planned_end: d(2026, 7, 1),
            ..acq
        };
        assert_eq!(depreciation_in(&empty, Period::year(2026)), Decimal::ZERO);
    }

    fn fill(day: u32, litres: Decimal, km: Option<Decimal>, full: bool) -> FuelFill {
        FuelFill {
            date: d(2026, 3, day),
            quantity: litres,
            odometer: km,
            full,
        }
    }

    // @test SPEC_CAR_FUEL_CONSUMPTION
    #[test]
    fn full_tank_method_with_partial_fills_and_missing_odometer() {
        let fills = [
            fill(1, dec!(40), Some(dec!(10000)), true),
            // Teilbetankung ohne Kilometerstand zählt zur Menge
            fill(5, dec!(10), None, false),
            fill(10, dec!(30), Some(dec!(10800)), true),
            // Vollbetankung ohne Kilometerstand: Abschnitt entfällt
            fill(15, dec!(35), None, true),
            fill(20, dec!(36), Some(dec!(11900)), true),
            // Rückwärts gezählt: entfällt
            fill(25, dec!(30), Some(dec!(11800)), true),
        ];
        let s = fuel_segments(&fills);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].km, s[0].litres), (dec!(800), dec!(40)));
        assert_eq!(s[0].per_100km, dec!(5));
        assert_eq!(fuel_average(&s, Period::year(2026)), Some(dec!(5)));
        assert_eq!(fuel_average(&s, Period::year(2025)), None);
    }

    fn profile(from: NaiveDate, kwh: Decimal, share: Option<Decimal>) -> ConsumptionProfile {
        ConsumptionProfile {
            valid_from: from,
            kwh_per_100km: kwh,
            charging_loss_pct: dec!(10),
            electric_share_pct: share,
        }
    }

    // @test SPEC_CAR_ENERGY_COST
    #[test]
    fn electric_vehicle_with_wallbox_meter() {
        let p = [profile(d(2026, 1, 1), dec!(18), None)];
        let e = energy_cost(&EnergyInput {
            drive: DriveType::Electric,
            period: Period::year(2026),
            km: Some(dec!(10000)),
            external: ChargingTotals {
                kwh: dec!(400),
                cost: dec!(200),
            },
            home_measured: Some(Estimated {
                value: dec!(1600),
                estimated: false,
            }),
            price: Some(Estimated {
                value: dec!(0.30),
                estimated: false,
            }),
            profiles: &p,
        });
        assert_eq!(
            e.home_cost,
            Some(Estimated {
                value: dec!(480),
                estimated: false
            })
        );
        assert_eq!(e.total(), dec!(680));
        // gemessen: (1600 + 400) / 10000 km
        assert_eq!(
            e.kwh_per_100km,
            Some(Estimated {
                value: dec!(20),
                estimated: false
            })
        );
    }

    // @test SPEC_CAR_ENERGY_COST
    #[test]
    fn estimate_without_submeter_uses_profiles_by_day() {
        // Winter 22 kWh bis Ende März, danach 16 kWh; 10 % Ladeverlust
        let p = [
            profile(d(2026, 1, 1), dec!(22), None),
            profile(d(2026, 4, 1), dec!(16), None),
        ];
        let period = Period::new(d(2026, 3, 1), d(2026, 5, 1));
        // 31 Tage zu 22, 30 Tage zu 16: 2000 km
        let demand = energy_demand(DriveType::Electric, dec!(2000), &p, period).unwrap();
        let expected =
            dec!(2000) * (dec!(31) * dec!(22) + dec!(30) * dec!(16)) / dec!(61) / dec!(100)
                * dec!(1.1);
        assert_eq!(demand.round_dp(6), expected.round_dp(6));
        let e = energy_cost(&EnergyInput {
            drive: DriveType::Electric,
            period,
            km: Some(dec!(2000)),
            external: ChargingTotals {
                kwh: dec!(100),
                cost: dec!(0),
            },
            home_measured: None,
            price: Some(Estimated {
                value: dec!(0.30),
                estimated: false,
            }),
            profiles: &p,
        });
        let home = e.home_kwh.unwrap();
        assert!(home.estimated && e.home_cost.unwrap().estimated);
        assert_eq!(home.value, (expected - dec!(100)).round_dp(3));
        assert!(e.kwh_per_100km.unwrap().estimated, "hinterlegt");
        // kostenloses externes Laden: nur Heimkosten
        assert_eq!(e.total(), e.home_cost.unwrap().value);
    }

    // @test SPEC_CAR_ENERGY_COST
    #[test]
    fn plugin_hybrid_uses_electric_share() {
        let period = Period::year(2026);
        let with = [profile(d(2026, 1, 1), dec!(20), Some(dec!(50)))];
        assert_eq!(
            energy_demand(DriveType::PluginHybrid, dec!(10000), &with, period),
            Some(dec!(1100))
        );
        let without = [profile(d(2026, 1, 1), dec!(20), None)];
        assert_eq!(
            energy_demand(DriveType::PluginHybrid, dec!(10000), &without, period),
            None
        );
        assert_eq!(
            energy_demand(DriveType::Combustion, dec!(10000), &with, period),
            None
        );
        // mehr extern geladen als benötigt: kein negativer Heimanteil
        let e = energy_cost(&EnergyInput {
            drive: DriveType::PluginHybrid,
            period,
            km: Some(dec!(10000)),
            external: ChargingTotals {
                kwh: dec!(2000),
                cost: dec!(700),
            },
            home_measured: None,
            price: None,
            profiles: &with,
        });
        assert_eq!(e.home_kwh.unwrap().value, Decimal::ZERO);
        assert_eq!(e.home_cost, None, "ohne Ladepreis keine Heimkosten");
    }

    #[test]
    fn drive_type_controls_categories() {
        assert!(!DriveType::Combustion.allows(VehicleCostCategory::Charging));
        assert!(!DriveType::Electric.allows(VehicleCostCategory::Fuel));
        assert!(DriveType::PluginHybrid.allows(VehicleCostCategory::Fuel));
        assert!(DriveType::PluginHybrid.allows(VehicleCostCategory::Charging));
        assert_eq!("plugin_hybrid".parse(), Ok(DriveType::PluginHybrid));
    }
}
