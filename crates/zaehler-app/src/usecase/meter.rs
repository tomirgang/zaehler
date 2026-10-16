//! Messstellen anlegen, ändern, anzeigen; Zählerwechsel.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use zaehler_domain::{
    meter::{Direction, MeterKind},
    reading::{Plausibility, Reading as DomainReading, ReadingSource, check_plausibility},
};

use crate::{
    AppError, FieldErrors, PortError, Role, Tenant,
    ids::{BlobKey, DeviceId, DraftId, MeterId, ReadingId, RegisterId},
    model::{
        DeviceData, Draft, DraftOutcome, MeterAggregate, MeterData, MeterSummary, MonthDay,
        Reading, RegisterData,
    },
    usecase::{Ports, use_case, validate},
};

pub const MAX_METER_NAME: usize = 80;
pub const MAX_UNIT: usize = 16;
pub const MAX_SERIAL: usize = 40;
pub const MAX_REGISTER_CODE: usize = 16;

/// Register im Formular; `id` ist bei vorhandenen Registern gesetzt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterInput {
    pub id: Option<RegisterId>,
    pub code: String,
    pub direction: Direction,
    /// Startstand beim Anlegen (wird beim Ändern ignoriert).
    pub start_value: Option<Decimal>,
}

/// Eingaben für Anlegen und Ändern einer Messstelle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeterInput {
    pub name: String,
    pub kind: MeterKind,
    pub unit: String,
    pub reading_interval_days: Option<u16>,
    pub max_daily_rate: Option<Decimal>,
    pub billing_start: Option<MonthDay>,
    pub serial_number: Option<String>,
    pub rollover_digits: Option<u8>,
    pub decimal_places: u8,
    pub registers: Vec<RegisterInput>,
    /// Zeitpunkt des Einbaus und der Startstände (nur beim Anlegen).
    pub start_at: DateTime<Utc>,
}

/// Messstelle mit allen Ablesungen (chronologisch).
#[derive(Debug, Clone)]
pub struct MeterDetail {
    pub aggregate: MeterAggregate,
    pub readings: Vec<Reading>,
}

/// Eingaben für den Zählerwechsel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapInput {
    pub at: DateTime<Utc>,
    pub end_values: Vec<(RegisterId, Decimal)>,
    pub serial_number: Option<String>,
    pub rollover_digits: Option<u8>,
    pub decimal_places: u8,
    pub start_values: Vec<(RegisterId, Decimal)>,
}

use_case!(CreateMeter, UpdateMeter, ListMeters, GetMeter, SwapDevice);

fn serial(input: Option<&String>) -> Option<String> {
    input
        .map(|s| validate::normalize_serial(s))
        .filter(|s| !s.is_empty())
}

fn billing_start_valid(md: MonthDay) -> bool {
    // 2025 ist kein Schaltjahr: der 29.02. ist als Beginn nicht zulässig.
    NaiveDate::from_ymd_opt(2025, u32::from(md.month), u32::from(md.day)).is_some()
}

