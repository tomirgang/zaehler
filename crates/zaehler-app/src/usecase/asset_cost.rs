//! Kostenbuch für Fahrzeuge und Anlagen (SPEC_ASSET_COSTS, ADR_020).

use chrono::{NaiveDate, NaiveTime};
use rust_decimal::Decimal;
use zaehler_domain::{
    assets::{AssetCategory, AssetType, Recurrence, VehicleCostCategory},
    calendar::Calendar,
    period::Period,
};

use crate::{
    AppError, FieldErrors, Role, Tenant,
    ids::{AssetCostId, DocumentId},
    model::{AssetCost, AssetRef},
    port::UnitOfWork,
    usecase::{
        Ports,
        contract::{clean, money, text_opt},
        use_case,
        vehicle::{SaveOutcome, load_vehicle, record_odometer, vehicle_meter},
    },
};

pub const MAX_NOTE: usize = 500;
/// Längster Turnus wiederkehrender Einträge in Monaten.
pub const MAX_RECURRENCE_MONTHS: u8 = 120;

use_case!(
    /// Kostenbuch einer Anlage für ein Jahr (Viewer).
    ListAssetCosts,
    /// Ein Eintrag (Viewer).
    GetAssetCost,
    /// Eintrag anlegen (Member).
    AddAssetCost,
    /// Eintrag ändern (Member).
    UpdateAssetCost,
    /// Eintrag löschen (Member).
    DeleteAssetCost,
);

/// Eingaben eines Eintrags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetCostInput {
    pub booked_on: NaiveDate,
    pub category: AssetCategory,
    /// Bei Kraftstoff genügen zwei von Betrag, Menge und Preis pro Liter
    /// (SPEC_CAR_FUEL_PRICE), sonst Pflicht
    pub amount: Option<Decimal>,
    pub quantity: Option<Decimal>,
    /// Preis pro Liter bei Kraftstoff
    pub unit_price: Option<Decimal>,
    pub odometer: Option<Decimal>,
    pub full_tank: bool,
    pub recurrence: Option<Recurrence>,
    pub note: Option<String>,
    /// Kilometerstand zusätzlich als Ablesung speichern (nur beim Anlegen)
    pub odometer_as_reading: bool,
    pub confirm_warnings: bool,
}

/// Einträge mit Buchungen im Jahr und Summen je Kategorie.
#[derive(Debug, Clone)]
pub struct AssetCostBook {
    pub year: i32,
    /// Eintrag und sein Betrag im Jahr
    pub entries: Vec<(AssetCost, Decimal)>,
    pub totals: Vec<(AssetCategory, Decimal)>,
    pub total: Decimal,
}

/// Summen je Kategorie im Zeitraum, in der Reihenfolge der Kategorien.
#[must_use]
pub fn totals(
    entries: &[AssetCost],
    asset: AssetType,
    period: Period,
) -> Vec<(AssetCategory, Decimal)> {
    AssetCategory::all(asset)
        .into_iter()
        .filter_map(|c| {
            let sum: Decimal = entries
                .iter()
                .filter(|e| e.category == c)
                .map(|e| e.amount_in(period))
                .sum();
            (!sum.is_zero()).then_some((c, sum))
        })
        .collect()
}

/// Anlage existiert im Haushalt; liefert die Kategorieregel der Anlage.
async fn check_asset(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    asset: AssetRef,
) -> Result<Option<zaehler_domain::vehicle::DriveType>, AppError> {
    match asset.asset_type {
        AssetType::Vehicle => Ok(Some(
            load_vehicle(uow, t, crate::ids::MeterId(asset.id))
                .await?
                .data
                .drive_type,
        )),
        AssetType::PvSystem => {
            uow.pv()
                .get(t, crate::ids::PvSystemId(asset.id))
                .await?
                .ok_or(AppError::NotFound)?;
            Ok(None)
        }
    }
}

/// Betrag, Menge und Preis pro Liter nach der Prüfung.
#[derive(Debug, Clone, Copy)]
struct Amounts {
    amount: Decimal,
    quantity: Option<Decimal>,
    unit_price: Option<Decimal>,
}

