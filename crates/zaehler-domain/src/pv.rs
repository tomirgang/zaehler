//! Photovoltaik: Anlagendaten, Energiekennzahlen je Hausanschluss,
//! Speicher, Ertragserwartung, Ertragsampel und Wirtschaftlichkeit
//! (SPEC_PV_KPI, SPEC_PV_BATTERY, SPEC_PV_YIELD_EXPECTATION,
//! SPEC_PV_MONITORING, SPEC_PV_ECONOMICS).

use chrono::{Datelike, Days, NaiveDate};
use rust_decimal::{Decimal, MathematicalOps, prelude::ToPrimitive};
use serde::{Deserialize, Serialize};

use crate::{
    expectation::{Band, HISTORY_K, Status},
    forecast::SeasonProfile,
    period::Period,
    series::MonthValue,
};

/// Anlagentyp (REQ_PV_001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PvKind {
    /// Überschusseinspeisung
    Surplus,
    /// Volleinspeisung
    FullFeedIn,
    /// Steckersolargerät
    PlugIn,
}

impl PvKind {
    pub const ALL: [Self; 3] = [Self::Surplus, Self::FullFeedIn, Self::PlugIn];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Surplus => "surplus",
            Self::FullFeedIn => "full_feed_in",
            Self::PlugIn => "plug_in",
        }
    }

    /// Setzt ein Einspeiseregister am Hausanschluss voraus (SPEC_PV_ASSIGNMENT).
    #[must_use]
    pub fn needs_export(self) -> bool {
        self != Self::PlugIn
    }
}

impl std::str::FromStr for PvKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|x| x.as_str() == s)
            .ok_or_else(|| format!("unbekannter Anlagentyp: {s}"))
    }
}

/// Rolle einer Messstelle an einer Anlage (ADR_021).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PvRole {
    Grid,
    Generation,
    Battery,
}

impl PvRole {
    pub const ALL: [Self; 3] = [Self::Grid, Self::Generation, Self::Battery];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::Generation => "generation",
            Self::Battery => "battery",
        }
    }
}

impl std::str::FromStr for PvRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|x| x.as_str() == s)
            .ok_or_else(|| format!("unbekannte Rolle: {s}"))
    }
}

/// Register eines Speichers.
pub const BATTERY_CHARGE: &str = "charge";
pub const BATTERY_DISCHARGE: &str = "discharge";

/// Standard-Monatsanteile in Prozent (Richtwerte für Deutschland,
/// SPEC_PV_YIELD_EXPECTATION).
pub const DEFAULT_MONTH_SHARES: [Decimal; 12] = [
    Decimal::from_parts(30, 0, 0, false, 1),
    Decimal::from_parts(45, 0, 0, false, 1),
    Decimal::from_parts(80, 0, 0, false, 1),
    Decimal::from_parts(115, 0, 0, false, 1),
    Decimal::from_parts(130, 0, 0, false, 1),
    Decimal::from_parts(135, 0, 0, false, 1),
    Decimal::from_parts(135, 0, 0, false, 1),
    Decimal::from_parts(120, 0, 0, false, 1),
    Decimal::from_parts(90, 0, 0, false, 1),
    Decimal::from_parts(60, 0, 0, false, 1),
    Decimal::from_parts(35, 0, 0, false, 1),
    Decimal::from_parts(25, 0, 0, false, 1),
];

/// Standard für den spezifischen Jahresertrag (kWh/kWp).
pub const DEFAULT_SPECIFIC_YIELD: Decimal = Decimal::from_parts(950, 0, 0, false, 0);
/// Standard für die Degradation (% je Jahr).
pub const DEFAULT_DEGRADATION_PCT: Decimal = Decimal::from_parts(5, 0, 0, false, 1);
/// Standard-Schwellwert des Ertragsalarms (% der Erwartung).
pub const DEFAULT_ALARM_THRESHOLD_PCT: u8 = 50;
/// Toleranz der Ampel mit Startwert in Prozent: grün bis 30 %, gelb bis 45 %.
pub const START_TOLERANCE_PCT: Decimal = Decimal::from_parts(30, 0, 0, false, 0);

