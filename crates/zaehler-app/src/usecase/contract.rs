//! Verträge, Preisstände, Abschläge, Anpassungen und Abrechnungen
//! (SPEC_TAR_CONTRACTS, SPEC_TAR_ACTUAL_SETTLEMENT).

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use zaehler_domain::{
    calendar::Calendar,
    meter::{Direction, MeterKind},
    period::Period,
    tariff::{
        Adjustment, AdjustmentKind, BasePriceInterval, Installment, PriceComponent, UnitPricing,
        tiers_valid,
    },
};

use crate::{
    AppError, FieldErrors, PortError, Role, Tenant,
    ids::{
        AdjustmentId, ContractId, DocumentId, InstallmentId, MeterId, PositionId, PriceComponentId,
        RegisterId, SettlementId,
    },
    model::{
        AdjustmentRow, Contract, ContractData, ContractEntry, InstallmentRow, MeterAggregate,
        MonthDay, Position, PriceComponentRow, SettlementRow,
    },
    port::UnitOfWork,
    usecase::{Ports, use_case, validate},
};

pub const MAX_PROVIDER: usize = 80;
pub const MAX_CONTRACT_NUMBER: usize = 40;
pub const MAX_POSITION_NAME: usize = 60;
pub const MAX_NOTE: usize = 500;
pub const MAX_POSITIONS: usize = 20;
pub const MAX_COMPONENTS: usize = 60;
pub const MAX_TIERS: usize = 10;
/// Betragsgrenze für Preise und Beträge (Festkomma 10⁻⁶ in `i64`).
const MAX_MONEY: Decimal = Decimal::from_parts(1_000_000_000, 0, 0, false, 0);

/// Preisstand im Formular; `id` ist bei vorhandenen Preisständen gesetzt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentInput {
    pub id: Option<PriceComponentId>,
    pub valid_from: NaiveDate,
    pub base_price: Decimal,
    pub base_interval: BasePriceInterval,
    pub unit_pricing: UnitPricing,
    /// Abweichender Umrechnungsfaktor ab diesem Preisstand
    pub factor: Option<Decimal>,
}

/// Kostenposition im Formular.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionInput {
    pub id: Option<PositionId>,
    pub name: String,
    /// `None`: alle Register der Richtung
    pub register_id: Option<RegisterId>,
    pub factor: Decimal,
    pub vat_pct: Option<Decimal>,
    pub components: Vec<ComponentInput>,
}

/// Eingaben für Anlegen und Ändern eines Vertrags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractInput {
    pub direction: Direction,
    pub provider: String,
    pub contract_number: Option<String>,
    pub starts_on: NaiveDate,
    pub ends_on: Option<NaiveDate>,
    /// Ohne Angabe Monat und Tag des Vertragsbeginns
    pub billing_start: Option<MonthDay>,
    pub note: Option<String>,
    pub positions: Vec<PositionInput>,
}

/// Neuer Abschlagsstand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallmentInput {
    pub valid_from: NaiveDate,
    pub amount: Decimal,
    pub every_months: u8,
}

/// Bonus oder Gutschrift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjustmentInput {
    pub kind: AdjustmentKind,
    pub booked_on: NaiveDate,
    pub amount: Decimal,
    pub note: Option<String>,
}

/// Tatsächliche Jahresabrechnung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementInput {
    pub period_start: NaiveDate,
    /// Letzter Tag (einschließlich)
    pub period_end: NaiveDate,
    pub amount: Decimal,
}

/// Vertrag mit seiner Messstelle.
#[derive(Debug, Clone)]
pub struct ContractDetail {
    pub contract: Contract,
    pub meter: MeterAggregate,
}

use_case!(
    /// Vertrag anlegen (Owner).
    CreateContract,
    /// Vertrag ändern; ersetzt Positionen und Preisstände (Owner).
    UpdateContract,
    /// Vertrag mit allen Einträgen löschen (Owner).
    DeleteContract,
    /// Vertrag mit Messstelle (Viewer).
    GetContract,
    /// Verträge einer Messstelle (Viewer).
    MeterContracts,
    /// Neuer Preisstand einer Position (Owner).
    AddPriceComponent,
    /// Neuer Abschlagsstand (Owner).
    SetInstallment,
    /// Bonus oder Gutschrift (Member).
    AddAdjustment,
    /// Tatsächliche Jahresabrechnung (Member).
    RecordSettlement,
    /// Abschlag (Owner), Anpassung oder Abrechnung (Member) löschen.
    DeleteContractEntry,
    /// Vorlagen für das Vertragsformular (Viewer).
    ContractTemplates,
);

