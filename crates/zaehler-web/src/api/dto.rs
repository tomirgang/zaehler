//! DTOs der REST-API (ADR_023): eigene Typen des Web-Adapters, Dezimalwerte
//! als JSON-Strings, Zeitpunkte RFC 3339 in UTC.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use zaehler_app::{
    model::{Contract, Device, MeterAggregate, Reading},
    usecase::{
        analysis::{Analysis, ForecastScope, ScopedForecast},
        api::{IngestResult, IngestTarget},
        cost::{ContractCost, MeterCostView, PeriodCost},
    },
};
use zaehler_domain::{
    analysis::PeriodForecast, expectation::Status, period::Reference, series::MonthValue,
};

fn month(p: zaehler_domain::period::Period) -> String {
    p.start.format("%Y-%m").to_string()
}

/// Register mit letztem Stand.
#[derive(Debug, Serialize, ToSchema)]
pub struct RegisterDto {
    pub id: Uuid,
    pub code: String,
    /// `import` oder `export`
    pub direction: String,
    #[schema(value_type = Option<String>)]
    pub last_value: Option<Decimal>,
    pub last_read_at: Option<DateTime<Utc>>,
}

/// Messstelle mit Registern.
#[derive(Debug, Serialize, ToSchema)]
pub struct MeterDto {
    pub id: Uuid,
    pub name: String,
    /// `electricity`, `gas`, `water`, `heat`, `vehicle`, …
    pub kind: String,
    pub unit: String,
    pub purpose: String,
    pub archived: bool,
    pub last_reading_at: Option<DateTime<Utc>>,
    pub registers: Vec<RegisterDto>,
    /// Ableseplan; fehlt ohne Plan
    pub schedule: Option<ScheduleDto>,
    /// Höchster plausibler Tagesverbrauch
    #[schema(value_type = Option<String>)]
    pub max_daily_rate: Option<Decimal>,
    /// Beginn des Abrechnungsjahres
    pub billing_start: Option<MonthDayDto>,
    /// Jahresziel, Toleranz und Saisonprofil
    pub targets: TargetsDto,
    /// Übergeordnete Messstelle des eingebauten Geräts (Zwischenzähler)
    pub parent_meter_id: Option<Uuid>,
    /// Zählernummer des eingebauten Geräts
    pub serial_number: Option<String>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Ableseplan einer Messstelle.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScheduleDto {
    /// `every_days` (1 bis 3650 Tage) oder `monthly` (Tag 1 bis 28)
    pub kind: String,
    pub value: u16,
}

/// Monat und Tag, z.B. Beginn des Abrechnungsjahres.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub struct MonthDayDto {
    pub month: u8,
    pub day: u8,
}

/// Vorgaben für Erwartung und Ampel.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct TargetsDto {
    /// Erwarteter Jahresverbrauch in der Einheit der Messstelle
    #[schema(value_type = Option<String>)]
    #[serde(default)]
    pub annual_target: Option<Decimal>,
    /// Toleranz in Prozent (1 bis 100); ohne Angabe gilt 20
    #[serde(default)]
    pub tolerance_pct: Option<u8>,
    /// Angepasstes Saisonprofil: 12 Gewichte Januar bis Dezember
    #[schema(value_type = Option<Vec<String>>)]
    #[serde(default)]
    pub season_weights: Option<Vec<Decimal>>,
}

