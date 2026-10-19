//! REST-API: Fahrzeuge mit Verbrauchsprofilen, Anschaffung, Ladevorgängen
//! und Auswertung sowie das Kostenbuch von Fahrzeugen und PV-Anlagen
//! (SPEC_API_REST, SPEC_CAR_VEHICLES, SPEC_ASSET_COSTS).

use std::str::FromStr;

use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::StatusCode,
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use zaehler_app::{
    AppError, Tenant,
    ids::{AssetCostId, ChargingId, MeterId, ProfileId, PvSystemId},
    model::{AssetCost, AssetRef, ChargingKind, ChargingSession, ProfileRow, VehicleData},
    usecase::{
        asset_cost::AssetCostInput,
        vehicle::{ChargingInput, NewVehicle, SaveOutcome, VehicleDetail, VehicleInput},
        vehicle_report::{VehiclePeriod, VehicleReportView},
    },
};
use zaehler_domain::{
    assets::{AssetCategory, AssetType, Recurrence},
    calendar::Calendar,
    cost::Estimated,
    vehicle::{
        ConsumptionProfile, CostPerKm, DepreciationMode, DriveType, EnergyCost, Holding,
        ProfileEnergy, Remaining,
    },
};

use super::{
    auth::ApiCtx,
    dto::MeterDto,
    error::{ApiError, Problem},
};
use crate::state::WebState;

type ApiResult<T> = Result<Json<T>, ApiError>;
type Created<T> = Result<(StatusCode, Json<T>), ApiError>;

pub(super) fn routes(r: OpenApiRouter<WebState>) -> OpenApiRouter<WebState> {
    r.routes(routes!(list_vehicles, create_vehicle))
        .routes(routes!(get_vehicle, update_vehicle))
        .routes(routes!(add_profile))
        .routes(routes!(delete_profile))
        .routes(routes!(set_acquisition, delete_acquisition))
        .routes(routes!(list_charging, record_charging))
        .routes(routes!(delete_charging))
        .routes(routes!(vehicle_report))
        .routes(routes!(list_vehicle_costs, add_vehicle_cost))
        .routes(routes!(list_pv_costs, add_pv_cost))
        .routes(routes!(
            get_asset_cost,
            update_asset_cost,
            delete_asset_cost
        ))
}

// --- DTOs ------------------------------------------------------------------

/// Stammdaten eines Fahrzeugs.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleDataDto {
    pub plate: String,
    pub model: String,
    /// `combustion`, `electric` oder `plugin_hybrid`
    pub drive_type: String,
    /// Wallbox-Zwischenzähler oder Hausstrom-Messstelle
    pub home_charging_meter_id: Option<Uuid>,
    /// PV-Anteil beim Heimladen in Prozent
    #[schema(value_type = Option<String>)]
    pub pv_share_pct: Option<Decimal>,
    /// Strompreis beim Heimladen in €/kWh, wenn kein Vertragspreis vorliegt
    #[schema(value_type = Option<String>)]
    pub home_charging_price: Option<Decimal>,
}

impl From<&VehicleData> for VehicleDataDto {
    fn from(d: &VehicleData) -> Self {
        Self {
            plate: d.plate.clone(),
            model: d.model.clone(),
            drive_type: d.drive_type.as_str().into(),
            home_charging_meter_id: d.home_charging_meter_id.map(|m| m.0),
            pv_share_pct: d.pv_share_pct,
            home_charging_price: d.home_charging_price,
        }
    }
}

/// Fahrzeug in der Liste; `data` fehlt bei Messstellen ohne Stammdaten.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleSummaryDto {
    /// ID der Messstelle (Kilometerstand)
    pub id: Uuid,
    pub name: String,
    pub archived: bool,
    pub data: Option<VehicleDataDto>,
}

/// Verbrauchsprofil ab einem Tag.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleProfileDto {
    pub id: Uuid,
    pub valid_from: NaiveDate,
    /// `electric` (kWh/100 km) oder `fuel` (l/100 km)
    pub energy: String,
    #[schema(value_type = String)]
    pub kwh_per_100km: Decimal,
    #[schema(value_type = String)]
    pub charging_loss_pct: Decimal,
    #[schema(value_type = Option<String>)]
    pub electric_share_pct: Option<Decimal>,
}

impl From<&ProfileRow> for VehicleProfileDto {
    fn from(r: &ProfileRow) -> Self {
        let p = &r.profile;
        Self {
            id: r.id.0,
            valid_from: p.valid_from,
            energy: p.energy.as_str().into(),
            kwh_per_100km: p.kwh_per_100km,
            charging_loss_pct: p.charging_loss_pct,
            electric_share_pct: p.electric_share_pct,
        }
    }
}

/// Anschaffung und Verkauf; `mode` wählt den Wertverlust nach Zeit oder
/// Laufleistung.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleAcquisitionDto {
    pub purchased_on: NaiveDate,
    #[schema(value_type = String)]
    pub price: Decimal,
    /// `time` oder `mileage`
    pub mode: String,
    /// Erstzulassung; ohne Angabe gilt das Fahrzeug beim Kauf als neu
    pub first_registration: Option<NaiveDate>,
    #[schema(value_type = Option<String>)]
    pub residual_value: Option<Decimal>,
    /// Ende der Haltedauer (ausschließlich)
    pub planned_end: Option<NaiveDate>,
    #[schema(value_type = Option<String>)]
    pub odometer_at_purchase: Option<Decimal>,
    /// Lebenslaufleistung als absoluter Kilometerstand
    #[schema(value_type = Option<String>)]
    pub lifetime_km: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub end_value: Option<Decimal>,
    pub sold_on: Option<NaiveDate>,
    #[schema(value_type = Option<String>)]
    pub sale_price: Option<Decimal>,
}