pub(crate) fn money(e: &mut FieldErrors, field: &str, v: Decimal, places: u32) {
    if v.normalize().scale() > places {
        e.add_with(
            field,
            "contract.too_precise",
            vec![("places", places.to_string())],
        );
    } else if v.abs() >= MAX_MONEY {
        e.add(field, "contract.too_large");
    }
}

pub(crate) fn text_opt(e: &mut FieldErrors, field: &str, v: Option<&String>, max: usize) {
    if v.is_some_and(|s| s.trim().chars().count() > max) {
        e.add_with(
            field,
            "contract.text_too_long",
            vec![("max", max.to_string())],
        );
    }
}

pub(crate) fn clean(v: Option<&String>) -> Option<String> {
    v.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

/// Prüft einen Preisstand; `field` ist das Feldpräfix.
// @impl SPEC_TAR_CONTRACTS, SPEC_TAR_TIERED
fn validate_component(e: &mut FieldErrors, field: &str, c: &ComponentInput) {
    if c.base_price.is_sign_negative() && !c.base_price.is_zero() {
        e.add(format!("{field}_base"), "contract.base_negative");
    }
    money(e, &format!("{field}_base"), c.base_price, 6);
    match &c.unit_pricing {
        UnitPricing::Flat { price } => money(e, &format!("{field}_price"), *price, 6),
        UnitPricing::Tiered { tiers } => {
            if tiers.len() > MAX_TIERS || !tiers_valid(tiers) {
                e.add(format!("{field}_tiers"), "contract.tiers_invalid");
            }
            for t in tiers {
                money(e, &format!("{field}_tiers"), t.price, 6);
                if let Some(up) = t.up_to {
                    money(e, &format!("{field}_tiers"), up, 3);
                }
            }
        }
        UnitPricing::Dynamic { .. } => {
            e.add(format!("{field}_price"), "contract.dynamic_unsupported");
        }
    }
    if let Some(f) = c.factor {
        if f <= Decimal::ZERO {
            e.add(format!("{field}_factor"), "contract.factor_invalid");
        }
        money(e, &format!("{field}_factor"), f, 6);
    }
}

/// Monat und Tag des Abrechnungsbeginns; der 29.02. ist nicht zulässig.
fn billing_start_valid(md: MonthDay) -> bool {
    NaiveDate::from_ymd_opt(2025, u32::from(md.month), u32::from(md.day)).is_some()
}

#[allow(clippy::too_many_lines)] // eine Prüfung je Feld
// @impl SPEC_TAR_CONTRACTS
fn validate_contract(input: &ContractInput, meter: &MeterAggregate) -> FieldErrors {
    let mut e = FieldErrors::default();
    validate::name(
        &mut e,
        "provider",
        &input.provider,
        MAX_PROVIDER,
        "contract.provider_invalid",
    );
    text_opt(
        &mut e,
        "contract_number",
        input.contract_number.as_ref(),
        MAX_CONTRACT_NUMBER,
    );
    text_opt(&mut e, "note", input.note.as_ref(), MAX_NOTE);
    if meter.meter.data.kind == MeterKind::Vehicle {
        e.add("direction", "contract.meter_kind_invalid");
    }
    if !meter
        .registers
        .iter()
        .any(|r| r.data.direction == input.direction)
    {
        e.add("direction", "contract.direction_invalid");
    }
    if input.ends_on.is_some_and(|end| end < input.starts_on) {
        e.add("ends_on", "contract.end_before_start");
    }
    if input
        .billing_start
        .is_some_and(|md| !billing_start_valid(md))
    {
        e.add("billing_start", "meter.billing_start_invalid");
    }
    if input.positions.is_empty() {
        e.add("positions", "contract.positions_required");
    }
    if input.positions.len() > MAX_POSITIONS {
        e.add_with(
            "positions",
            "contract.positions_too_many",
            vec![("max", MAX_POSITIONS.to_string())],
        );
    }
    for (i, p) in input.positions.iter().enumerate() {
        let f = format!("pos{i}");
        validate::name(
            &mut e,
            &format!("{f}_name"),
            &p.name,
            MAX_POSITION_NAME,
            "contract.position_name_invalid",
        );
        if let Some(r) = p.register_id
            && !meter
                .register(r)
                .is_some_and(|r| r.data.direction == input.direction)
        {
            e.add(format!("{f}_register"), "contract.register_invalid");
        }
        if p.factor <= Decimal::ZERO {
            e.add(format!("{f}_factor"), "contract.factor_invalid");
        }
        money(&mut e, &format!("{f}_factor"), p.factor, 6);
        if let Some(v) = p.vat_pct {
            if v.is_sign_negative() || v > Decimal::ONE_HUNDRED {
                e.add(format!("{f}_vat"), "contract.vat_invalid");
            }
            money(&mut e, &format!("{f}_vat"), v, 3);
        }
        if p.components.is_empty() {
            e.add(format!("{f}_components"), "contract.components_required");
        }
        if p.components.len() > MAX_COMPONENTS {
            e.add(format!("{f}_components"), "contract.components_too_many");
        }
        let mut seen = std::collections::HashSet::new();
        for (j, c) in p.components.iter().enumerate() {
            let cf = format!("{f}_c{j}");
            validate_component(&mut e, &cf, c);
            if !seen.insert(c.valid_from) {
                e.add(format!("{cf}_valid_from"), "contract.component_duplicate");
            }
        }
        if p.components
            .iter()
            .map(|c| c.valid_from)
            .min()
            .is_some_and(|first| first > input.starts_on)
        {
            e.add(
                format!("{f}_c0_valid_from"),
                "contract.component_after_start",
            );
        }
    }
    e
}

/// Vertragsdaten mit IDs: vorhandene IDs bleiben nur, wenn sie zum
/// bisherigen Vertrag gehören.
fn contract_data(p: &Ports, input: &ContractInput, existing: Option<&Contract>) -> ContractData {
    let known_position = |id: Option<PositionId>| {
        id.filter(|id| existing.is_some_and(|c| c.data.positions.iter().any(|x| x.id == *id)))
    };
    let known_component = |id: Option<PriceComponentId>| {
        id.filter(|id| {
            existing.is_some_and(|c| {
                c.data
                    .positions
                    .iter()
                    .any(|x| x.components.iter().any(|y| y.id == *id))
            })
        })
    };
    let billing_start = input.billing_start.unwrap_or_else(|| {
        let s = input.starts_on;
        let md = MonthDay {
            month: u8::try_from(s.month()).unwrap_or(1),
            day: u8::try_from(s.day()).unwrap_or(1),
        };
        // 29.02. als Beginn wird 01.03.
        if billing_start_valid(md) {
            md
        } else {
            MonthDay { month: 3, day: 1 }
        }
    });
    ContractData {
        direction: input.direction,
        provider: input.provider.trim().to_owned(),
        contract_number: clean(input.contract_number.as_ref()),
        starts_on: input.starts_on,
        ends_on: input.ends_on,
        billing_start,
        note: clean(input.note.as_ref()),
        positions: input
            .positions
            .iter()
            .map(|pos| {
                let mut components: Vec<PriceComponentRow> = pos
                    .components
                    .iter()
                    .map(|c| PriceComponentRow {
                        id: known_component(c.id)
                            .unwrap_or_else(|| PriceComponentId(p.ids.new_uuid())),
                        component: PriceComponent {
                            valid_from: c.valid_from,
                            base_price: c.base_price,
                            base_interval: c.base_interval,
                            unit_pricing: c.unit_pricing.clone(),
                            factor: c.factor,
                        },
                    })
                    .collect();
                components.sort_by_key(|c| c.component.valid_from);
                Position {
                    id: known_position(pos.id).unwrap_or_else(|| PositionId(p.ids.new_uuid())),
                    name: pos.name.trim().to_owned(),
                    register_id: pos.register_id,
                    factor: pos.factor,
                    vat_pct: pos.vat_pct,
                    components,
                }
            })
            .collect(),
    }
}

fn validity(data: &ContractData) -> Period {
    Period::new(
        data.starts_on,
        data.ends_on
            .map_or(NaiveDate::MAX, |d| d + chrono::Days::new(1)),
    )
}

fn overlaps(a: Period, b: Period) -> bool {
    a.start < b.end && b.start < a.end
}

/// Verträge derselben Messstelle und Richtung überlappen nicht; ein
/// Vertrag überschneidet keine Einbauzeit eines Geräts mit übergeordneter
/// Messstelle (SPEC_TAR_CONTRACTS, SPEC_MTR_SUBMETER).
// @impl SPEC_TAR_CONTRACTS, SPEC_MTR_SUBMETER
async fn check_conflicts(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meter: &MeterAggregate,
    own: Option<ContractId>,
    data: &ContractData,
) -> Result<(), AppError> {
    let period = validity(data);
    let others = uow.contracts().for_meter(t, meter.meter.id).await?;
    if others.iter().any(|c| {
        Some(c.id) != own && c.data.direction == data.direction && overlaps(c.validity(), period)
    }) {
        return Err(AppError::Conflict("contract.overlap"));
    }
    let cal = Calendar::new(uow.households().get(t).await?.data.timezone);
    if submeter_windows(meter, cal)
        .into_iter()
        .any(|w| overlaps(w, period))
    {
        return Err(AppError::Conflict("contract.submeter"));
    }
    Ok(())
}

/// Einbauzeiten der Geräte mit übergeordneter Messstelle als Kalendertage.
#[must_use]
pub fn submeter_windows(meter: &MeterAggregate, cal: Calendar) -> Vec<Period> {
    meter
        .devices
        .iter()
        .filter(|d| d.data.parent_meter_id.is_some())
        .map(|d| {
            Period::new(
                cal.local_date(d.data.installed_at),
                d.removed_at.map_or(NaiveDate::MAX, |r| cal.local_date(r)),
            )
        })
        .filter(|p| p.start < p.end)
        .collect()
}

/// Ein Vertrag der Messstelle überschneidet den Zeitraum `[from, until)`
/// (Einbauzeit eines Geräts mit übergeordneter Messstelle).
// @impl SPEC_MTR_SUBMETER
pub(crate) async fn contract_overlaps(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meter: MeterId,
    from: chrono::DateTime<chrono::Utc>,
    until: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<bool, AppError> {
    let contracts = uow.contracts().for_meter(t, meter).await?;
    if contracts.is_empty() {
        return Ok(false);
    }
    let cal = Calendar::new(uow.households().get(t).await?.data.timezone);
    let window = Period::new(
        cal.local_date(from),
        until.map_or(NaiveDate::MAX, |u| cal.local_date(u)),
    );
    Ok(window.start < window.end && contracts.iter().any(|c| overlaps(c.validity(), window)))
}

async fn load_meter(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meter: MeterId,
) -> Result<MeterAggregate, AppError> {
    uow.meters().get(t, meter).await?.ok_or(AppError::NotFound)
}

pub(crate) async fn load_contract(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    id: ContractId,
) -> Result<Contract, AppError> {
    uow.contracts().get(t, id).await?.ok_or(AppError::NotFound)
}

impl CreateContract {
    // @impl SPEC_TAR_CONTRACTS
    #[tracing::instrument(name = "CreateContract", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        meter: MeterId,
        input: ContractInput,
    ) -> Result<ContractId, AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let agg = load_meter(uow.as_mut(), t, meter).await?;
        let id = create_in(&self.p, uow.as_mut(), t, &agg, &input).await?;
        uow.commit().await?;
        Ok(id)
    }
}

