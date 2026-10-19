//! Fahrzeuge: Stammdaten, Verbrauchsprofile, Anschaffung und Ladevorgänge
//! (SPEC_CAR_VEHICLES, SPEC_CAR_ENERGY_COST).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use zaehler_domain::{
    meter::{Direction, MeterKind, MeterPurpose},
    reading::ReadingSource,
    vehicle::{ConsumptionProfile, DriveType, Holding},
};

use crate::{
    AppError, FieldErrors, PortError, Role, Tenant,
    ids::{ChargingId, DocumentId, MeterId, ProfileId},
    model::{
        AssetRef, ChargingKind, ChargingSession, EntrySource, MeterAggregate, MeterTargets,
        ProfileRow, Vehicle, VehicleData,
    },
    port::UnitOfWork,
    usecase::{
        Ports,
        contract::{clean, money, text_opt},
        meter::{MeterInput, RegisterInput, insert_meter, validate_new},
        reading::{ReadingExtras, ReadingsInput, RecordOutcome, RegisterCheck, record_in},
        use_case,
    },
};

pub const MAX_PLATE: usize = 20;
pub const MAX_MODEL: usize = 80;
pub const MAX_PROVIDER: usize = 80;
/// Code des Kilometerregisters neuer Fahrzeuge.
pub const ODOMETER_REGISTER: &str = "km";

use_case!(
    /// Fahrzeuge des Haushalts mit ihren Messstellen (Viewer).
    ListVehicles,
    /// Fahrzeug mit Messstelle und Ladevorgängen (Viewer).
    GetVehicle,
    /// Messstelle der Art `vehicle` und Fahrzeug anlegen (Owner).
    CreateVehicle,
    /// Stammdaten ändern oder für eine bestehende Fahrzeug-Messstelle
    /// anlegen (Owner).
    UpdateVehicle,
    /// Verbrauchsprofil ab einem Tag (Owner).
    SetConsumptionProfile,
    /// Verbrauchsprofil löschen (Owner).
    DeleteConsumptionProfile,
    /// Anschaffung und Verkauf setzen oder entfernen (Owner).
    SetAcquisition,
    /// Externen Ladevorgang erfassen (Member).
    RecordCharging,
    /// Ladevorgang löschen (Member).
    DeleteCharging,
);

/// Stammdaten aus dem Formular.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VehicleInput {
    pub plate: String,
    pub model: String,
    pub drive_type: DriveType,
    pub home_charging_meter_id: Option<MeterId>,
    pub pv_share_pct: Option<Decimal>,
}

/// Neues Fahrzeug mit Messstelle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVehicle {
    pub name: String,
    pub vehicle: VehicleInput,
    /// Kilometerstand bei Beginn der Erfassung
    pub odometer: Decimal,
    pub start_at: DateTime<Utc>,
}

/// Messstelle mit optionalem Fahrzeug-Datensatz.
#[derive(Debug, Clone)]
pub struct VehicleSummary {
    pub meter: crate::model::Meter,
    pub vehicle: Option<Vehicle>,
}

/// Fahrzeug mit Messstelle und Ladevorgängen.
#[derive(Debug, Clone)]
pub struct VehicleDetail {
    pub meter: MeterAggregate,
    pub vehicle: Option<Vehicle>,
    pub charging: Vec<ChargingSession>,
    pub initial_odometer: Option<Decimal>,
}

/// Ergebnis einer Erfassung mit optional übernommenem Kilometerstand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome<T> {
    Saved(T),
    /// Plausibilitätswarnungen zum Kilometerstand; nichts gespeichert
    NeedsConfirmation(Vec<RegisterCheck>),
}

/// Externer Ladevorgang aus dem Formular oder einem Beleg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChargingInput {
    pub charged_at: DateTime<Utc>,
    pub energy: Decimal,
    pub amount: Decimal,
    pub odometer: Option<Decimal>,
    pub provider: Option<String>,
    pub kind: ChargingKind,
    /// Kilometerstand zusätzlich als Ablesung speichern
    pub odometer_as_reading: bool,
    pub confirm_warnings: bool,
}

/// Messstelle der Art `vehicle` im Haushalt.
pub(crate) async fn vehicle_meter(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meter: MeterId,
) -> Result<MeterAggregate, AppError> {
    uow.meters()
        .get(t, meter)
        .await?
        .filter(|m| m.meter.data.kind == MeterKind::Vehicle)
        .ok_or(AppError::NotFound)
}

