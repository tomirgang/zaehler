//! REST-API: Verträge, Preisstände, Abschläge, Anpassungen, Abrechnungen
//! und Haushaltskosten (SPEC_API_REST). Die Rollen prüfen die
//! Anwendungsfälle.

use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::StatusCode,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{
        AdjustmentId, ContractId, InstallmentId, MeterId, PositionId, PriceComponentId, RegisterId,
        SettlementId,
    },
    model::{ContractEntry, MonthDay},
    usecase::{
        contract::{
            AdjustmentInput, ComponentInput, ContractInput, ContractTemplate, InstallmentInput,
            PositionInput, SettlementInput,
        },
        cost::HouseholdCostView,
        vehicle_report::VehicleYearCost,
    },
};
use zaehler_domain::{
    meter::Direction,
    tariff::{AdjustmentKind, BasePriceInterval, Tier, UnitPricing},
};

use super::{
    auth::ApiCtx,
    dto::{ContractDto, MonthCostDto, PeriodCostDto},
    error::{ApiError, Problem},
};
use crate::state::WebState;

type ApiResult<T> = Result<Json<T>, ApiError>;
type Created<T> = Result<(StatusCode, Json<T>), ApiError>;

pub(super) fn routes(r: OpenApiRouter<WebState>) -> OpenApiRouter<WebState> {
    r.routes(routes!(list_meter_contracts, create_contract))
        .routes(routes!(update_contract, delete_contract))
        .routes(routes!(add_price_component))
        .routes(routes!(add_installment))
        .routes(routes!(add_adjustment))
        .routes(routes!(add_settlement))
        .routes(routes!(delete_entry))
        .routes(routes!(contract_templates))
        .routes(routes!(household_costs))
}

/// Stufe eines Staffelpreises.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TierBody {
    /// Obergrenze (exklusiv); ohne Angabe unbegrenzt
    #[schema(value_type = Option<String>)]
    pub up_to: Option<Decimal>,
    #[schema(value_type = String)]
    pub price: Decimal,
}

/// Arbeitspreis: fest oder Staffel nach kumuliertem Verbrauch.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum UnitPricingBody {
    Flat {
        #[schema(value_type = String)]
        price: Decimal,
    },
    Tiered {
        tiers: Vec<TierBody>,
    },
}

impl From<UnitPricingBody> for UnitPricing {
    fn from(p: UnitPricingBody) -> Self {
        match p {
            UnitPricingBody::Flat { price } => Self::Flat { price },
            UnitPricingBody::Tiered { tiers } => Self::Tiered {
                tiers: tiers
                    .into_iter()
                    .map(|t| Tier {
                        up_to: t.up_to,
                        price: t.price,
                    })
                    .collect(),
            },
        }
    }
}

/// Preisstand.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PriceComponentBody {
    pub valid_from: NaiveDate,
    /// Grundpreis, Standard 0
    #[schema(value_type = Option<String>)]
    pub base_price: Option<Decimal>,
    /// `day`, `month` oder `year` (Standard)
    pub base_interval: Option<String>,
    pub unit_pricing: UnitPricingBody,
    /// Abweichender Umrechnungsfaktor ab diesem Preisstand
    #[schema(value_type = Option<String>)]
    pub factor: Option<Decimal>,
}

impl PriceComponentBody {
    fn input(self, e: &mut FieldErrors, field: &str, id: Option<Uuid>) -> ComponentInput {
        let base_interval = match self.base_interval.as_deref() {
            None => BasePriceInterval::Year,
            Some(s) => s.parse().unwrap_or_else(|_| {
                e.add(format!("{field}_interval"), "form.kind_invalid");
                BasePriceInterval::Year
            }),
        };
        ComponentInput {
            id: id.map(PriceComponentId),
            valid_from: self.valid_from,
            base_price: self.base_price.unwrap_or(Decimal::ZERO),
            base_interval,
            unit_pricing: self.unit_pricing.into(),
            factor: self.factor,
        }
    }
}

