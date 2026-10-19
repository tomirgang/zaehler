//! REST-API Photovoltaik (SPEC_API_REST): Anlagen mit Zuordnung, Bericht,
//! Kennzahlen und Sammelerfassung je Hausanschluss, Übersicht.

use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{MeterId, PvSystemId, RegisterId},
    model::PvSystem,
    usecase::pv::{
        BulkInput, BulkOutcome, GridPeriod, GridView, PvCard, PvPeriodEconomics, PvReport,
        PvSystemInput,
    },
};
use zaehler_domain::{
    expectation::Status,
    period::Period,
    pv::{
        DEFAULT_ALARM_THRESHOLD_PCT, DEFAULT_DEGRADATION_PCT, DEFAULT_SPECIFIC_YIELD, PvKind,
        PvRole, PvSystemData, YieldMonth, YieldSource,
    },
};

use super::{
    ApiResult,
    auth::ApiCtx,
    dto::MeterDto,
    error::{ApiError, Problem},
};
use crate::state::WebState;

pub(super) fn routes(r: OpenApiRouter<WebState>) -> OpenApiRouter<WebState> {
    r.routes(routes!(pv_overview))
        .routes(routes!(list_systems, create_system))
        .routes(routes!(get_system, update_system, delete_system))
        .routes(routes!(system_report))
        .routes(routes!(grid_analysis))
        .routes(routes!(grid_meters))
        .routes(routes!(record_bulk))
}

fn month(p: Period) -> String {
    p.start.format("%Y-%m").to_string()
}

fn status_str(s: Status) -> String {
    match s {
        Status::Green => "green",
        Status::Yellow => "yellow",
        Status::Red => "red",
    }
    .into()
}

/// Messstelle einer Anlage mit Rolle.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PvMeterDto {
    pub meter_id: Uuid,
    /// `grid`, `generation` oder `battery`
    pub role: String,
}

/// PV-Anlage mit Zuordnung und Einstellungen für Ertrag und Alarm.
#[derive(Debug, Serialize, ToSchema)]
pub struct PvSystemDto {
    pub id: Uuid,
    pub name: String,
    /// `surplus`, `full_feed_in` oder `plug_in`
    pub kind: String,
    #[schema(value_type = String)]
    pub kwp: Decimal,
    pub commissioned_on: NaiveDate,
    /// Ausrichtung in Grad (0–360)
    pub azimuth: Option<i16>,
    /// Neigung in Grad (0–90)
    pub tilt: Option<i16>,
    pub inverter: Option<String>,
    /// Speicherkapazität in kWh
    #[schema(value_type = Option<String>)]
    pub battery_kwh: Option<Decimal>,
    /// kWh/kWp und Jahr
    #[schema(value_type = String)]
    pub specific_yield: Decimal,
    /// Degradation in Prozent je Jahr
    #[schema(value_type = String)]
    pub degradation_pct: Decimal,
    /// Eigene Monatsanteile Januar–Dezember in Prozent; `null` = Standard
    #[schema(value_type = Option<Vec<String>>)]
    pub month_shares: Option<Vec<Decimal>>,
    /// Monatsanteile, mit denen gerechnet wird
    #[schema(value_type = Vec<String>)]
    pub effective_month_shares: Vec<Decimal>,
    pub alarm_threshold_pct: u8,
    pub meters: Vec<PvMeterDto>,
}

impl From<&PvSystem> for PvSystemDto {
    fn from(s: &PvSystem) -> Self {
        let d = &s.data;
        Self {
            id: s.id.0,
            name: d.name.clone(),
            kind: d.kind.as_str().into(),
            kwp: d.kwp,
            commissioned_on: d.commissioned_on,
            azimuth: d.azimuth,
            tilt: d.tilt,
            inverter: d.inverter.clone(),
            battery_kwh: d.battery_kwh,
            specific_yield: d.specific_yield,
            degradation_pct: d.degradation_pct,
            month_shares: d.month_shares.map(|m| m.to_vec()),
            effective_month_shares: d.shares().to_vec(),
            alarm_threshold_pct: d.alarm_threshold_pct,
            meters: s
                .meters
                .iter()
                .map(|(m, r)| PvMeterDto {
                    meter_id: m.0,
                    role: r.as_str().into(),
                })
                .collect(),
        }
    }
}