impl MeterDto {
    #[must_use]
    pub fn new(agg: &MeterAggregate, readings: &[Reading]) -> Self {
        let m = &agg.meter;
        Self {
            id: m.id.0,
            name: m.data.name.clone(),
            kind: m.data.kind.as_str().into(),
            unit: m.data.unit.clone(),
            purpose: m.data.purpose.as_str().into(),
            archived: m.archived,
            last_reading_at: readings.iter().map(|r| r.read_at).max(),
            registers: agg
                .registers
                .iter()
                .map(|r| {
                    let last = readings
                        .iter()
                        .filter(|x| x.register_id == r.id)
                        .max_by_key(|x| (x.read_at, x.id));
                    RegisterDto {
                        id: r.id.0,
                        code: r.data.code.clone(),
                        direction: r.data.direction.as_str().into(),
                        last_value: last.map(|x| x.value),
                        last_read_at: last.map(|x| x.read_at),
                    }
                })
                .collect(),
            schedule: m.data.schedule.map(|s| ScheduleDto {
                kind: s.kind().into(),
                value: s.value(),
            }),
            max_daily_rate: m.data.max_daily_rate,
            billing_start: m.data.billing_start.map(|b| MonthDayDto {
                month: b.month,
                day: b.day,
            }),
            targets: TargetsDto {
                annual_target: m.data.targets.annual_target,
                tolerance_pct: m.data.targets.tolerance_pct,
                season_weights: m
                    .data
                    .targets
                    .season_weights
                    .as_ref()
                    .map(|p| p.monthly_weights.to_vec()),
            },
            parent_meter_id: agg
                .active_device()
                .and_then(|d| d.data.parent_meter_id)
                .map(|p| p.0),
            serial_number: agg
                .active_device()
                .and_then(|d| d.data.serial_number.clone()),
            archived_at: m.archived_at,
            created_at: m.created_at,
        }
    }
}

/// Gerät einer Messstelle.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceDto {
    pub id: Uuid,
    pub serial_number: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub removed_at: Option<DateTime<Utc>>,
    pub decimal_places: u8,
    pub rollover_digits: Option<u8>,
    pub parent_meter_id: Option<Uuid>,
}

impl From<&Device> for DeviceDto {
    fn from(d: &Device) -> Self {
        Self {
            id: d.id.0,
            serial_number: d.data.serial_number.clone(),
            installed_at: d.data.installed_at,
            removed_at: d.removed_at,
            decimal_places: d.data.decimal_places,
            rollover_digits: d.data.rollover_digits,
            parent_meter_id: d.data.parent_meter_id.map(|m| m.0),
        }
    }
}

/// Messstelle mit Geräten.
#[derive(Debug, Serialize, ToSchema)]
pub struct MeterDetailDto {
    pub meter: MeterDto,
    pub devices: Vec<DeviceDto>,
}

/// Ablesung.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReadingDto {
    pub id: Uuid,
    pub register_id: Uuid,
    pub device_id: Uuid,
    pub read_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub value: Decimal,
    /// `manual`, `photo`, `csv_import`, `invoice`, `api`
    pub source: String,
    pub note: Option<String>,
}

impl From<&Reading> for ReadingDto {
    fn from(r: &Reading) -> Self {
        Self {
            id: r.id.0,
            register_id: r.register_id.0,
            device_id: r.device_id.0,
            read_at: r.read_at,
            value: r.value,
            source: r.source.as_str().into(),
            note: r.note.clone(),
        }
    }
}

/// Eine Seite Ablesungen, chronologisch.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReadingPage {
    pub items: Vec<ReadingDto>,
    /// Für die nächste Seite als `cursor` übergeben; fehlt auf der letzten
    pub next_cursor: Option<String>,
}

/// Filter und Blättern der Ablesungen.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReadingQuery {
    /// Ab diesem Zeitpunkt (einschließlich)
    pub from: Option<DateTime<Utc>>,
    /// Bis zu diesem Zeitpunkt (ausschließlich)
    pub to: Option<DateTime<Utc>>,
    /// Höchstens 1000, Standard 100
    pub limit: Option<usize>,
    /// `next_cursor` der vorherigen Seite
    pub cursor: Option<String>,
}

/// Wert eines Registers.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ValueDto {
    pub register_id: Uuid,
    #[schema(value_type = String)]
    pub value: Decimal,
}

/// Ablesungen erfassen wie im Formular.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RecordReadingsBody {
    pub read_at: DateTime<Utc>,
    pub values: Vec<ValueDto>,
    pub note: Option<String>,
    /// Plausibilitätswarnungen bestätigen
    #[serde(default)]
    pub confirm: bool,
}

