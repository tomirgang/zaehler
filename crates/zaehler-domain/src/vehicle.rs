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

/// Verteilung des Wertverlusts: nach Tagen der Haltedauer oder nach
/// gefahrenen km bis zur Lebenslaufleistung (REQ_CAR_004, REQ_CAR_012).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepreciationMode {
    #[default]
    Time,
    Mileage,
}

impl DepreciationMode {
    pub const ALL: [Self; 2] = [Self::Time, Self::Mileage];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Mileage => "mileage",
        }
    }
}

impl std::str::FromStr for DepreciationMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|x| x.as_str() == s)
            .ok_or_else(|| format!("unbekannter Modus des Wertverlusts: {s}"))
    }
}

/// Erfasste Anschaffung mit optionalem Verkauf (REQ_CAR_004). Die Werte
/// beider Modi bleiben gespeichert, `mode` wählt (SPEC_CAR_MILEAGE_UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holding {
    pub purchased_on: NaiveDate,
    pub price: Decimal,
    pub mode: DepreciationMode,
    /// Erstzulassung; ohne Angabe gilt das Fahrzeug beim Kauf als neu
    #[serde(default)]
    pub first_registration: Option<NaiveDate>,
    /// Zeitmodus: geschätzter Restwert am geplanten Ende
    pub residual_value: Option<Decimal>,
    /// Zeitmodus: Ende der Haltedauer (ausschließlich)
    pub planned_end: Option<NaiveDate>,
    /// Laufleistungsmodus: Kilometerstand bei Kauf
    pub odometer_at_purchase: Option<Decimal>,
    /// Laufleistungsmodus: Lebenslaufleistung als absoluter Kilometerstand,
    /// nur für die Restlebensdauer
    pub lifetime_km: Option<Decimal>,
    /// Nicht mehr verwendet (Endwert des früheren linearen
    /// Laufleistungsmodus); bleibt für ältere Daten erhalten
    pub end_value: Option<Decimal>,
    pub sold_on: Option<NaiveDate>,
    pub sale_price: Option<Decimal>,
}

/// Restwertkurve: Anteil des Neupreises nach Fahrzeugalter in Jahren
/// (SPEC_CAR_DEPRECIATION). Zwischen den Stützstellen linear.
const RESIDUAL_CURVE: [(i64, i64); 5] = [(0, 1000), (1, 775), (3, 500), (5, 425), (10, 250)];

/// Nach zehn Jahren je Jahr 2 %-Punkte weniger, höchstens bis 10 %.
const LATE_LOSS_PER_YEAR: Decimal = Decimal::from_parts(2, 0, 0, false, 2);
const RESIDUAL_FLOOR: Decimal = Decimal::from_parts(10, 0, 0, false, 2);

/// Durchschnittliche Jahresfahrleistung und Zu- bzw. Abschlag je 10.000 km
/// Abweichung (SPEC_CAR_MILEAGE_DEPRECIATION).
pub const AVERAGE_KM_PER_YEAR: Decimal = Decimal::from_parts(13_500, 0, 0, false, 0);
const MILEAGE_SHARE_PER_10K: Decimal = Decimal::from_parts(15, 0, 0, false, 3);

const DAYS_PER_YEAR: Decimal = Decimal::from_parts(36_525, 0, 0, false, 2);

/// Restwert in Anteilen des Neupreises für ein Fahrzeugalter in Jahren.
// @impl SPEC_CAR_DEPRECIATION
#[must_use]
pub fn residual_ratio(age_years: Decimal) -> Decimal {
    let age = age_years.max(Decimal::ZERO);
    for pair in RESIDUAL_CURVE.windows(2) {
        let (y0, r0) = (Decimal::from(pair[0].0), Decimal::new(pair[0].1, 3));
        let (y1, r1) = (Decimal::from(pair[1].0), Decimal::new(pair[1].1, 3));
        if age <= y1 {
            return r0 + (r1 - r0) * (age - y0) / (y1 - y0);
        }
    }
    let (last_year, last) = RESIDUAL_CURVE[RESIDUAL_CURVE.len() - 1];
    (Decimal::new(last, 3) - LATE_LOSS_PER_YEAR * (age - Decimal::from(last_year)))
        .max(RESIDUAL_FLOOR)
}

