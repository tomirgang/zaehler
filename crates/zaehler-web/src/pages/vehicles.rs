//! Fahrzeuge: Liste, Anlegen und Bearbeiten, Auswertung mit Laden,
//! Verbrauchsprofilen und Anschaffung (SPEC_CAR_VEHICLES, SPEC_CAR_REPORT,
//! SPEC_CAR_ENERGY_COST).

use std::collections::HashMap;

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};
use chrono::{NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{ChargingId, MeterId, ProfileId},
    model::{ChargingKind, MeterSummary, Vehicle},
    usecase::{
        vehicle::{ChargingInput, NewVehicle, SaveOutcome, VehicleDetail, VehicleInput},
        vehicle_report::{VehiclePeriod, VehicleReportView},
    },
};
use zaehler_domain::{
    assets::VehicleCostCategory,
    calendar::Calendar,
    cost::Estimated,
    meter::{MeterKind, MeterPurpose},
    vehicle::{ConsumptionProfile, DepreciationMode, DriveType, Holding, ProfileEnergy, Remaining},
};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format, parse},
    pages::{
        FormData,
        contracts::{date, date_input, decimal, money, plain, required_date},
        field_errors,
        readings::hint,
        render, render_status, see_other,
    },
    state::WebState,
};

pub(crate) fn parse_meter(id: &str) -> Result<MeterId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

fn drive_options(lang: Lang) -> Vec<(String, String)> {
    DriveType::ALL
        .iter()
        .map(|d| {
            (
                d.as_str().to_owned(),
                lang.t(&format!("drive-{}", d.as_str())),
            )
        })
        .collect()
}

pub(crate) fn kind_options() -> Vec<(String, String)> {
    ChargingKind::ALL
        .iter()
        .map(|k| (k.as_str().to_owned(), k.as_str().to_uppercase()))
        .collect()
}

/// Zahl mit Kennzeichnung einer Schätzung.
fn estimated(lang: Lang, v: Option<Estimated<Decimal>>, places: u8, unit: &str) -> String {
    v.map_or_else(String::new, |e| {
        let s = format!("{} {unit}", format::decimal(lang, e.value, places));
        if e.estimated { format!("≈ {s}") } else { s }
    })
}

fn opt_money(lang: Lang, v: Option<Decimal>) -> String {
    v.map(|v| money(lang, v)).unwrap_or_default()
}

/// Erforderliche Zahl.
pub(crate) fn required_decimal(e: &mut FieldErrors, field: &str, value: &str) -> Option<Decimal> {
    if value.trim().is_empty() {
        e.add(field, "form.number_required");
        return None;
    }
    decimal(e, field, value)
}

// --- Liste -----------------------------------------------------------------

pub struct VehicleRow {
    pub id: String,
    pub name: String,
    pub plate: String,
    pub drive: String,
    pub archived: bool,
}

#[derive(Template)]
#[template(path = "pages/vehicles/list.html")]
struct ListPage {
    ctx: PageCtx,
    vehicles: Vec<VehicleRow>,
}

// @impl SPEC_CAR_VEHICLES
pub async fn list(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    let lang = ctx.page.lang;
    let vehicles = state
        .uc
        .list_vehicles
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?
        .into_iter()
        .map(|v| VehicleRow {
            id: v.meter.id.to_string(),
            name: v.meter.data.name.clone(),
            plate: v
                .vehicle
                .as_ref()
                .map(|x| x.data.plate.clone())
                .unwrap_or_default(),
            drive: v.vehicle.as_ref().map_or_else(
                || lang.t("vehicle-data-missing"),
                |x| lang.t(&format!("drive-{}", x.data.drive_type.as_str())),
            ),
            archived: v.meter.archived_at.is_some(),
        })
        .collect();
    Ok(render(&ListPage {
        ctx: ctx.page.clone(),
        vehicles,
    }))
}

// --- Formular --------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct VehicleForm {
    pub name: String,
    pub plate: String,
    pub model: String,
    pub drive_type: String,
    pub home: String,
    pub pv_share: String,
    pub odometer: String,
    pub start: String,
}

#[derive(Template)]
#[template(path = "pages/vehicles/form.html")]
struct FormPage {
    ctx: PageCtx,
    /// `None` beim Anlegen
    meter_id: Option<String>,
    drives: Vec<(String, String)>,
    /// (ID, Bezeichnung) möglicher Heimlade-Messstellen
    home_meters: Vec<(String, String)>,
    form: VehicleForm,
    errors: FieldErrors,
    conflict: Option<String>,
}