/// Fahrzeug mit Datensatz.
pub(crate) async fn load_vehicle(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meter: MeterId,
) -> Result<Vehicle, AppError> {
    uow.vehicles()
        .get(t, meter)
        .await?
        .ok_or(AppError::NotFound)
}

fn validate_vehicle(e: &mut FieldErrors, v: &VehicleInput) {
    text_opt(e, "plate", Some(&v.plate), MAX_PLATE);
    text_opt(e, "model", Some(&v.model), MAX_MODEL);
    if v.pv_share_pct
        .is_some_and(|p| p < Decimal::ZERO || p > Decimal::ONE_HUNDRED)
    {
        e.add("pv_share_pct", "vehicle.share_invalid");
    }
    if v.drive_type == DriveType::Combustion && v.home_charging_meter_id.is_some() {
        e.add("home_charging_meter_id", "vehicle.home_charging_invalid");
    }
}

/// Heimlade-Messstelle: Strom im Haushalt; Zweck Wallbox (gemessen) oder
/// allgemein (Hausstrom, Schätzung), kein Fahrzeug selbst.
async fn check_home_meter(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meter: Option<MeterId>,
) -> Result<(), AppError> {
    let Some(id) = meter else {
        return Ok(());
    };
    let ok = uow.meters().get(t, id).await?.is_some_and(|m| {
        m.meter.data.kind == MeterKind::Electricity
            && matches!(
                m.meter.data.purpose,
                MeterPurpose::Wallbox | MeterPurpose::General
            )
    });
    if ok {
        Ok(())
    } else {
        Err(AppError::field(
            "home_charging_meter_id",
            "vehicle.home_charging_invalid",
        ))
    }
}

fn vehicle_data(v: &VehicleInput) -> VehicleData {
    VehicleData {
        plate: v.plate.trim().to_owned(),
        model: v.model.trim().to_owned(),
        drive_type: v.drive_type,
        home_charging_meter_id: v.home_charging_meter_id,
        pv_share_pct: v.pv_share_pct,
    }
}

/// Kilometerstand als Ablesung der Fahrzeug-Messstelle (Plausibilität wie
/// bei der Erfassung).
pub(crate) async fn record_odometer(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meter: &MeterAggregate,
    at: DateTime<Utc>,
    odometer: Decimal,
    confirmed: bool,
) -> Result<Option<Vec<RegisterCheck>>, AppError> {
    let register = meter
        .registers
        .iter()
        .find(|r| r.data.direction == Direction::Import)
        .ok_or(AppError::NotFound)?;
    let input = ReadingsInput {
        meter_id: meter.meter.id,
        at,
        values: vec![(register.id, odometer)],
        note: None,
    };
    match record_in(
        p,
        uow,
        t,
        &input,
        confirmed,
        ReadingSource::Manual,
        ReadingExtras::default(),
    )
    .await?
    {
        RecordOutcome::Saved(_) => Ok(None),
        RecordOutcome::NeedsConfirmation(checks) => Ok(Some(checks)),
    }
}

impl ListVehicles {
    #[tracing::instrument(name = "ListVehicles", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<Vec<VehicleSummary>, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let vehicles = uow.vehicles().list(t).await?;
        Ok(uow
            .meters()
            .list(t)
            .await?
            .into_iter()
            .filter(|m| m.meter.data.kind == MeterKind::Vehicle)
            .map(|m| VehicleSummary {
                vehicle: vehicles.iter().find(|v| v.meter_id == m.meter.id).cloned(),
                meter: m.meter,
            })
            .collect())
    }
}

impl GetVehicle {
    #[tracing::instrument(name = "GetVehicle", skip_all)]
    pub async fn execute(&self, t: &Tenant, meter: MeterId) -> Result<VehicleDetail, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let agg = vehicle_meter(uow.as_mut(), t, meter).await?;
        Ok(VehicleDetail {
            vehicle: uow.vehicles().get(t, meter).await?,
            charging: uow.vehicles().charging(t, meter).await?,
            initial_odometer: uow
                .readings()
                .list_for_meter(t, meter)
                .await?
                .first()
                .map(|r| r.value),
            meter: agg,
        })
    }
}