impl From<&Holding> for VehicleAcquisitionDto {
    fn from(h: &Holding) -> Self {
        Self {
            purchased_on: h.purchased_on,
            price: h.price,
            mode: h.mode.as_str().into(),
            first_registration: h.first_registration,
            residual_value: h.residual_value,
            planned_end: h.planned_end,
            odometer_at_purchase: h.odometer_at_purchase,
            lifetime_km: h.lifetime_km,
            end_value: h.end_value,
            sold_on: h.sold_on,
            sale_price: h.sale_price,
        }
    }
}

/// Externer Ladevorgang.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChargingSessionDto {
    pub id: Uuid,
    pub charged_at: DateTime<Utc>,
    /// kWh
    #[schema(value_type = String)]
    pub energy: Decimal,
    /// Betrag brutto, 0 bei kostenlosem Laden
    #[schema(value_type = String)]
    pub amount: Decimal,
    #[schema(value_type = Option<String>)]
    pub odometer: Option<Decimal>,
    pub provider: Option<String>,
    /// `ac` oder `dc`
    pub kind: String,
    /// `form` oder `receipt`
    pub source: String,
    pub document_id: Option<Uuid>,
}

impl From<&ChargingSession> for ChargingSessionDto {
    fn from(c: &ChargingSession) -> Self {
        Self {
            id: c.id.0,
            charged_at: c.charged_at,
            energy: c.energy,
            amount: c.amount,
            odometer: c.odometer,
            provider: c.provider.clone(),
            kind: c.kind.as_str().into(),
            source: c.source.as_str().into(),
            document_id: c.document_id.map(|d| d.0),
        }
    }
}

/// Fahrzeug mit Messstelle, Stammdaten, Profilen, Anschaffung und
/// Ladevorgängen.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleDetailDto {
    pub meter: MeterDto,
    pub data: Option<VehicleDataDto>,
    /// Aufsteigend nach `valid_from`
    pub profiles: Vec<VehicleProfileDto>,
    pub acquisition: Option<VehicleAcquisitionDto>,
    pub charging: Vec<ChargingSessionDto>,
    /// Kilometerstand bei Beginn der Erfassung
    #[schema(value_type = Option<String>)]
    pub initial_odometer: Option<Decimal>,
}

/// Stammdaten wie im Formular.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VehicleBody {
    #[serde(default)]
    pub plate: String,
    #[serde(default)]
    pub model: String,
    /// `combustion`, `electric` oder `plugin_hybrid`
    pub drive_type: String,
    pub home_charging_meter_id: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub pv_share_pct: Option<Decimal>,
    /// Strompreis beim Heimladen in €/kWh, wenn kein Vertragspreis vorliegt
    #[schema(value_type = Option<String>)]
    pub home_charging_price: Option<Decimal>,
}

/// Neues Fahrzeug mit Messstelle (Einheit km, Register `km`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateVehicleBody {
    /// Name der Messstelle
    pub name: String,
    #[serde(flatten)]
    pub vehicle: VehicleBody,
    /// Kilometerstand bei Beginn der Erfassung
    #[schema(value_type = String)]
    pub odometer: Decimal,
    /// Beginn der Erfassung
    pub start_at: DateTime<Utc>,
}

/// Verbrauchsprofil ab `valid_from`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VehicleProfileBody {
    pub valid_from: NaiveDate,
    /// `electric` (Standard) oder `fuel`
    pub energy: Option<String>,
    /// Verbrauch je 100 km: kWh bei `electric`, l bei `fuel`
    #[schema(value_type = String)]
    pub kwh_per_100km: Decimal,
    /// Ladeverlust in Prozent, Standard 0
    #[schema(value_type = Option<String>)]
    pub charging_loss_pct: Option<Decimal>,
    /// Elektrischer Fahranteil in Prozent (Plug-in-Hybrid)
    #[schema(value_type = Option<String>)]
    pub electric_share_pct: Option<Decimal>,
}

/// Anschaffung setzen. Zeitmodus: Restwert und Halteende optional, nur
/// gemeinsam; Laufleistungsmodus: Kilometerstand bei Kauf Pflicht,
/// Lebenslaufleistung optional (Restlebensdauer). `end_value` wird nicht mehr
/// verwendet.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VehicleAcquisitionBody {
    pub purchased_on: NaiveDate,
    #[schema(value_type = String)]
    pub price: Decimal,
    /// `time` (Standard) oder `mileage`
    pub mode: Option<String>,
    /// Erstzulassung; ohne Angabe gilt das Fahrzeug beim Kauf als neu
    pub first_registration: Option<NaiveDate>,
    #[schema(value_type = Option<String>)]
    pub residual_value: Option<Decimal>,
    pub planned_end: Option<NaiveDate>,
    #[schema(value_type = Option<String>)]
    pub odometer_at_purchase: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub lifetime_km: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub end_value: Option<Decimal>,
    pub sold_on: Option<NaiveDate>,
    #[schema(value_type = Option<String>)]
    pub sale_price: Option<Decimal>,
}