impl FormPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn has_errs(&self, field: &str) -> bool {
        !self.errors.for_field(field).is_empty()
    }
}

/// Strom-Messstellen als Heimlade-Messstelle: Wallbox gemessen, Hausstrom
/// geschätzt.
fn home_options(lang: Lang, meters: &[MeterSummary]) -> Vec<(String, String)> {
    meters
        .iter()
        .filter(|m| m.meter.data.kind == MeterKind::Electricity && m.meter.archived_at.is_none())
        .filter_map(|m| {
            let key = match m.meter.data.purpose {
                MeterPurpose::Wallbox => "vehicle-home-wallbox",
                MeterPurpose::General => "vehicle-home-house",
                MeterPurpose::HeatPump => return None,
            };
            Some((
                m.meter.id.to_string(),
                format!("{} ({})", m.meter.data.name, lang.t(key)),
            ))
        })
        .collect()
}

async fn form_page(
    state: &WebState,
    ctx: &Ctx,
    meter_id: Option<MeterId>,
    form: VehicleForm,
    errors: FieldErrors,
    conflict: Option<&'static str>,
) -> Result<FormPage, Response> {
    let lang = ctx.page.lang;
    let meters = state
        .uc
        .list_meters
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?;
    Ok(FormPage {
        meter_id: meter_id.map(|m| m.to_string()),
        drives: drive_options(lang),
        home_meters: home_options(lang, &meters),
        form,
        errors,
        conflict: conflict.map(|c| lang.code_text(c)),
        ctx: ctx.page.clone(),
    })
}

fn read_form(f: &FormData) -> VehicleForm {
    let g = |k: &str| f.get(k).to_owned();
    VehicleForm {
        name: g("name"),
        plate: g("plate"),
        model: g("model"),
        drive_type: g("drive_type"),
        home: g("home_charging_meter_id"),
        pv_share: g("pv_share_pct"),
        odometer: g("odometer"),
        start: g("start_at"),
    }
}

fn parse_vehicle(e: &mut FieldErrors, f: &VehicleForm) -> Option<VehicleInput> {
    let drive = f.drive_type.parse::<DriveType>().map_or_else(
        |_| {
            e.add("drive_type", "form.kind_invalid");
            None
        },
        Some,
    );
    let home = if f.home.is_empty() {
        None
    } else {
        f.home.parse::<MeterId>().map_or_else(
            |_| {
                e.add("home_charging_meter_id", "vehicle.home_charging_invalid");
                None
            },
            Some,
        )
    };
    let pv = decimal(e, "pv_share_pct", &f.pv_share);
    Some(VehicleInput {
        plate: f.plate.clone(),
        model: f.model.clone(),
        drive_type: drive?,
        home_charging_meter_id: home,
        pv_share_pct: pv,
    })
}

fn field_or_conflict(err: AppError) -> Result<(FieldErrors, Option<&'static str>), AppError> {
    match err {
        AppError::Validation(e) => Ok((e, None)),
        AppError::Conflict(c) => Ok((FieldErrors::default(), Some(c))),
        other => Err(other),
    }
}

// @impl SPEC_CAR_VEHICLES
pub async fn new_form(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    if !ctx.page.is_owner {
        return Err(ctx.fail(AppError::Forbidden));
    }
    let today = Calendar::new(ctx.page.tz).local_date(Utc::now());
    let form = VehicleForm {
        drive_type: DriveType::Combustion.as_str().into(),
        start: date_input(today),
        ..VehicleForm::default()
    };
    Ok(render(
        &form_page(&state, &ctx, None, form, FieldErrors::default(), None).await?,
    ))
}

// @impl SPEC_CAR_VEHICLES
pub async fn create(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let form = read_form(&FormData::new(pairs));
    let mut e = FieldErrors::default();
    let vehicle = parse_vehicle(&mut e, &form);
    let odometer = required_decimal(&mut e, "odometer", &form.odometer);
    let start = required_date(&mut e, "start_at", &form.start);
    let result = match (vehicle, odometer, start, e.is_empty()) {
        (Some(vehicle), Some(odometer), Some(start), true) => {
            state
                .uc
                .create_vehicle
                .execute(
                    &ctx.tenant,
                    NewVehicle {
                        name: form.name.clone(),
                        vehicle,
                        odometer,
                        start_at: Calendar::new(ctx.page.tz).start_of(start),
                    },
                )
                .await
        }
        _ => Err(AppError::Validation(e)),
    };
    match result {
        Ok(id) => {
            ctx.flash("flash-vehicle-created").await;
            Ok(see_other(&format!("/vehicles/{id}")))
        }
        Err(err) => {
            let (errors, conflict) = field_or_conflict(err).map_err(|e| ctx.fail(e))?;
            let page = form_page(&state, &ctx, None, form, errors, conflict).await?;
            Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
        }
    }
}