/// Anlagendaten (REQ_PV_001).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PvSystemData {
    pub name: String,
    pub kind: PvKind,
    pub kwp: Decimal,
    pub commissioned_on: NaiveDate,
    pub azimuth: Option<i16>,
    pub tilt: Option<i16>,
    pub inverter: Option<String>,
    /// Nutzbare Speicherkapazität in kWh
    pub battery_kwh: Option<Decimal>,
    /// Spezifischer Jahresertrag in kWh/kWp
    pub specific_yield: Decimal,
    pub degradation_pct: Decimal,
    /// Monatsanteile Januar bis Dezember in Prozent; `None`: Standard
    pub month_shares: Option<[Decimal; 12]>,
    pub alarm_threshold_pct: u8,
}

impl PvSystemData {
    #[must_use]
    pub fn shares(&self) -> [Decimal; 12] {
        self.month_shares.unwrap_or(DEFAULT_MONTH_SHARES)
    }
}

/// Größen eines Zeitraums am Hausanschluss; `None`: nicht gemessen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flows {
    /// Erzeugung aller Anlagen; `None` ohne Erzeugungszähler oder Messung
    pub generation: Option<Decimal>,
    pub import: Option<Decimal>,
    pub export: Option<Decimal>,
    pub charge: Option<Decimal>,
    pub discharge: Option<Decimal>,
}

/// Aufbau des Hausanschlusses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)] // unabhängige Merkmale des Aufbaus
pub struct GridSetup {
    pub has_generation: bool,
    pub has_export_register: bool,
    pub has_battery: bool,
    /// Alle Anlagen speisen voll ein
    pub full_feed_in: bool,
}

/// Energiekennzahlen eines Zeitraums (SPEC_PV_KPI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Kpi {
    pub generation: Option<Decimal>,
    pub import: Option<Decimal>,
    pub export: Option<Decimal>,
    pub charge: Option<Decimal>,
    pub discharge: Option<Decimal>,
    /// Eigenverbrauch `EV`
    pub self_consumption: Option<Decimal>,
    /// Gesamtverbrauch `V`
    pub consumption: Option<Decimal>,
    /// Eigenverbrauchsquote `EV / G` (0..1)
    pub self_consumption_ratio: Option<Decimal>,
    /// Autarkiegrad `EV / V` (0..1)
    pub autarky: Option<Decimal>,
    /// Ohne Einspeiseregister: `E = 0` angenommen
    pub export_assumed: bool,
    /// `EV < 0`: Ablese- oder Zuordnungsfehler
    pub negative_self_consumption: bool,
}

fn ratio(a: Option<Decimal>, b: Option<Decimal>) -> Option<Decimal> {
    let (a, b) = (a?, b?);
    (!b.is_zero()).then(|| (a / b).round_dp(4))
}

/// Kennzahlen aus den Größen eines Zeitraums: `EV = G - E - (L - U)`, bei
/// Volleinspeisung 0; `V = B + EV`; ohne Einspeiseregister `E = 0`
/// (gekennzeichnet); ohne Erzeugungszähler nur Bezug und Einspeisung. Eine
/// Kennzahl ist undefiniert, sobald eine beteiligte Größe fehlt.
// @impl SPEC_PV_KPI
#[must_use]
pub fn kpis(setup: GridSetup, f: Flows) -> Kpi {
    let export = if setup.has_export_register {
        f.export
    } else {
        Some(Decimal::ZERO)
    };
    let battery = if setup.has_battery {
        f.charge.zip(f.discharge).map(|(l, u)| l - u)
    } else {
        Some(Decimal::ZERO)
    };
    let self_consumption = if !setup.has_generation {
        None
    } else if setup.full_feed_in {
        Some(Decimal::ZERO)
    } else {
        f.generation
            .zip(export)
            .zip(battery)
            .map(|((g, e), b)| g - e - b)
    };
    let consumption = f.import.zip(self_consumption).map(|(b, ev)| b + ev);
    Kpi {
        generation: if setup.has_generation {
            f.generation
        } else {
            None
        },
        import: f.import,
        export: if setup.has_export_register {
            f.export
        } else {
            None
        },
        charge: if setup.has_battery { f.charge } else { None },
        discharge: if setup.has_battery { f.discharge } else { None },
        self_consumption_ratio: ratio(
            self_consumption,
            f.generation.filter(|_| setup.has_generation),
        ),
        autarky: ratio(self_consumption, consumption),
        self_consumption,
        consumption,
        export_assumed: setup.has_generation && !setup.has_export_register,
        negative_self_consumption: self_consumption.is_some_and(|v| v < Decimal::ZERO),
    }
}