/// Jahre zwischen zwei Tagen (365,25 Tage je Jahr), nicht negativ.
fn years_between(from: NaiveDate, to: NaiveDate) -> Decimal {
    Decimal::from((to - from).num_days().max(0)) / DAYS_PER_YEAR
}

impl Holding {
    /// Anschaffung des Zeitmodus. Bei Verkauf ersetzt der Erlös den
    /// Restwert, die Haltedauer endet mit dem Verkauf
    /// (SPEC_CAR_DEPRECIATION); ohne Restwert und Halteende `None`.
    #[must_use]
    pub fn acquisition(&self) -> Option<Acquisition> {
        if self.mode != DepreciationMode::Time {
            return None;
        }
        match (self.sold_on, self.sale_price) {
            (Some(sold), Some(price)) => Some(Acquisition {
                date: self.purchased_on,
                price: self.price,
                residual_value: price,
                planned_end: sold,
            }),
            _ => Some(Acquisition {
                date: self.purchased_on,
                price: self.price,
                residual_value: self.residual_value?,
                planned_end: self.sold_on.or(self.planned_end)?,
            }),
        }
    }

    /// Grundlage der Restlebensdauer im Laufleistungsmodus.
    #[must_use]
    pub fn mileage(&self) -> Option<Mileage> {
        if self.mode != DepreciationMode::Mileage {
            return None;
        }
        Some(Mileage {
            odometer_at_purchase: self.odometer_at_purchase?,
            lifetime_km: self.lifetime_km,
            sold_on: self.sold_on,
        })
    }

    /// Erstzulassung, ohne Angabe der Kauftag.
    fn registered(&self) -> NaiveDate {
        self.first_registration
            .unwrap_or(self.purchased_on)
            .min(self.purchased_on)
    }

    /// Wert nach der Restwertkurve relativ zum Kaufpreis am Tag `d`.
    fn curve_value(&self, d: NaiveDate) -> Decimal {
        let d = d.max(self.purchased_on);
        let at_purchase = residual_ratio(years_between(self.registered(), self.purchased_on));
        if at_purchase.is_zero() {
            return self.price;
        }
        self.price * residual_ratio(years_between(self.registered(), d)) / at_purchase
    }

    /// Wertverlust vom Kauf bis zum Beginn des Tages `d` im Zeitmodus: die
    /// Restwertkurve, mit Restwert und Halteende so skaliert, dass sie am
    /// Halteende genau den Restwert trifft.
    fn time_loss(&self, d: NaiveDate) -> Decimal {
        let curve_loss = |x: NaiveDate| self.price - self.curve_value(x);
        match self.acquisition() {
            Some(a) => {
                if a.planned_end <= a.date {
                    return Decimal::ZERO;
                }
                let at = d.clamp(a.date, a.planned_end);
                let full = curve_loss(a.planned_end);
                let share = if full > Decimal::ZERO {
                    curve_loss(at) / full
                } else {
                    // ohne Kurvenverlauf linear über die Tage
                    Decimal::from((at - a.date).num_days())
                        / Decimal::from((a.planned_end - a.date).num_days())
                };
                (a.price - a.residual_value) * share
            }
            None => curve_loss(self.sold_on.map_or(d, |s| d.min(s))),
        }
    }

    /// Wertverlust vom Kauf bis zum Beginn des Tages `d` im
    /// Laufleistungsmodus: Restwertkurve plus Zu- bzw. Abschlag von 1,5 %
    /// des Kaufpreises je 10.000 km Abweichung von 13.500 km je Jahr; nach
    /// einem Verkauf `Kaufpreis - Erlös`.
    fn mileage_loss(
        &self,
        d: NaiveDate,
        odometer: &impl Fn(NaiveDate) -> Option<Decimal>,
    ) -> Decimal {
        if let Some(sold) = self.sold_on
            && d > sold
        {
            if let Some(p) = self.sale_price {
                return self.price - p;
            }
            return self.mileage_loss(sold, odometer);
        }
        let d = d.max(self.purchased_on);
        let s0 = self.odometer_at_purchase.unwrap_or(Decimal::ZERO);
        let driven = odometer(d)
            .map_or(Decimal::ZERO, |s| s - s0)
            .max(Decimal::ZERO);
        let expected = AVERAGE_KM_PER_YEAR * years_between(self.purchased_on, d);
        let adjustment =
            self.price * MILEAGE_SHARE_PER_10K * (driven - expected) / Decimal::from(10_000);
        let value = (self.curve_value(d) - adjustment).clamp(Decimal::ZERO, self.price);
        self.price - value
    }