fn form_of(v: Option<&Vehicle>, lang: Lang) -> VehicleForm {
    v.map_or_else(
        || VehicleForm {
            drive_type: DriveType::Combustion.as_str().into(),
            ..VehicleForm::default()
        },
        |v| VehicleForm {
            plate: v.data.plate.clone(),
            model: v.data.model.clone(),
            drive_type: v.data.drive_type.as_str().into(),
            home: v
                .data
                .home_charging_meter_id
                .map(|m| m.to_string())
                .unwrap_or_default(),
            pv_share: v
                .data
                .pv_share_pct
                .map(|p| plain(lang, p))
                .unwrap_or_default(),
            ..VehicleForm::default()
        },
    )
}

// @impl SPEC_CAR_VEHICLES
pub async fn edit_form(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    if !ctx.page.is_owner {
        return Err(ctx.fail(AppError::Forbidden));
    }
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let detail = state
        .uc
        .get_vehicle
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let mut form = form_of(detail.vehicle.as_ref(), ctx.page.lang);
    form.name.clone_from(&detail.meter.meter.data.name);
    Ok(render(
        &form_page(&state, &ctx, Some(id), form, FieldErrors::default(), None).await?,
    ))
}

// @impl SPEC_CAR_VEHICLES
pub async fn update(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let form = read_form(&FormData::new(pairs));
    let mut e = FieldErrors::default();
    let result = match parse_vehicle(&mut e, &form) {
        Some(v) if e.is_empty() => state.uc.update_vehicle.execute(&ctx.tenant, id, v).await,
        _ => Err(AppError::Validation(e)),
    };
    match result {
        Ok(()) => {
            ctx.flash("flash-vehicle-saved").await;
            Ok(see_other(&format!("/vehicles/{id}")))
        }
        Err(err) => {
            let (errors, conflict) = field_or_conflict(err).map_err(|e| ctx.fail(e))?;
            let page = form_page(&state, &ctx, Some(id), form, errors, conflict).await?;
            Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
        }
    }
}

// --- Detail ----------------------------------------------------------------

/// Zeile der Jahrestabelle.
pub struct ReportRow {
    pub label: String,
    pub km: String,
    pub variable: String,
    pub fixed: String,
    pub depreciation: String,
    pub variable_per_km: String,
    pub total_per_km: String,
    pub fuel: String,
    pub kwh: String,
    pub external: String,
    pub home: String,
    pub estimated: bool,
}

pub struct EntryRow {
    pub id: String,
    pub cells: Vec<String>,
}

pub struct ForecastView {
    pub year: i32,
    pub km: String,
    pub fixed: String,
    pub variable_per_km: String,
    pub expected: String,
}

#[derive(Template)]
#[template(path = "pages/vehicles/detail.html")]
#[allow(clippy::struct_excessive_bools)] // unabhängige Sichtbarkeiten im Template
struct DetailPage {
    ctx: PageCtx,
    id: String,
    name: String,
    facts: Vec<(String, String)>,
    has_vehicle: bool,
    charges: bool,
    plugin_hybrid: bool,
    fuels: bool,
    residual: Vec<(String, String)>,
    fuel_price: Vec<(String, String)>,
    fuel_prices: Vec<(String, String)>,
    rows: Vec<ReportRow>,
    /// (Kategorie, laufendes Jahr, Haltedauer)
    categories: Vec<(String, String, String)>,
    category_year: String,
    forecast: Option<ForecastView>,
    fuel: Vec<EntryRow>,
    charging: Vec<EntryRow>,
    profiles: Vec<EntryRow>,
    kinds: Vec<(String, String)>,
    /// Gesendete Werte eines fehlgeschlagenen Formulars
    values: HashMap<String, String>,
    errors: FieldErrors,
    /// Plausibilitätswarnung zum Kilometerstand
    warning: Option<String>,
}

impl DetailPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn val(&self, field: &str) -> String {
        self.values.get(field).cloned().unwrap_or_default()
    }

    fn val_or(&self, field: &str, default: &str) -> String {
        self.values
            .get(field)
            .cloned()
            .unwrap_or_else(|| default.to_owned())
    }
}

