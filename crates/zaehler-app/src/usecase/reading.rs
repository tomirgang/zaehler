//! Zählerstände prüfen, erfassen und löschen.

use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use zaehler_domain::reading::{
    Plausibility, Reading as DomainReading, ReadingSource, check_in_sequence,
};

use crate::{
    AppError, FieldErrors, PortError, Role, Tenant,
    ids::{BlobKey, DeviceId, MeterId, OutboxId, ReadingId, RegisterId, RevisionId, UserId},
    model::{MeterAggregate, OutboxEvent, Reading, ReadingUpdate, Revision, RevisionAction},
    port::UnitOfWork,
    usecase::{
        Ports,
        meter::{MeterDetail, neighbours, to_domain},
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
    GetReadingPhoto,
    /// Ablesung mit Messstelle (Formular der Korrektur).
    GetReading,
    /// Zeitpunkt, Wert und Notiz einer Ablesung ändern.
    CorrectReading,
    /// Änderungsverlauf der Ablesungen einer Messstelle.
    ReadingHistory,
);

/// Formale Prüfung und Plausibilität aller Werte gegen die zeitlich
/// benachbarten Ablesungen des aktiven Geräts.
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
        let (prev, next) = neighbours(readings, *register_id, device.id, input.at);
        let (prev, next) = (
            prev.map(|r| to_domain(r, device)),
            next.map(|r| to_domain(r, device)),
        );
        let candidate = DomainReading {
            at: input.at,
            value: *value,
            device_id: device.id.0,
            rollover_digits: device.data.rollover_digits,
        };
        let plausibility = if input.at < device.data.installed_at {
            Plausibility::OutOfOrder
        } else {
            check_in_sequence(
                prev.as_ref(),
                &candidate,
                next.as_ref(),
                agg.meter.data.max_daily_rate,
            )
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
    #[tracing::instrument(name = "CheckReadings", skip_all)]
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
    // @impl SPEC_MTR_ARCHIVE
    if agg.meter.archived {
        return Err(AppError::field("meter", "meter.archived"));
    }
    let readings = uow.readings().list_for_meter(t, input.meter_id).await?;
    let checks = check_all(&agg, &readings, input)?;
    let device = agg
        .active_device()
        .ok_or(AppError::Conflict("meter.no_active_device"))?;
    // Vor dem Einbau des aktiven Geräts ist nur beim ersten Gerät zulässig;
    // vor einem Zählerwechsel gehört der Stand zum alten Gerät.
    let before_installation = input.at < device.data.installed_at;
    let first_device = agg.devices.first().is_some_and(|d| d.id == device.id);
    if before_installation && !first_device {
        let mut e = FieldErrors::default();
        e.add("at", "reading.before_installation");
        e.into_result()?;
    }
    if !confirmed && checks.iter().any(|c| c.plausibility != Plausibility::Ok) {
        return Ok(RecordOutcome::NeedsConfirmation(checks));
    }
    if before_installation {
        // wie beim CSV-Import: Einbau des ersten Geräts vorverlegen
        // (SPEC_RDG_PLAUSIBILITY, SPEC_IMP_CSV_FORMAT)
        let mut data = device.data.clone();
        data.installed_at = input.at;
        uow.meters().update_device(t, device.id, &data).await?;
    }
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

/// Outbox-Eintrag `readings_recorded` in der Unit of Work der Erfassung
/// (SPEC_JOB_OUTBOX); der CSV-Import schreibt keinen.
// @impl SPEC_JOB_OUTBOX
pub(crate) async fn enqueue_readings(
    p: &Ports,
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meter_id: MeterId,
    reading_ids: &[ReadingId],
) -> Result<(), AppError> {
    let event = OutboxEvent::ReadingsRecorded {
        meter_id,
        reading_ids: reading_ids.to_vec(),
    };
    uow.outbox()
        .enqueue(t, OutboxId(p.ids.new_uuid()), &event, p.clock.now())
        .await?;
    Ok(())
}

impl RecordReadings {
    /// Speichert alle Werte in einer Transaktion. Mit Warnungen und ohne
    /// `confirmed` wird nichts gespeichert.
    // @impl SPEC_RDG_FORM, SPEC_RDG_PLAUSIBILITY
    #[tracing::instrument(name = "RecordReadings", skip_all)]
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
            enqueue_readings(&self.p, uow.as_mut(), t, input.meter_id, ids).await?;
            uow.commit().await?;
            metrics::counter!("zaehler_readings_created_total", "source" => source.as_str())
                .increment(ids.len() as u64);
        }
        Ok(outcome)
    }
}