/// Prüft und legt einen Vertrag in der Unit of Work an.
// @impl SPEC_TAR_CONTRACTS
pub(crate) async fn create_in(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    agg: &MeterAggregate,
    input: &ContractInput,
) -> Result<ContractId, AppError> {
    validate_contract(input, agg).into_result()?;
    let data = contract_data(p, input, None);
    check_conflicts(uow, t, agg, None, &data).await?;
    let id = ContractId(p.ids.new_uuid());
    uow.contracts().create(t, id, agg.meter.id, &data).await?;
    Ok(id)
}

impl UpdateContract {
    // @impl SPEC_TAR_CONTRACTS
    #[tracing::instrument(name = "UpdateContract", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        id: ContractId,
        input: ContractInput,
    ) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let existing = load_contract(uow.as_mut(), t, id).await?;
        let agg = load_meter(uow.as_mut(), t, existing.meter_id).await?;
        validate_contract(&input, &agg).into_result()?;
        let data = contract_data(&self.p, &input, Some(&existing));
        check_conflicts(uow.as_mut(), t, &agg, Some(id), &data).await?;
        uow.contracts().update(t, id, &data).await?;
        uow.commit().await?;
        Ok(())
    }
}

impl DeleteContract {
    /// Liefert die Messstelle des gelöschten Vertrags.
    #[tracing::instrument(name = "DeleteContract", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: ContractId) -> Result<MeterId, AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let meter = load_contract(uow.as_mut(), t, id).await?.meter_id;
        uow.contracts().delete(t, id).await?;
        uow.commit().await?;
        Ok(meter)
    }
}