/// Preisstand einer Position im Vertrag; mit `id` bleibt ein vorhandener
/// Preisstand erhalten.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PositionComponentBody {
    pub id: Option<Uuid>,
    #[serde(flatten)]
    pub component: PriceComponentBody,
}

/// Kostenposition; mit `id` bleibt eine vorhandene Position erhalten.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PositionBody {
    pub id: Option<Uuid>,
    pub name: String,
    /// Ohne Angabe alle Register der Richtung
    pub register_id: Option<Uuid>,
    /// Umrechnungsfaktor, Standard 1
    #[schema(value_type = Option<String>)]
    pub factor: Option<Decimal>,
    /// MwSt-Satz in Prozent
    #[schema(value_type = Option<String>)]
    pub vat_pct: Option<Decimal>,
    pub components: Vec<PositionComponentBody>,
}

/// Vertrag anlegen oder ändern; beim Ändern werden Positionen und
/// Preisstände vollständig ersetzt.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ContractBody {
    /// `import` (Bezug) oder `export` (Einspeisung)
    pub direction: String,
    pub provider: String,
    pub contract_number: Option<String>,
    pub starts_on: NaiveDate,
    /// Letzter Tag (einschließlich)
    pub ends_on: Option<NaiveDate>,
    /// Beginn des Abrechnungsjahres `MM-DD`; ohne Angabe der Vertragsbeginn
    pub billing_start: Option<String>,
    pub note: Option<String>,
    pub positions: Vec<PositionBody>,
}

fn month_day(s: &str) -> Option<MonthDay> {
    let (m, d) = s.split_once('-')?;
    if m.len() != 2 || d.len() != 2 {
        return None;
    }
    Some(MonthDay {
        month: m.parse().ok()?,
        day: d.parse().ok()?,
    })
}

impl ContractBody {
    fn input(self) -> Result<ContractInput, AppError> {
        let mut e = FieldErrors::default();
        let direction = self.direction.parse().unwrap_or_else(|_| {
            e.add("direction", "form.kind_invalid");
            Direction::Import
        });
        let billing_start = self.billing_start.as_deref().and_then(|s| {
            let md = month_day(s);
            if md.is_none() {
                e.add("billing_start", "meter.billing_start_invalid");
            }
            md
        });
        let positions = self
            .positions
            .into_iter()
            .enumerate()
            .map(|(i, p)| PositionInput {
                id: p.id.map(PositionId),
                name: p.name,
                register_id: p.register_id.map(RegisterId),
                factor: p.factor.unwrap_or(Decimal::ONE),
                vat_pct: p.vat_pct,
                components: p
                    .components
                    .into_iter()
                    .enumerate()
                    .map(|(j, c)| c.component.input(&mut e, &format!("pos{i}_c{j}"), c.id))
                    .collect(),
            })
            .collect();
        e.into_result()?;
        Ok(ContractInput {
            direction,
            provider: self.provider,
            contract_number: self.contract_number,
            starts_on: self.starts_on,
            ends_on: self.ends_on,
            billing_start,
            note: self.note,
            positions,
        })
    }
}

/// Neuer Abschlagsstand; die bisherigen bleiben als Historie erhalten.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InstallmentBody {
    pub valid_from: NaiveDate,
    #[schema(value_type = String)]
    pub amount: Decimal,
    /// Turnus in Monaten (1–12), Standard 1
    pub every_months: Option<u8>,
}

/// Bonus oder Gutschrift.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AdjustmentBody {
    /// `bonus` oder `credit`
    pub kind: String,
    pub booked_on: NaiveDate,
    /// Positiver Betrag senkt die Kosten
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub note: Option<String>,
}

/// Tatsächliche Jahresabrechnung.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SettlementBody {
    pub period_start: NaiveDate,
    /// Letzter Tag (einschließlich)
    pub period_end: NaiveDate,
    #[schema(value_type = String)]
    pub amount: Decimal,
}