impl DeleteReading {
    /// Löscht eine Ablesung und schreibt eine Revision. Ablesungen eines
    /// Zählerwechsels (Endstand des alten, Startstand eines nachfolgenden
    /// Geräts) sind gesperrt.
    // @impl SPEC_RDG_DELETE, SPEC_RDG_HISTORY
    #[tracing::instrument(name = "DeleteReading", skip_all)]
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
        let revision = Revision {
            id: RevisionId(self.p.ids.new_uuid()),
            reading_id: id,
            meter_id,
            register_id: reading.register_id,
            action: RevisionAction::Delete,
            changed_by: t.user_id(),
            changed_at: self.p.clock.now(),
            old_read_at: reading.read_at,
            old_value: reading.value,
            old_note: reading.note.clone(),
            new_read_at: None,
            new_value: None,
            new_note: None,
        };
        uow.revisions().append(t, &revision).await?;
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
    #[tracing::instrument(name = "GetReadingPhoto", skip_all)]
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

/// Ablesung mit ihrer Messstelle.
#[derive(Debug, Clone)]
pub struct ReadingContext {
    pub detail: MeterDetail,
    pub reading: Reading,
    /// End- oder Startstand eines Zählerwechsels: der Zeitpunkt ändert sich
    /// nur gemeinsam mit den übrigen Ständen des Wechsels.
    pub part_of_swap: bool,
}

/// Geräte eines Zählerwechsels, an dem die Ablesung beteiligt ist: altes
/// Gerät (Endstand) und neues Gerät (Startstand).
fn swap_devices(agg: &MeterAggregate, reading: &Reading) -> Option<(DeviceId, DeviceId)> {
    let i = agg.devices.iter().position(|d| d.id == reading.device_id)?;
    let device = &agg.devices[i];
    if device.removed_at == Some(reading.read_at) {
        let next = agg.devices.get(i + 1)?;
        Some((device.id, next.id))
    } else if i > 0 && device.data.installed_at == reading.read_at {
        Some((agg.devices[i - 1].id, device.id))
    } else {
        None
    }
}

impl GetReading {
    #[tracing::instrument(name = "GetReading", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: ReadingId) -> Result<ReadingContext, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let (meter_id, reading) = uow.readings().get(t, id).await?.ok_or(AppError::NotFound)?;
        let aggregate = uow
            .meters()
            .get(t, meter_id)
            .await?
            .ok_or(AppError::NotFound)?;
        let readings = uow.readings().list_for_meter(t, meter_id).await?;
        Ok(ReadingContext {
            part_of_swap: swap_devices(&aggregate, &reading).is_some(),
            detail: MeterDetail {
                aggregate,
                readings,
            },
            reading,
        })
    }
}

/// Neue Werte einer Ablesung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionInput {
    pub at: DateTime<Utc>,
    pub value: Decimal,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorrectionOutcome {
    Saved(MeterId),
    /// Warnung liegt vor; Speichern erst nach Bestätigung.
    NeedsConfirmation(Plausibility),
}