/// Externen Ladevorgang erfassen; Warnungen zum Kilometerstand ohne
/// `confirm: true` ergeben 422 mit `warnings`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ChargingBody {
    pub charged_at: DateTime<Utc>,
    /// kWh
    #[schema(value_type = String)]
    pub energy: Decimal,
    #[schema(value_type = String)]
    pub amount: Decimal,
    #[schema(value_type = Option<String>)]
    pub odometer: Option<Decimal>,
    pub provider: Option<String>,
    /// `ac` (Standard) oder `dc`
    pub kind: Option<String>,
    /// Kilometerstand zusätzlich als Ablesung speichern
    #[serde(default)]
    pub odometer_as_reading: bool,
    #[serde(default)]
    pub confirm: bool,
}

/// Wert mit Kennzeichnung einer Schätzung.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleEstimateDto {
    #[schema(value_type = String)]
    pub value: Decimal,
    pub estimated: bool,
}

impl From<Estimated<Decimal>> for VehicleEstimateDto {
    fn from(e: Estimated<Decimal>) -> Self {
        Self {
            value: e.value,
            estimated: e.estimated,
        }
    }
}

/// Betrag einer Kostenkategorie.
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetCostCategoryDto {
    pub category: String,
    #[schema(value_type = String)]
    pub amount: Decimal,
}

/// Laden extern und zu Hause.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleEnergyDto {
    #[schema(value_type = String)]
    pub external_kwh: Decimal,
    #[schema(value_type = String)]
    pub external_cost: Decimal,
    pub home_kwh: Option<VehicleEstimateDto>,
    pub home_cost: Option<VehicleEstimateDto>,
    /// Gemessen nur mit Heimlade-Zwischenzähler, sonst hinterlegt
    pub kwh_per_100km: Option<VehicleEstimateDto>,
    /// Ladekosten gesamt
    #[schema(value_type = String)]
    pub total: Decimal,
}

impl From<&EnergyCost> for VehicleEnergyDto {
    fn from(e: &EnergyCost) -> Self {
        Self {
            external_kwh: e.external.kwh,
            external_cost: e.external.cost,
            home_kwh: e.home_kwh.map(Into::into),
            home_cost: e.home_cost.map(Into::into),
            kwh_per_100km: e.kwh_per_100km.map(Into::into),
            total: e.total(),
        }
    }
}

/// Kosten pro km eines Zeitraums.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleCostPerKmDto {
    #[schema(value_type = String)]
    pub km: Decimal,
    #[schema(value_type = String)]
    pub variable: Decimal,
    #[schema(value_type = String)]
    pub fixed: Decimal,
    #[schema(value_type = String)]
    pub depreciation: Decimal,
    #[schema(value_type = Option<String>)]
    pub variable_per_km: Option<Decimal>,
    /// Vollkosten (variabel + fix + Wertverlust) pro km
    #[schema(value_type = Option<String>)]
    pub total_per_km: Option<Decimal>,
}

impl From<&CostPerKm> for VehicleCostPerKmDto {
    fn from(c: &CostPerKm) -> Self {
        Self {
            km: c.km,
            variable: c.variable,
            fixed: c.fixed,
            depreciation: c.depreciation,
            variable_per_km: c.variable_per_km,
            total_per_km: c.total_per_km,
        }
    }
}

/// Auswertung eines Kalenderjahres oder der Haltedauer.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehiclePeriodDto {
    /// Kalenderjahr; fehlt bei der Haltedauer
    pub year: Option<i32>,
    pub period_start: NaiveDate,
    /// Ende (ausschließlich)
    pub period_end: NaiveDate,
    /// Gefahrene km
    pub km: Option<VehicleEstimateDto>,
    /// Kosten je Kategorie; `charging` enthält die Ladekosten
    pub categories: Vec<AssetCostCategoryDto>,
    pub energy: Option<VehicleEnergyDto>,
    /// Durchschnitt l/100 km der Vollbetankungsabschnitte
    #[schema(value_type = Option<String>)]
    pub fuel_per_100km: Option<Decimal>,
    pub cost: VehicleCostPerKmDto,
}

impl From<&VehiclePeriod> for VehiclePeriodDto {
    fn from(p: &VehiclePeriod) -> Self {
        Self {
            year: p.year,
            period_start: p.period.start,
            period_end: p.period.end,
            km: p.km.map(Into::into),
            categories: p
                .categories
                .iter()
                .map(|(c, v)| AssetCostCategoryDto {
                    category: c.as_str().into(),
                    amount: *v,
                })
                .collect(),
            energy: p.energy.as_ref().map(Into::into),
            fuel_per_100km: p.fuel_per_100km,
            cost: (&p.cost).into(),
        }
    }
}

/// Prognose für das laufende Jahr.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleForecastDto {
    pub year: i32,
    pub km: VehicleEstimateDto,
    #[schema(value_type = String)]
    pub fixed: Decimal,
    #[schema(value_type = Option<String>)]
    pub variable_per_km: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub expected: Option<Decimal>,
    pub fuel_cost: Option<VehicleEstimateDto>,
}

/// Abschnitt zwischen zwei Vollbetankungen.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleFuelSegmentDto {
    pub until: NaiveDate,
    #[schema(value_type = String)]
    pub km: Decimal,
    #[schema(value_type = String)]
    pub litres: Decimal,
    #[schema(value_type = String)]
    pub per_100km: Decimal,
}