fn report_row(lang: Lang, label: String, p: &VehiclePeriod) -> ReportRow {
    let e = p.energy.as_ref();
    let pair = |kwh: Option<Estimated<Decimal>>, cost: Option<Estimated<Decimal>>| match (kwh, cost)
    {
        (Some(k), Some(c)) => format!(
            "{} / {}",
            estimated(lang, Some(k), 0, "kWh"),
            estimated(lang, Some(c), 2, "€")
        ),
        (Some(k), None) => estimated(lang, Some(k), 0, "kWh"),
        _ => String::new(),
    };
    ReportRow {
        label,
        km: estimated(
            lang,
            p.km.map(|k| Estimated {
                value: k.value.round(),
                ..k
            }),
            0,
            "km",
        ),
        variable: money(lang, p.cost.variable),
        fixed: money(lang, p.cost.fixed),
        depreciation: money(lang, p.cost.depreciation),
        variable_per_km: p
            .cost
            .variable_per_km
            .map(|v| format!("{} €", format::decimal(lang, v, 3)))
            .unwrap_or_default(),
        total_per_km: p
            .cost
            .total_per_km
            .map(|v| format!("{} €", format::decimal(lang, v, 3)))
            .unwrap_or_default(),
        fuel: p
            .fuel_per_100km
            .map(|v| format!("{} l", format::decimal(lang, v, 1)))
            .unwrap_or_default(),
        kwh: estimated(lang, e.and_then(|e| e.kwh_per_100km), 1, "kWh"),
        external: e
            .filter(|e| !e.external.kwh.is_zero())
            .map(|e| {
                format!(
                    "{} kWh / {}",
                    format::decimal(lang, e.external.kwh, 0),
                    money(lang, e.external.cost)
                )
            })
            .unwrap_or_default(),
        home: e.map(|e| pair(e.home_kwh, e.home_cost)).unwrap_or_default(),
        estimated: p.has_estimate(),
    }
}

fn charging_rows(lang: Lang, tz: chrono_tz::Tz, d: &VehicleDetail) -> Vec<EntryRow> {
    d.charging
        .iter()
        .rev()
        .map(|c| EntryRow {
            id: c.id.to_string(),
            cells: vec![
                format::datetime_in(lang, c.charged_at, tz),
                format!("{} kWh", format::decimal(lang, c.energy, 2)),
                money(lang, c.amount),
                c.kind.as_str().to_uppercase(),
                c.provider.clone().unwrap_or_default(),
                c.document_id.map(|doc| doc.to_string()).unwrap_or_default(),
            ],
        })
        .collect()
}