// @impl SPEC_MTR_MANAGE
fn validate_input(input: &MeterInput, creating: bool) -> FieldErrors {
    let mut e = FieldErrors::default();
    validate::name(
        &mut e,
        "name",
        &input.name,
        MAX_METER_NAME,
        "meter.name_invalid",
    );
    if input.kind == MeterKind::Custom && input.unit.trim().is_empty() {
        e.add("unit", "meter.unit_required");
    }
    if input.unit.trim().chars().count() > MAX_UNIT {
        e.add_with(
            "unit",
            "meter.unit_invalid",
            vec![("max", MAX_UNIT.to_string())],
        );
    }
    if input
        .reading_interval_days
        .is_some_and(|d| d == 0 || d > 3650)
    {
        e.add("reading_interval_days", "meter.interval_invalid");
    }
    if input.max_daily_rate.is_some_and(|r| r <= Decimal::ZERO) {
        e.add("max_daily_rate", "meter.max_rate_invalid");
    }
    if input
        .billing_start
        .is_some_and(|md| !billing_start_valid(md))
    {
        e.add("billing_start", "meter.billing_start_invalid");
    }
    if input.rollover_digits.is_some_and(|d| d == 0 || d > 9) {
        e.add("rollover_digits", "meter.digits_invalid");
    }
    if input.decimal_places > 3 {
        e.add("decimal_places", "meter.decimals_invalid");
    }
    if input
        .serial_number
        .as_ref()
        .is_some_and(|s| s.trim().chars().count() > MAX_SERIAL)
    {
        e.add_with(
            "serial_number",
            "meter.serial_invalid",
            vec![("max", MAX_SERIAL.to_string())],
        );
    }
    if input.registers.is_empty() {
        e.add("registers", "meter.registers_required");
    }
    let mut seen = std::collections::HashSet::new();
    for (i, r) in input.registers.iter().enumerate() {
        let field = format!("register_{i}_code");
        validate::name(
            &mut e,
            &field,
            &r.code,
            MAX_REGISTER_CODE,
            "meter.register_code_invalid",
        );
        if !seen.insert(r.code.trim().to_lowercase()) {
            e.add(field, "meter.register_code_duplicate");
        }
        if creating && let Some(v) = r.start_value {
            validate::meter_value(
                &mut e,
                &format!("register_{i}_start"),
                v,
                input.decimal_places,
                input.rollover_digits,
            );
        }
    }
    e
}

fn meter_data(input: &MeterInput) -> MeterData {
    let unit = input.unit.trim();
    MeterData {
        name: input.name.trim().to_owned(),
        kind: input.kind,
        unit: if unit.is_empty() {
            input.kind.default_unit().to_owned()
        } else {
            unit.to_owned()
        },
        reading_interval_days: input.reading_interval_days,
        max_daily_rate: input.max_daily_rate,
        billing_start: input.billing_start,
    }
}

/// Prüft, dass Name und Zählernummer im Haushalt frei sind.
async fn check_unique(
    uow: &mut dyn crate::port::UnitOfWork,
    t: &Tenant,
    input: &MeterInput,
    own: Option<(MeterId, Option<DeviceId>)>,
) -> Result<(), AppError> {
    let mut e = FieldErrors::default();
    if let Some(other) = uow.meters().find_by_name(t, input.name.trim()).await?
        && Some(other) != own.map(|o| o.0)
    {
        e.add("name", "meter.name_taken");
    }
    if let Some(s) = serial(input.serial_number.as_ref())
        && let Some((_, device)) = uow.meters().find_by_serial(t, &s).await?
        && Some(device) != own.and_then(|o| o.1)
    {
        e.add("serial_number", "meter.serial_taken");
    }
    e.into_result()
}

impl CreateMeter {
    // @impl SPEC_MTR_MANAGE
    #[tracing::instrument(name = "CreateMeter", skip_all)]
    pub async fn execute(&self, t: &Tenant, input: MeterInput) -> Result<MeterId, AppError> {
        self.create(t, input, None).await
    }

    /// Legt die Messstelle aus einem offenen Entwurf der Foto-Erfassung an:
    /// Der erste Startstand erhält das Foto (`source = photo`), der Entwurf
    /// wird als bestätigt abgeschlossen.
    // @impl SPEC_AI_NEW_METER
    #[tracing::instrument(name = "CreateMeter::execute_from_draft", skip_all)]
    pub async fn execute_from_draft(
        &self,
        t: &Tenant,
        input: MeterInput,
        draft: DraftId,
    ) -> Result<MeterId, AppError> {
        self.create(t, input, Some(draft)).await
    }

