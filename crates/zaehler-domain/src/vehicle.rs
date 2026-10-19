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
    /// Zeitmodus: geschätzter Restwert am geplanten Ende
    pub residual_value: Option<Decimal>,
    /// Zeitmodus: Ende der Haltedauer (ausschließlich)
    pub planned_end: Option<NaiveDate>,
    /// Laufleistungsmodus: Kilometerstand bei Kauf
    pub odometer_at_purchase: Option<Decimal>,
    /// Laufleistungsmodus: Lebenslaufleistung als absoluter Kilometerstand
    pub lifetime_km: Option<Decimal>,
    /// Laufleistungsmodus: Wert bei Erreichen der Lebenslaufleistung
    pub end_value: Option<Decimal>,
    pub sold_on: Option<NaiveDate>,
    pub sale_price: Option<Decimal>,
}

impl Holding {
    /// Anschaffung des Zeitmodus. Bei Verkauf ersetzt der Erlös den
    /// Restwert, die Haltedauer endet mit dem Verkauf
    /// (SPEC_CAR_DEPRECIATION); ohne Werte des Zeitmodus `None`.
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

    /// Grundlage des Laufleistungsmodus; bei Verkauf ersetzt der Erlös den
    /// Endwert (SPEC_CAR_MILEAGE_DEPRECIATION).
    #[must_use]
    pub fn mileage(&self) -> Option<Mileage> {
        if self.mode != DepreciationMode::Mileage {
            return None;
        }
        let end_value = match (self.sold_on, self.sale_price) {
            (Some(_), Some(p)) => p,
            _ => self.end_value.unwrap_or(Decimal::ZERO),
        };
        Some(Mileage {
            purchased_on: self.purchased_on,
            price: self.price,
            end_value,
            odometer_at_purchase: self.odometer_at_purchase?,
            lifetime_km: self.lifetime_km?,
            sold_on: self.sold_on,
        })
    }