    /// Wertverlust im Zeitraum je nach Modus; `odometer` liefert den
    /// Kilometerstand zu Beginn eines Tages. Ohne Verkauf im Zeitraum nie
    /// negativ (wenig gefahrene Monate im Laufleistungsmodus).
    #[must_use]
    pub fn depreciation_in(
        &self,
        period: Period,
        odometer: impl Fn(NaiveDate) -> Option<Decimal>,
    ) -> Decimal {
        if period.end <= self.purchased_on {
            return Decimal::ZERO;
        }
        let (a, b) = (period.start.max(self.purchased_on), period.end);
        match self.mode {
            DepreciationMode::Time => (self.time_loss(b) - self.time_loss(a)).max(Decimal::ZERO),
            DepreciationMode::Mileage => {
                if self.mileage().is_none() {
                    return Decimal::ZERO;
                }
                let diff = self.mileage_loss(b, &odometer) - self.mileage_loss(a, &odometer);
                let sale_inside = self.sold_on.is_some_and(|s| s >= a && s < b);
                if sale_inside {
                    diff
                } else {
                    diff.max(Decimal::ZERO)
                }
            }
        }
    }

    /// Haltedauer bis `today` (ausschließlich) bzw. bis zum Verkauf.
    #[must_use]
    pub fn held(&self, today: NaiveDate) -> Period {
        let sold_end = self.sold_on.map(|sold| {
            if self.mode == DepreciationMode::Mileage {
                sold.succ_opt().unwrap_or(sold)
            } else {
                sold
            }
        });
        let end = sold_end.unwrap_or(today).max(self.purchased_on);
        Period::new(self.purchased_on, end)
    }
}

/// Grundlage der Restlebensdauer im Laufleistungsmodus
/// (SPEC_CAR_MILEAGE_DEPRECIATION).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mileage {
    /// Kilometerstand bei Kauf *s₀*
    pub odometer_at_purchase: Decimal,
    /// Lebenslaufleistung *S* als absoluter Kilometerstand, optional
    pub lifetime_km: Option<Decimal>,
    pub sold_on: Option<NaiveDate>,
}

impl Mileage {
    /// Restlebensdauer aus dem heutigen Kilometerstand und der
    /// Jahresfahrleistung; ohne Lebenslaufleistung oder Fahrleistung
    /// undefiniert.
    // @impl SPEC_CAR_MILEAGE_DEPRECIATION
    #[must_use]
    pub fn remaining(
        &self,
        today: NaiveDate,
        odometer: Option<Decimal>,
        annual_km: Option<Decimal>,
    ) -> Remaining {
        if self.sold_on.is_some_and(|s| s <= today) {
            return Remaining::Sold;
        }
        let Some(lifetime) = self.lifetime_km else {
            return Remaining::InsufficientData;
        };
        let rest = lifetime - odometer.unwrap_or(self.odometer_at_purchase);
        if rest <= Decimal::ZERO {
            return Remaining::Reached;
        }
        match annual_km {
            Some(a) if a > Decimal::ZERO => {
                let years = rest / a;
                Remaining::Until {
                    years: years.round_dp(1),
                    until: add_years(today, years),
                }
            }
            _ => Remaining::InsufficientData,
        }
    }
}

/// Restlebensdauer bzw. Restlaufzeit der Haltedauer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remaining {
    /// Voraussichtliches Ende in Jahren und als Datum
    Until {
        years: Decimal,
        until: NaiveDate,
    },
    /// Lebenslaufleistung bzw. geplantes Ende erreicht
    Reached,
    Sold,
    /// Keine belastbare Fahrleistungsprognose
    InsufficientData,
}