/// Speicherkennzahlen (SPEC_PV_BATTERY).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryKpi {
    pub charge: Decimal,
    pub discharge: Decimal,
    /// `L - U`
    pub losses: Decimal,
    /// `U / L` (0..1)
    pub efficiency: Option<Decimal>,
    /// `U / nutzbare Kapazität`
    pub full_cycles: Option<Decimal>,
}

// @impl SPEC_PV_BATTERY
#[must_use]
pub fn battery_kpis(charge: Decimal, discharge: Decimal, capacity: Option<Decimal>) -> BatteryKpi {
    BatteryKpi {
        charge,
        discharge,
        losses: charge - discharge,
        efficiency: (!charge.is_zero()).then(|| (discharge / charge).round_dp(4)),
        full_cycles: capacity
            .filter(|c| *c > Decimal::ZERO)
            .map(|c| (discharge / c).round_dp(1)),
    }
}

/// Herkunft einer Ertragserwartung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YieldSource {
    /// Leistung × spezifischer Ertrag × Monatsanteil × Degradation
    Start,
    /// Saisonprofil und Jahresniveau aus eigenen Monatswerten
    History,
}

/// Erwartung im Startwert-Modus: `kWp · Ertrag · w_m · (1 - d/100)^(j - j0)`,
/// tagesanteilig im Monat der Inbetriebnahme, 0 davor.
// @impl SPEC_PV_YIELD_EXPECTATION
#[must_use]
pub fn start_expectation(d: &PvSystemData, month: Period) -> Decimal {
    if month.end <= d.commissioned_on {
        return Decimal::ZERO;
    }
    let shares = d.shares();
    let idx = month.start.month0() as usize;
    let years = (month.start.year() - d.commissioned_on.year()).max(0);
    let factor = (Decimal::ONE - d.degradation_pct / Decimal::ONE_HUNDRED).powi(i64::from(years));
    let mut value = d.kwp * d.specific_yield * shares[idx] / Decimal::ONE_HUNDRED * factor;
    if d.commissioned_on > month.start {
        let active = (month.end - d.commissioned_on).num_days();
        value = value * Decimal::from(active) / Decimal::from(month.days());
    }
    value.round_dp(3)
}

/// Letztes vollständig gemessenes Kalenderjahr vor `year` mit seiner Summe.
fn complete_year_before(series: &[MonthValue], year: i32) -> Option<(i32, Decimal)> {
    (1900..year).rev().find_map(|y| {
        let values: Vec<Decimal> = series
            .iter()
            .filter(|m| m.month.start.year() == y)
            .filter_map(|m| m.value)
            .collect();
        (values.len() == 12).then(|| (y, values.iter().sum()))
    })
}

/// Erwartung eines Monats: mit einem vollständigen Kalenderjahr vor dem
/// Monat aus der Historie (Jahresniveau des letzten vollständigen Jahres,
/// Saisonprofil aus den eigenen Monatswerten bzw. den Monatsanteilen),
/// sonst Startwert.
// @impl SPEC_PV_YIELD_EXPECTATION
#[must_use]
pub fn expectation(
    d: &PvSystemData,
    series: &[MonthValue],
    month: Period,
) -> (Decimal, YieldSource) {
    let history: Vec<MonthValue> = series
        .iter()
        .filter(|m| m.month.start < month.start)
        .copied()
        .collect();
    match complete_year_before(&history, month.start.year()) {
        Some((_, level)) => {
            let profile = SeasonProfile::learn_series(&history).unwrap_or(SeasonProfile {
                monthly_weights: d.shares(),
            });
            (
                (level * profile.share_of_year(month)).round_dp(3),
                YieldSource::History,
            )
        }
        None => (start_expectation(d, month), YieldSource::Start),
    }
}