/// Prüft den neuen Zeitpunkt gegen die Einbauzeit des Geräts bzw. bei einem
/// Zählerwechsel gegen die übrigen Stände beider Geräte.
fn check_time(
    agg: &MeterAggregate,
    readings: &[Reading],
    reading: &Reading,
    swap: Option<(DeviceId, DeviceId)>,
    at: DateTime<Utc>,
    e: &mut FieldErrors,
) {
    let Some(device) = agg.device(reading.device_id) else {
        return;
    };
    if let Some((old, new)) = swap {
        let old_installed = agg.device(old).map(|d| d.data.installed_at);
        let lower = readings
            .iter()
            .filter(|r| r.device_id == old && r.read_at < reading.read_at)
            .map(|r| r.read_at)
            .chain(old_installed)
            .max();
        let upper = readings
            .iter()
            .filter(|r| r.device_id == new && r.read_at > reading.read_at)
            .map(|r| r.read_at)
            .chain(agg.device(new).and_then(|d| d.removed_at))
            .min();
        if lower.is_some_and(|l| at <= l) || upper.is_some_and(|u| at >= u) {
            e.add("at", "reading.swap_time_invalid");
        }
    } else {
        let first = agg.devices.first().is_some_and(|d| d.id == device.id);
        if at < device.data.installed_at && !first {
            e.add("at", "reading.before_installation");
        }
        if device.removed_at.is_some_and(|r| at > r) {
            e.add("at", "reading.after_removal");
        }
    }
}

impl CorrectReading {
    /// Ändert Zeitpunkt, Wert und Notiz und schreibt im selben Commit je
    /// geänderter Ablesung eine Revision. Plausibilität gegen die zeitlichen
    /// Nachbarn ohne die geänderte Ablesung; Warnungen erfordern Bestätigung.
    /// Bei einem Zählerwechsel verschiebt eine Zeitänderung alle End- und
    /// Startstände des Wechsels und die Ein- und Ausbauzeit der Geräte.
    // @impl SPEC_RDG_HISTORY, SPEC_RDG_PLAUSIBILITY
    #[tracing::instrument(name = "CorrectReading", skip_all)]
    #[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf mit Zählerwechsel
    pub async fn execute(
        &self,
        t: &Tenant,
        id: ReadingId,
        input: &CorrectionInput,
        confirmed: bool,
    ) -> Result<CorrectionOutcome, AppError> {
        t.require(Role::Member)?;
        let note = input
            .note
            .as_ref()
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty());
        let mut e = FieldErrors::default();
        if input.at > self.p.clock.now() + Duration::minutes(5) {
            e.add("at", "reading.in_future");
        }
        if note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE) {
            e.add_with(
                "note",
                "reading.note_too_long",
                vec![("max", MAX_NOTE.to_string())],
            );
        }
        let mut uow = self.p.uow.begin().await?;
        let (meter_id, reading) = uow.readings().get(t, id).await?.ok_or(AppError::NotFound)?;
        let agg = uow
            .meters()
            .get(t, meter_id)
            .await?
            .ok_or(AppError::NotFound)?;
        let readings = uow.readings().list_for_meter(t, meter_id).await?;
        let device = agg
            .device(reading.device_id)
            .cloned()
            .ok_or(AppError::NotFound)?;
        validate::meter_value(
            &mut e,
            "value",
            input.value,
            device.data.decimal_places,
            device.data.rollover_digits,
        );
        let moved = input.at != reading.read_at;
        let swap = swap_devices(&agg, &reading).filter(|_| moved);
        if moved {
            check_time(&agg, &readings, &reading, swap, input.at, &mut e);
        }
        e.into_result()?;

        let others: Vec<Reading> = readings.iter().filter(|r| r.id != id).cloned().collect();
        let (prev, next) = neighbours(&others, reading.register_id, device.id, input.at);
        let candidate = DomainReading {
            at: input.at,
            value: input.value,
            device_id: device.id.0,
            rollover_digits: device.data.rollover_digits,
        };
        let plausibility = if input.at < device.data.installed_at {
            Plausibility::OutOfOrder
        } else {
            check_in_sequence(
                prev.map(|r| to_domain(r, &device)).as_ref(),
                &candidate,
                next.map(|r| to_domain(r, &device)).as_ref(),
                agg.meter.data.max_daily_rate,
            )
        };
        if !confirmed && plausibility != Plausibility::Ok {
            return Ok(CorrectionOutcome::NeedsConfirmation(plausibility));
        }
        if !moved && input.value == reading.value && note == reading.note {
            return Ok(CorrectionOutcome::Saved(meter_id));
        }

        let now = self.p.clock.now();
        // betroffene Ablesungen: die geänderte und bei einem Wechsel alle
        // übrigen Stände desselben Zeitpunkts beider Geräte
        let group: Vec<&Reading> = match swap {
            Some((old, new)) => readings
                .iter()
                .filter(|r| {
                    r.read_at == reading.read_at && (r.device_id == old || r.device_id == new)
                })
                .collect(),
            None => vec![&reading],
        };
        for r in group {
            let edited = r.id == id;
            let update = ReadingUpdate {
                read_at: input.at,
                value: if edited { input.value } else { r.value },
                note: if edited { note.clone() } else { r.note.clone() },
                updated_at: now,
            };
            uow.readings()
                .update(t, r.id, &update)
                .await
                .map_err(|err| match err {
                    PortError::Conflict(_) => AppError::Conflict("reading.duplicate"),
                    other => other.into(),
                })?;
            let revision = revision_of(&self.p, t, meter_id, r, &update, now);
            uow.revisions().append(t, &revision).await?;
        }
        move_devices(uow.as_mut(), t, &agg, &device, swap, input.at).await?;
        enqueue_readings(&self.p, uow.as_mut(), t, meter_id, &[id]).await?;
        uow.commit().await?;
        tracing::info!(reading = %id, meter = %meter_id, "reading corrected");
        Ok(CorrectionOutcome::Saved(meter_id))
    }
}