#[allow(clippy::too_many_lines)] // alle Abschnitte der Seite
fn detail_page(
    ctx: &Ctx,
    d: &VehicleDetail,
    report: Option<&VehicleReportView>,
    values: HashMap<String, String>,
    errors: FieldErrors,
    warning: Option<String>,
) -> DetailPage {
    let lang = ctx.page.lang;
    let v = d.vehicle.as_ref();
    let mut values = values;
    if let Some(h) = v.and_then(|v| v.holding.as_ref())
        && !values.contains_key("purchased_on")
    {
        let opt = |x: Option<Decimal>| x.map(|x| plain(lang, x)).unwrap_or_default();
        for (k, val) in [
            ("purchased_on", date_input(h.purchased_on)),
            ("price", plain(lang, h.price)),
            ("depreciation_mode", h.mode.as_str().to_owned()),
            ("residual_value", opt(h.residual_value)),
            (
                "planned_end",
                h.planned_end.map(date_input).unwrap_or_default(),
            ),
            ("odometer_at_purchase", opt(h.odometer_at_purchase)),
            ("lifetime_km", opt(h.lifetime_km)),
            ("end_value", opt(h.end_value)),
            ("sold_on", h.sold_on.map(date_input).unwrap_or_default()),
            ("sale_price", opt(h.sale_price)),
        ] {
            values.insert(k.to_owned(), val);
        }
    }
    let mut facts = Vec::new();
    if let Some(v) = v {
        if !v.data.plate.is_empty() {
            facts.push((lang.t("vehicle-plate"), v.data.plate.clone()));
        }
        if !v.data.model.is_empty() {
            facts.push((lang.t("vehicle-model"), v.data.model.clone()));
        }
        facts.push((
            lang.t("vehicle-drive"),
            lang.t(&format!("drive-{}", v.data.drive_type.as_str())),
        ));
        if let Some(h) = &v.holding {
            facts.push((
                lang.t("vehicle-purchase"),
                format!(
                    "{}, {}",
                    format::date(lang, h.purchased_on),
                    money(lang, h.price)
                ),
            ));
            if let (Some(on), Some(price)) = (h.sold_on, h.sale_price) {
                facts.push((
                    lang.t("vehicle-sale"),
                    format!("{}, {}", format::date(lang, on), money(lang, price)),
                ));
            }
        }
    }
    if let Some(home) = report.and_then(|r| r.home.as_ref()) {
        let key = if home.wallbox {
            "vehicle-home-wallbox"
        } else {
            "vehicle-home-house"
        };
        facts.push((
            lang.t("vehicle-home"),
            format!("{} ({})", home.name, lang.t(key)),
        ));
    }
    let mut rows = Vec::new();
    let mut categories = Vec::new();
    let mut category_year = String::new();
    if let Some(r) = report {
        for y in &r.years {
            rows.push(report_row(lang, y.year.unwrap_or_default().to_string(), y));
        }
        if let Some(h) = &r.holding {
            rows.push(report_row(lang, lang.t("vehicle-holding"), h));
        }
        let latest = r.years.first();
        category_year = latest
            .and_then(|y| y.year)
            .map(|y| y.to_string())
            .unwrap_or_default();
        for c in VehicleCostCategory::ALL {
            let of = |p: Option<&VehiclePeriod>| {
                p.and_then(|p| p.categories.iter().find(|(x, _)| *x == c))
                    .map(|(_, v)| *v)
            };
            let (a, b) = (of(latest), of(r.holding.as_ref()));
            if a.is_some() || b.is_some() {
                categories.push((
                    lang.t(&format!("cost-cat-{}", c.as_str())),
                    opt_money(lang, a),
                    opt_money(lang, b),
                ));
            }
        }
    }
    let forecast = report
        .and_then(|r| r.forecast.as_ref())
        .map(|f| ForecastView {
            year: f.year,
            km: estimated(
                lang,
                Some(Estimated {
                    value: f.km.value.round(),
                    ..f.km
                }),
                0,
                "km",
            ),
            fixed: money(lang, f.fixed),
            variable_per_km: f
                .variable_per_km
                .map(|v| format!("{} €", format::decimal(lang, v, 3)))
                .unwrap_or_default(),
            expected: opt_money(lang, f.expected),
        });
    let fuel = report
        .map(|r| {
            r.fuel_segments
                .iter()
                .rev()
                .take(10)
                .map(|s| EntryRow {
                    id: String::new(),
                    cells: vec![
                        format::date(lang, s.until),
                        format!("{} km", format::decimal(lang, s.km, 0)),
                        format!("{} l", format::decimal(lang, s.litres, 2)),
                        format!("{} l", format::decimal(lang, s.per_100km, 1)),
                    ],
                })
                .collect()
        })
        .unwrap_or_default();
    let profiles = v
        .map(|v| {
            v.profiles
                .iter()
                .rev()
                .map(|p| EntryRow {
                    id: p.id.to_string(),
                    cells: vec![
                        format::date(lang, p.profile.valid_from),
                        format!(
                            "{} {}",
                            plain(lang, p.profile.kwh_per_100km),
                            if p.profile.energy == ProfileEnergy::Fuel {
                                "l"
                            } else {
                                "kWh"
                            }
                        ),
                        format!("{} %", plain(lang, p.profile.charging_loss_pct)),
                        p.profile
                            .electric_share_pct
                            .map(|s| format!("{} %", plain(lang, s)))
                            .unwrap_or_default(),
                    ],
                })
                .collect()
        })
        .unwrap_or_default();
    let drive = v.map(|v| v.data.drive_type);
    if !values.contains_key("odometer_at_purchase")
        && let Some(initial) = d.initial_odometer
    {
        values.insert("odometer_at_purchase".into(), plain(lang, initial));
    }
    let residual = report
        .and_then(|r| r.residual)
        .map(|r| {
            let remaining = match r.remaining {
                Remaining::Until { years, until } => lang.ta(
                    "vehicle-remaining-until",
                    &[
                        ("years", format::decimal(lang, years, 1)),
                        ("date", format::date(lang, until)),
                    ],
                ),
                Remaining::Reached => lang.t("vehicle-remaining-reached"),
                Remaining::Sold => lang.t("vehicle-remaining-sold"),
                Remaining::InsufficientData => lang.t("vehicle-remaining-insufficient"),
            };
            vec![
                (lang.t("vehicle-current-value"), money(lang, r.value)),
                (lang.t("vehicle-remaining"), remaining),
            ]
        })
        .unwrap_or_default();
    let fuel_price = report
        .and_then(|r| r.fuel_price.as_ref())
        .map(|p| {
            let mut facts = Vec::new();
            if let Some((date, price)) = p.last {
                facts.push((
                    lang.t("vehicle-fuel-last"),
                    format!(
                        "{} €/l ({})",
                        format::decimal(lang, price, 3),
                        format::date(lang, date)
                    ),
                ));
            } else if let Some(price) = p.estimate {
                facts.push((
                    lang.t("vehicle-fuel-estimate"),
                    format!("{} €/l", format::decimal(lang, price, 3)),
                ));
            }
            if let Some(f) = report
                .and_then(|r| r.forecast.as_ref())
                .and_then(|f| f.fuel_cost)
            {
                facts.push((
                    lang.t("vehicle-fuel-forecast"),
                    estimated(lang, Some(f), 2, "€"),
                ));
            }
            facts
        })
        .unwrap_or_default();
    let fuel_prices = report
        .and_then(|r| r.fuel_price.as_ref())
        .map(|p| {
            p.points
                .iter()
                .map(|(date, price)| (format::date(lang, *date), format::decimal(lang, *price, 3)))
                .collect()
        })
        .unwrap_or_default();
    DetailPage {
        id: d.meter.meter.id.to_string(),
        name: d.meter.meter.data.name.clone(),
        facts,
        has_vehicle: v.is_some(),
        charges: drive.is_some_and(DriveType::charges),
        plugin_hybrid: drive == Some(DriveType::PluginHybrid),
        fuels: drive.is_some_and(DriveType::fuels),
        residual,
        fuel_price,
        fuel_prices,
        rows,
        categories,
        category_year,
        forecast,
        fuel,
        charging: charging_rows(lang, ctx.page.tz, d),
        profiles,
        kinds: kind_options(),
        values,
        errors,
        warning,
        ctx: ctx.page.clone(),
    }
}