/// Erzeugung eines Monats gegen die Erwartung (SPEC_PV_MONITORING).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YieldMonth {
    pub month: Period,
    pub actual: Option<Decimal>,
    pub expected: Decimal,
    pub source: YieldSource,
    pub band: Band,
    pub status: Option<Status>,
}

/// Ampel je Monat: mit mindestens zwei Werten desselben Monats aus
/// Vorjahren Band aus der Historie, sonst grün bis ±30 %, gelb bis ±45 % um
/// die Erwartung.
// @impl SPEC_PV_MONITORING, SPEC_PV_YIELD_EXPECTATION
#[must_use]
pub fn yield_months(d: &PvSystemData, series: &[MonthValue]) -> Vec<YieldMonth> {
    series
        .iter()
        .map(|m| {
            let (expected, source) = expectation(d, series, m.month);
            let earlier: Vec<Decimal> = series
                .iter()
                .filter(|x| {
                    x.month.start.month() == m.month.start.month()
                        && x.month.start.year() < m.month.start.year()
                })
                .filter_map(|x| x.value)
                .collect();
            let band = Band::from_history(&earlier, HISTORY_K)
                .unwrap_or_else(|| Band::from_target(expected, START_TOLERANCE_PCT));
            YieldMonth {
                month: m.month,
                actual: m.value,
                expected,
                source,
                band,
                status: m
                    .value
                    .filter(|_| band.expected > Decimal::ZERO)
                    .map(|v| band.classify(v)),
            }
        })
        .collect()
}

/// Erwartung eines beliebigen Zeitraums aus den Monatserwartungen,
/// tagesanteilig je Monat.
#[must_use]
pub fn expectation_in(d: &PvSystemData, series: &[MonthValue], period: Period) -> Decimal {
    let mut total = Decimal::ZERO;
    let mut month = Period::month_of(period.start);
    while month.start < period.end {
        let from = period.start.max(month.start);
        let to = period.end.min(month.end);
        if from < to {
            let (e, _) = expectation(d, series, month);
            total += e * Decimal::from((to - from).num_days()) / Decimal::from(month.days());
        }
        month = month.shift_months(1);
    }
    total
}

/// Alarm, wenn die Erzeugung seit der vorherigen Ablesung unter dem
/// Schwellwert der Erwartung desselben Zeitraums liegt; gleichbedeutend mit
/// der Tagesrate gegen `Schwellwert · Erwartung / Tage` (SPEC_PV_MONITORING).
// @impl SPEC_PV_MONITORING
#[must_use]
pub fn yield_alarm(actual: Decimal, expected: Decimal, threshold_pct: u8) -> bool {
    expected > Decimal::ZERO
        && actual < expected * Decimal::from(threshold_pct) / Decimal::ONE_HUNDRED
}

/// Wirtschaftliche Größen eines Zeitraums einer Anlage (SPEC_PV_ECONOMICS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Economics {
    /// Einspeisevergütung (Anteil der Anlage)
    pub feed_in: Decimal,
    /// Ersparnis `EV · p_haus` (Anteil der Anlage)
    pub savings: Decimal,
    /// Vorteil Eigenverbrauch `EV · (p_haus - v_ein)` (Anteil der Anlage)
    pub advantage: Decimal,
    /// Kosten aus dem Kostenbuch
    pub costs: Decimal,
}

impl Economics {
    /// Bilanz = Vergütung + Ersparnis - Kosten.
    #[must_use]
    pub fn balance(&self) -> Decimal {
        self.feed_in + self.savings - self.costs
    }
}