/// Restlaufzeit bis zum geplanten Ende (Zeitmodus).
// @impl SPEC_CAR_MILEAGE_DEPRECIATION
#[must_use]
pub fn remaining_time(acq: &Acquisition, sold: bool, today: NaiveDate) -> Remaining {
    if sold {
        return Remaining::Sold;
    }
    let days = (acq.planned_end - today).num_days();
    if days <= 0 {
        return Remaining::Reached;
    }
    Remaining::Until {
        years: (Decimal::from(days) / Decimal::new(36525, 2)).round_dp(1),
        until: acq.planned_end,
    }
}

/// `today` plus Jahre als Bruchteil (365,25 Tage je Jahr), auf Tage gerundet.
fn add_years(today: NaiveDate, years: Decimal) -> NaiveDate {
    use rust_decimal::prelude::ToPrimitive;
    let days = (years * Decimal::new(36525, 2))
        .round()
        .to_i64()
        .unwrap_or(i64::MAX);
    today
        .checked_add_signed(chrono::Duration::days(days.min(365 * 200)))
        .unwrap_or(NaiveDate::MAX)
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

/// Energieart eines Verbrauchsprofils: Strom in kWh/100 km oder Kraftstoff
/// in l/100 km (SPEC_CAR_FUEL_PRICE).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileEnergy {
    #[default]
    Electric,
    Fuel,
}

impl ProfileEnergy {
    pub const ALL: [Self; 2] = [Self::Electric, Self::Fuel];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Electric => "electric",
            Self::Fuel => "fuel",
        }
    }
}

impl std::str::FromStr for ProfileEnergy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|x| x.as_str() == s)
            .ok_or_else(|| format!("unbekannte Energieart: {s}"))
    }
}

/// Verbrauchsprofil ab `valid_from` (REQ_CAR_009).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumptionProfile {
    pub valid_from: NaiveDate,
    /// Verbrauch je 100 km: kWh bei `Electric`, l bei `Fuel`
    pub kwh_per_100km: Decimal,
    pub charging_loss_pct: Decimal,
    /// Elektrischer Fahranteil in Prozent (Plug-in-Hybrid)
    pub electric_share_pct: Option<Decimal>,
    #[serde(default)]
    pub energy: ProfileEnergy,
}

/// Tagesanteile der Profile einer Energieart im Zeitraum; Tage vor dem
/// ersten Profil zählen zum ersten.
fn profile_shares(
    profiles: &[ConsumptionProfile],
    energy: ProfileEnergy,
    period: Period,
) -> Vec<(Decimal, &ConsumptionProfile)> {
    let mut sorted: Vec<&ConsumptionProfile> =
        profiles.iter().filter(|p| p.energy == energy).collect();
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
    profile_average(profiles, ProfileEnergy::Electric, period)
}

/// Hinterlegter Kraftstoffverbrauch im Zeitraum (l/100 km, tagesanteilig).
#[must_use]
pub fn fuel_profile_consumption(
    profiles: &[ConsumptionProfile],
    period: Period,
) -> Option<Decimal> {
    profile_average(profiles, ProfileEnergy::Fuel, period)
}

fn profile_average(
    profiles: &[ConsumptionProfile],
    energy: ProfileEnergy,
    period: Period,
) -> Option<Decimal> {
    let shares = profile_shares(profiles, energy, period);
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
    let shares = profile_shares(profiles, ProfileEnergy::Electric, period);
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

/// Betrag, Menge und Preis pro Liter eines Tankvorgangs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuelFields {
    pub amount: Decimal,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
}

/// Fehler der Felder Betrag, Menge und Preis pro Liter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuelFieldsError {
    /// Betrag lässt sich nicht bestimmen
    Missing,
    /// Menge oder Preis nicht positiv
    NotPositive,
    /// `|B - M · p| > 0,01`
    Mismatch,
}