/// Position einer Vertragsvorlage.
#[derive(Debug, Serialize, ToSchema)]
pub struct TemplatePositionDto {
    /// Übersetzungsschlüssel des Namens, z.B. `fresh_water`
    pub name: String,
    pub register_id: Option<Uuid>,
    /// Ohne Angabe ist eine Eingabe nötig (z.B. Brennwert × Zustandszahl)
    #[schema(value_type = Option<String>)]
    pub factor: Option<Decimal>,
    /// `day`, `month` oder `year`
    pub base_interval: String,
    /// Grundpreis vorgesehen
    pub base: bool,
    /// Arbeitspreis vorgesehen
    pub work: bool,
}

/// Vorlage für einen Vertrag passend zur Messstelle.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContractTemplateDto {
    pub code: String,
    /// `import` oder `export`
    pub direction: String,
    pub positions: Vec<TemplatePositionDto>,
}

impl From<&ContractTemplate> for ContractTemplateDto {
    fn from(t: &ContractTemplate) -> Self {
        Self {
            code: t.code.into(),
            direction: t.direction.as_str().into(),
            positions: t
                .positions
                .iter()
                .map(|p| TemplatePositionDto {
                    name: p.name.into(),
                    register_id: p.register_id.map(|r| r.0),
                    factor: p.factor,
                    base_interval: p.base_interval.as_str().into(),
                    base: p.base,
                    work: p.work,
                })
                .collect(),
        }
    }
}

/// Kosten als Zwischenzähler, enthalten in den Kosten der übergeordneten
/// Messstelle.
#[derive(Debug, Serialize, ToSchema)]
pub struct IncludedCostDto {
    /// Name der übergeordneten Messstelle
    pub parent: String,
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub estimated: bool,
}

/// Zeile der Haushaltsübersicht je Messstelle.
#[derive(Debug, Serialize, ToSchema)]
pub struct HouseholdCostRowDto {
    pub meter_id: Uuid,
    pub name: String,
    pub unit: String,
    pub import: Option<PeriodCostDto>,
    pub export: Option<PeriodCostDto>,
    /// Nicht Teil der Summen
    pub included_in: Option<IncludedCostDto>,
}

/// Kosten des Haushalts in einem Kalenderjahr.
#[derive(Debug, Serialize, ToSchema)]
pub struct HouseholdCostsDto {
    pub year: i32,
    pub today: NaiveDate,
    /// Summe Bezug (ohne Zwischenzähler-Anteile)
    #[schema(value_type = String)]
    pub import_total: Decimal,
    /// Summe Einspeisung (negativ = Gutschrift)
    #[schema(value_type = String)]
    pub export_total: Decimal,
    #[schema(value_type = String)]
    pub net_total: Decimal,
    /// Enthält geschätzte oder prognostizierte Anteile
    pub has_estimate: bool,
    pub rows: Vec<HouseholdCostRowDto>,
    pub months: Vec<MonthCostDto>,
    /// Fahrzeugkosten des Jahres bis heute; nicht im Saldo enthalten, da
    /// Heimladen bereits in den Stromkosten steckt
    pub vehicles: Vec<VehicleYearCostDto>,
}

/// Kosten eines Fahrzeugs im Kalenderjahr bis heute (SPEC_CAR_REPORT).
#[derive(Debug, Serialize, ToSchema)]
pub struct VehicleYearCostDto {
    pub meter_id: Uuid,
    pub name: String,
    #[schema(value_type = String)]
    pub km: Decimal,
    /// km ganz oder teilweise geschätzt
    pub km_estimated: bool,
    #[schema(value_type = String)]
    pub variable: Decimal,
    #[schema(value_type = String)]
    pub fixed: Decimal,
    #[schema(value_type = String)]
    pub depreciation: Decimal,
    #[schema(value_type = String)]
    pub total: Decimal,
    /// Vollkosten je km; ohne km `null`
    #[schema(value_type = Option<String>)]
    pub total_per_km: Option<Decimal>,
}