/// Anteil einer Anlage an Vergütung und Ersparnis des Hausanschlusses nach
/// ihrem Anteil an der Erzeugung `G_anlage / G`; ohne Erzeugung 0.
// @impl SPEC_PV_ECONOMICS
#[must_use]
pub fn economics(
    share: Decimal,
    self_consumption: Decimal,
    house_price: Option<Decimal>,
    feed_in_total: Decimal,
    export: Decimal,
    costs: Decimal,
) -> Economics {
    let price = house_price.unwrap_or(Decimal::ZERO);
    let v_ein = if export.is_zero() {
        Decimal::ZERO
    } else {
        feed_in_total / export
    };
    Economics {
        feed_in: (feed_in_total * share).round_dp(2),
        savings: (self_consumption * price * share).round_dp(2),
        advantage: (self_consumption * (price - v_ein) * share).round_dp(2),
        costs,
    }
}

/// Erster Tag, an dem die kumulierte Bilanz nach einem negativen Stand
/// wieder die Null erreicht. Je Monat (zeitlich geordnet, gemessen oder
/// prognostiziert) fallen die Kosten am Monatsanfang an, die Erträge
/// (Vergütung und Ersparnis) gleichmäßig über den Monat. `None`, wenn die
/// Bilanz nie negativ war oder die Null nicht wieder erreicht.
// @impl SPEC_PV_ECONOMICS
#[must_use]
pub fn break_even(start: Decimal, months: &[(Period, Decimal, Decimal)]) -> Option<NaiveDate> {
    let mut cumulative = start;
    for (month, income, costs) in months {
        let before = cumulative - *costs;
        cumulative = before + *income;
        if before < Decimal::ZERO && cumulative >= Decimal::ZERO && *income > Decimal::ZERO {
            // Tag im Monat (1-basiert), an dem der Rest ausgeglichen ist
            let day = ((-before) * Decimal::from(month.days()) / *income)
                .ceil()
                .to_u64()
                .unwrap_or(1)
                .clamp(1, u64::try_from(month.days()).unwrap_or(1));
            return month.start.checked_add_days(Days::new(day - 1));
        }
    }
    None
}

/// Einfache Rendite p.a.: durchschnittliche jährliche Bilanz ohne
/// Investitionen / Summe der Investitionen.
// @impl SPEC_PV_ECONOMICS
#[must_use]
pub fn simple_return(
    yearly_without_investment: &[Decimal],
    investments: Decimal,
) -> Option<Decimal> {
    if yearly_without_investment.is_empty() || investments <= Decimal::ZERO {
        return None;
    }
    let avg = yearly_without_investment.iter().sum::<Decimal>()
        / Decimal::from(yearly_without_investment.len());
    Some((avg / investments).round_dp(4))
}