impl GetContract {
    #[tracing::instrument(name = "GetContract", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: ContractId) -> Result<ContractDetail, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let contract = load_contract(uow.as_mut(), t, id).await?;
        let meter = load_meter(uow.as_mut(), t, contract.meter_id).await?;
        Ok(ContractDetail { contract, meter })
    }
}

impl MeterContracts {
    #[tracing::instrument(name = "MeterContracts", skip_all)]
    pub async fn execute(&self, t: &Tenant, meter: MeterId) -> Result<Vec<Contract>, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        load_meter(uow.as_mut(), t, meter).await?;
        Ok(uow.contracts().for_meter(t, meter).await?)
    }
}

impl AddPriceComponent {
    /// Neuer Preisstand einer Position; Preisänderungen nach Beginn eines
    /// Abrechnungszeitraums wirken über neue Preisstände (SPEC_TAR_CONTRACTS).
    // @impl SPEC_TAR_CONTRACTS
    #[tracing::instrument(name = "AddPriceComponent", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        contract: ContractId,
        position: PositionId,
        input: ComponentInput,
    ) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let c = load_contract(uow.as_mut(), t, contract).await?;
        add_component_in(&self.p, uow.as_mut(), t, &c, position, input, "component").await?;
        uow.commit().await?;
        Ok(())
    }
}