/// Anlage anlegen oder vollständig ersetzen; fehlende Ertragswerte erhalten
/// die Standardwerte.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PvSystemBody {
    pub name: String,
    /// `surplus`, `full_feed_in` oder `plug_in`
    pub kind: String,
    #[schema(value_type = String)]
    pub kwp: Decimal,
    pub commissioned_on: NaiveDate,
    pub azimuth: Option<i16>,
    pub tilt: Option<i16>,
    pub inverter: Option<String>,
    #[schema(value_type = Option<String>)]
    pub battery_kwh: Option<Decimal>,
    /// Standard 950
    #[schema(value_type = Option<String>)]
    pub specific_yield: Option<Decimal>,
    /// Standard 0.5
    #[schema(value_type = Option<String>)]
    pub degradation_pct: Option<Decimal>,
    /// Genau zwölf Werte, Summe 100
    #[schema(value_type = Option<Vec<String>>)]
    pub month_shares: Option<Vec<Decimal>>,
    /// Standard 50
    pub alarm_threshold_pct: Option<u8>,
    /// Genau ein `grid`, mindestens ein `generation`, optional `battery`
    pub meters: Vec<PvMeterDto>,
}

impl PvSystemBody {
    fn into_input(self) -> Result<PvSystemInput, AppError> {
        let mut e = FieldErrors::default();
        let kind = self
            .kind
            .parse::<PvKind>()
            .map_err(|_| e.add("kind", "form.kind_invalid"))
            .ok();
        let shares = self.month_shares.and_then(|v| {
            <[Decimal; 12]>::try_from(v)
                .map_err(|_| e.add("month_shares", "pv.shares_invalid"))
                .ok()
        });
        let mut meters = Vec::new();
        for m in &self.meters {
            match m.role.parse::<PvRole>() {
                Ok(r) => meters.push((MeterId(m.meter_id), r)),
                Err(_) => e.add("meters", "pv.role_invalid"),
            }
        }
        e.into_result()?;
        let Some(kind) = kind else {
            return Err(AppError::field("kind", "form.kind_invalid"));
        };
        Ok(PvSystemInput {
            data: PvSystemData {
                name: self.name,
                kind,
                kwp: self.kwp,
                commissioned_on: self.commissioned_on,
                azimuth: self.azimuth,
                tilt: self.tilt,
                inverter: self.inverter,
                battery_kwh: self.battery_kwh,
                specific_yield: self.specific_yield.unwrap_or(DEFAULT_SPECIFIC_YIELD),
                degradation_pct: self.degradation_pct.unwrap_or(DEFAULT_DEGRADATION_PCT),
                month_shares: shares,
                alarm_threshold_pct: self
                    .alarm_threshold_pct
                    .unwrap_or(DEFAULT_ALARM_THRESHOLD_PCT),
            },
            meters,
        })
    }
}

/// Erzeugung gegen Erwartung eines Monats.
#[derive(Debug, Serialize, ToSchema)]
pub struct YieldMonthDto {
    /// `YYYY-MM`
    pub month: String,
    #[schema(value_type = Option<String>)]
    pub actual: Option<Decimal>,
    #[schema(value_type = String)]
    pub expected: Decimal,
    /// `start` (Startwerte) oder `history` (eigene Monatswerte)
    pub source: String,
    #[schema(value_type = String)]
    pub lower: Decimal,
    #[schema(value_type = String)]
    pub upper: Decimal,
    /// `green`, `yellow` oder `red`
    pub status: Option<String>,
}

impl From<&YieldMonth> for YieldMonthDto {
    fn from(y: &YieldMonth) -> Self {
        Self {
            month: month(y.month),
            actual: y.actual,
            expected: y.expected,
            source: match y.source {
                YieldSource::Start => "start",
                YieldSource::History => "history",
            }
            .into(),
            lower: y.band.lower,
            upper: y.band.upper,
            status: y.status.map(status_str),
        }
    }
}

/// Jahresprognose der Erzeugung.
#[derive(Debug, Serialize, ToSchema)]
pub struct YieldForecastDto {
    pub year: i32,
    #[schema(value_type = String)]
    pub measured: Decimal,
    #[schema(value_type = String)]
    pub expected_rest: Decimal,
    #[schema(value_type = String)]
    pub total: Decimal,
}