impl VehicleYearCostDto {
    fn from_year(v: &VehicleYearCost) -> Option<Self> {
        let p = v.period.as_ref()?;
        Some(Self {
            meter_id: v.meter_id.0,
            name: v.name.clone(),
            km: p.km.map_or(Decimal::ZERO, |k| k.value),
            km_estimated: p.km.is_some_and(|k| k.estimated),
            variable: p.cost.variable,
            fixed: p.cost.fixed,
            depreciation: p.cost.depreciation,
            total: v.total(),
            total_per_km: p.cost.total_per_km,
        })
    }
}

impl From<&HouseholdCostView> for HouseholdCostsDto {
    fn from(v: &HouseholdCostView) -> Self {
        Self {
            year: v.year,
            today: v.today,
            import_total: v.import_total(),
            export_total: v.export_total(),
            net_total: v.import_total() + v.export_total(),
            has_estimate: v.has_estimate(),
            rows: v
                .rows
                .iter()
                .map(|r| HouseholdCostRowDto {
                    meter_id: r.meter_id.0,
                    name: r.name.clone(),
                    unit: r.unit.clone(),
                    import: r.import.as_ref().map(PeriodCostDto::from),
                    export: r.export.as_ref().map(PeriodCostDto::from),
                    included_in: r.included_in.as_ref().map(|(parent, c)| IncludedCostDto {
                        parent: parent.clone(),
                        amount: c.value,
                        estimated: c.estimated,
                    }),
                })
                .collect(),
            months: v
                .months
                .iter()
                .map(|m| MonthCostDto {
                    month: m.month.start.format("%Y-%m").to_string(),
                    measured: m.measured,
                    estimated: m.estimated,
                    export: m.export,
                })
                .collect(),
            vehicles: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct YearQuery {
    /// Kalenderjahr (1970–9999); ohne Angabe das laufende
    pub year: Option<i32>,
}

async fn contract_dto(s: &WebState, ctx: &ApiCtx, id: ContractId) -> Result<ContractDto, ApiError> {
    let d = s.uc.get_contract.execute(&ctx.tenant, id).await?;
    Ok(ContractDto::from(&d.contract))
}

/// Verträge einer Messstelle.
#[utoipa::path(
    get,
    path = "/api/v1/meters/{id}/contracts",
    params(("id" = Uuid, Path, description = "Messstelle")),
    responses(
        (status = OK, body = Vec<ContractDto>),
        (status = NOT_FOUND, body = Problem),
    ),
    tag = "contracts"
)]
async fn list_meter_contracts(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<Vec<ContractDto>> {
    let list =
        s.uc.meter_contracts
            .execute(&ctx.tenant, MeterId(id))
            .await?;
    Ok(Json(list.iter().map(ContractDto::from).collect()))
}

/// Vertrag anlegen (Owner).
#[utoipa::path(
    post,
    path = "/api/v1/meters/{id}/contracts",
    params(("id" = Uuid, Path, description = "Messstelle")),
    request_body = ContractBody,
    responses(
        (status = CREATED, body = ContractDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    ),
    tag = "contracts"
)]
async fn create_contract(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<ContractBody>, JsonRejection>,
) -> Created<ContractDto> {
    let b = super::json(body)?;
    let input = b.input()?;
    let cid =
        s.uc.create_contract
            .execute(&ctx.tenant, MeterId(id), input)
            .await?;
    Ok((
        StatusCode::CREATED,
        Json(contract_dto(&s, &ctx, cid).await?),
    ))
}

/// Vertrag ändern; Positionen und Preisstände werden ersetzt, Abschläge,
/// Anpassungen und Abrechnungen bleiben (Owner).
#[utoipa::path(
    put,
    path = "/api/v1/contracts/{id}",
    params(("id" = Uuid, Path, description = "Vertrag")),
    request_body = ContractBody,
    responses(
        (status = OK, body = ContractDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    ),
    tag = "contracts"
)]
async fn update_contract(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<ContractBody>, JsonRejection>,
) -> ApiResult<ContractDto> {
    let b = super::json(body)?;
    let input = b.input()?;
    s.uc.update_contract
        .execute(&ctx.tenant, ContractId(id), input)
        .await?;
    Ok(Json(contract_dto(&s, &ctx, ContractId(id)).await?))
}