/// Ergänzt die fehlende von drei Angaben: `B = M · p` (2 Stellen),
/// `M = B / p` und `p = B / M` (3 Stellen). Ein Betrag allein bleibt
/// zulässig (Tankvorgang ohne Menge).
// @impl SPEC_CAR_FUEL_PRICE
pub fn complete_fuel_fields(
    amount: Option<Decimal>,
    quantity: Option<Decimal>,
    unit_price: Option<Decimal>,
) -> Result<FuelFields, FuelFieldsError> {
    if quantity.is_some_and(|q| q <= Decimal::ZERO)
        || unit_price.is_some_and(|p| p <= Decimal::ZERO)
    {
        return Err(FuelFieldsError::NotPositive);
    }
    let cent = Decimal::new(1, 2);
    match (amount, quantity, unit_price) {
        (Some(b), Some(m), Some(p)) => {
            if (b - m * p).abs() > cent {
                return Err(FuelFieldsError::Mismatch);
            }
            Ok(FuelFields {
                amount: b,
                quantity: Some(m),
                unit_price: Some(p),
            })
        }
        (Some(b), Some(m), None) => Ok(FuelFields {
            amount: b,
            quantity: Some(m),
            unit_price: Some((b / m).round_dp(3)),
        }),
        (Some(b), None, Some(p)) => Ok(FuelFields {
            amount: b,
            quantity: Some((b / p).round_dp(3)),
            unit_price: Some(p),
        }),
        (None, Some(m), Some(p)) => Ok(FuelFields {
            amount: (m * p).round_dp(2),
            quantity: Some(m),
            unit_price: Some(p),
        }),
        (Some(b), None, None) => Ok(FuelFields {
            amount: b,
            quantity: None,
            unit_price: None,
        }),
        _ => Err(FuelFieldsError::Missing),
    }
}

/// Letzter bekannter Preis pro Liter: der jüngste Tankvorgang mit Preis.
// @impl SPEC_CAR_FUEL_PRICE
#[must_use]
pub fn last_fuel_price(
    fills: impl IntoIterator<Item = (NaiveDate, Option<Decimal>)>,
) -> Option<(NaiveDate, Decimal)> {
    fills
        .into_iter()
        .filter_map(|(d, p)| Some((d, p?)))
        .max_by_key(|(d, _)| *d)
}