/// Wirtschaftlichkeit eines Zeitraums `[start, end)`.
#[derive(Debug, Serialize, ToSchema)]
pub struct PvEconomicsDto {
    pub start: NaiveDate,
    /// Exklusiv
    pub end: NaiveDate,
    #[schema(value_type = Option<String>)]
    pub generation: Option<Decimal>,
    /// Einspeisevergütung
    #[schema(value_type = String)]
    pub feed_in: Decimal,
    /// Ersparnis durch Eigenverbrauch
    #[schema(value_type = String)]
    pub savings: Decimal,
    #[schema(value_type = String)]
    pub advantage: Decimal,
    #[schema(value_type = String)]
    pub costs: Decimal,
    /// Vorteil abzüglich Kosten
    #[schema(value_type = String)]
    pub balance: Decimal,
    /// davon Investitionen und Ersatzinvestitionen
    #[schema(value_type = String)]
    pub investments: Decimal,
    /// Bilanz kumuliert seit Inbetriebnahme am Ende des Zeitraums
    #[schema(value_type = String)]
    pub cumulative: Decimal,
    /// fortgeschrieben (Zukunft)
    pub projected: bool,
}

impl From<&PvPeriodEconomics> for PvEconomicsDto {
    fn from(p: &PvPeriodEconomics) -> Self {
        Self {
            start: p.period.start,
            end: p.period.end,
            generation: p.generation,
            feed_in: p.economics.feed_in,
            savings: p.economics.savings,
            advantage: p.economics.advantage,
            costs: p.economics.costs,
            balance: p.economics.balance(),
            investments: p.investments,
            cumulative: p.cumulative,
            projected: p.projected,
        }
    }
}

/// Anlagenbericht: Ertrag gegen Erwartung, Prognose und Wirtschaftlichkeit.
#[derive(Debug, Serialize, ToSchema)]
pub struct PvReportDto {
    pub system: PvSystemDto,
    pub grid_name: Option<String>,
    /// Je Monat seit Inbetriebnahme
    pub yields: Vec<YieldMonthDto>,
    pub forecast: Option<YieldForecastDto>,
    /// Jahre seit Inbetriebnahme
    pub years: Vec<PvEconomicsDto>,
    /// Monate gemessen und fortgeschrieben
    pub months: Vec<PvEconomicsDto>,
    /// Amortisationszeitpunkt
    pub break_even: Option<NaiveDate>,
    /// Amortisation liegt in der Fortschreibung
    pub break_even_projected: bool,
    /// Einfache Rendite in Prozent
    #[schema(value_type = Option<String>)]
    pub simple_return: Option<Decimal>,
    #[schema(value_type = String)]
    pub investments: Decimal,
}

impl From<&PvReport> for PvReportDto {
    fn from(r: &PvReport) -> Self {
        Self {
            system: PvSystemDto::from(&r.system),
            grid_name: r.grid_name.clone(),
            yields: r.yields.iter().map(YieldMonthDto::from).collect(),
            forecast: r.forecast.map(|f| YieldForecastDto {
                year: f.year,
                measured: f.measured,
                expected_rest: f.expected_rest,
                total: f.total(),
            }),
            years: r.years.iter().map(PvEconomicsDto::from).collect(),
            months: r.months.iter().map(PvEconomicsDto::from).collect(),
            break_even: r.break_even.map(|(d, _)| d),
            break_even_projected: r.break_even.is_some_and(|(_, p)| p),
            simple_return: r.simple_return,
            investments: r.investments,
        }
    }
}

/// Speicherkennzahlen eines Zeitraums.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatteryKpiDto {
    #[schema(value_type = String)]
    pub charge: Decimal,
    #[schema(value_type = String)]
    pub discharge: Decimal,
    #[schema(value_type = String)]
    pub losses: Decimal,
    #[schema(value_type = Option<String>)]
    pub efficiency: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub full_cycles: Option<Decimal>,
}

/// Energiekennzahlen eines Zeitraums `[start, end)` in kWh.
#[derive(Debug, Serialize, ToSchema)]
pub struct GridPeriodDto {
    pub start: NaiveDate,
    /// Exklusiv
    pub end: NaiveDate,
    /// Endet vor dem Ende des Zeitraums (bis zur letzten Ablesung)
    pub partial: bool,
    #[schema(value_type = Option<String>)]
    pub generation: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub import: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub export: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub charge: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub discharge: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub self_consumption: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub consumption: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub self_consumption_ratio: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub autarky: Option<Decimal>,
    /// Einspeisung angenommen (ohne Einspeiseregister)
    pub export_assumed: bool,
    pub negative_self_consumption: bool,
    pub battery: Option<BatteryKpiDto>,
}