/// Vertrag mit allen Einträgen löschen (Owner).
#[utoipa::path(
    delete,
    path = "/api/v1/contracts/{id}",
    params(("id" = Uuid, Path, description = "Vertrag")),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
    ),
    tag = "contracts"
)]
async fn delete_contract(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.uc.delete_contract
        .execute(&ctx.tenant, ContractId(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Neuer Preisstand einer Position (Owner); liefert den Vertrag.
#[utoipa::path(
    post,
    path = "/api/v1/contracts/{id}/positions/{position}/price-components",
    params(
        ("id" = Uuid, Path, description = "Vertrag"),
        ("position" = Uuid, Path, description = "Position des Vertrags"),
    ),
    request_body = PriceComponentBody,
    responses(
        (status = CREATED, body = ContractDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    ),
    tag = "contracts"
)]
async fn add_price_component(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path((id, position)): Path<(Uuid, Uuid)>,
    body: Result<Json<PriceComponentBody>, JsonRejection>,
) -> Created<ContractDto> {
    let b = super::json(body)?;
    let mut e = FieldErrors::default();
    let input = b.input(&mut e, "component", None);
    e.into_result()?;
    s.uc.add_price_component
        .execute(&ctx.tenant, ContractId(id), PositionId(position), input)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(contract_dto(&s, &ctx, ContractId(id)).await?),
    ))
}

/// Neuer Abschlagsstand (Owner); liefert den Vertrag.
#[utoipa::path(
    post,
    path = "/api/v1/contracts/{id}/installments",
    params(("id" = Uuid, Path, description = "Vertrag")),
    request_body = InstallmentBody,
    responses(
        (status = CREATED, body = ContractDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    ),
    tag = "contracts"
)]
async fn add_installment(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<InstallmentBody>, JsonRejection>,
) -> Created<ContractDto> {
    let b = super::json(body)?;
    s.uc.set_installment
        .execute(
            &ctx.tenant,
            ContractId(id),
            InstallmentInput {
                valid_from: b.valid_from,
                amount: b.amount,
                every_months: b.every_months.unwrap_or(1),
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(contract_dto(&s, &ctx, ContractId(id)).await?),
    ))
}

/// Bonus oder Gutschrift (Member); liefert den Vertrag.
#[utoipa::path(
    post,
    path = "/api/v1/contracts/{id}/adjustments",
    params(("id" = Uuid, Path, description = "Vertrag")),
    request_body = AdjustmentBody,
    responses(
        (status = CREATED, body = ContractDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    ),
    tag = "contracts"
)]
async fn add_adjustment(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<AdjustmentBody>, JsonRejection>,
) -> Created<ContractDto> {
    let b = super::json(body)?;
    let Ok(kind) = b.kind.parse::<AdjustmentKind>() else {
        let mut e = FieldErrors::default();
        e.add("adjustment_kind", "form.kind_invalid");
        return Err(AppError::Validation(e).into());
    };
    s.uc.add_adjustment
        .execute(
            &ctx.tenant,
            ContractId(id),
            AdjustmentInput {
                kind,
                booked_on: b.booked_on,
                amount: b.amount,
                note: b.note,
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(contract_dto(&s, &ctx, ContractId(id)).await?),
    ))
}