/// Heimlade-Messstelle der Auswertung.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleHomeMeterDto {
    pub id: Uuid,
    pub name: String,
    /// Wallbox-Zwischenzähler (gemessen) statt Hausstrom (Schätzung)
    pub wallbox: bool,
    /// Name der Messstelle des Ladepreises
    pub price_meter: Option<String>,
}

/// Restwert und Restlebensdauer.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleResidualDto {
    /// `time` oder `mileage`
    pub mode: String,
    #[schema(value_type = String)]
    pub value: Decimal,
    /// `until`, `reached`, `sold` oder `insufficient_data`
    pub remaining: String,
    /// Bei `until`: Restjahre und voraussichtliches Ende
    #[schema(value_type = Option<String>)]
    pub remaining_years: Option<Decimal>,
    pub remaining_until: Option<NaiveDate>,
    #[schema(value_type = Option<String>)]
    pub annual_km: Option<Decimal>,
}

/// Preis pro Liter an einem Tag.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleFuelPriceDto {
    pub date: NaiveDate,
    #[schema(value_type = String)]
    pub price: Decimal,
}

/// Preis pro Liter: letzter bekannter, Schätzung und Verlauf.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleFuelPricesDto {
    pub last: Option<VehicleFuelPriceDto>,
    #[schema(value_type = Option<String>)]
    pub estimate: Option<Decimal>,
    pub points: Vec<VehicleFuelPriceDto>,
}

/// Auswertung eines Fahrzeugs je Kalenderjahr und Haltedauer.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleReportDto {
    pub today: NaiveDate,
    /// Kalenderjahre, neueste zuerst
    pub years: Vec<VehiclePeriodDto>,
    pub holding: Option<VehiclePeriodDto>,
    pub forecast: Option<VehicleForecastDto>,
    pub fuel_segments: Vec<VehicleFuelSegmentDto>,
    pub home: Option<VehicleHomeMeterDto>,
    pub residual: Option<VehicleResidualDto>,
    /// Nur für Fahrzeuge, die tanken
    pub fuel_price: Option<VehicleFuelPricesDto>,
}

fn price(p: (NaiveDate, Decimal)) -> VehicleFuelPriceDto {
    VehicleFuelPriceDto {
        date: p.0,
        price: p.1,
    }
}

impl From<&VehicleReportView> for VehicleReportDto {
    fn from(r: &VehicleReportView) -> Self {
        Self {
            today: r.today,
            years: r.years.iter().map(Into::into).collect(),
            holding: r.holding.as_ref().map(Into::into),
            forecast: r.forecast.as_ref().map(|f| VehicleForecastDto {
                year: f.year,
                km: f.km.into(),
                fixed: f.fixed,
                variable_per_km: f.variable_per_km,
                expected: f.expected,
                fuel_cost: f.fuel_cost.map(Into::into),
            }),
            fuel_segments: r
                .fuel_segments
                .iter()
                .map(|s| VehicleFuelSegmentDto {
                    until: s.until,
                    km: s.km,
                    litres: s.litres,
                    per_100km: s.per_100km,
                })
                .collect(),
            home: r.home.as_ref().map(|h| VehicleHomeMeterDto {
                id: h.id.0,
                name: h.name.clone(),
                wallbox: h.wallbox,
                price_meter: h.price_meter.clone(),
            }),
            residual: r.residual.map(|x| {
                let (remaining, years, until) = match x.remaining {
                    Remaining::Until { years, until } => ("until", Some(years), Some(until)),
                    Remaining::Reached => ("reached", None, None),
                    Remaining::Sold => ("sold", None, None),
                    Remaining::InsufficientData => ("insufficient_data", None, None),
                };
                VehicleResidualDto {
                    mode: x.mode.as_str().into(),
                    value: x.value,
                    remaining: remaining.into(),
                    remaining_years: years,
                    remaining_until: until,
                    annual_km: x.annual_km,
                }
            }),
            fuel_price: r.fuel_price.as_ref().map(|f| VehicleFuelPricesDto {
                last: f.last.map(price),
                estimate: f.estimate,
                points: f.points.iter().copied().map(price).collect(),
            }),
        }
    }
}

/// Wiederholung alle `every_months` Monate, optional bis `until`
/// (einschließlich).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetCostRecurrenceDto {
    pub every_months: u8,
    pub until: Option<NaiveDate>,
}

/// Eintrag im Kostenbuch.
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetCostDto {
    pub id: Uuid,
    /// `vehicle` oder `pv_system`
    pub asset_type: String,
    /// Fahrzeug: ID der Messstelle, sonst der PV-Anlage
    pub asset_id: Uuid,
    pub booked_on: NaiveDate,
    /// Kategorie des Anlagentyps, z.B. `fuel` oder `maintenance`
    pub category: String,
    #[schema(value_type = String)]
    pub amount: Decimal,
    /// Liter bei Kraftstoff
    #[schema(value_type = Option<String>)]
    pub quantity: Option<Decimal>,
    /// Preis pro Liter bei Kraftstoff
    #[schema(value_type = Option<String>)]
    pub unit_price: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub odometer: Option<Decimal>,
    /// Nur bei Kraftstoff
    pub full_tank: Option<bool>,
    pub recurrence: Option<AssetCostRecurrenceDto>,
    pub document_id: Option<Uuid>,
    pub note: Option<String>,
}