impl CreateVehicle {
    /// Messstelle (Einheit km, Register `km`, Startstand) und Fahrzeug in
    /// einer Unit of Work (SPEC_CAR_VEHICLES).
    // @impl SPEC_CAR_VEHICLES
    #[tracing::instrument(name = "CreateVehicle", skip_all)]
    pub async fn execute(&self, t: &Tenant, input: NewVehicle) -> Result<MeterId, AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let id = self.execute_in(t, input, uow.as_mut()).await?;
        uow.commit().await?;
        Ok(id)
    }

    /// Legt Messstelle und Fahrzeug in einer vom Aufrufer verwalteten
    /// Transaktion an.
    pub async fn execute_in(
        &self,
        t: &Tenant,
        input: NewVehicle,
        uow: &mut dyn UnitOfWork,
    ) -> Result<MeterId, AppError> {
        t.require(Role::Owner)?;
        let meter = MeterInput {
            name: input.name.clone(),
            kind: MeterKind::Vehicle,
            unit: "km".into(),
            schedule: None,
            max_daily_rate: None,
            billing_start: None,
            purpose: MeterPurpose::General,
            parent_meter_id: None,
            targets: MeterTargets::default(),
            serial_number: None,
            rollover_digits: None,
            decimal_places: 0,
            registers: vec![RegisterInput {
                id: None,
                code: ODOMETER_REGISTER.into(),
                direction: Direction::Import,
                start_value: Some(input.odometer),
            }],
            start_at: input.start_at,
        };
        let mut e = match validate_new(&meter) {
            Ok(()) => FieldErrors::default(),
            Err(AppError::Validation(e)) => e,
            Err(other) => return Err(other),
        };
        validate_vehicle(&mut e, &input.vehicle);
        e.into_result()?;
        check_home_meter(uow, t, input.vehicle.home_charging_meter_id).await?;
        let (id, _) = insert_meter(&self.p, uow, t, &meter, None).await?;
        uow.vehicles()
            .save(t, id, &vehicle_data(&input.vehicle))
            .await?;
        Ok(id)
    }
}

impl UpdateVehicle {
    /// Die Antriebsart ist nach dem ersten Kostenbuch-Eintrag oder
    /// Ladevorgang gesperrt (`vehicle.drive_type_locked`).
    // @impl SPEC_CAR_VEHICLES
    #[tracing::instrument(name = "UpdateVehicle", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        meter: MeterId,
        input: VehicleInput,
    ) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut e = FieldErrors::default();
        validate_vehicle(&mut e, &input);
        e.into_result()?;
        let mut uow = self.p.uow.begin().await?;
        vehicle_meter(uow.as_mut(), t, meter).await?;
        if let Some(current) = uow.vehicles().get(t, meter).await?
            && current.data.drive_type != input.drive_type
        {
            let booked = !uow.vehicles().charging(t, meter).await?.is_empty()
                || !uow
                    .asset_costs()
                    .list(t, AssetRef::vehicle(meter))
                    .await?
                    .is_empty();
            if booked {
                return Err(AppError::Conflict("vehicle.drive_type_locked"));
            }
        }
        check_home_meter(uow.as_mut(), t, input.home_charging_meter_id).await?;
        uow.vehicles().save(t, meter, &vehicle_data(&input)).await?;
        uow.commit().await?;
        Ok(())
    }
}

fn percent(e: &mut FieldErrors, field: &str, v: Decimal) {
    if v < Decimal::ZERO || v > Decimal::ONE_HUNDRED {
        e.add(field, "vehicle.share_invalid");
    } else {
        money(e, field, v, 3);
    }
}