    // @impl SPEC_MTR_MANAGE, SPEC_AI_NEW_METER
    async fn create(
        &self,
        t: &Tenant,
        input: MeterInput,
        draft_id: Option<DraftId>,
    ) -> Result<MeterId, AppError> {
        t.require(Role::Owner)?;
        let mut e = validate_input(&input, true);
        if draft_id.is_some() && input.registers.iter().all(|r| r.start_value.is_none()) {
            e.add("register_0_start", "meter.photo_start_required");
        }
        e.into_result()?;
        let mut uow = self.p.uow.begin().await?;
        let draft = match draft_id {
            Some(id) => Some(ready_draft(uow.as_mut(), t, id).await?),
            None => None,
        };
        check_unique(&mut *uow, t, &input, None).await?;

        let now = self.p.clock.now();
        let meter_id = MeterId(self.p.ids.new_uuid());
        uow.meters()
            .create(t, meter_id, &meter_data(&input), now)
            .await?;
        let device_id = DeviceId(self.p.ids.new_uuid());
        let device = DeviceData {
            serial_number: serial(input.serial_number.as_ref()),
            installed_at: input.start_at,
            rollover_digits: input.rollover_digits,
            decimal_places: input.decimal_places,
        };
        uow.meters()
            .add_device(t, meter_id, device_id, &device)
            .await?;

        let mut starts = Vec::new();
        for r in &input.registers {
            let register_id = RegisterId(self.p.ids.new_uuid());
            let data = RegisterData {
                code: r.code.trim().to_owned(),
                direction: r.direction,
            };
            uow.meters()
                .add_register(t, meter_id, register_id, &data)
                .await?;
            if let Some(value) = r.start_value {
                starts.push(new_reading(
                    &self.p,
                    t,
                    register_id,
                    device_id,
                    input.start_at,
                    value,
                ));
            }
        }
        // Foto des Entwurfs wird Belegfoto des ersten Startstands
        let photo = match (&draft, starts.first_mut()) {
            (Some(d), Some(first)) => {
                let (key, corrected) = take_draft_photo(t, d, first);
                Some((d, first.id, key, corrected))
            }
            _ => None,
        };
        if !starts.is_empty() {
            uow.readings().insert(t, &starts).await?;
        }
        if let Some((d, reading, key, corrected)) = &photo {
            self.p
                .blobs
                .copy(&d.photo_key, key)
                .await
                .map_err(AppError::from_storage)?;
            uow.drafts()
                .finish(
                    t,
                    d.id,
                    DraftOutcome::Confirmed {
                        reading: *reading,
                        corrected: *corrected,
                    },
                    now,
                )
                .await?;
        }
        uow.commit().await?;
        if let Some((d, _, _, corrected)) = photo {
            metrics::counter!("zaehler_readings_created_total", "source" => ReadingSource::Photo.as_str())
                .increment(1);
            if let Some(c) = corrected {
                metrics::counter!("zaehler_ai_corrections_total", "corrected" => c.to_string())
                    .increment(1);
            }
            // Entwurfsfoto erst nach dem commit (SPEC_ARCH_UOW)
            if let Err(e) = self.p.blobs.delete(&d.photo_key).await {
                tracing::warn!(error = %e, draft = %d.id, "draft photo not deleted");
            }
        }
        Ok(meter_id)
    }
}

/// Offener Entwurf mit abgeschlossener Analyse.
async fn ready_draft(
    uow: &mut dyn crate::port::UnitOfWork,
    t: &Tenant,
    id: DraftId,
) -> Result<Draft, AppError> {
    let d = uow
        .drafts()
        .get_open(t, id)
        .await?
        .ok_or(AppError::NotFound)?;
    if d.status.is_ready() {
        Ok(d)
    } else {
        Err(AppError::Conflict("capture.not_ready"))
    }
}

/// Macht das Foto des Entwurfs zum Belegfoto der Ablesung; liefert den neuen
/// Schlüssel und ob der Wert vom Vorschlag abweicht.
// @impl SPEC_AI_NEW_METER
fn take_draft_photo(t: &Tenant, d: &Draft, r: &mut Reading) -> (BlobKey, Option<bool>) {
    let key = BlobKey::reading_photo(t.household_id(), r.id);
    let suggestion = d.suggestion.as_ref();
    r.source = ReadingSource::Photo;
    r.photo_key = Some(key.clone());
    r.ai_confidence = suggestion.map(|s| s.confidence);
    (key, suggestion.map(|s| s.value != Some(r.value)))
}

fn new_reading(
    p: &Ports,
    t: &Tenant,
    register: RegisterId,
    device: DeviceId,
    at: DateTime<Utc>,
    value: Decimal,
) -> Reading {
    Reading {
        id: ReadingId(p.ids.new_uuid()),
        register_id: register,
        device_id: device,
        read_at: at,
        value,
        source: ReadingSource::Manual,
        note: None,
        photo_key: None,
        ai_confidence: None,
        created_by: Some(t.user_id()),
    }
}

