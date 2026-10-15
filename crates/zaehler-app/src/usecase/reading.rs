//! Zählerstände prüfen, erfassen und löschen.

use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use zaehler_domain::reading::{
    Plausibility, Reading as DomainReading, ReadingSource, check_plausibility,
};

use crate::{
    AppError, FieldErrors, PortError, Role, Tenant,
    ids::{BlobKey, MeterId, ReadingId, RegisterId},
    model::{MeterAggregate, Reading},
    port::UnitOfWork,
    usecase::{
        Ports,
        meter::{last_of, to_domain},
        use_case, validate,
    },
};

pub const MAX_NOTE: usize = 500;

/// Erfasste Werte einer Messstelle zu einem Zeitpunkt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingsInput {
    pub meter_id: MeterId,
    pub at: DateTime<Utc>,
    pub values: Vec<(RegisterId, Decimal)>,
    pub note: Option<String>,
}

/// Ergebnis der Plausibilitätsprüfung je Register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterCheck {
    pub register_id: RegisterId,
    pub plausibility: Plausibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordOutcome {
    Saved(Vec<ReadingId>),
    /// Warnungen liegen vor; Speichern erst nach Bestätigung.
    NeedsConfirmation(Vec<RegisterCheck>),
}

use_case!(
    CheckReadings,
    RecordReadings,
    DeleteReading,
    GetReadingPhoto
);

/// Formale Prüfung und Plausibilität aller Werte gegen die letzte Ablesung
/// des aktiven Geräts.
// @impl SPEC_RDG_FORM, SPEC_RDG_PLAUSIBILITY
pub(crate) fn check_all(
    agg: &MeterAggregate,
    readings: &[Reading],
    input: &ReadingsInput,
) -> Result<Vec<RegisterCheck>, AppError> {
    let device = agg
        .active_device()
        .ok_or(AppError::Conflict("meter.no_active_device"))?;
    let mut e = FieldErrors::default();
    let mut checks = Vec::new();
    for (register_id, value) in &input.values {
        if agg.register(*register_id).is_none() {
            return Err(AppError::NotFound);
        }
        let field = format!("value_{register_id}");
        validate::meter_value(
            &mut e,
            &field,
            *value,
            device.data.decimal_places,
            device.data.rollover_digits,
        );
        let prev = last_of(readings, *register_id, device.id).map(|r| to_domain(r, device));
        let candidate = DomainReading {
            at: input.at,
            value: *value,
            device_id: device.id.0,
            rollover_digits: device.data.rollover_digits,
        };
        let plausibility = if input.at < device.data.installed_at {
            Plausibility::OutOfOrder
        } else {
            check_plausibility(prev.as_ref(), &candidate, agg.meter.data.max_daily_rate)
        };
        checks.push(RegisterCheck {
            register_id: *register_id,
            plausibility,
        });
    }
    e.into_result()?;
    Ok(checks)
}

impl CheckReadings {
    /// Prüfung ohne Speichern (Hinweise im Formular).
    pub async fn execute(
        &self,
        t: &Tenant,
        input: &ReadingsInput,
    ) -> Result<Vec<RegisterCheck>, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let agg = uow
            .meters()
            .get(t, input.meter_id)
            .await?
            .ok_or(AppError::NotFound)?;
        let readings = uow.readings().list_for_meter(t, input.meter_id).await?;
        check_all(&agg, &readings, input)
    }
}

/// Zusatzangaben einer Ablesung aus der Foto-Erfassung.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReadingExtras {
    /// Vorgegebene ID (nur bei genau einem Wert)
    pub id: Option<ReadingId>,
    pub photo_key: Option<BlobKey>,
    pub ai_confidence: Option<Decimal>,
}