/// Mischpreis beim Heimladen mit PV: `s · v_ein + (1 - s) · p_haus`
/// (SPEC_CAR_ENERGY_COST).
// @impl SPEC_CAR_ENERGY_COST
#[must_use]
pub fn charging_price(pv_share: Decimal, feed_in_rate: Decimal, house_price: Decimal) -> Decimal {
    let s = pv_share.clamp(Decimal::ZERO, Decimal::ONE);
    s * feed_in_rate + (Decimal::ONE - s) * house_price
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn setup(generation: bool, export: bool, battery: bool) -> GridSetup {
        GridSetup {
            has_generation: generation,
            has_export_register: export,
            has_battery: battery,
            full_feed_in: false,
        }
    }

    fn flows(g: Decimal, b: Decimal, e: Decimal) -> Flows {
        Flows {
            generation: Some(g),
            import: Some(b),
            export: Some(e),
            charge: Some(dec!(300)),
            discharge: Some(dec!(270)),
        }
    }

    // @test SPEC_PV_KPI
    #[test]
    fn surplus_with_battery() {
        let k = kpis(
            setup(true, true, true),
            flows(dec!(1000), dec!(200), dec!(400)),
        );
        // EV = 1000 - 400 - (300 - 270) = 570, V = 200 + 570
        assert_eq!(k.self_consumption, Some(dec!(570)));
        assert_eq!(k.consumption, Some(dec!(770)));
        assert_eq!(k.self_consumption_ratio, Some(dec!(0.57)));
        assert_eq!(k.autarky, Some(dec!(0.7403)));
        assert!(!k.export_assumed && !k.negative_self_consumption);
    }

    // @test SPEC_PV_KPI
    #[test]
    fn edge_cases_of_req_pv_004() {
        // ohne Einspeiseregister: E = 0 angenommen, EV = G
        let k = kpis(
            setup(true, false, false),
            flows(dec!(50), dec!(200), dec!(0)),
        );
        assert_eq!(k.self_consumption, Some(dec!(50)));
        assert!(k.export_assumed);
        assert_eq!(k.export, None);
        // Volleinspeisung: EV = 0, V = B
        let full = GridSetup {
            full_feed_in: true,
            ..setup(true, true, false)
        };
        let k = kpis(full, flows(dec!(1000), dec!(200), dec!(990)));
        assert_eq!(k.self_consumption, Some(Decimal::ZERO));
        assert_eq!(k.consumption, Some(dec!(200)));
        // ohne Erzeugungszähler nur Bezug und Einspeisung
        let k = kpis(
            setup(false, true, false),
            flows(dec!(1000), dec!(200), dec!(50)),
        );
        assert_eq!((k.import, k.export), (Some(dec!(200)), Some(dec!(50))));
        assert_eq!(
            (k.generation, k.self_consumption, k.consumption),
            (None, None, None)
        );
        // fehlende Größe: undefiniert
        let mut f = flows(dec!(1000), dec!(200), dec!(400));
        f.export = None;
        assert_eq!(kpis(setup(true, true, false), f).self_consumption, None);
        // negativer Eigenverbrauch bleibt negativ und wird gemeldet
        let k = kpis(
            setup(true, true, false),
            flows(dec!(100), dec!(200), dec!(150)),
        );
        assert_eq!(k.self_consumption, Some(dec!(-50)));
        assert!(k.negative_self_consumption);
    }

    // @test SPEC_PV_BATTERY
    #[test]
    fn battery_figures() {
        let b = battery_kpis(dec!(300), dec!(270), Some(dec!(10)));
        assert_eq!(b.losses, dec!(30));
        assert_eq!(b.efficiency, Some(dec!(0.9)));
        assert_eq!(b.full_cycles, Some(dec!(27)));
        assert_eq!(battery_kpis(dec!(0), dec!(0), None).efficiency, None);
    }

    fn system() -> PvSystemData {
        PvSystemData {
            name: "Dach".into(),
            kind: PvKind::Surplus,
            kwp: dec!(10),
            commissioned_on: d(2025, 4, 16),
            azimuth: None,
            tilt: None,
            inverter: None,
            battery_kwh: None,
            specific_yield: dec!(950),
            degradation_pct: dec!(0.5),
            month_shares: None,
            alarm_threshold_pct: 50,
        }
    }

    // @test SPEC_PV_YIELD_EXPECTATION
    #[test]
    fn start_expectation_by_hand() {
        let s = system();
        // Juni 2025: 10 · 950 · 13,5 % = 1282,5
        assert_eq!(
            start_expectation(&s, Period::month_of(d(2025, 6, 1))),
            dec!(1282.5)
        );
        // April 2025: ab dem 16., 15 von 30 Tagen: 10 · 950 · 11,5 % / 2
        assert_eq!(
            start_expectation(&s, Period::month_of(d(2025, 4, 1))),
            dec!(546.25)
        );
        assert_eq!(
            start_expectation(&s, Period::month_of(d(2025, 3, 1))),
            Decimal::ZERO
        );
        // Juni 2027: zwei Jahre Degradation 0,995² = 0,990025
        assert_eq!(
            start_expectation(&s, Period::month_of(d(2027, 6, 1))),
            dec!(1269.707)
        );
        let total: Decimal = DEFAULT_MONTH_SHARES.iter().sum();
        assert_eq!(total, dec!(100));
    }

    fn series(year: i32, values: [u32; 12]) -> Vec<MonthValue> {
        (1..=12)
            .map(|m| MonthValue {
                month: Period::month_of(d(year, m, 1)),
                value: Some(Decimal::from(values[m as usize - 1])),
            })
            .collect()
    }

    // @test SPEC_PV_YIELD_EXPECTATION, SPEC_PV_MONITORING
    #[test]
    fn history_after_one_complete_year() {
        let mut s = system();
        s.commissioned_on = d(2025, 1, 1);
        let mut all = series(
            2025,
            [100, 200, 400, 600, 800, 900, 900, 800, 500, 300, 150, 100],
        );
        all.extend(
            series(2026, [90, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
                .into_iter()
                .take(1),
        );
        // 2025: Startwert
        let (_, source) = expectation(&s, &all, Period::month_of(d(2025, 6, 1)));
        assert_eq!(source, YieldSource::Start);
        // 2026: Historie, Jahresniveau 5750, Juni-Anteil 900/5750 nach Tagen gewichtet
        let (june, source) = expectation(&s, &all, Period::month_of(d(2026, 6, 1)));
        assert_eq!(source, YieldSource::History);
        assert!((june - dec!(900)).abs() < dec!(5), "{june}");
        // Ampel Januar 2026: Erwartung ≈ 100, 90 liegt im Band
        let months = yield_months(&s, &all);
        let jan = months.last().unwrap();
        assert_eq!(jan.source, YieldSource::History);
        assert_eq!(jan.status, Some(Status::Green));
        // Startwert-Ampel: 30 % grün, 45 % gelb, darüber rot
        let b = Band::from_target(dec!(100), START_TOLERANCE_PCT);
        assert_eq!(b.classify(dec!(71)), Status::Green);
        assert_eq!(b.classify(dec!(60)), Status::Yellow);
        assert_eq!(b.classify(dec!(54)), Status::Red);
    }

    // @test SPEC_PV_MONITORING
    #[test]
    fn alarm_below_threshold() {
        assert!(yield_alarm(dec!(40), dec!(100), 50));
        assert!(!yield_alarm(dec!(50), dec!(100), 50));
        assert!(
            !yield_alarm(dec!(0), dec!(0), 50),
            "ohne Erwartung kein Alarm"
        );
    }

    // @test SPEC_PV_ECONOMICS
    #[test]
    fn economics_and_break_even() {
        // zwei Anlagen, diese mit 60 % der Erzeugung
        let e = economics(
            dec!(0.6),
            dec!(2000),
            Some(dec!(0.30)),
            dec!(400),
            dec!(5000),
            dec!(100),
        );
        assert_eq!(e.feed_in, dec!(240));
        assert_eq!(e.savings, dec!(360));
        // v_ein = 0,08: 2000 · 0,22 · 0,6
        assert_eq!(e.advantage, dec!(264));
        assert_eq!(e.balance(), dec!(500));
        // Investition 1000 am Anfang des ersten Monats, je Monat +300: Null im
        // vierten Monat am 10. Tag
        let months: Vec<(Period, Decimal, Decimal)> = (1..=6)
            .map(|m| {
                let costs = if m == 1 { dec!(1000) } else { Decimal::ZERO };
                (Period::month_of(d(2026, m, 1)), dec!(300), costs)
            })
            .collect();
        assert_eq!(break_even(Decimal::ZERO, &months), Some(d(2026, 4, 10)));
        assert_eq!(break_even(dec!(-4000), &months), None);
        // Investition im selben Monat gedeckt: 10 € bei 31 € Ertrag am 10.
        let one = [(Period::month_of(d(2026, 1, 1)), dec!(31), dec!(10))];
        assert_eq!(break_even(Decimal::ZERO, &one), Some(d(2026, 1, 10)));
        // nie negativ
        let free = [(Period::month_of(d(2026, 1, 1)), dec!(31), Decimal::ZERO)];
        assert_eq!(break_even(Decimal::ZERO, &free), None);
        assert_eq!(
            simple_return(&[dec!(600), dec!(800)], dec!(10000)),
            Some(dec!(0.07))
        );
        assert_eq!(simple_return(&[], dec!(1)), None);
    }

    // @test SPEC_CAR_ENERGY_COST
    #[test]
    fn charging_price_with_pv() {
        assert_eq!(
            charging_price(dec!(0.5), dec!(0.08), dec!(0.30)),
            dec!(0.19)
        );
        // Steckersolargerät ohne Vergütung: PV-Anteil mit Preis 0
        assert_eq!(charging_price(dec!(0.25), dec!(0), dec!(0.40)), dec!(0.30));
    }
}