    /// Wertverlust im Zeitraum je nach Modus; `odometer` liefert den
    /// Kilometerstand zu Beginn eines Tages.
    #[must_use]
    pub fn depreciation_in(
        &self,
        period: Period,
        odometer: impl Fn(NaiveDate) -> Option<Decimal>,
    ) -> Decimal {
        match self.mode {
            DepreciationMode::Time => self
                .acquisition()
                .map_or(Decimal::ZERO, |a| depreciation_in(&a, period)),
            DepreciationMode::Mileage => self
                .mileage()
                .map_or(Decimal::ZERO, |m| m.depreciation_in(period, odometer)),
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

/// Grundlage des Wertverlusts nach Lebenslaufleistung
/// (SPEC_CAR_MILEAGE_DEPRECIATION).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mileage {
    pub purchased_on: NaiveDate,
    pub price: Decimal,
    /// Endwert, bei Verkauf der Erlös
    pub end_value: Decimal,
    /// Kilometerstand bei Kauf *s₀*
    pub odometer_at_purchase: Decimal,
    /// Lebenslaufleistung *S* als absoluter Kilometerstand
    pub lifetime_km: Decimal,
    pub sold_on: Option<NaiveDate>,
}

impl Mileage {
    /// Nutzbare Strecke `S - s₀`.
    fn usable(&self) -> Decimal {
        self.lifetime_km - self.odometer_at_purchase
    }

    /// `s*(t) = min(max(s(t), s₀), S)`.
    fn clamp(&self, odometer: Decimal) -> Decimal {
        odometer
            .max(self.odometer_at_purchase)
            .min(self.lifetime_km)
    }

    /// Wertverlust je km bis zur Lebenslaufleistung.
    fn per_km(&self) -> Decimal {
        (self.price - self.end_value) / self.usable()
    }

    /// `(K - W) · (s*(b) - s*(a)) / (S - s₀)` innerhalb der Haltedauer;
    /// liegt der Verkauf im Zeitraum, zusätzlich der Rest bis zur
    /// Lebenslaufleistung, damit die Summe `K - Erlös` ergibt. Ohne
    /// Kilometerstand gilt der Stand bei Kauf.
    // @impl SPEC_CAR_MILEAGE_DEPRECIATION
    #[must_use]
    pub fn depreciation_in(
        &self,
        period: Period,
        odometer: impl Fn(NaiveDate) -> Option<Decimal>,
    ) -> Decimal {
        if self.usable() <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        let at = |d: NaiveDate| self.clamp(odometer(d).unwrap_or(self.odometer_at_purchase));
        let start = period.start.max(self.purchased_on);
        let end = self.sold_on.map_or(period.end, |s| period.end.min(s));
        let mut out = Decimal::ZERO;
        if start < end {
            out += self.per_km() * (at(end) - at(start)).max(Decimal::ZERO);
        }
        if let Some(sold) = self.sold_on
            && sold >= period.start
            && sold < period.end
            && sold > self.purchased_on
        {
            out += self.per_km() * (self.lifetime_km - at(sold));
        }
        out
    }

    /// Restlebensdauer aus dem heutigen Kilometerstand und der
    /// Jahresfahrleistung; ohne Fahrleistung undefiniert.
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
        let rest = self.lifetime_km - odometer.unwrap_or(self.odometer_at_purchase);
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
            mode: DepreciationMode::Time,
            residual_value: Some(dec!(2000)),
            planned_end: Some(d(2028, 7, 1)),
            odometer_at_purchase: None,
            lifetime_km: None,
            end_value: None,
            sold_on: Some(d(2027, 7, 1)),
            sale_price: Some(dec!(9000)),
        };
        assert_eq!(
            depreciation_in(
                &holding.acquisition().unwrap(),
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
            residual_value: Some(dec!(10000)),
            planned_end: Some(d(2031, 1, 1)),
            odometer_at_purchase: Some(dec!(20000)),
            lifetime_km: Some(dec!(220000)),
            end_value: None,
            sold_on: sold.map(|s| s.0),
            sale_price: sold.map(|s| s.1),
        }
    }

    /// 15.000 km je Jahr ab Kauf, linear über die Tage.
    #[allow(clippy::unnecessary_wraps)] // dieselbe Signatur wie die Kilometerstand-Abfrage
    fn odometer(day: NaiveDate) -> Option<Decimal> {
        let days = Decimal::from((day - d(2026, 1, 1)).num_days().max(0));
        Some(dec!(20000) + days * dec!(15000) / dec!(365))
    }

    // @test SPEC_CAR_MILEAGE_DEPRECIATION
    #[test]
    fn depreciation_follows_the_driven_km() {
        let h = holding(None);
        // 15.000 km von 200.000 km nutzbarer Laufleistung: 30.000 € · 0,075
        assert_eq!(
            h.depreciation_in(Period::year(2026), odometer).round_dp(2),
            dec!(2250.00)
        );
        assert_eq!(
            h.depreciation_in(Period::year(2025), odometer),
            Decimal::ZERO
        );
        // Zeitmodus mit denselben Daten bleibt unverändert verfügbar
        let time = Holding {
            mode: DepreciationMode::Time,
            ..h.clone()
        };
        assert_eq!(
            time.depreciation_in(Period::year(2026), odometer)
                .round_dp(2),
            dec!(3997.81)
        );
        assert!(time.mileage().is_none() && h.acquisition().is_none());
    }

    // @test SPEC_CAR_MILEAGE_DEPRECIATION
    #[test]
    fn depreciation_stops_at_the_lifetime_mileage() {
        let h = holding(None);
        let far = |day: NaiveDate| {
            Some(if day.year() < 2030 {
                dec!(20000)
            } else {
                dec!(500000)
            })
        };
        let total = h.depreciation_in(Period::new(d(2026, 1, 1), d(2050, 1, 1)), far);
        assert_eq!(total, dec!(30000));
        // Endwert wird nicht unterschritten
        let h = Holding {
            end_value: Some(dec!(2000)),
            ..holding(None)
        };
        assert_eq!(
            h.depreciation_in(Period::new(d(2026, 1, 1), d(2050, 1, 1)), far),
            dec!(28000)
        );
    }

    // @test SPEC_CAR_MILEAGE_DEPRECIATION
    #[test]
    fn sale_distributes_the_full_loss() {
        let h = holding(Some((d(2027, 7, 1), dec!(21000))));
        let parts: Vec<Decimal> = [2026, 2027, 2028]
            .map(|y| h.depreciation_in(Period::year(y), odometer))
            .to_vec();
        assert_eq!(parts[2], Decimal::ZERO, "nach dem Verkauf");
        assert_eq!(parts.iter().sum::<Decimal>().round_dp(6), dec!(9000));
        // Haltedauer bis zum Verkauf
        assert_eq!(
            h.depreciation_in(h.held(d(2030, 1, 1)), odometer)
                .round_dp(6),
            dec!(9000)
        );
    }

    // @test SPEC_CAR_MILEAGE_DEPRECIATION
    #[test]
    fn sale_on_new_year_belongs_to_the_sale_year() {
        let h = holding(Some((d(2027, 1, 1), dec!(21000))));
        let before = h.depreciation_in(Period::year(2026), odometer);
        let sale_year = h.depreciation_in(Period::year(2027), odometer);
        assert!(sale_year > before);
        assert_eq!((before + sale_year).round_dp(6), dec!(9000));
        assert_eq!(
            h.depreciation_in(Period::year(2028), odometer),
            Decimal::ZERO
        );
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