/// Gespeicherte Ablesungen.
#[derive(Debug, Serialize, ToSchema)]
pub struct RecordedDto {
    pub reading_ids: Vec<Uuid>,
}

/// Automatisch gelieferter Stand: `register_id` oder `serial` und `register`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct IngestDto {
    pub register_id: Option<Uuid>,
    /// Zählernummer des eingebauten Geräts
    pub serial: Option<String>,
    /// Code des Registers, z.B. `1.8.0`
    pub register: Option<String>,
    pub read_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub value: Decimal,
}

impl IngestDto {
    /// Ziel des Stands; `None` ohne `register_id` und ohne Zählernummer.
    #[must_use]
    pub fn target(&self) -> Option<IngestTarget> {
        match (&self.register_id, &self.serial, &self.register) {
            (Some(id), None, None) => {
                Some(IngestTarget::Register(zaehler_app::ids::RegisterId(*id)))
            }
            (None, Some(serial), Some(register)) => Some(IngestTarget::Serial {
                serial: serial.clone(),
                register: register.clone(),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct IngestBody {
    /// Höchstens 1000 Stände
    pub readings: Vec<IngestDto>,
}

/// Ergebnis je Stand in der Reihenfolge der Anfrage.
#[derive(Debug, Serialize, ToSchema)]
pub struct IngestResultDto {
    /// `stored`, `replaced`, `ignored` oder `error`
    pub status: String,
    pub reading_id: Option<Uuid>,
    /// Fehlercode bei `error`
    pub error: Option<String>,
}

impl From<&IngestResult> for IngestResultDto {
    fn from(r: &IngestResult) -> Self {
        let (reading_id, error) = match r {
            IngestResult::Stored(id) | IngestResult::Replaced(id) => (Some(id.0), None),
            IngestResult::Ignored => (None, None),
            IngestResult::Error(code) => (None, Some((*code).to_owned())),
        };
        Self {
            status: r.as_str().into(),
            reading_id,
            error,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IngestResponse {
    pub results: Vec<IngestResultDto>,
}

/// Monat der Auswertung mit Erwartung und Ampel.
#[derive(Debug, Serialize, ToSchema)]
pub struct MonthDto {
    /// `YYYY-MM`
    pub month: String,
    #[schema(value_type = Option<String>)]
    pub import: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub export: Option<Decimal>,
    /// Gesamtverbrauch bei einem Hausanschluss mit PV
    #[schema(value_type = Option<String>)]
    pub total: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub expected: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub lower: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub upper: Option<Decimal>,
    /// Ampel: `green`, `yellow`, `red`
    pub status: Option<String>,
}

/// Vergleich des Bezugsmonats.
#[derive(Debug, Serialize, ToSchema)]
pub struct ComparisonDto {
    /// `previous_month`, `previous_year`, `trailing_average_3`, …
    pub reference: String,
    #[schema(value_type = Option<String>)]
    pub current: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub reference_value: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub abs_diff: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub rel_diff_percent: Option<Decimal>,
}

/// Prognose eines Zeitraums.
#[derive(Debug, Serialize, ToSchema)]
pub struct ForecastDto {
    /// `calendar_year` oder `billing_year`
    pub scope: String,
    pub period_start: NaiveDate,
    /// Erster Tag nach dem Zeitraum
    pub period_end: NaiveDate,
    /// `projected`, `too_short` oder `no_reading`
    pub status: String,
    #[schema(value_type = Option<String>)]
    pub observed: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub projected_total: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub coverage: Option<Decimal>,
}

impl From<&ScopedForecast> for ForecastDto {
    fn from(f: &ScopedForecast) -> Self {
        let (status, p) = match &f.forecast {
            PeriodForecast::Projected(p) => ("projected", Some(p)),
            PeriodForecast::TooShort => ("too_short", None),
            PeriodForecast::NoReading => ("no_reading", None),
        };
        Self {
            scope: match f.scope {
                ForecastScope::CalendarYear => "calendar_year",
                ForecastScope::BillingYear => "billing_year",
            }
            .into(),
            period_start: f.period.start,
            period_end: f.period.end,
            status: status.into(),
            observed: p.map(|p| p.observed),
            projected_total: p.map(|p| p.projected_total),
            coverage: p.map(|p| p.coverage),
        }
    }
}

/// Monatsserie, Vergleiche, Prognose und Ampel einer Messstelle.
#[derive(Debug, Serialize, ToSchema)]
pub struct AnalysisDto {
    pub meter_id: Uuid,
    pub unit: String,
    /// Hauptrichtung: `import` oder `export`
    pub primary: String,
    /// Vergleiche und Ampel beziehen sich auf den Gesamtverbrauch (PV)
    pub uses_total: bool,
    pub months: Vec<MonthDto>,
    /// Bezugsmonat der Vergleiche (`YYYY-MM`)
    pub reference_month: Option<String>,
    pub comparisons: Vec<ComparisonDto>,
    pub forecasts: Vec<ForecastDto>,
}

fn value(series: &[MonthValue], m: zaehler_domain::period::Period) -> Option<Decimal> {
    series.iter().find(|x| x.month == m).and_then(|x| x.value)
}

fn reference(r: Reference) -> String {
    match r {
        Reference::PreviousMonth => "previous_month".into(),
        Reference::PreviousYear => "previous_year".into(),
        Reference::TrailingAverage(n) => format!("trailing_average_{n}"),
    }
}

impl From<&Analysis> for AnalysisDto {
    fn from(a: &Analysis) -> Self {
        let base = if a.import.is_empty() {
            &a.export
        } else {
            &a.import
        };
        let months = base
            .iter()
            .map(|mv| {
                let e = a
                    .complete_months
                    .iter()
                    .position(|c| *c == mv.month)
                    .and_then(|i| a.expectations.get(i).copied().flatten());
                MonthDto {
                    month: month(mv.month),
                    import: value(&a.import, mv.month),
                    export: value(&a.export, mv.month),
                    total: value(&a.total_consumption, mv.month),
                    expected: e.map(|e| e.band.expected),
                    lower: e.map(|e| e.band.lower),
                    upper: e.map(|e| e.band.upper),
                    status: e.and_then(|e| e.status).map(|s| {
                        match s {
                            Status::Green => "green",
                            Status::Yellow => "yellow",
                            Status::Red => "red",
                        }
                        .into()
                    }),
                }
            })
            .collect();
        Self {
            meter_id: a.detail.aggregate.meter.id.0,
            unit: a.detail.aggregate.meter.data.unit.clone(),
            primary: a.primary.as_str().into(),
            uses_total: a.uses_total,
            months,
            reference_month: a.month.map(month),
            comparisons: a
                .comparisons
                .iter()
                .map(|c| ComparisonDto {
                    reference: reference(c.reference),
                    current: c.comparison.as_ref().map(|x| x.current),
                    reference_value: c.comparison.as_ref().map(|x| x.reference),
                    abs_diff: c.comparison.as_ref().map(|x| x.abs_diff),
                    rel_diff_percent: c.comparison.as_ref().and_then(|x| x.rel_diff_percent),
                })
                .collect(),
            forecasts: a.forecasts.iter().map(ForecastDto::from).collect(),
        }
    }
}

/// Kosten eines Zeitraums über alle Verträge einer Richtung.
#[derive(Debug, Serialize, ToSchema)]
pub struct PeriodCostDto {
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    #[schema(value_type = String)]
    pub measured: Decimal,
    #[schema(value_type = String)]
    pub estimated: Decimal,
    #[schema(value_type = String)]
    pub adjustments: Decimal,
    #[schema(value_type = String)]
    pub total: Decimal,
    /// Gemessene Tage / Tage mit Vertrag
    #[schema(value_type = String)]
    pub coverage: Decimal,
    pub without_contract: bool,
}

impl From<&PeriodCost> for PeriodCostDto {
    fn from(p: &PeriodCost) -> Self {
        Self {
            period_start: p.period.start,
            period_end: p.period.end,
            measured: p.measured,
            estimated: p.estimated,
            adjustments: p.adjustments,
            total: p.total(),
            coverage: p.coverage,
            without_contract: p.without_contract,
        }
    }
}

/// Abrechnungszeitraum eines Vertrags mit Prognose und Abschlägen.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContractCostDto {
    pub contract_id: Uuid,
    pub period_start: Option<NaiveDate>,
    pub period_end: Option<NaiveDate>,
    #[schema(value_type = Option<String>)]
    pub total: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub estimated: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub quantity: Option<Decimal>,
    pub quantity_estimated: bool,
    #[schema(value_type = Option<String>)]
    pub unit_price: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub installments_paid: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub installments_open: Option<Decimal>,
    /// Prognose minus Abschläge; positiv = Nachzahlung
    #[schema(value_type = Option<String>)]
    pub balance: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub suggested_installment: Option<Decimal>,
}

impl From<&ContractCost> for ContractCostDto {
    fn from(c: &ContractCost) -> Self {
        let i = c.installments.as_ref();
        Self {
            contract_id: c.contract.id.0,
            period_start: c.period.map(|p| p.start),
            period_end: c.period.map(|p| p.end),
            total: c
                .breakdown
                .as_ref()
                .map(zaehler_domain::cost::CostBreakdown::total),
            estimated: c
                .breakdown
                .as_ref()
                .map(zaehler_domain::cost::CostBreakdown::estimated),
            quantity: c.quantity.map(|q| q.value),
            quantity_estimated: c.quantity.is_some_and(|q| q.estimated),
            unit_price: c.unit_price(),
            installments_paid: i.map(|i| i.paid),
            installments_open: i.map(|i| i.open),
            balance: i.and_then(|i| i.balance),
            suggested_installment: i.and_then(|i| i.suggestion),
        }
    }
}

/// Monat der Kostenansicht.
#[derive(Debug, Serialize, ToSchema)]
pub struct MonthCostDto {
    pub month: String,
    #[schema(value_type = String)]
    pub measured: Decimal,
    #[schema(value_type = String)]
    pub estimated: Decimal,
    /// Einspeisung (negativ = Gutschrift)
    #[schema(value_type = String)]
    pub export: Decimal,
}

/// Kostenansicht einer Messstelle.
#[derive(Debug, Serialize, ToSchema)]
pub struct CostsDto {
    pub meter_id: Uuid,
    pub today: NaiveDate,
    pub import: Option<ContractCostDto>,
    pub export: Option<ContractCostDto>,
    pub year_import: Option<PeriodCostDto>,
    pub year_export: Option<PeriodCostDto>,
    pub months: Vec<MonthCostDto>,
}

impl From<&MeterCostView> for CostsDto {
    fn from(v: &MeterCostView) -> Self {
        Self {
            meter_id: v.meter.meter.id.0,
            today: v.today,
            import: v.import.as_ref().map(ContractCostDto::from),
            export: v.export.as_ref().map(ContractCostDto::from),
            year_import: v.year_import.as_ref().map(PeriodCostDto::from),
            year_export: v.year_export.as_ref().map(PeriodCostDto::from),
            months: v
                .months
                .iter()
                .map(|m| MonthCostDto {
                    month: month(m.month),
                    measured: m.measured,
                    estimated: m.estimated,
                    export: m.export,
                })
                .collect(),
        }
    }
}

/// Preisstand einer Position.
#[derive(Debug, Serialize, ToSchema)]
pub struct PriceComponentDto {
    pub id: Uuid,
    pub valid_from: NaiveDate,
    #[schema(value_type = String)]
    pub base_price: Decimal,
    /// `day`, `month` oder `year`
    pub base_interval: String,
    /// Arbeitspreis: `{"kind": "flat", "price": "0.30"}` oder Staffel
    #[schema(value_type = Object)]
    pub unit_pricing: serde_json::Value,
    #[schema(value_type = Option<String>)]
    pub factor: Option<Decimal>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PositionDto {
    pub id: Uuid,
    pub name: String,
    pub register_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub factor: Decimal,
    #[schema(value_type = Option<String>)]
    pub vat_pct: Option<Decimal>,
    pub components: Vec<PriceComponentDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InstallmentDto {
    pub id: Uuid,
    pub valid_from: NaiveDate,
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub every_months: u8,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdjustmentDto {
    pub id: Uuid,
    /// `bonus` oder `credit`
    pub kind: String,
    pub booked_on: NaiveDate,
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub note: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SettlementDto {
    pub id: Uuid,
    pub period_start: NaiveDate,
    /// Letzter Tag (einschließlich)
    pub period_end: NaiveDate,
    #[schema(value_type = String)]
    pub amount: Decimal,
    /// Beleg aus dem Rechnungsimport
    pub document_id: Option<Uuid>,
}

/// Vertrag mit Positionen, Preisständen, Abschlägen, Anpassungen und
/// Abrechnungen.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContractDto {
    pub id: Uuid,
    pub meter_id: Uuid,
    /// `import` (Bezug) oder `export` (Einspeisung)
    pub direction: String,
    pub provider: String,
    pub contract_number: Option<String>,
    pub starts_on: NaiveDate,
    /// Letzter Tag (einschließlich)
    pub ends_on: Option<NaiveDate>,
    /// Beginn des Abrechnungsjahres `MM-DD`
    pub billing_start: String,
    pub note: Option<String>,
    pub positions: Vec<PositionDto>,
    pub installments: Vec<InstallmentDto>,
    pub adjustments: Vec<AdjustmentDto>,
    pub settlements: Vec<SettlementDto>,
}

impl From<&Contract> for ContractDto {
    fn from(c: &Contract) -> Self {
        let d = &c.data;
        Self {
            id: c.id.0,
            meter_id: c.meter_id.0,
            direction: d.direction.as_str().into(),
            provider: d.provider.clone(),
            contract_number: d.contract_number.clone(),
            starts_on: d.starts_on,
            ends_on: d.ends_on,
            billing_start: format!("{:02}-{:02}", d.billing_start.month, d.billing_start.day),
            note: d.note.clone(),
            positions: d
                .positions
                .iter()
                .map(|p| PositionDto {
                    id: p.id.0,
                    name: p.name.clone(),
                    register_id: p.register_id.map(|r| r.0),
                    factor: p.factor,
                    vat_pct: p.vat_pct,
                    components: p
                        .components
                        .iter()
                        .map(|x| PriceComponentDto {
                            id: x.id.0,
                            valid_from: x.component.valid_from,
                            base_price: x.component.base_price,
                            base_interval: x.component.base_interval.as_str().into(),
                            unit_pricing: serde_json::to_value(&x.component.unit_pricing)
                                .unwrap_or_default(),
                            factor: x.component.factor,
                        })
                        .collect(),
                })
                .collect(),
            installments: c
                .installments
                .iter()
                .map(|x| InstallmentDto {
                    id: x.id.0,
                    valid_from: x.installment.valid_from,
                    amount: x.installment.amount,
                    every_months: x.installment.every_months,
                })
                .collect(),
            adjustments: c
                .adjustments
                .iter()
                .map(|x| AdjustmentDto {
                    id: x.id.0,
                    kind: x.adjustment.kind.as_str().into(),
                    booked_on: x.adjustment.booked_on,
                    amount: x.adjustment.amount,
                    note: x.note.clone(),
                })
                .collect(),
            settlements: c
                .settlements
                .iter()
                .map(|x| SettlementDto {
                    id: x.id.0,
                    period_start: x.period_start,
                    period_end: x.period_end,
                    amount: x.amount,
                    document_id: x.document_id,
                })
                .collect(),
        }
    }
}