impl From<&GridPeriod> for GridPeriodDto {
    fn from(p: &GridPeriod) -> Self {
        let k = &p.kpi;
        Self {
            start: p.period.start,
            end: p.period.end,
            partial: p.partial,
            generation: k.generation,
            import: k.import,
            export: k.export,
            charge: k.charge,
            discharge: k.discharge,
            self_consumption: k.self_consumption,
            consumption: k.consumption,
            self_consumption_ratio: k.self_consumption_ratio,
            autarky: k.autarky,
            export_assumed: k.export_assumed,
            negative_self_consumption: k.negative_self_consumption,
            battery: p.battery.map(|b| BatteryKpiDto {
                charge: b.charge,
                discharge: b.discharge,
                losses: b.losses,
                efficiency: b.efficiency,
                full_cycles: b.full_cycles,
            }),
        }
    }
}

/// Ausstattung des Hausanschlusses.
#[derive(Debug, Serialize, ToSchema)]
#[allow(clippy::struct_excessive_bools)] // Abbild von `GridSetup`
pub struct GridSetupDto {
    pub has_generation: bool,
    pub has_export_register: bool,
    pub has_battery: bool,
    pub full_feed_in: bool,
}

/// Spezifischer Ertrag einer Anlage in einem Jahr.
#[derive(Debug, Serialize, ToSchema)]
pub struct SystemYearDto {
    pub system_id: Uuid,
    pub name: String,
    pub year: i32,
    #[schema(value_type = Option<String>)]
    pub generation: Option<Decimal>,
    /// kWh/kWp
    #[schema(value_type = Option<String>)]
    pub specific: Option<Decimal>,
}

/// Anlage am Hausanschluss.
#[derive(Debug, Serialize, ToSchema)]
pub struct GridSystemDto {
    pub id: Uuid,
    pub name: String,
}

/// Energiekennzahlen eines Hausanschlusses.
#[derive(Debug, Serialize, ToSchema)]
pub struct GridAnalysisDto {
    pub grid_meter_id: Uuid,
    pub grid_name: String,
    pub systems: Vec<GridSystemDto>,
    pub setup: GridSetupDto,
    /// Monate von der ersten Ablesung bis heute
    pub months: Vec<GridPeriodDto>,
    /// Kalenderjahre, neuestes zuerst
    pub years: Vec<GridPeriodDto>,
    pub billing_year: Option<GridPeriodDto>,
    /// Letzter vollständig gemessener Monat (`YYYY-MM`)
    pub last_complete_month: Option<String>,
    pub yields: Vec<SystemYearDto>,
}

impl From<&GridView> for GridAnalysisDto {
    fn from(v: &GridView) -> Self {
        Self {
            grid_meter_id: v.grid.meter.id.0,
            grid_name: v.grid.meter.data.name.clone(),
            systems: v
                .systems
                .iter()
                .map(|s| GridSystemDto {
                    id: s.id.0,
                    name: s.data.name.clone(),
                })
                .collect(),
            setup: GridSetupDto {
                has_generation: v.setup.has_generation,
                has_export_register: v.setup.has_export_register,
                has_battery: v.setup.has_battery,
                full_feed_in: v.setup.full_feed_in,
            },
            months: v.months.iter().map(GridPeriodDto::from).collect(),
            years: v.years.iter().map(GridPeriodDto::from).collect(),
            billing_year: v.billing_year.as_ref().map(GridPeriodDto::from),
            last_complete_month: v.last_complete().map(|p| month(p.period)),
            yields: v
                .yields
                .iter()
                .map(|y| SystemYearDto {
                    system_id: y.system.0,
                    name: y.name.clone(),
                    year: y.year,
                    generation: y.generation,
                    specific: y.specific,
                })
                .collect(),
        }
    }
}

/// Karte eines Hausanschlusses in der PV-Übersicht.
#[derive(Debug, Serialize, ToSchema)]
pub struct PvCardDto {
    pub grid_meter_id: Uuid,
    pub name: String,
    /// Letzter Monat mit Erzeugung (`YYYY-MM`)
    pub month: Option<String>,
    #[schema(value_type = Option<String>)]
    pub generation: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub autarky: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub self_consumption_ratio: Option<Decimal>,
    /// Ungünstigste Ertragsampel: `green`, `yellow` oder `red`
    pub status: Option<String>,
}

impl From<&PvCard> for PvCardDto {
    fn from(c: &PvCard) -> Self {
        Self {
            grid_meter_id: c.grid.0,
            name: c.name.clone(),
            month: c.month.map(month),
            generation: c.generation,
            autarky: c.autarky,
            self_consumption_ratio: c.self_consumption_ratio,
            status: c.status.map(status_str),
        }
    }
}