impl From<&AssetCost> for AssetCostDto {
    fn from(c: &AssetCost) -> Self {
        Self {
            id: c.id.0,
            asset_type: c.asset.asset_type.as_str().into(),
            asset_id: c.asset.id,
            booked_on: c.booked_on,
            category: c.category.as_str().into(),
            amount: c.amount,
            quantity: c.quantity,
            unit_price: c.unit_price,
            odometer: c.odometer,
            full_tank: c.full_tank,
            recurrence: c.recurrence.map(|r| AssetCostRecurrenceDto {
                every_months: r.every_months,
                until: r.until,
            }),
            document_id: c.document_id.map(|d| d.0),
            note: c.note.clone(),
        }
    }
}

/// Eintrag mit seinem Betrag im Jahr.
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetCostBookEntryDto {
    #[serde(flatten)]
    pub entry: AssetCostDto,
    /// Summe der Buchungen im Jahr
    #[schema(value_type = String)]
    pub year_amount: Decimal,
}

/// Kostenbuch einer Anlage für ein Kalenderjahr.
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetCostBookDto {
    pub year: i32,
    /// Einträge mit Buchungen im Jahr
    pub entries: Vec<AssetCostBookEntryDto>,
    /// Summen je Kategorie
    pub totals: Vec<AssetCostCategoryDto>,
    #[schema(value_type = String)]
    pub total: Decimal,
}

/// Eintrag anlegen oder ändern. Bei Kraftstoff genügen zwei von `amount`,
/// `quantity` und `unit_price`, sonst ist `amount` Pflicht.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AssetCostBody {
    pub booked_on: NaiveDate,
    pub category: String,
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub quantity: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub unit_price: Option<Decimal>,
    /// Nur bei Fahrzeugen
    #[schema(value_type = Option<String>)]
    pub odometer: Option<Decimal>,
    #[serde(default)]
    pub full_tank: bool,
    pub recurrence: Option<AssetCostRecurrenceDto>,
    pub note: Option<String>,
    /// Kilometerstand zusätzlich als Ablesung speichern (nur beim Anlegen)
    #[serde(default)]
    pub odometer_as_reading: bool,
    /// Plausibilitätswarnungen zum Kilometerstand bestätigen
    #[serde(default)]
    pub confirm: bool,
}

/// Kalenderjahr des Kostenbuchs.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AssetCostYearQuery {
    /// Standard: laufendes Jahr in der Zeitzone des Haushalts
    pub year: Option<i32>,
}

// --- Umwandlung ------------------------------------------------------------

/// Aufzählungswert als String; unbekannte Werte ergeben 422.
fn parse_kind<T: FromStr>(field: &'static str, v: &str) -> Result<T, AppError> {
    v.parse()
        .map_err(|_| AppError::field(field, "form.kind_invalid"))
}

/// Optionaler Aufzählungswert mit Standard.
fn parse_kind_or<T: FromStr>(
    field: &'static str,
    v: Option<&str>,
    default: T,
) -> Result<T, AppError> {
    v.map_or(Ok(default), |v| parse_kind(field, v))
}

fn vehicle_input(b: VehicleBody) -> Result<VehicleInput, AppError> {
    let drive_type = parse_kind::<DriveType>("drive_type", &b.drive_type)?;
    Ok(VehicleInput {
        plate: b.plate,
        model: b.model,
        drive_type,
        home_charging_meter_id: b.home_charging_meter_id.map(MeterId),
        pv_share_pct: b.pv_share_pct,
        home_charging_price: b.home_charging_price,
    })
}

fn cost_input(asset: AssetType, b: AssetCostBody) -> Result<AssetCostInput, AppError> {
    let category = AssetCategory::parse(asset, &b.category)
        .ok_or_else(|| AppError::field("category", "form.kind_invalid"))?;
    Ok(AssetCostInput {
        booked_on: b.booked_on,
        category,
        amount: b.amount,
        quantity: b.quantity,
        unit_price: b.unit_price,
        odometer: b.odometer,
        full_tank: b.full_tank,
        recurrence: b.recurrence.map(|r| Recurrence {
            every_months: r.every_months,
            until: r.until,
        }),
        note: b.note,
        odometer_as_reading: b.odometer_as_reading,
        confirm_warnings: b.confirm,
    })
}

async fn detail(s: &WebState, t: &Tenant, id: MeterId) -> Result<VehicleDetailDto, AppError> {
    let VehicleDetail {
        vehicle,
        charging,
        initial_odometer,
        ..
    } = s.uc.get_vehicle.execute(t, id).await?;
    let m = s.uc.get_meter.execute(t, id).await?;
    Ok(VehicleDetailDto {
        meter: MeterDto::new(&m.aggregate, &m.readings),
        data: vehicle.as_ref().map(|v| (&v.data).into()),
        profiles: vehicle
            .as_ref()
            .map(|v| v.profiles.iter().map(Into::into).collect())
            .unwrap_or_default(),
        acquisition: vehicle
            .as_ref()
            .and_then(|v| v.holding.as_ref())
            .map(Into::into),
        charging: charging.iter().map(Into::into).collect(),
        initial_odometer,
    })
}

async fn this_year(s: &WebState, t: &Tenant) -> Result<i32, AppError> {
    let h = s.uc.get_household.execute(t).await?;
    Ok(Calendar::new(h.data.timezone).local_date(Utc::now()).year())
}