/// Prüft und speichert einen Preisstand in der Unit of Work; `field` ist
/// das Feldpräfix der Fehler.
// @impl SPEC_TAR_CONTRACTS
pub(crate) async fn add_component_in(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    c: &Contract,
    position: PositionId,
    input: ComponentInput,
    field: &str,
) -> Result<(), AppError> {
    let pos = c
        .data
        .positions
        .iter()
        .find(|p| p.id == position)
        .ok_or(AppError::NotFound)?;
    let mut e = FieldErrors::default();
    validate_component(&mut e, field, &input);
    if !c.active_on(input.valid_from) {
        e.add(format!("{field}_valid_from"), "contract.date_outside");
    }
    e.into_result()?;
    if pos
        .components
        .iter()
        .any(|x| x.component.valid_from == input.valid_from)
    {
        return Err(AppError::Conflict("contract.component_exists"));
    }
    let row = PriceComponentRow {
        id: PriceComponentId(p.ids.new_uuid()),
        component: PriceComponent {
            valid_from: input.valid_from,
            base_price: input.base_price,
            base_interval: input.base_interval,
            unit_pricing: input.unit_pricing,
            factor: input.factor,
        },
    };
    uow.contracts()
        .add_component(t, position, &row)
        .await
        .map_err(conflict("contract.component_exists"))
}

pub(crate) fn conflict(code: &'static str) -> impl Fn(PortError) -> AppError {
    move |e| match e {
        PortError::Conflict(_) => AppError::Conflict(code),
        other => other.into(),
    }
}