/// Messstelle der Sammelerfassung mit Rolle.
#[derive(Debug, Serialize, ToSchema)]
pub struct PvGridMeterDto {
    /// `grid`, `generation` oder `battery`
    pub role: String,
    pub meter: MeterDto,
}

/// Stand einer Messstelle in der Sammelerfassung.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkValueDto {
    pub meter_id: Uuid,
    pub register_id: Uuid,
    #[schema(value_type = String)]
    pub value: Decimal,
}

/// Stände aller Messstellen eines Hausanschlusses mit gemeinsamem Zeitpunkt.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkReadingsBody {
    pub read_at: DateTime<Utc>,
    pub values: Vec<BulkValueDto>,
    pub note: Option<String>,
    /// Plausibilitätswarnungen bestätigen
    #[serde(default)]
    pub confirm: bool,
}

/// Anzahl gespeicherter Registerstände.
#[derive(Debug, Serialize, ToSchema)]
pub struct BulkRecordedDto {
    pub saved: usize,
}

/// PV-Übersicht: je Hausanschluss mit Anlage Erzeugung des letzten Monats,
/// Autarkiegrad, Eigenverbrauchsquote und Ampel.
#[utoipa::path(
    get,
    path = "/api/v1/pv",
    responses((status = OK, body = Vec<PvCardDto>)),
    tag = "pv"
)]
async fn pv_overview(State(s): State<WebState>, ctx: ApiCtx) -> ApiResult<Vec<PvCardDto>> {
    let cards = s.uc.pv_dashboard.execute(&ctx.tenant).await?;
    Ok(Json(cards.iter().map(PvCardDto::from).collect()))
}

/// PV-Anlagen des Haushalts.
#[utoipa::path(
    get,
    path = "/api/v1/pv-systems",
    responses((status = OK, body = Vec<PvSystemDto>)),
    tag = "pv"
)]
async fn list_systems(State(s): State<WebState>, ctx: ApiCtx) -> ApiResult<Vec<PvSystemDto>> {
    let list = s.uc.list_pv_systems.execute(&ctx.tenant).await?;
    Ok(Json(list.iter().map(PvSystemDto::from).collect()))
}

/// Anlage mit Zuordnung anlegen (Owner).
#[utoipa::path(
    post,
    path = "/api/v1/pv-systems",
    request_body = PvSystemBody,
    responses(
        (status = CREATED, body = PvSystemDto),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
    ),
    tag = "pv"
)]
async fn create_system(
    State(s): State<WebState>,
    ctx: ApiCtx,
    body: Result<Json<PvSystemBody>, JsonRejection>,
) -> Result<(StatusCode, Json<PvSystemDto>), ApiError> {
    let input = super::json(body)?.into_input()?;
    let id = s.uc.create_pv_system.execute(&ctx.tenant, input).await?;
    let sys = s.uc.get_pv_system.execute(&ctx.tenant, id).await?;
    Ok((StatusCode::CREATED, Json(PvSystemDto::from(&sys))))
}

/// Anlage mit Zuordnung und Einstellungen.
#[utoipa::path(
    get,
    path = "/api/v1/pv-systems/{id}",
    params(("id" = Uuid, Path, description = "PV-Anlage")),
    responses((status = OK, body = PvSystemDto), (status = NOT_FOUND, body = Problem)),
    tag = "pv"
)]
async fn get_system(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<PvSystemDto> {
    let sys =
        s.uc.get_pv_system
            .execute(&ctx.tenant, PvSystemId(id))
            .await?;
    Ok(Json(PvSystemDto::from(&sys)))
}

/// Anlagendaten und Zuordnung vollständig ersetzen (Owner).
#[utoipa::path(
    put,
    path = "/api/v1/pv-systems/{id}",
    params(("id" = Uuid, Path, description = "PV-Anlage")),
    request_body = PvSystemBody,
    responses(
        (status = OK, body = PvSystemDto),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
    ),
    tag = "pv"
)]
async fn update_system(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<PvSystemBody>, JsonRejection>,
) -> ApiResult<PvSystemDto> {
    let input = super::json(body)?.into_input()?;
    let id = PvSystemId(id);
    s.uc.update_pv_system
        .execute(&ctx.tenant, id, input)
        .await?;
    let sys = s.uc.get_pv_system.execute(&ctx.tenant, id).await?;
    Ok(Json(PvSystemDto::from(&sys)))
}