impl SetConsumptionProfile {
    /// Durchschnittsverbrauch, Ladeverlust und elektrischer Fahranteil ab
    /// `valid_from` (REQ_CAR_009, SPEC_CAR_FUEL_PRICE).
    // @impl SPEC_CAR_ENERGY_COST
    #[tracing::instrument(name = "SetConsumptionProfile", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        meter: MeterId,
        mut input: ConsumptionProfile,
    ) -> Result<ProfileId, AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let vehicle = load_vehicle(uow.as_mut(), t, meter).await?;
        let mut e = FieldErrors::default();
        let supported = match input.energy {
            zaehler_domain::vehicle::ProfileEnergy::Electric => vehicle.data.drive_type.charges(),
            zaehler_domain::vehicle::ProfileEnergy::Fuel => vehicle.data.drive_type.fuels(),
        };
        if !supported {
            e.add("profile_energy", "vehicle.profile_energy_invalid");
        }
        if input.kwh_per_100km <= Decimal::ZERO || input.kwh_per_100km > Decimal::from(200) {
            e.add("profile_kwh", "vehicle.consumption_invalid");
        } else {
            money(&mut e, "profile_kwh", input.kwh_per_100km, 3);
        }
        percent(&mut e, "profile_loss", input.charging_loss_pct);
        if vehicle.data.drive_type == DriveType::Electric {
            input.electric_share_pct = None;
        }
        if let Some(s) = input.electric_share_pct {
            percent(&mut e, "profile_share", s);
        }
        e.into_result()?;
        let id = ProfileId(self.p.ids.new_uuid());
        uow.vehicles()
            .add_profile(t, meter, &ProfileRow { id, profile: input })
            .await
            .map_err(|e| match e {
                PortError::Conflict(_) => {
                    AppError::field("profile_valid_from", "vehicle.profile_exists")
                }
                other => other.into(),
            })?;
        uow.commit().await?;
        Ok(id)
    }
}

impl DeleteConsumptionProfile {
    #[tracing::instrument(name = "DeleteConsumptionProfile", skip_all)]
    pub async fn execute(&self, t: &Tenant, meter: MeterId, id: ProfileId) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        uow.vehicles().delete_profile(t, meter, id).await?;
        uow.commit().await?;
        Ok(())
    }
}

impl SetAcquisition {
    /// Kaufdatum, Preis, Restwert, geplantes Ende und Verkauf (REQ_CAR_004);
    /// `None` entfernt die Anschaffung.
    // @impl SPEC_CAR_DEPRECIATION
    #[tracing::instrument(name = "SetAcquisition", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        meter: MeterId,
        input: Option<Holding>,
    ) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        self.execute_in(t, meter, input, uow.as_mut()).await?;
        uow.commit().await?;
        Ok(())
    }

    /// Validiert und speichert die Anschaffung in der Transaktion des
    /// Aufrufers.
    pub async fn execute_in(
        &self,
        t: &Tenant,
        meter: MeterId,
        input: Option<Holding>,
        uow: &mut dyn UnitOfWork,
    ) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        if let Some(h) = &input {
            let mut e = FieldErrors::default();
            validate_holding_modes(&mut e, h);
            if h.sold_on.is_some_and(|s| s < h.purchased_on) {
                e.add("sold_on", "vehicle.end_before_purchase");
            }
            match (h.sold_on, h.sale_price) {
                (None, Some(_)) => e.add("sold_on", "form.date_required"),
                (_, Some(v)) if v < Decimal::ZERO => {
                    e.add("sale_price", "contract.amount_negative");
                }
                (_, Some(v)) => money(&mut e, "sale_price", v, 2),
                _ => {}
            }
            e.into_result()?;
        }
        load_vehicle(uow, t, meter).await?;
        uow.vehicles().set_holding(t, meter, input.as_ref()).await?;
        Ok(())
    }
}

/// Werte beider Modi werden geprüft, soweit angegeben; die des gewählten
/// Modus sind Pflicht (SPEC_CAR_MILEAGE_UI).
// @impl SPEC_CAR_MILEAGE_UI
fn validate_holding_modes(e: &mut FieldErrors, h: &Holding) {
    use zaehler_domain::vehicle::DepreciationMode;
    let amounts = [
        ("price", Some(h.price)),
        ("residual_value", h.residual_value),
        ("end_value", h.end_value),
    ];
    for (field, v) in amounts {
        if let Some(v) = v {
            if v < Decimal::ZERO {
                e.add(field, "contract.amount_negative");
            }
            money(e, field, v, 2);
        }
    }
    if h.planned_end.is_some_and(|p| p <= h.purchased_on) {
        e.add("planned_end", "vehicle.end_before_purchase");
    }
    for (field, v) in [
        ("odometer_at_purchase", h.odometer_at_purchase),
        ("lifetime_km", h.lifetime_km),
    ] {
        if let Some(v) = v {
            if v < Decimal::ZERO {
                e.add(field, "reading.negative");
            }
            money(e, field, v, 3);
        }
    }
    if let (Some(s0), Some(s)) = (h.odometer_at_purchase, h.lifetime_km)
        && s <= s0
    {
        e.add("lifetime_km", "vehicle.lifetime_below_purchase");
    }
    if h.end_value.is_some_and(|w| w > h.price) {
        e.add("end_value", "vehicle.end_value_above_price");
    }
    match h.mode {
        DepreciationMode::Time => {
            if h.residual_value.is_none() {
                e.add("residual_value", "form.number_required");
            }
            if h.planned_end.is_none() {
                e.add("planned_end", "form.date_required");
            }
        }
        DepreciationMode::Mileage => {
            if h.odometer_at_purchase.is_none() {
                e.add("odometer_at_purchase", "form.number_required");
            }
            if h.lifetime_km.is_none() {
                e.add("lifetime_km", "form.number_required");
            }
        }
    }
}