async fn load_detail(
    state: &WebState,
    ctx: &Ctx,
    id: MeterId,
) -> Result<(VehicleDetail, Option<VehicleReportView>), Response> {
    let detail = state
        .uc
        .get_vehicle
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let report = if detail.vehicle.is_some() {
        Some(
            state
                .uc
                .vehicle_report
                .execute(&ctx.tenant, id)
                .await
                .map_err(|e| ctx.fail(e))?,
        )
    } else {
        None
    };
    Ok((detail, report))
}

// @impl SPEC_CAR_REPORT
pub async fn detail(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let (d, report) = load_detail(&state, &ctx, id).await?;
    Ok(render(&detail_page(
        &ctx,
        &d,
        report.as_ref(),
        HashMap::new(),
        FieldErrors::default(),
        None,
    )))
}

/// Nach einem Eintrag zurück zur Detailseite; bei Fehlern oder Warnungen
/// die Seite mit den gesendeten Werten.
async fn after_entry(
    state: &WebState,
    ctx: &Ctx,
    id: MeterId,
    form: FormData,
    result: Result<Option<String>, AppError>,
    flash: &str,
) -> Result<Response, Response> {
    let (errors, warning) = match result {
        Ok(None) => {
            ctx.flash(flash).await;
            return Ok(see_other(&format!("/vehicles/{id}")));
        }
        Ok(Some(w)) => (FieldErrors::default(), Some(w)),
        Err(AppError::Validation(e)) => (e, None),
        Err(AppError::Conflict(code)) => {
            let mut e = FieldErrors::default();
            e.add("entry", code);
            (e, None)
        }
        Err(e) => return Err(ctx.fail(e)),
    };
    let (d, report) = load_detail(state, ctx, id).await?;
    let page = detail_page(ctx, &d, report.as_ref(), form.0, errors, warning);
    Ok(render_status(StatusCode::UNPROCESSABLE_ENTITY, &page))
}

/// Warnungstext einer Kilometerstand-Übernahme.
pub(crate) fn warning_of<T>(lang: Lang, outcome: SaveOutcome<T>) -> Option<String> {
    match outcome {
        SaveOutcome::Saved(_) => None,
        SaveOutcome::NeedsConfirmation(checks) => Some(
            checks
                .iter()
                .find_map(|c| hint(lang, &c.plausibility))
                .unwrap_or_else(|| lang.t("readings-warnings")),
        ),
    }
}