/// Anlage mit ihrem Kostenbuch löschen (Owner).
#[utoipa::path(
    delete,
    path = "/api/v1/pv-systems/{id}",
    params(("id" = Uuid, Path, description = "PV-Anlage")),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
    ),
    tag = "pv"
)]
async fn delete_system(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.uc.delete_pv_system
        .execute(&ctx.tenant, PvSystemId(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ertrag gegen Erwartung, Jahresprognose, Vergütung, Ersparnis, Bilanz,
/// Amortisation und Rendite.
#[utoipa::path(
    get,
    path = "/api/v1/pv-systems/{id}/report",
    params(("id" = Uuid, Path, description = "PV-Anlage")),
    responses((status = OK, body = PvReportDto), (status = NOT_FOUND, body = Problem)),
    tag = "pv"
)]
async fn system_report(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<PvReportDto> {
    let r =
        s.uc.pv_system_report
            .execute(&ctx.tenant, PvSystemId(id))
            .await?;
    Ok(Json(PvReportDto::from(&r)))
}

/// Energiekennzahlen eines Hausanschlusses je Monat, Kalender- und
/// Abrechnungsjahr sowie spezifischer Ertrag je Anlage.
#[utoipa::path(
    get,
    path = "/api/v1/pv-grid/{id}/analysis",
    params(("id" = Uuid, Path, description = "Messstelle des Hausanschlusses")),
    responses((status = OK, body = GridAnalysisDto), (status = NOT_FOUND, body = Problem)),
    tag = "pv"
)]
async fn grid_analysis(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<GridAnalysisDto> {
    let v = s.uc.grid_analysis.execute(&ctx.tenant, MeterId(id)).await?;
    Ok(Json(GridAnalysisDto::from(&v)))
}

/// Messstellen der Sammelerfassung: Hausanschluss zuerst, dann Erzeugung
/// und Speicher aller Anlagen am Hausanschluss.
#[utoipa::path(
    get,
    path = "/api/v1/pv-grid/{id}/meters",
    params(("id" = Uuid, Path, description = "Messstelle des Hausanschlusses")),
    responses((status = OK, body = Vec<PvGridMeterDto>), (status = NOT_FOUND, body = Problem)),
    tag = "pv"
)]
async fn grid_meters(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<Vec<PvGridMeterDto>> {
    let meters =
        s.uc.record_bulk_readings
            .meters(&ctx.tenant, MeterId(id))
            .await?;
    let mut out = Vec::with_capacity(meters.len());
    for (agg, role) in meters {
        let d = s.uc.get_meter.execute(&ctx.tenant, agg.meter.id).await?;
        out.push(PvGridMeterDto {
            role: role.as_str().into(),
            meter: MeterDto::new(&d.aggregate, &d.readings),
        });
    }
    Ok(Json(out))
}

/// Sammelerfassung (Member): alle Stände in einer Transaktion; Warnungen
/// ohne `confirm: true` ergeben 422 mit `warnings`, nichts wird gespeichert.
#[utoipa::path(
    post,
    path = "/api/v1/pv-grid/{id}/readings",
    params(("id" = Uuid, Path, description = "Messstelle des Hausanschlusses")),
    request_body = BulkReadingsBody,
    responses(
        (status = CREATED, body = BulkRecordedDto),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
    ),
    tag = "pv"
)]
async fn record_bulk(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<BulkReadingsBody>, JsonRejection>,
) -> Result<(StatusCode, Json<BulkRecordedDto>), ApiError> {
    let b = super::json(body)?;
    let input = BulkInput {
        at: b.read_at,
        values: b
            .values
            .iter()
            .map(|v| (MeterId(v.meter_id), RegisterId(v.register_id), v.value))
            .collect(),
        note: b.note,
        confirm_warnings: b.confirm,
        source: zaehler_domain::reading::ReadingSource::Api,
        drafts: Vec::new(),
    };
    match s
        .uc
        .record_bulk_readings
        .execute(&ctx.tenant, MeterId(id), input)
        .await?
    {
        BulkOutcome::Saved(saved) => Ok((StatusCode::CREATED, Json(BulkRecordedDto { saved }))),
        BulkOutcome::NeedsConfirmation(w) => {
            let checks: Vec<_> = w.into_iter().flat_map(|(_, c)| c).collect();
            Err(Problem::warnings(&checks).into())
        }
    }
}