impl UpdateMeter {
    /// Art, Einheit und Register-Richtung sind nach der ersten Ablesung fest;
    /// Register werden umbenannt oder ergänzt, nie gelöscht.
    // @impl SPEC_MTR_MANAGE
    #[tracing::instrument(name = "UpdateMeter", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        id: MeterId,
        input: MeterInput,
    ) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut e = validate_input(&input, false);
        let mut uow = self.p.uow.begin().await?;
        let agg = uow.meters().get(t, id).await?.ok_or(AppError::NotFound)?;
        let has_readings = !uow.readings().list_for_meter(t, id).await?.is_empty();
        let data = meter_data(&input);

        if has_readings {
            if data.kind != agg.meter.data.kind {
                e.add("kind", "meter.kind_locked");
            }
            if data.unit != agg.meter.data.unit {
                e.add("unit", "meter.unit_locked");
            }
        }
        for existing in &agg.registers {
            match input
                .registers
                .iter()
                .position(|r| r.id == Some(existing.id))
            {
                None => e.add("registers", "meter.register_locked"),
                Some(i)
                    if has_readings && input.registers[i].direction != existing.data.direction =>
                {
                    e.add(format!("register_{i}_direction"), "meter.direction_locked");
                }
                Some(_) => {}
            }
        }
        if input
            .registers
            .iter()
            .any(|r| r.id.is_some_and(|rid| agg.register(rid).is_none()))
        {
            return Err(AppError::NotFound);
        }
        e.into_result()?;
        let active = agg.active_device().cloned();
        check_unique(
            &mut *uow,
            t,
            &input,
            Some((id, active.as_ref().map(|d| d.id))),
        )
        .await?;

        uow.meters().update(t, id, &data).await?;
        for r in &input.registers {
            if let Some(rid) = r.id {
                let code = r.code.trim();
                if agg.register(rid).is_some_and(|x| x.data.code != code) {
                    uow.meters().rename_register(t, rid, code).await?;
                }
            } else {
                let data = RegisterData {
                    code: r.code.trim().to_owned(),
                    direction: r.direction,
                };
                uow.meters()
                    .add_register(t, id, RegisterId(self.p.ids.new_uuid()), &data)
                    .await?;
            }
        }
        if let Some(device) = active {
            let updated = DeviceData {
                serial_number: serial(input.serial_number.as_ref()),
                rollover_digits: input.rollover_digits,
                decimal_places: input.decimal_places,
                ..device.data
            };
            uow.meters().update_device(t, device.id, &updated).await?;
        }
        uow.commit().await?;
        Ok(())
    }
}

impl ListMeters {
    #[tracing::instrument(name = "ListMeters", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<Vec<MeterSummary>, AppError> {
        let mut uow = self.p.uow.begin().await?;
        Ok(uow.meters().list(t).await?)
    }
}

impl GetMeter {
    #[tracing::instrument(name = "GetMeter", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: MeterId) -> Result<MeterDetail, AppError> {
        let mut uow = self.p.uow.begin().await?;
        let aggregate = uow.meters().get(t, id).await?.ok_or(AppError::NotFound)?;
        let readings = uow.readings().list_for_meter(t, id).await?;
        Ok(MeterDetail {
            aggregate,
            readings,
        })
    }
}