/// Bei Kraftstoff ergänzt die fehlende von drei Angaben
/// (SPEC_CAR_FUEL_PRICE); sonst ist der Betrag Pflicht.
// @impl SPEC_CAR_FUEL_PRICE
fn amounts(e: &mut FieldErrors, fuel: bool, input: &AssetCostInput) -> Amounts {
    use zaehler_domain::vehicle::{FuelFieldsError, complete_fuel_fields};
    let fallback = Amounts {
        amount: input.amount.unwrap_or_default(),
        quantity: input.quantity,
        unit_price: input.unit_price,
    };
    if !fuel {
        if input.amount.is_none() {
            e.add("amount", "form.number_required");
        }
        if input.unit_price.is_some() {
            e.add("unit_price", "asset.quantity_fuel_only");
        }
        return fallback;
    }
    match complete_fuel_fields(input.amount, input.quantity, input.unit_price) {
        Ok(f) => Amounts {
            amount: f.amount,
            quantity: f.quantity,
            unit_price: f.unit_price,
        },
        Err(err) => {
            match err {
                FuelFieldsError::Missing => e.add("amount", "asset_cost.price_fields_missing"),
                FuelFieldsError::Mismatch => e.add("unit_price", "asset_cost.price_mismatch"),
                FuelFieldsError::NotPositive => {
                    if input.quantity.is_some_and(|q| q <= Decimal::ZERO) {
                        e.add("quantity", "vehicle.energy_invalid");
                    } else {
                        e.add("unit_price", "asset_cost.price_invalid");
                    }
                }
            }
            fallback
        }
    }
}

// @impl SPEC_ASSET_COSTS, SPEC_CAR_VEHICLES
fn validate(
    e: &mut FieldErrors,
    asset: AssetRef,
    drive: Option<zaehler_domain::vehicle::DriveType>,
    input: &AssetCostInput,
) -> Amounts {
    if input.category.asset_type() != asset.asset_type {
        e.add("category", "asset.category_invalid");
    }
    let fuel = input.category == AssetCategory::Vehicle(VehicleCostCategory::Fuel);
    if let (Some(d), AssetCategory::Vehicle(c)) = (drive, input.category)
        && !d.allows(c)
    {
        e.add("category", "vehicle.category_not_allowed");
    }
    if let Some(amount) = input.amount {
        money(e, "amount", amount, 2);
    }
    if let Some(q) = input.quantity {
        if !fuel {
            e.add("quantity", "asset.quantity_fuel_only");
        }
        money(e, "quantity", q, 3);
    }
    if let Some(p) = input.unit_price {
        money(e, "unit_price", p, 3);
    }
    let a = if e.is_empty() {
        amounts(e, fuel, input)
    } else {
        Amounts {
            amount: input.amount.unwrap_or_default(),
            quantity: input.quantity,
            unit_price: input.unit_price,
        }
    };
    if a.amount < Decimal::ZERO {
        e.add("amount", "contract.amount_negative");
    }
    money(e, "amount", a.amount, 2);
    if let Some(quantity) = a.quantity {
        money(e, "quantity", quantity, 3);
    }
    if let Some(price) = a.unit_price {
        money(e, "unit_price", price, 3);
    }
    if input.full_tank && !fuel {
        e.add("full_tank", "asset.quantity_fuel_only");
    }
    if let Some(o) = input.odometer {
        if asset.asset_type != AssetType::Vehicle {
            e.add("odometer", "asset.odometer_vehicle_only");
        }
        if o < Decimal::ZERO {
            e.add("odometer", "reading.negative");
        }
        money(e, "odometer", o, 3);
    }
    if input.odometer_as_reading && input.odometer.is_none() {
        e.add("odometer", "form.number_required");
    }
    if let Some(r) = input.recurrence {
        if r.every_months == 0 || r.every_months > MAX_RECURRENCE_MONTHS {
            e.add_with(
                "recurrence_months",
                "asset.recurrence_invalid",
                vec![("max", MAX_RECURRENCE_MONTHS.to_string())],
            );
        }
        if r.until.is_some_and(|u| u < input.booked_on) {
            e.add("recurrence_until", "asset.until_before_booking");
        }
    }
    text_opt(e, "note", input.note.as_ref(), MAX_NOTE);
    a
}

fn entry(
    id: AssetCostId,
    asset: AssetRef,
    input: &AssetCostInput,
    a: Amounts,
    document: Option<DocumentId>,
) -> AssetCost {
    let fuel = input.category == AssetCategory::Vehicle(VehicleCostCategory::Fuel);
    AssetCost {
        id,
        asset,
        booked_on: input.booked_on,
        category: input.category,
        amount: a.amount,
        quantity: a.quantity,
        unit_price: a.unit_price,
        odometer: input.odometer,
        full_tank: fuel.then_some(input.full_tank),
        recurrence: input.recurrence,
        document_id: document,
        note: clean(input.note.as_ref()),
    }
}