/// Ladevorgang aus Formularfeldern mit Präfix `charging_`.
pub(crate) fn charging_input(
    e: &mut FieldErrors,
    form: &FormData,
    cal: Calendar,
) -> Option<ChargingInput> {
    let at = parse::datetime_local(form.get("charging_at")).map_or_else(
        |_| {
            e.add("charging_at", "form.datetime_invalid");
            None
        },
        |l| Some(cal.to_utc(l)),
    );
    let energy = required_decimal(e, "charging_energy", form.get("charging_energy"));
    let amount = required_decimal(e, "charging_amount", form.get("charging_amount"));
    let odometer = decimal(e, "charging_odometer", form.get("charging_odometer"));
    let kind = form
        .get("charging_kind")
        .parse::<ChargingKind>()
        .unwrap_or(ChargingKind::Ac);
    Some(ChargingInput {
        charged_at: at?,
        energy: energy?,
        amount: amount?,
        odometer,
        provider: form.opt("charging_provider"),
        kind,
        odometer_as_reading: form.has("charging_odometer_reading"),
        confirm_warnings: form.get("confirm_warnings") == "true",
    })
}

/// Fehlerfelder der Anwendungsfälle auf die Formularfelder abbilden.
pub(crate) fn prefixed(e: FieldErrors, prefix: &str) -> FieldErrors {
    let mut out = FieldErrors::default();
    for x in e.0 {
        let field = if x.field.starts_with(prefix) {
            x.field
        } else {
            format!("{prefix}{}", x.field)
        };
        out.0.push(zaehler_app::FieldError { field, ..x });
    }
    out
}

// @impl SPEC_CAR_ENERGY_COST
pub async fn add_charging(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let mut e = FieldErrors::default();
    let input = charging_input(&mut e, &form, Calendar::new(ctx.page.tz));
    let result = match input {
        Some(i) if e.is_empty() => state
            .uc
            .record_charging
            .execute(&ctx.tenant, id, i)
            .await
            .map(|o| warning_of(ctx.page.lang, o))
            .map_err(|err| match err {
                AppError::Validation(e) => AppError::Validation(prefixed(e, "charging_")),
                other => other,
            }),
        _ => Err(AppError::Validation(e)),
    };
    after_entry(&state, &ctx, id, form, result, "flash-charging-saved").await
}

pub async fn delete_charging(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let entry: ChargingId = entry.parse().map_err(|_| ctx.fail(AppError::NotFound))?;
    state
        .uc
        .delete_charging
        .execute(&ctx.tenant, id, entry)
        .await
        .map_err(|e| ctx.fail(e))?;
    ctx.flash("flash-entry-deleted").await;
    Ok(see_other(&format!("/vehicles/{id}")))
}

// @impl SPEC_CAR_ENERGY_COST
pub async fn add_profile(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let mut e = FieldErrors::default();
    let from = required_date(&mut e, "profile_valid_from", form.get("profile_valid_from"));
    let kwh = required_decimal(&mut e, "profile_kwh", form.get("profile_kwh"));
    let loss = decimal(&mut e, "profile_loss", form.get("profile_loss")).unwrap_or(Decimal::ZERO);
    let share = decimal(&mut e, "profile_share", form.get("profile_share"));
    let energy = match form.get("profile_energy") {
        "" | "electric" => Some(ProfileEnergy::Electric),
        "fuel" => Some(ProfileEnergy::Fuel),
        _ => {
            e.add("profile_energy", "form.kind_invalid");
            None
        }
    };
    let result = match (from, kwh, energy, e.is_empty()) {
        (Some(valid_from), Some(kwh), Some(energy), true) => state
            .uc
            .set_consumption_profile
            .execute(
                &ctx.tenant,
                id,
                ConsumptionProfile {
                    energy,
                    valid_from,
                    kwh_per_100km: kwh,
                    charging_loss_pct: loss,
                    electric_share_pct: share,
                },
            )
            .await
            .map(|_| None),
        _ => Err(AppError::Validation(e)),
    };
    after_entry(&state, &ctx, id, form, result, "flash-profile-saved").await
}

pub async fn delete_profile(
    State(state): State<WebState>,
    ctx: Ctx,
    Path((id, entry)): Path<(String, String)>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let entry: ProfileId = entry.parse().map_err(|_| ctx.fail(AppError::NotFound))?;
    state
        .uc
        .delete_consumption_profile
        .execute(&ctx.tenant, id, entry)
        .await
        .map_err(|e| ctx.fail(e))?;
    ctx.flash("flash-entry-deleted").await;
    Ok(see_other(&format!("/vehicles/{id}")))
}