/// Geschätzter Durchschnittspreis `Tankkosten / (km · v / 100)`; ohne
/// Kosten, km oder Verbrauch undefiniert.
// @impl SPEC_CAR_FUEL_PRICE
#[must_use]
pub fn estimated_fuel_price(cost: Decimal, km: Decimal, per_100km: Decimal) -> Option<Decimal> {
    let litres = km * per_100km / Decimal::ONE_HUNDRED;
    (cost > Decimal::ZERO && litres > Decimal::ZERO).then(|| (cost / litres).round_dp(3))
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

    // @test SPEC_CAR_COST_PER_KM
    #[test]
    fn full_cost_per_km() {
        let r = cost_per_km(dec!(1000), dec!(500), dec!(3997.81), dec!(10000));
        assert_eq!(r.variable_per_km, Some(dec!(0.1)));
        assert_eq!(r.total_per_km, Some(dec!(0.5498)));
        assert_eq!(
            cost_per_km(dec!(1), dec!(1), dec!(0), dec!(0)).total_per_km,
            None
        );
    }

    fn time_holding(registered: Option<NaiveDate>) -> Holding {
        Holding {
            purchased_on: d(2026, 1, 1),
            price: dec!(30000),
            mode: DepreciationMode::Time,
            first_registration: registered,
            residual_value: Some(dec!(10000)),
            planned_end: Some(d(2031, 1, 1)),
            odometer_at_purchase: None,
            lifetime_km: None,
            end_value: None,
            sold_on: None,
            sale_price: None,
        }
    }

    fn none(_: NaiveDate) -> Option<Decimal> {
        None
    }

    // @test SPEC_CAR_DEPRECIATION
    #[test]
    fn residual_curve_follows_the_guide_values() {
        assert_eq!(residual_ratio(dec!(0)), dec!(1));
        assert_eq!(residual_ratio(dec!(1)), dec!(0.775));
        assert_eq!(residual_ratio(dec!(2)), dec!(0.6375));
        assert_eq!(residual_ratio(dec!(3)), dec!(0.5));
        assert_eq!(residual_ratio(dec!(5)), dec!(0.425));
        assert_eq!(residual_ratio(dec!(10)), dec!(0.25));
        assert_eq!(residual_ratio(dec!(12)), dec!(0.21));
        assert_eq!(residual_ratio(dec!(40)), dec!(0.10), "Untergrenze");
        assert_eq!(residual_ratio(dec!(-1)), dec!(1));
    }

    // @test SPEC_CAR_DEPRECIATION
    #[test]
    fn time_mode_follows_the_curve_and_meets_the_residual_value() {
        let h = time_holding(None);
        let years: Vec<Decimal> = (2025..=2032)
            .map(|y| h.depreciation_in(Period::year(y), none))
            .collect();
        assert_eq!(years[0], Decimal::ZERO, "vor dem Kauf");
        assert_eq!(years[7], Decimal::ZERO, "nach dem geplanten Ende");
        assert_eq!(years.iter().sum::<Decimal>().round_dp(6), dec!(20000));
        // im ersten Jahr der größte Verlust, ab dem vierten Jahr deutlich
        // weniger (2028 hat einen Tag mehr)
        assert!(years[1] > years[2] && years[3] > years[4], "{years:?}");
        // 22,5 % von 57,5 % Kurvenverlust bis zum Halteende ≈ 39 % von 20.000 €
        assert_eq!(years[1].round_dp(0), dec!(7821));
    }

    // @test SPEC_CAR_DEPRECIATION
    #[test]
    fn time_mode_without_residual_uses_the_curve_alone() {
        let h = Holding {
            residual_value: None,
            planned_end: None,
            ..time_holding(None)
        };
        // nach drei Jahren rund 50 % des Neupreises
        let three = h.depreciation_in(Period::new(d(2026, 1, 1), d(2029, 1, 1)), none);
        assert_eq!(three.round_dp(0), dec!(15001));
        // Gebrauchtwagen, Erstzulassung 2016: Kurve ab Alter 10 relativ zum
        // Kaufpreis (25 % → 23 % des Neupreises im ersten Jahr)
        let used = Holding {
            first_registration: Some(d(2016, 1, 1)),
            ..h
        };
        let first = used.depreciation_in(Period::year(2026), none);
        assert_eq!(first.round_dp(0), dec!(2399));
    }

    // @test SPEC_CAR_DEPRECIATION
    #[test]
    fn sale_replaces_the_residual_value() {
        let h = Holding {
            sold_on: Some(d(2027, 7, 1)),
            sale_price: Some(dec!(19000)),
            ..time_holding(None)
        };
        let total = h.depreciation_in(Period::new(d(2026, 1, 1), d(2030, 1, 1)), none);
        assert_eq!(total.round_dp(6), dec!(11000));
        assert_eq!(h.held(d(2030, 1, 1)).end, d(2027, 7, 1));
        // Haltedauer ohne Tage
        let empty = Holding {
            planned_end: Some(d(2026, 1, 1)),
            ..time_holding(None)
        };
        assert_eq!(
            empty.depreciation_in(Period::year(2026), none),
            Decimal::ZERO
        );
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
            energy: ProfileEnergy::Electric,
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

#[cfg(test)]
mod mileage_tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn holding(sold: Option<(NaiveDate, Decimal)>) -> Holding {
        Holding {
            purchased_on: d(2026, 1, 1),
            price: dec!(30000),
            mode: DepreciationMode::Mileage,
            first_registration: None,
            residual_value: Some(dec!(10000)),
            planned_end: Some(d(2031, 1, 1)),
            odometer_at_purchase: Some(dec!(20000)),
            lifetime_km: Some(dec!(220000)),
            end_value: None,
            sold_on: sold.map(|s| s.0),
            sale_price: sold.map(|s| s.1),
        }
    }

    /// Kilometerstand bei `per_year` km je Jahr ab Kauf, linear über die Tage.
    fn driving(per_year: Decimal) -> impl Fn(NaiveDate) -> Option<Decimal> {
        move |day| {
            let years = Decimal::from((day - d(2026, 1, 1)).num_days().max(0)) / dec!(365.25);
            Some(dec!(20000) + years * per_year)
        }
    }

    // @test SPEC_CAR_MILEAGE_DEPRECIATION
    #[test]
    fn average_driving_follows_the_age_curve() {
        let h = holding(None);
        let curve_only = Holding {
            mode: DepreciationMode::Time,
            residual_value: None,
            planned_end: None,
            ..h.clone()
        };
        for y in 2026..2031 {
            assert_eq!(
                h.depreciation_in(Period::year(y), driving(AVERAGE_KM_PER_YEAR))
                    .round_dp(6),
                curve_only
                    .depreciation_in(Period::year(y), |_| None)
                    .round_dp(6),
                "{y}"
            );
        }
        assert_eq!(
            h.depreciation_in(Period::year(2025), driving(AVERAGE_KM_PER_YEAR)),
            Decimal::ZERO
        );
        assert!(curve_only.mileage().is_none() && h.acquisition().is_none());
    }

    // @test SPEC_CAR_MILEAGE_DEPRECIATION
    #[test]
    fn more_km_cost_value_less_km_save_value() {
        let h = holding(None);
        let year = Period::year(2026);
        let average = h.depreciation_in(year, driving(AVERAGE_KM_PER_YEAR));
        let many = h.depreciation_in(year, driving(dec!(33500)));
        let few = h.depreciation_in(year, driving(dec!(3500)));
        // ±20.000 km je Jahr: 2 · 1,5 % von 30.000 €, anteilig für 365 von
        // 365,25 Tagen
        assert_eq!((many - average).round_dp(2), dec!(899.38));
        // 3.500 km je Jahr: 10.000 km unter dem Schnitt
        assert_eq!((average - few).round_dp(2), dec!(449.69));
        // kein Wertzuwachs in Zeiträumen ohne Fahrt
        let parked = |_: NaiveDate| Some(dec!(20000));
        assert!(h.depreciation_in(Period::year(2040), parked) >= Decimal::ZERO);
    }

    // @test SPEC_CAR_MILEAGE_DEPRECIATION
    #[test]
    fn sale_distributes_the_full_loss() {
        let h = holding(Some((d(2027, 7, 1), dec!(21000))));
        let km = driving(dec!(15000));
        let parts: Vec<Decimal> = [2026, 2027, 2028]
            .map(|y| h.depreciation_in(Period::year(y), &km))
            .to_vec();
        assert_eq!(parts[2], Decimal::ZERO, "nach dem Verkauf");
        assert_eq!(parts.iter().sum::<Decimal>().round_dp(6), dec!(9000));
        assert_eq!(
            h.depreciation_in(h.held(d(2030, 1, 1)), &km).round_dp(6),
            dec!(9000)
        );
        // Verkauf am Neujahrstag gehört zum Verkaufsjahr
        let h = holding(Some((d(2027, 1, 1), dec!(21000))));
        let before = h.depreciation_in(Period::year(2026), &km);
        let sale_year = h.depreciation_in(Period::year(2027), &km);
        assert_eq!((before + sale_year).round_dp(6), dec!(9000));
        assert_eq!(h.depreciation_in(Period::year(2028), &km), Decimal::ZERO);
    }

    // @test SPEC_CAR_MILEAGE_DEPRECIATION
    #[test]
    fn remaining_life_uses_the_annual_mileage() {
        let m = holding(None).mileage().unwrap();
        let r = m.remaining(d(2027, 1, 1), Some(dec!(35000)), Some(dec!(15000)));
        // 185.000 km bei 15.000 km je Jahr
        assert_eq!(
            r,
            Remaining::Until {
                years: dec!(12.3),
                until: d(2039, 5, 3)
            }
        );
        assert_eq!(
            m.remaining(d(2027, 1, 1), Some(dec!(35000)), None),
            Remaining::InsufficientData
        );
        assert_eq!(
            m.remaining(d(2027, 1, 1), Some(dec!(230000)), Some(dec!(1))),
            Remaining::Reached
        );
        // ohne Lebenslaufleistung keine Restlebensdauer
        let no_lifetime = Holding {
            lifetime_km: None,
            ..holding(None)
        };
        assert_eq!(
            no_lifetime.mileage().unwrap().remaining(
                d(2027, 1, 1),
                Some(dec!(35000)),
                Some(dec!(15000))
            ),
            Remaining::InsufficientData
        );
        let acq = Acquisition {
            date: d(2026, 1, 1),
            price: dec!(30000),
            residual_value: dec!(10000),
            planned_end: d(2031, 1, 1),
        };
        assert_eq!(
            remaining_time(&acq, false, d(2026, 1, 1)),
            Remaining::Until {
                years: dec!(5.0),
                until: d(2031, 1, 1)
            }
        );
        assert_eq!(remaining_time(&acq, true, d(2026, 1, 1)), Remaining::Sold);
    }

    #[test]
    fn mode_round_trips_as_text() {
        for m in DepreciationMode::ALL {
            assert_eq!(m.as_str().parse::<DepreciationMode>(), Ok(m));
        }
        assert!("x".parse::<DepreciationMode>().is_err());
    }
}

#[cfg(test)]
mod fuel_price_tests {
    use rust_decimal_macros::dec;

    use super::*;

    // @test SPEC_CAR_FUEL_PRICE
    #[test]
    fn two_of_three_fields_complete_the_third() {
        let f = complete_fuel_fields(None, Some(dec!(45.32)), Some(dec!(1.789))).unwrap();
        assert_eq!(f.amount, dec!(81.08));
        assert_eq!(f.unit_price, Some(dec!(1.789)), "eingegebener Preis bleibt");
        let f = complete_fuel_fields(Some(dec!(81.08)), Some(dec!(45.32)), None).unwrap();
        assert_eq!(f.unit_price, Some(dec!(1.789)));
        let f = complete_fuel_fields(Some(dec!(81.08)), None, Some(dec!(1.789))).unwrap();
        assert_eq!(f.quantity, Some(dec!(45.321)));
        let f = complete_fuel_fields(Some(dec!(81.08)), None, None).unwrap();
        assert_eq!((f.quantity, f.unit_price), (None, None));
        assert!(
            complete_fuel_fields(Some(dec!(81.08)), Some(dec!(45.32)), Some(dec!(1.789))).is_ok()
        );
        assert_eq!(
            complete_fuel_fields(Some(dec!(81.10)), Some(dec!(45.32)), Some(dec!(1.789))),
            Err(FuelFieldsError::Mismatch)
        );
        assert_eq!(
            complete_fuel_fields(None, Some(dec!(45.32)), None),
            Err(FuelFieldsError::Missing)
        );
        assert_eq!(
            complete_fuel_fields(Some(dec!(10)), Some(dec!(0)), None),
            Err(FuelFieldsError::NotPositive)
        );
    }

    // @test SPEC_CAR_FUEL_PRICE
    #[test]
    fn last_price_and_estimate() {
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        assert_eq!(
            last_fuel_price([
                (d(1, 5), Some(dec!(1.7))),
                (d(3, 1), None),
                (d(2, 1), Some(dec!(1.8)))
            ]),
            Some((d(2, 1), dec!(1.8)))
        );
        assert_eq!(last_fuel_price([(d(1, 5), None)]), None);
        // 240 € für 2.000 km bei 6 l/100 km: 120 l
        assert_eq!(
            estimated_fuel_price(dec!(240), dec!(2000), dec!(6)),
            Some(dec!(2))
        );
        assert_eq!(estimated_fuel_price(dec!(240), dec!(0), dec!(6)), None);
        assert_eq!(estimated_fuel_price(dec!(0), dec!(2000), dec!(6)), None);
    }

    // @test SPEC_CAR_FUEL_PRICE
    #[test]
    fn fuel_profiles_do_not_affect_charging() {
        let p = |energy, v| ConsumptionProfile {
            valid_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            kwh_per_100km: v,
            charging_loss_pct: Decimal::ZERO,
            electric_share_pct: None,
            energy,
        };
        let profiles = [
            p(ProfileEnergy::Electric, dec!(18)),
            p(ProfileEnergy::Fuel, dec!(6.5)),
        ];
        let y = Period::year(2026);
        assert_eq!(profile_consumption(&profiles, y), Some(dec!(18)));
        assert_eq!(fuel_profile_consumption(&profiles, y), Some(dec!(6.5)));
        assert_eq!(fuel_profile_consumption(&profiles[..1], y), None);
    }
}