impl SwapDevice {
    /// Endstände des alten und Startstände des neuen Geräts zum selben Zeitpunkt.
    // @impl SPEC_MTR_SWAP_FLOW, SPEC_CONS_DEVICE_SWAP
    #[tracing::instrument(name = "SwapDevice", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        id: MeterId,
        input: SwapInput,
    ) -> Result<DeviceId, AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let agg = uow.meters().get(t, id).await?.ok_or(AppError::NotFound)?;
        let old = agg
            .active_device()
            .cloned()
            .ok_or(AppError::Conflict("meter.no_active_device"))?;
        let readings = uow.readings().list_for_meter(t, id).await?;

        let mut e = FieldErrors::default();
        if readings.iter().any(|r| r.read_at >= input.at) || input.at <= old.data.installed_at {
            e.add("at", "swap.before_last_reading");
        }
        if input.rollover_digits.is_some_and(|d| d == 0 || d > 9) {
            e.add("rollover_digits", "meter.digits_invalid");
        }
        if input.decimal_places > 3 {
            e.add("decimal_places", "meter.decimals_invalid");
        }
        for reg in &agg.registers {
            let end_field = format!("end_{}", reg.id);
            match input.end_values.iter().find(|(r, _)| *r == reg.id) {
                None => e.add(&end_field, "swap.end_required"),
                Some((_, value)) => {
                    validate::meter_value(
                        &mut e,
                        &end_field,
                        *value,
                        old.data.decimal_places,
                        old.data.rollover_digits,
                    );
                    let prev = last_of(&readings, reg.id, old.id).map(|r| to_domain(r, &old));
                    let candidate = DomainReading {
                        at: input.at,
                        value: *value,
                        device_id: old.id.0,
                        rollover_digits: old.data.rollover_digits,
                    };
                    if check_plausibility(prev.as_ref(), &candidate, None)
                        == Plausibility::Decreasing
                    {
                        e.add(&end_field, "reading.decreasing");
                    }
                }
            }
            let start_field = format!("start_{}", reg.id);
            match input.start_values.iter().find(|(r, _)| *r == reg.id) {
                None => e.add(start_field, "swap.start_required"),
                Some((_, value)) => {
                    validate::meter_value(
                        &mut e,
                        &start_field,
                        *value,
                        input.decimal_places,
                        input.rollover_digits,
                    );
                }
            }
        }
        e.into_result()?;
        if let Some(s) = serial(input.serial_number.as_ref())
            && uow.meters().find_by_serial(t, &s).await?.is_some()
        {
            return Err(AppError::field("serial_number", "meter.serial_taken"));
        }

        let ends: Vec<Reading> = input
            .end_values
            .iter()
            .map(|(reg, v)| new_reading(&self.p, t, *reg, old.id, input.at, *v))
            .collect();
        uow.readings().insert(t, &ends).await?;
        uow.meters().retire_device(t, old.id, input.at).await?;
        let new_id = DeviceId(self.p.ids.new_uuid());
        let device = DeviceData {
            serial_number: serial(input.serial_number.as_ref()),
            installed_at: input.at,
            rollover_digits: input.rollover_digits,
            decimal_places: input.decimal_places,
        };
        uow.meters().add_device(t, id, new_id, &device).await?;
        let starts: Vec<Reading> = input
            .start_values
            .iter()
            .map(|(reg, v)| new_reading(&self.p, t, *reg, new_id, input.at, *v))
            .collect();
        uow.readings()
            .insert(t, &starts)
            .await
            .map_err(|err| match err {
                PortError::Conflict(_) => AppError::Conflict("reading.duplicate"),
                other => other.into(),
            })?;
        uow.commit().await?;
        Ok(new_id)
    }
}

/// Letzte Ablesung eines Registers auf einem Gerät.
pub(crate) fn last_of(
    readings: &[Reading],
    register: RegisterId,
    device: DeviceId,
) -> Option<&Reading> {
    readings
        .iter()
        .filter(|r| r.register_id == register && r.device_id == device)
        .max_by_key(|r| r.read_at)
}

/// Zeitliche Nachbarn eines Zeitpunkts: letzte Ablesung davor und erste
/// danach, je Register und Gerät.
pub(crate) fn neighbours(
    readings: &[Reading],
    register: RegisterId,
    device: DeviceId,
    at: DateTime<Utc>,
) -> (Option<&Reading>, Option<&Reading>) {
    let same = || {
        readings
            .iter()
            .filter(move |r| r.register_id == register && r.device_id == device)
    };
    (
        same().filter(|r| r.read_at < at).max_by_key(|r| r.read_at),
        same().filter(|r| r.read_at > at).min_by_key(|r| r.read_at),
    )
}

pub(crate) fn to_domain(r: &Reading, device: &crate::model::Device) -> DomainReading {
    DomainReading {
        at: r.read_at,
        value: r.value,
        device_id: device.id.0,
        rollover_digits: device.data.rollover_digits,
    }
}