/// Anschaffung setzen; `action=remove` entfernt sie.
// @impl SPEC_CAR_DEPRECIATION
pub async fn set_acquisition(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let form = FormData::new(pairs);
    let result = if form.get("action") == "remove" {
        state
            .uc
            .set_acquisition
            .execute(&ctx.tenant, id, None)
            .await
    } else {
        let mut e = FieldErrors::default();
        let purchased = required_date(&mut e, "purchased_on", form.get("purchased_on"));
        let price = required_decimal(&mut e, "price", form.get("price"));
        let mode = match form.get("depreciation_mode") {
            "" | "time" => Some(DepreciationMode::Time),
            "mileage" => Some(DepreciationMode::Mileage),
            _ => {
                e.add("depreciation_mode", "form.kind_invalid");
                None
            }
        };
        let residual = decimal(&mut e, "residual_value", form.get("residual_value"));
        let planned = date(&mut e, "planned_end", form.get("planned_end"));
        let odometer_at_purchase = decimal(
            &mut e,
            "odometer_at_purchase",
            form.get("odometer_at_purchase"),
        );
        let lifetime_km = decimal(&mut e, "lifetime_km", form.get("lifetime_km"));
        let end_value = decimal(&mut e, "end_value", form.get("end_value"));
        let sold_on = date(&mut e, "sold_on", form.get("sold_on"));
        let sale_price = decimal(&mut e, "sale_price", form.get("sale_price"));
        match (purchased, price, mode, e.is_empty()) {
            (Some(purchased_on), Some(price), Some(mode), true) => {
                state
                    .uc
                    .set_acquisition
                    .execute(
                        &ctx.tenant,
                        id,
                        Some(Holding {
                            purchased_on,
                            price,
                            mode,
                            residual_value: residual,
                            planned_end: planned,
                            odometer_at_purchase,
                            lifetime_km,
                            end_value,
                            sold_on,
                            sale_price,
                        }),
                    )
                    .await
            }
            _ => Err(AppError::Validation(e)),
        }
    };
    after_entry(
        &state,
        &ctx,
        id,
        form,
        result.map(|()| None),
        "flash-acquisition-saved",
    )
    .await
}

/// Vorgabe für Datum und Uhrzeit eines neuen Eintrags: jetzt, lokal.
#[allow(clippy::trivially_copy_pass_by_ref)] // Aufruf aus dem Template mit Referenz
pub(crate) fn now_input(tz: &chrono_tz::Tz) -> String {
    format::datetime_input(Utc::now().with_timezone(tz).naive_local())
}

/// Mittag des Tages als Vorgabe für einen Ladevorgang aus einem Beleg.
pub(crate) fn noon_input(d: NaiveDate) -> String {
    format::datetime_input(d.and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap_or_default()))
}

/// Kosten je Kalenderjahr für das Diagramm (SPEC_CAR_REPORT).
// @impl SPEC_CAR_REPORT
pub async fn chart(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    use rust_decimal::prelude::ToPrimitive;
    use serde_json::{Value, json};

    let id = parse_meter(&id).map_err(|e| ctx.fail(e))?;
    let r = state
        .uc
        .vehicle_report
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let lang = ctx.page.lang;
    let years: Vec<&VehiclePeriod> = r.years.iter().rev().collect();
    let series = |f: fn(&VehiclePeriod) -> Decimal| -> Vec<Value> {
        years
            .iter()
            .map(|p| json!(f(p).round_dp(2).to_f64()))
            .collect()
    };
    Ok(super::costs::json_response(json!({
        "unit": "€",
        "labels": years.iter().map(|p| p.year.unwrap_or_default().to_string()).collect::<Vec<_>>(),
        "series": {
            "variable": { "name": lang.t("vehicle-variable"), "data": series(|p| p.cost.variable) },
            "fixed": { "name": lang.t("vehicle-fixed"), "data": series(|p| p.cost.fixed) },
            "depreciation": { "name": lang.t("vehicle-depreciation"), "data": series(|p| p.cost.depreciation) },
            "fuel_price": { "name": lang.t("asset-unit-price"), "data": r.fuel_price.as_ref().map(|p| p.points.iter().map(|(date, price)| json!([date.to_string(), price.round_dp(3).to_f64()])).collect::<Vec<_>>()).unwrap_or_default() },
        }
    })))
}