/// Legt einen Eintrag in der Unit of Work an; ein Kilometerstand kann als
/// Ablesung (12:00 Uhr lokal am Buchungstag) übernommen werden.
pub(crate) async fn add_in(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    asset: AssetRef,
    input: &AssetCostInput,
    document: Option<DocumentId>,
) -> Result<SaveOutcome<AssetCostId>, AppError> {
    let drive = check_asset(uow, t, asset).await?;
    let mut e = FieldErrors::default();
    let a = validate(&mut e, asset, drive, input);
    e.into_result()?;
    if input.odometer_as_reading
        && let Some(o) = input.odometer
    {
        let meter = vehicle_meter(uow, t, crate::ids::MeterId(asset.id)).await?;
        let cal = Calendar::new(uow.households().get(t).await?.data.timezone);
        let at = cal.to_utc(
            input
                .booked_on
                .and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap_or_default()),
        );
        if let Some(checks) =
            record_odometer(p, uow, t, &meter, at, o, input.confirm_warnings).await?
        {
            return Ok(SaveOutcome::NeedsConfirmation(checks));
        }
    }
    let id = AssetCostId(p.ids.new_uuid());
    uow.asset_costs()
        .create(t, &entry(id, asset, input, a, document))
        .await?;
    Ok(SaveOutcome::Saved(id))
}

impl ListAssetCosts {
    /// Einträge mit Buchungen im Kalenderjahr, wiederkehrende je Buchung
    /// (SPEC_ASSET_COSTS).
    // @impl SPEC_ASSET_COSTS
    #[tracing::instrument(name = "ListAssetCosts", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        asset: AssetRef,
        year: i32,
    ) -> Result<AssetCostBook, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        check_asset(uow.as_mut(), t, asset).await?;
        let all = uow.asset_costs().list(t, asset).await?;
        let period = Period::year(year);
        let entries: Vec<(AssetCost, Decimal)> = all
            .iter()
            .filter(|c| {
                !zaehler_domain::assets::occurrences(c.booked_on, c.recurrence, period).is_empty()
            })
            .map(|c| (c.clone(), c.amount_in(period)))
            .collect();
        let totals = totals(&all, asset.asset_type, period);
        Ok(AssetCostBook {
            year,
            total: totals.iter().map(|(_, v)| *v).sum(),
            entries,
            totals,
        })
    }
}

impl GetAssetCost {
    #[tracing::instrument(name = "GetAssetCost", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: AssetCostId) -> Result<AssetCost, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        uow.asset_costs()
            .get(t, id)
            .await?
            .ok_or(AppError::NotFound)
    }
}

impl AddAssetCost {
    // @impl SPEC_ASSET_COSTS
    #[tracing::instrument(name = "AddAssetCost", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        asset: AssetRef,
        input: AssetCostInput,
    ) -> Result<SaveOutcome<AssetCostId>, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let outcome = add_in(&self.p, uow.as_mut(), t, asset, &input, None).await?;
        if matches!(outcome, SaveOutcome::Saved(_)) {
            uow.commit().await?;
        }
        Ok(outcome)
    }
}

impl UpdateAssetCost {
    /// Ändert Werte eines Eintrags; Anlage und Beleg bleiben.
    // @impl SPEC_ASSET_COSTS
    #[tracing::instrument(name = "UpdateAssetCost", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        id: AssetCostId,
        input: AssetCostInput,
    ) -> Result<(), AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let current = uow
            .asset_costs()
            .get(t, id)
            .await?
            .ok_or(AppError::NotFound)?;
        let drive = check_asset(uow.as_mut(), t, current.asset).await?;
        let mut e = FieldErrors::default();
        let a = validate(
            &mut e,
            current.asset,
            drive,
            &AssetCostInput {
                odometer_as_reading: false,
                ..input.clone()
            },
        );
        e.into_result()?;
        uow.asset_costs()
            .update(t, &entry(id, current.asset, &input, a, current.document_id))
            .await?;
        uow.commit().await?;
        Ok(())
    }
}

impl DeleteAssetCost {
    /// Ein Beleg ohne weiteren Verweis entfernt später das Housekeeping.
    #[tracing::instrument(name = "DeleteAssetCost", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: AssetCostId) -> Result<AssetRef, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let current = uow
            .asset_costs()
            .get(t, id)
            .await?
            .ok_or(AppError::NotFound)?;
        uow.asset_costs().delete(t, id).await?;
        uow.commit().await?;
        Ok(current.asset)
    }
}