/// Prüft und fügt Ablesungen in die Unit of Work ein, ohne `commit`. Mit
/// Warnungen und ohne `confirmed` wird nichts eingefügt.
// @impl SPEC_RDG_FORM, SPEC_RDG_PLAUSIBILITY
pub(crate) async fn record_in(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    input: &ReadingsInput,
    confirmed: bool,
    source: ReadingSource,
    extras: ReadingExtras,
) -> Result<RecordOutcome, AppError> {
    let mut e = FieldErrors::default();
    if input.values.is_empty() {
        e.add("values", "reading.values_required");
    }
    if input.at > p.clock.now() + Duration::minutes(5) {
        e.add("at", "reading.in_future");
    }
    if input
        .note
        .as_ref()
        .is_some_and(|n| n.chars().count() > MAX_NOTE)
    {
        e.add_with(
            "note",
            "reading.note_too_long",
            vec![("max", MAX_NOTE.to_string())],
        );
    }
    e.into_result()?;

    let agg = uow
        .meters()
        .get(t, input.meter_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let readings = uow.readings().list_for_meter(t, input.meter_id).await?;
    let checks = check_all(&agg, &readings, input)?;
    if !confirmed && checks.iter().any(|c| c.plausibility != Plausibility::Ok) {
        return Ok(RecordOutcome::NeedsConfirmation(checks));
    }
    let device = agg
        .active_device()
        .ok_or(AppError::Conflict("meter.no_active_device"))?;
    let note = input
        .note
        .as_ref()
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty());
    let fixed_id = extras.id.filter(|_| input.values.len() == 1);
    let new: Vec<Reading> = input
        .values
        .iter()
        .map(|(register_id, value)| Reading {
            id: fixed_id.unwrap_or_else(|| ReadingId(p.ids.new_uuid())),
            register_id: *register_id,
            device_id: device.id,
            read_at: input.at,
            value: *value,
            source,
            note: note.clone(),
            photo_key: extras.photo_key.clone(),
            ai_confidence: extras.ai_confidence,
            created_by: Some(t.user_id()),
        })
        .collect();
    uow.readings()
        .insert(t, &new)
        .await
        .map_err(|err| match err {
            PortError::Conflict(_) => AppError::Conflict("reading.duplicate"),
            other => other.into(),
        })?;
    Ok(RecordOutcome::Saved(
        new.into_iter().map(|r| r.id).collect(),
    ))
}

impl RecordReadings {
    /// Speichert alle Werte in einer Transaktion. Mit Warnungen und ohne
    /// `confirmed` wird nichts gespeichert.
    // @impl SPEC_RDG_FORM, SPEC_RDG_PLAUSIBILITY
    pub async fn execute(
        &self,
        t: &Tenant,
        input: &ReadingsInput,
        confirmed: bool,
        source: ReadingSource,
    ) -> Result<RecordOutcome, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let outcome = record_in(
            &self.p,
            uow.as_mut(),
            t,
            input,
            confirmed,
            source,
            ReadingExtras::default(),
        )
        .await?;
        if let RecordOutcome::Saved(ids) = &outcome {
            uow.commit().await?;
            metrics::counter!("zaehler_readings_created_total", "source" => source.as_str())
                .increment(ids.len() as u64);
        }
        Ok(outcome)
    }
}

impl DeleteReading {
    /// Löscht eine Ablesung. Ablesungen eines Zählerwechsels (Endstand des
    /// alten, Startstand eines nachfolgenden Geräts) sind gesperrt.
    // @impl SPEC_RDG_DELETE
    pub async fn execute(&self, t: &Tenant, id: ReadingId) -> Result<MeterId, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let (meter_id, reading) = uow.readings().get(t, id).await?.ok_or(AppError::NotFound)?;
        let agg = uow
            .meters()
            .get(t, meter_id)
            .await?
            .ok_or(AppError::NotFound)?;
        let position = agg.devices.iter().position(|d| d.id == reading.device_id);
        let part_of_swap = position.is_some_and(|i| {
            let d = &agg.devices[i];
            d.removed_at == Some(reading.read_at)
                || (i > 0 && d.data.installed_at == reading.read_at)
        });
        if part_of_swap {
            return Err(AppError::Conflict("reading.part_of_swap"));
        }
        uow.readings().delete(t, id).await?;
        uow.commit().await?;
        // Belegfoto erst nach dem commit (SPEC_ARCH_UOW); ein Fehler hier
        // hinterlässt nur ein unerreichbares Objekt.
        if let Some(key) = reading.photo_key
            && let Err(e) = self.p.blobs.delete(&key).await
        {
            tracing::warn!(error = %e, reading = %id, "photo not deleted");
        }
        Ok(meter_id)
    }
}

impl GetReadingPhoto {
    /// Belegfoto einer Ablesung des Haushalts (JPEG).
    // @impl SPEC_SEC_UPLOADS, SPEC_STORE_BLOBS
    pub async fn execute(&self, t: &Tenant, id: ReadingId) -> Result<Bytes, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let (_, reading) = uow.readings().get(t, id).await?.ok_or(AppError::NotFound)?;
        drop(uow);
        let key = reading
            .photo_key
            .filter(|k| k.household() == t.household_id())
            .ok_or(AppError::NotFound)?;
        self.p.blobs.get(&key).await.map_err(AppError::from_storage)
    }
}