fn amount(e: &mut FieldErrors, field: &str, v: Decimal) {
    money(e, field, v, 2);
}

impl SetInstallment {
    /// Neuer Abschlagsstand ab `valid_from`; die bisherigen bleiben als
    /// Historie erhalten (REQ_TAR_008).
    // @impl SPEC_TAR_INSTALLMENT_SETTLEMENT
    #[tracing::instrument(name = "SetInstallment", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        contract: ContractId,
        input: InstallmentInput,
    ) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let c = load_contract(uow.as_mut(), t, contract).await?;
        set_installment_in(&self.p, uow.as_mut(), t, &c, &input).await?;
        uow.commit().await?;
        Ok(())
    }
}

/// Prüft und speichert einen Abschlagsstand in der Unit of Work.
// @impl SPEC_TAR_INSTALLMENT_SETTLEMENT
pub(crate) async fn set_installment_in(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    c: &Contract,
    input: &InstallmentInput,
) -> Result<(), AppError> {
    let mut e = FieldErrors::default();
    if input.amount.is_sign_negative() && !input.amount.is_zero() {
        e.add("installment_amount", "contract.amount_negative");
    }
    amount(&mut e, "installment_amount", input.amount);
    if !(1..=12).contains(&input.every_months) {
        e.add("installment_interval", "contract.interval_invalid");
    }
    if !c.active_on(input.valid_from) {
        e.add("installment_valid_from", "contract.date_outside");
    }
    e.into_result()?;
    let row = InstallmentRow {
        id: InstallmentId(p.ids.new_uuid()),
        installment: Installment {
            amount: input.amount,
            valid_from: input.valid_from,
            every_months: input.every_months,
        },
    };
    uow.contracts()
        .add_installment(t, c.id, &row)
        .await
        .map_err(conflict("contract.installment_exists"))
}

impl AddAdjustment {
    // @impl SPEC_TAR_COST_VIEW
    #[tracing::instrument(name = "AddAdjustment", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        contract: ContractId,
        input: AdjustmentInput,
    ) -> Result<(), AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        load_contract(uow.as_mut(), t, contract).await?;
        let mut e = FieldErrors::default();
        if input.amount <= Decimal::ZERO {
            e.add("adjustment_amount", "contract.amount_positive");
        }
        amount(&mut e, "adjustment_amount", input.amount);
        text_opt(&mut e, "adjustment_note", input.note.as_ref(), MAX_NOTE);
        e.into_result()?;
        let row = AdjustmentRow {
            id: AdjustmentId(self.p.ids.new_uuid()),
            adjustment: Adjustment {
                kind: input.kind,
                booked_on: input.booked_on,
                amount: input.amount,
            },
            note: clean(input.note.as_ref()),
        };
        uow.contracts().add_adjustment(t, contract, &row).await?;
        uow.commit().await?;
        Ok(())
    }
}

impl RecordSettlement {
    // @impl SPEC_TAR_ACTUAL_SETTLEMENT
    #[tracing::instrument(name = "RecordSettlement", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        contract: ContractId,
        input: SettlementInput,
    ) -> Result<(), AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        load_contract(uow.as_mut(), t, contract).await?;
        record_settlement_in(&self.p, uow.as_mut(), t, contract, &input, None).await?;
        uow.commit().await?;
        Ok(())
    }
}

/// Prüft und speichert eine Abrechnung in der Unit of Work.
// @impl SPEC_TAR_ACTUAL_SETTLEMENT
pub(crate) async fn record_settlement_in(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    contract: ContractId,
    input: &SettlementInput,
    document_id: Option<DocumentId>,
) -> Result<(), AppError> {
    let mut e = FieldErrors::default();
    if input.period_end < input.period_start {
        e.add("settlement_period_end", "contract.end_before_start");
    }
    amount(&mut e, "settlement_amount", input.amount);
    e.into_result()?;
    let row = SettlementRow {
        id: SettlementId(p.ids.new_uuid()),
        period_start: input.period_start,
        period_end: input.period_end,
        amount: input.amount,
        document_id: document_id.map(DocumentId::into_uuid),
    };
    uow.contracts().add_settlement(t, contract, &row).await?;
    Ok(())
}