async fn cost_book(
    s: &WebState,
    t: &Tenant,
    asset: AssetRef,
    query: Result<Query<AssetCostYearQuery>, QueryRejection>,
) -> Result<AssetCostBookDto, ApiError> {
    let Query(q) = query.map_err(|_| Problem::bad_request("query.invalid"))?;
    let year = match q.year {
        Some(y) => y,
        None => this_year(s, t).await?,
    };
    let book = s.uc.list_asset_costs.execute(t, asset, year).await?;
    Ok(AssetCostBookDto {
        year: book.year,
        entries: book
            .entries
            .iter()
            .map(|(c, v)| AssetCostBookEntryDto {
                entry: c.into(),
                year_amount: *v,
            })
            .collect(),
        totals: book
            .totals
            .iter()
            .map(|(c, v)| AssetCostCategoryDto {
                category: c.as_str().into(),
                amount: *v,
            })
            .collect(),
        total: book.total,
    })
}

async fn add_cost(
    s: &WebState,
    t: &Tenant,
    asset: AssetRef,
    body: Result<Json<AssetCostBody>, JsonRejection>,
) -> Created<AssetCostDto> {
    let input = cost_input(asset.asset_type, super::json(body)?)?;
    match s.uc.add_asset_cost.execute(t, asset, input).await? {
        SaveOutcome::Saved(id) => {
            let c = s.uc.get_asset_cost.execute(t, id).await?;
            Ok((StatusCode::CREATED, Json((&c).into())))
        }
        SaveOutcome::NeedsConfirmation(checks) => Err(Problem::warnings(&checks).into()),
    }
}

// --- Fahrzeuge -------------------------------------------------------------

/// Fahrzeuge des Haushalts.
#[utoipa::path(
    get,
    path = "/api/v1/vehicles",
    tag = "vehicles",
    responses(
        (status = OK, body = Vec<VehicleSummaryDto>),
        (status = UNAUTHORIZED, body = Problem),
    )
)]
async fn list_vehicles(
    State(s): State<WebState>,
    ctx: ApiCtx,
) -> ApiResult<Vec<VehicleSummaryDto>> {
    let list = s.uc.list_vehicles.execute(&ctx.tenant).await?;
    Ok(Json(
        list.iter()
            .map(|v| VehicleSummaryDto {
                id: v.meter.id.0,
                name: v.meter.data.name.clone(),
                archived: v.meter.archived,
                data: v.vehicle.as_ref().map(|x| (&x.data).into()),
            })
            .collect(),
    ))
}