/// Tatsächliche Jahresabrechnung (Member); liefert den Vertrag.
#[utoipa::path(
    post,
    path = "/api/v1/contracts/{id}/settlements",
    params(("id" = Uuid, Path, description = "Vertrag")),
    request_body = SettlementBody,
    responses(
        (status = CREATED, body = ContractDto),
        (status = BAD_REQUEST, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = UNPROCESSABLE_ENTITY, body = Problem),
    ),
    tag = "contracts"
)]
async fn add_settlement(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
    body: Result<Json<SettlementBody>, JsonRejection>,
) -> Created<ContractDto> {
    let b = super::json(body)?;
    s.uc.record_settlement
        .execute(
            &ctx.tenant,
            ContractId(id),
            SettlementInput {
                period_start: b.period_start,
                period_end: b.period_end,
                amount: b.amount,
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(contract_dto(&s, &ctx, ContractId(id)).await?),
    ))
}

/// Abschlag (Owner), Anpassung oder Abrechnung (Member) löschen; `kind`
/// ist `installments`, `adjustments` oder `settlements`.
#[utoipa::path(
    delete,
    path = "/api/v1/contracts/{id}/{kind}/{entry}",
    params(
        ("id" = Uuid, Path, description = "Vertrag"),
        ("kind" = String, Path, description = "`installments`, `adjustments` oder `settlements`"),
        ("entry" = Uuid, Path, description = "Eintrag"),
    ),
    responses(
        (status = NO_CONTENT),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
    ),
    tag = "contracts"
)]
async fn delete_entry(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path((id, kind, entry)): Path<(Uuid, String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let entry = match kind.as_str() {
        "installments" => ContractEntry::Installment(InstallmentId(entry)),
        "adjustments" => ContractEntry::Adjustment(AdjustmentId(entry)),
        "settlements" => ContractEntry::Settlement(SettlementId(entry)),
        _ => return Err(AppError::NotFound.into()),
    };
    s.uc.delete_contract_entry
        .execute(&ctx.tenant, ContractId(id), entry)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Vorlagen für einen Vertrag passend zu Art und Registern der Messstelle.
#[utoipa::path(
    get,
    path = "/api/v1/meters/{id}/contract-templates",
    params(("id" = Uuid, Path, description = "Messstelle")),
    responses(
        (status = OK, body = Vec<ContractTemplateDto>),
        (status = NOT_FOUND, body = Problem),
    ),
    tag = "contracts"
)]
async fn contract_templates(
    State(s): State<WebState>,
    ctx: ApiCtx,
    Path(id): Path<Uuid>,
) -> ApiResult<Vec<ContractTemplateDto>> {
    let (_, templates) =
        s.uc.contract_templates
            .execute(&ctx.tenant, MeterId(id))
            .await?;
    Ok(Json(
        templates.iter().map(ContractTemplateDto::from).collect(),
    ))
}

/// Kosten des Haushalts je Messstelle und Monat (SPEC_TAR_COST_VIEW).
#[utoipa::path(
    get,
    path = "/api/v1/costs",
    params(YearQuery),
    responses(
        (status = OK, body = HouseholdCostsDto),
        (status = BAD_REQUEST, body = Problem),
    ),
    tag = "contracts"
)]
async fn household_costs(
    State(s): State<WebState>,
    ctx: ApiCtx,
    query: Result<Query<YearQuery>, QueryRejection>,
) -> ApiResult<HouseholdCostsDto> {
    let Query(q) = query.map_err(|_| Problem::bad_request("query.invalid"))?;
    if q.year.is_some_and(|y| !(1970..=9999).contains(&y)) {
        return Err(Problem::bad_request("query.year").into());
    }
    let v = s.uc.household_costs.execute(&ctx.tenant, q.year).await?;
    let (_, vehicles) =
        s.uc.household_vehicle_costs
            .execute(&ctx.tenant, Some(v.year))
            .await?;
    let mut dto = HouseholdCostsDto::from(&v);
    dto.vehicles = vehicles
        .iter()
        .filter_map(VehicleYearCostDto::from_year)
        .collect();
    Ok(Json(dto))
}