/// Ein- und Ausbauzeit nach einer Zeitänderung: bei einem Zählerwechsel
/// beide Geräte, sonst Vorverlegen des Einbaus des ersten Geräts wie bei
/// der Erfassung.
async fn move_devices(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    agg: &MeterAggregate,
    device: &crate::model::Device,
    swap: Option<(DeviceId, DeviceId)>,
    at: DateTime<Utc>,
) -> Result<(), AppError> {
    if let Some((old, new)) = swap {
        uow.meters().retire_device(t, old, at).await?;
        if let Some(d) = agg.device(new) {
            let mut data = d.data.clone();
            data.installed_at = at;
            uow.meters().update_device(t, new, &data).await?;
        }
    } else if at < device.data.installed_at {
        let mut data = device.data.clone();
        data.installed_at = at;
        uow.meters().update_device(t, device.id, &data).await?;
    }
    Ok(())
}

fn revision_of(
    p: &Ports,
    t: &Tenant,
    meter_id: MeterId,
    old: &Reading,
    new: &ReadingUpdate,
    at: DateTime<Utc>,
) -> Revision {
    Revision {
        id: RevisionId(p.ids.new_uuid()),
        reading_id: old.id,
        meter_id,
        register_id: old.register_id,
        action: RevisionAction::Update,
        changed_by: t.user_id(),
        changed_at: at,
        old_read_at: old.read_at,
        old_value: old.value,
        old_note: old.note.clone(),
        new_read_at: Some(new.read_at),
        new_value: Some(new.value),
        new_note: new.note.clone(),
    }
}

/// Revision mit dem Namen der ändernden Person, falls bekannt.
#[derive(Debug, Clone)]
pub struct RevisionEntry {
    pub revision: Revision,
    pub changed_by_name: Option<String>,
}

impl ReadingHistory {
    /// Revisionen der Messstelle, neueste zuerst; enthält auch gelöschte
    /// Ablesungen.
    // @impl SPEC_RDG_HISTORY
    #[tracing::instrument(name = "ReadingHistory", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        meter: MeterId,
    ) -> Result<Vec<RevisionEntry>, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        uow.meters()
            .get(t, meter)
            .await?
            .ok_or(AppError::NotFound)?;
        let revisions = uow.revisions().for_meter(t, meter).await?;
        let mut names: std::collections::HashMap<UserId, Option<String>> =
            std::collections::HashMap::new();
        for r in &revisions {
            if let std::collections::hash_map::Entry::Vacant(entry) = names.entry(r.changed_by) {
                entry.insert(uow.users().get(r.changed_by).await?.map(|u| u.display_name));
            }
        }
        Ok(revisions
            .into_iter()
            .map(|revision| RevisionEntry {
                changed_by_name: names.get(&revision.changed_by).cloned().flatten(),
                revision,
            })
            .collect())
    }
}