/// Speichert einen Ladevorgang in der Unit of Work.
pub(crate) async fn record_charging_in(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meter: MeterId,
    input: &ChargingInput,
    source: EntrySource,
    document: Option<DocumentId>,
) -> Result<SaveOutcome<ChargingId>, AppError> {
    let agg = vehicle_meter(uow, t, meter).await?;
    let vehicle = load_vehicle(uow, t, meter).await?;
    let mut e = FieldErrors::default();
    if !vehicle.data.drive_type.charges() {
        e.add("drive_type", "vehicle.no_charging");
    }
    if input.energy <= Decimal::ZERO {
        e.add("energy", "vehicle.energy_invalid");
    }
    money(&mut e, "energy", input.energy, 3);
    if input.amount < Decimal::ZERO {
        e.add("amount", "contract.amount_negative");
    }
    money(&mut e, "amount", input.amount, 2);
    if let Some(o) = input.odometer {
        if o < Decimal::ZERO {
            e.add("odometer", "reading.negative");
        }
        money(&mut e, "odometer", o, 3);
    }
    if input.odometer_as_reading && input.odometer.is_none() {
        e.add("odometer", "form.number_required");
    }
    text_opt(&mut e, "provider", input.provider.as_ref(), MAX_PROVIDER);
    e.into_result()?;
    if input.odometer_as_reading
        && let Some(o) = input.odometer
        && let Some(checks) =
            record_odometer(p, uow, t, &agg, input.charged_at, o, input.confirm_warnings).await?
    {
        return Ok(SaveOutcome::NeedsConfirmation(checks));
    }
    let id = ChargingId(p.ids.new_uuid());
    uow.vehicles()
        .add_charging(
            t,
            &ChargingSession {
                id,
                vehicle: meter,
                charged_at: input.charged_at,
                energy: input.energy,
                amount: input.amount,
                odometer: input.odometer,
                provider: clean(input.provider.as_ref()),
                kind: input.kind,
                source,
                document_id: document,
            },
        )
        .await?;
    Ok(SaveOutcome::Saved(id))
}

impl RecordCharging {
    /// Energie, Betrag (0 bei kostenlosem Laden), optional Kilometerstand,
    /// Anbieter und Ladeart (REQ_CAR_010); nicht für Verbrenner.
    // @impl SPEC_CAR_VEHICLES, SPEC_CAR_ENERGY_COST
    #[tracing::instrument(name = "RecordCharging", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        meter: MeterId,
        input: ChargingInput,
    ) -> Result<SaveOutcome<ChargingId>, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let outcome = record_charging_in(
            &self.p,
            uow.as_mut(),
            t,
            meter,
            &input,
            EntrySource::Form,
            None,
        )
        .await?;
        if matches!(outcome, SaveOutcome::Saved(_)) {
            uow.commit().await?;
        }
        Ok(outcome)
    }
}

impl DeleteCharging {
    #[tracing::instrument(name = "DeleteCharging", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        meter: MeterId,
        id: ChargingId,
    ) -> Result<(), AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        uow.vehicles().delete_charging(t, meter, id).await?;
        uow.commit().await?;
        Ok(())
    }
}

impl<T> SaveOutcome<T> {
    /// Gespeicherten Wert abbilden.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> SaveOutcome<U> {
        match self {
            Self::Saved(x) => SaveOutcome::Saved(f(x)),
            Self::NeedsConfirmation(c) => SaveOutcome::NeedsConfirmation(c),
        }
    }
}