impl DeleteContractEntry {
    #[tracing::instrument(name = "DeleteContractEntry", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        contract: ContractId,
        entry: ContractEntry,
    ) -> Result<(), AppError> {
        t.require(match entry {
            ContractEntry::Installment(_) => Role::Owner,
            ContractEntry::Adjustment(_) | ContractEntry::Settlement(_) => Role::Member,
        })?;
        let mut uow = self.p.uow.begin().await?;
        uow.contracts().delete_entry(t, contract, entry).await?;
        uow.commit().await?;
        Ok(())
    }
}

/// Position einer Vorlage; der Name ist ein Übersetzungsschlüssel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplatePosition {
    pub name: &'static str,
    pub register_id: Option<RegisterId>,
    /// `None`: Eingabe nötig (Brennwert × Zustandszahl)
    pub factor: Option<Decimal>,
    pub base_interval: BasePriceInterval,
    /// Grundpreis vorgesehen
    pub base: bool,
    /// Arbeitspreis vorgesehen
    pub work: bool,
}

/// Vorlage für das Vertragsformular (SPEC_TAR_CONTRACTS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractTemplate {
    pub code: &'static str,
    pub direction: Direction,
    pub positions: Vec<TemplatePosition>,
}

fn tpl(
    name: &'static str,
    register_id: Option<RegisterId>,
    factor: Option<Decimal>,
    base: bool,
    work: bool,
) -> TemplatePosition {
    TemplatePosition {
        name,
        register_id,
        factor,
        base_interval: BasePriceInterval::Year,
        base,
        work,
    }
}

/// Vorlagen passend zu Art und Registern der Messstelle.
// @impl SPEC_TAR_CONTRACTS
#[must_use]
pub fn templates_for(meter: &MeterAggregate) -> Vec<ContractTemplate> {
    let one = Some(Decimal::ONE);
    let regs = |d: Direction| -> Vec<RegisterId> {
        meter
            .registers
            .iter()
            .filter(|r| r.data.direction == d)
            .map(|r| r.id)
            .collect()
    };
    let import = regs(Direction::Import);
    let export = regs(Direction::Export);
    let simple = |code, name| ContractTemplate {
        code,
        direction: Direction::Import,
        positions: vec![tpl(name, None, one, true, true)],
    };
    let mut out = Vec::new();
    if !import.is_empty() {
        match meter.meter.data.kind {
            MeterKind::Electricity => {
                out.push(simple("electricity", "electricity"));
                if let [ht, nt, ..] = import.as_slice() {
                    out.push(ContractTemplate {
                        code: "electricity_ht_nt",
                        direction: Direction::Import,
                        positions: vec![
                            tpl("ht", Some(*ht), one, true, true),
                            tpl("nt", Some(*nt), one, false, true),
                        ],
                    });
                }
            }
            MeterKind::Gas => out.push(ContractTemplate {
                code: "gas",
                direction: Direction::Import,
                positions: vec![tpl("gas", None, None, true, true)],
            }),
            MeterKind::Water => out.push(ContractTemplate {
                code: "water",
                direction: Direction::Import,
                positions: vec![
                    tpl("fresh_water", None, one, true, true),
                    tpl("waste_water", None, one, false, true),
                    tpl("meter_rent", None, one, true, false),
                ],
            }),
            MeterKind::Heat => out.push(simple("heat", "heat")),
            MeterKind::Custom => out.push(simple("custom", "costs")),
            MeterKind::Vehicle => {}
        }
    }
    if !export.is_empty() && meter.meter.data.kind != MeterKind::Vehicle {
        out.push(ContractTemplate {
            code: "feed_in",
            direction: Direction::Export,
            positions: vec![tpl("feed_in", None, one, false, true)],
        });
    }
    out
}

impl ContractTemplates {
    #[tracing::instrument(name = "ContractTemplates", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        meter: MeterId,
    ) -> Result<(MeterAggregate, Vec<ContractTemplate>), AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let agg = load_meter(uow.as_mut(), t, meter).await?;
        let templates = templates_for(&agg);
        Ok((agg, templates))
    }
}