/// Fahrzeug mit Messstelle (Einheit km, Register `km`, Startstand) anlegen
/// (Owner).
#[utoipa::path(
    post,
    path = "/api/v1/vehicles",
    tag = "vehicles",
    request_body = CreateVehicleBody,
    responses(
        (status = CREATED, body = VehicleDetailDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn create_vehicle(
    State(s): State<WebState>,
    ctx: ApiCtx,
    body: Result<Json<CreateVehicleBody>, JsonRejection>,
) -> Created<VehicleDetailDto> {
    let b = super::json(body)?;
    let vehicle = vehicle_input(b.vehicle)?;
    let id =
        s.uc.create_vehicle
            .execute(
                &ctx.tenant,
                NewVehicle {
                    name: b.name,
                    vehicle,
                    odometer: b.odometer,
                    start_at: b.start_at,
                },
            )
            .await?;
    Ok((
        StatusCode::CREATED,
        Json(detail(&s, &ctx.tenant, id).await?),
    ))
}

/// Fahrzeug mit Stammdaten, Verbrauchsprofilen, Anschaffung und
/// Ladevorgängen.
#[utoipa::path(
    get,
    path = "/api/v1/vehicles/{id}",
    tag = "vehicles",
    params(("id" = Uuid, Path, description = "Messstelle des Fahrzeugs")),
    responses(
        (status = OK, body = VehicleDetailDto),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn get_vehicle(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<VehicleDetailDto> {
    Ok(Json(detail(&s, &ctx.tenant, MeterId(id)).await?))
}

/// Stammdaten ändern oder für eine Fahrzeug-Messstelle anlegen (Owner); die
/// Antriebsart ist nach dem ersten Kostenbuch-Eintrag oder Ladevorgang
/// gesperrt (409 `vehicle.drive_type_locked`).
#[utoipa::path(
    put,
    path = "/api/v1/vehicles/{id}",
    tag = "vehicles",
    params(("id" = Uuid, Path, description = "Messstelle des Fahrzeugs")),
    request_body = VehicleBody,
    responses(
        (status = OK, body = VehicleDetailDto),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn update_vehicle(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<VehicleBody>, JsonRejection>,
) -> ApiResult<VehicleDetailDto> {
    let input = vehicle_input(super::json(body)?)?;
    s.uc.update_vehicle
        .execute(&ctx.tenant, MeterId(id), input)
        .await?;
    Ok(Json(detail(&s, &ctx.tenant, MeterId(id)).await?))
}

/// Verbrauchsprofil ab einem Tag anlegen (Owner); je Tag und Energieart
/// eines (422 `vehicle.profile_exists`).
#[utoipa::path(
    post,
    path = "/api/v1/vehicles/{id}/profiles",
    tag = "vehicles",
    params(("id" = Uuid, Path, description = "Messstelle des Fahrzeugs")),
    request_body = VehicleProfileBody,
    responses(
        (status = CREATED, body = VehicleProfileDto),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn add_profile(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<VehicleProfileBody>, JsonRejection>,
) -> Created<VehicleProfileDto> {
    let b = super::json(body)?;
    let energy = parse_kind_or("energy", b.energy.as_deref(), ProfileEnergy::Electric)?;
    let profile = ConsumptionProfile {
        valid_from: b.valid_from,
        kwh_per_100km: b.kwh_per_100km,
        charging_loss_pct: b.charging_loss_pct.unwrap_or(Decimal::ZERO),
        electric_share_pct: b.electric_share_pct,
        energy,
    };
    let pid =
        s.uc.set_consumption_profile
            .execute(&ctx.tenant, MeterId(id), profile)
            .await?;
    let v = s.uc.get_vehicle.execute(&ctx.tenant, MeterId(id)).await?;
    let row = v
        .vehicle
        .iter()
        .flat_map(|v| v.profiles.iter())
        .find(|p| p.id == pid)
        .ok_or(AppError::NotFound)?;
    Ok((StatusCode::CREATED, Json(row.into())))
}

/// Verbrauchsprofil löschen (Owner).
#[utoipa::path(
    delete,
    path = "/api/v1/vehicles/{id}/profiles/{profile}",
    tag = "vehicles",
    params(
        ("id" = Uuid, Path, description = "Messstelle des Fahrzeugs"),
        ("profile" = Uuid, Path, description = "Verbrauchsprofil"),
    ),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn delete_profile(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path((id, profile)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.uc.delete_consumption_profile
        .execute(&ctx.tenant, MeterId(id), ProfileId(profile))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Anschaffung und Verkauf setzen (Owner).
#[utoipa::path(
    put,
    path = "/api/v1/vehicles/{id}/acquisition",
    tag = "vehicles",
    params(("id" = Uuid, Path, description = "Messstelle des Fahrzeugs")),
    request_body = VehicleAcquisitionBody,
    responses(
        (status = OK, body = VehicleAcquisitionDto),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn set_acquisition(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<VehicleAcquisitionBody>, JsonRejection>,
) -> ApiResult<VehicleAcquisitionDto> {
    let b = super::json(body)?;
    let mode = parse_kind_or("mode", b.mode.as_deref(), DepreciationMode::Time)?;
    let holding = Holding {
        purchased_on: b.purchased_on,
        price: b.price,
        mode,
        first_registration: b.first_registration,
        residual_value: b.residual_value,
        planned_end: b.planned_end,
        odometer_at_purchase: b.odometer_at_purchase,
        lifetime_km: b.lifetime_km,
        end_value: b.end_value,
        sold_on: b.sold_on,
        sale_price: b.sale_price,
    };
    s.uc.set_acquisition
        .execute(&ctx.tenant, MeterId(id), Some(holding.clone()))
        .await?;
    Ok(Json((&holding).into()))
}

/// Anschaffung entfernen (Owner).
#[utoipa::path(
    delete,
    path = "/api/v1/vehicles/{id}/acquisition",
    tag = "vehicles",
    params(("id" = Uuid, Path, description = "Messstelle des Fahrzeugs")),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn delete_acquisition(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.uc.set_acquisition
        .execute(&ctx.tenant, MeterId(id), None)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Externe Ladevorgänge des Fahrzeugs.
#[utoipa::path(
    get,
    path = "/api/v1/vehicles/{id}/charging",
    tag = "vehicles",
    params(("id" = Uuid, Path, description = "Messstelle des Fahrzeugs")),
    responses(
        (status = OK, body = Vec<ChargingSessionDto>),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn list_charging(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<Vec<ChargingSessionDto>> {
    let d = s.uc.get_vehicle.execute(&ctx.tenant, MeterId(id)).await?;
    Ok(Json(d.charging.iter().map(Into::into).collect()))
}

/// Externen Ladevorgang erfassen (Member); nicht für Verbrenner.
#[utoipa::path(
    post,
    path = "/api/v1/vehicles/{id}/charging",
    tag = "vehicles",
    params(("id" = Uuid, Path, description = "Messstelle des Fahrzeugs")),
    request_body = ChargingBody,
    responses(
        (status = CREATED, body = ChargingSessionDto),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn record_charging(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<ChargingBody>, JsonRejection>,
) -> Created<ChargingSessionDto> {
    let b = super::json(body)?;
    let kind = parse_kind_or("kind", b.kind.as_deref(), ChargingKind::Ac)?;
    let input = ChargingInput {
        charged_at: b.charged_at,
        energy: b.energy,
        amount: b.amount,
        odometer: b.odometer,
        provider: b.provider,
        kind,
        odometer_as_reading: b.odometer_as_reading,
        confirm_warnings: b.confirm,
    };
    match s
        .uc
        .record_charging
        .execute(&ctx.tenant, MeterId(id), input)
        .await?
    {
        SaveOutcome::Saved(cid) => {
            let d = s.uc.get_vehicle.execute(&ctx.tenant, MeterId(id)).await?;
            let c = d
                .charging
                .iter()
                .find(|c| c.id == cid)
                .ok_or(AppError::NotFound)?;
            Ok((StatusCode::CREATED, Json(c.into())))
        }
        SaveOutcome::NeedsConfirmation(checks) => Err(Problem::warnings(&checks).into()),
    }
}

/// Ladevorgang löschen (Member).
#[utoipa::path(
    delete,
    path = "/api/v1/vehicles/{id}/charging/{session}",
    tag = "vehicles",
    params(
        ("id" = Uuid, Path, description = "Messstelle des Fahrzeugs"),
        ("session" = Uuid, Path, description = "Ladevorgang"),
    ),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn delete_charging(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path((id, session)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.uc.delete_charging
        .execute(&ctx.tenant, MeterId(id), ChargingId(session))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Auswertung: Kosten je Kategorie, Kosten pro km, Verbrauch, Laden,
/// Prognose und Restwert.
#[utoipa::path(
    get,
    path = "/api/v1/vehicles/{id}/report",
    tag = "vehicles",
    params(("id" = Uuid, Path, description = "Messstelle des Fahrzeugs")),
    responses(
        (status = OK, body = VehicleReportDto),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn vehicle_report(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<VehicleReportDto> {
    let r =
        s.uc.vehicle_report
            .execute(&ctx.tenant, MeterId(id))
            .await?;
    Ok(Json((&r).into()))
}

// --- Kostenbuch ------------------------------------------------------------

/// Kostenbuch des Fahrzeugs für ein Kalenderjahr.
#[utoipa::path(
    get,
    path = "/api/v1/vehicles/{id}/costs",
    tag = "asset-costs",
    params(("id" = Uuid, Path, description = "Messstelle des Fahrzeugs"), AssetCostYearQuery),
    responses(
        (status = OK, body = AssetCostBookDto),
        (status = BAD_REQUEST, body = Problem),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn list_vehicle_costs(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    query: Result<Query<AssetCostYearQuery>, QueryRejection>,
) -> ApiResult<AssetCostBookDto> {
    let asset = AssetRef::vehicle(MeterId(id));
    Ok(Json(cost_book(&s, &ctx.tenant, asset, query).await?))
}

/// Eintrag im Kostenbuch des Fahrzeugs (Member); Warnungen zum
/// übernommenen Kilometerstand ohne `confirm: true` ergeben 422.
#[utoipa::path(
    post,
    path = "/api/v1/vehicles/{id}/costs",
    tag = "asset-costs",
    params(("id" = Uuid, Path, description = "Messstelle des Fahrzeugs")),
    request_body = AssetCostBody,
    responses(
        (status = CREATED, body = AssetCostDto),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn add_vehicle_cost(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<AssetCostBody>, JsonRejection>,
) -> Created<AssetCostDto> {
    add_cost(&s, &ctx.tenant, AssetRef::vehicle(MeterId(id)), body).await
}

/// Kostenbuch der PV-Anlage für ein Kalenderjahr.
#[utoipa::path(
    get,
    path = "/api/v1/pv-systems/{id}/costs",
    tag = "asset-costs",
    params(("id" = Uuid, Path, description = "PV-Anlage"), AssetCostYearQuery),
    responses(
        (status = OK, body = AssetCostBookDto),
        (status = BAD_REQUEST, body = Problem),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn list_pv_costs(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    query: Result<Query<AssetCostYearQuery>, QueryRejection>,
) -> ApiResult<AssetCostBookDto> {
    let asset = AssetRef::pv(PvSystemId(id));
    Ok(Json(cost_book(&s, &ctx.tenant, asset, query).await?))
}

/// Eintrag im Kostenbuch der PV-Anlage (Member).
#[utoipa::path(
    post,
    path = "/api/v1/pv-systems/{id}/costs",
    tag = "asset-costs",
    params(("id" = Uuid, Path, description = "PV-Anlage")),
    request_body = AssetCostBody,
    responses(
        (status = CREATED, body = AssetCostDto),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn add_pv_cost(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<AssetCostBody>, JsonRejection>,
) -> Created<AssetCostDto> {
    add_cost(&s, &ctx.tenant, AssetRef::pv(PvSystemId(id)), body).await
}

/// Eintrag im Kostenbuch.
#[utoipa::path(
    get,
    path = "/api/v1/asset-costs/{id}",
    tag = "asset-costs",
    params(("id" = Uuid, Path, description = "Eintrag")),
    responses(
        (status = OK, body = AssetCostDto),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn get_asset_cost(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<AssetCostDto> {
    let c =
        s.uc.get_asset_cost
            .execute(&ctx.tenant, AssetCostId(id))
            .await?;
    Ok(Json((&c).into()))
}

/// Eintrag ändern (Member); Anlage und Beleg bleiben, `odometer_as_reading`
/// wird ignoriert.
#[utoipa::path(
    put,
    path = "/api/v1/asset-costs/{id}",
    tag = "asset-costs",
    params(("id" = Uuid, Path, description = "Eintrag")),
    request_body = AssetCostBody,
    responses(
        (status = OK, body = AssetCostDto),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    )
)]
async fn update_asset_cost(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<AssetCostBody>, JsonRejection>,
) -> ApiResult<AssetCostDto> {
    let b = super::json(body)?;
    let id = AssetCostId(id);
    let current = s.uc.get_asset_cost.execute(&ctx.tenant, id).await?;
    let input = cost_input(current.asset.asset_type, b)?;
    s.uc.update_asset_cost
        .execute(&ctx.tenant, id, input)
        .await?;
    let c = s.uc.get_asset_cost.execute(&ctx.tenant, id).await?;
    Ok(Json((&c).into()))
}

/// Eintrag löschen (Member).
#[utoipa::path(
    delete,
    path = "/api/v1/asset-costs/{id}",
    tag = "asset-costs",
    params(("id" = Uuid, Path, description = "Eintrag")),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
    )
)]
async fn delete_asset_cost(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.uc.delete_asset_cost
        .execute(&ctx.tenant, AssetCostId(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
