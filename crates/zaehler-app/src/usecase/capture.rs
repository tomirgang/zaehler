//! Foto-Erfassung: Foto normalisieren, Zählerstand erkennen, Entwurf
//! zuordnen, bestätigen oder verwerfen (SPEC_AI_FLOW, SPEC_AI_DRAFTS).

use std::time::Instant;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use zaehler_domain::reading::{
    Plausibility, Reading as DomainReading, ReadingSource, check_plausibility,
};

use crate::{
    AppError, Role, Tenant,
    ids::{BlobKey, DraftId, MeterId, ReadingId, RegisterId},
    model::{Draft, DraftOutcome, DraftStatus, MeterAggregate, MeterSummary, Reading, Recognition},
    port::UnitOfWork,
    usecase::{
        meter::{MeterDetail, last_of, to_domain},
        reading::{ReadingExtras, ReadingsInput, RecordOutcome, check_all, record_in},
        use_case,
    },
};

use_case!(
    /// Foto analysieren und Entwurf anlegen.
    AnalysePhoto,
    /// Entwurf mit Zuordnung und Plausibilität für das Bestätigungsformular.
    GetDraft,
    /// Entwurf als Ablesung speichern.
    ConfirmDraft,
    /// Entwurf verwerfen.
    DiscardDraft,
    /// Foto eines offenen Entwurfs.
    GetDraftPhoto,
);

/// Zählernummer für den Vergleich: Großbuchstaben, ohne Leerzeichen und
/// Bindestriche.
#[must_use]
pub fn normalize_serial(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

fn normalize_code(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

/// Register zum erkannten Code, sonst `main`, sonst das einzige Register.
// @impl SPEC_AI_DRAFTS
#[must_use]
pub fn choose_register(agg: &MeterAggregate, code: Option<&str>) -> Option<RegisterId> {
    let by_code = |c: &str| {
        agg.registers
            .iter()
            .find(|r| normalize_code(&r.data.code) == normalize_code(c))
            .map(|r| r.id)
    };
    code.and_then(by_code)
        .or_else(|| by_code("main"))
        .or_else(|| (agg.registers.len() == 1).then(|| agg.registers[0].id))
}

/// Wert passt plausibel zu mindestens einem Register der Messstelle.
fn value_fits(
    agg: &MeterAggregate,
    readings: &[Reading],
    value: Decimal,
    at: DateTime<Utc>,
) -> bool {
    let Some(device) = agg.active_device() else {
        return false;
    };
    let candidate = DomainReading {
        at,
        value,
        device_id: device.id.0,
        rollover_digits: device.data.rollover_digits,
    };
    agg.registers.iter().any(|r| {
        let prev = last_of(readings, r.id, device.id).map(|x| to_domain(x, device));
        check_plausibility(prev.as_ref(), &candidate, agg.meter.data.max_daily_rate)
            == Plausibility::Ok
    })
}

/// Zuordnung: (1) Zählernummer exakt nach Normalisierung gegen aktive Geräte;
/// (2) eindeutige Messstelle passender Art, zu der der Wert plausibel passt;
/// (3) keine Zuordnung (Auswahl durch den Nutzer).
// @impl SPEC_AI_DRAFTS
async fn match_meter(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meters: &[MeterSummary],
    rec: &Recognition,
    now: DateTime<Utc>,
) -> Result<Option<MeterId>, AppError> {
    let active: Vec<&MeterSummary> = meters.iter().filter(|m| !m.meter.archived).collect();
    let serial = rec
        .serial_number
        .as_deref()
        .map(normalize_serial)
        .filter(|s| !s.is_empty());
    let mut aggregates = Vec::new();
    for m in &active {
        if let Some(agg) = uow.meters().get(t, m.meter.id).await? {
            aggregates.push(agg);
        }
    }
    if let Some(serial) = serial {
        let hit = aggregates.iter().find(|agg| {
            agg.active_device()
                .and_then(|d| d.data.serial_number.as_deref())
                .is_some_and(|s| normalize_serial(s) == serial)
        });
        if let Some(agg) = hit {
            return Ok(Some(agg.meter.id));
        }
    }
    let (Some(kind), Some(value)) = (rec.meter_kind, rec.value) else {
        return Ok(None);
    };
    let mut fitting = Vec::new();
    for agg in aggregates.iter().filter(|a| a.meter.data.kind == kind) {
        let readings = uow.readings().list_for_meter(t, agg.meter.id).await?;
        if value_fits(agg, &readings, value, now) {
            fitting.push(agg.meter.id);
        }
    }
    Ok(if fitting.len() == 1 {
        fitting.pop()
    } else {
        None
    })
}

impl AnalysePhoto {
    /// Erkennung ist konfiguriert.
    #[must_use]
    pub fn ai_enabled(&self) -> bool {
        self.p.recognizer.is_some()
    }

    /// Normalisiert das Foto, speichert es, ruft die Erkennung auf und legt
    /// den Entwurf an. Ohne AI-Konfiguration oder bei Fehlern der Erkennung
    /// entsteht ein Entwurf mit Status `failed` zur manuellen Eingabe.
    // @impl SPEC_AI_FLOW, SPEC_AI_DRAFTS, SPEC_IMG_PROCESSING
    pub async fn execute(&self, t: &Tenant, photo: Bytes) -> Result<DraftId, AppError> {
        t.require(Role::Member)?;
        let jpeg = self.p.images.normalize(photo).await?;
        let id = DraftId(self.p.ids.new_uuid());
        let key = BlobKey::draft_photo(t.household_id(), id);
        self.p
            .blobs
            .put(&key, jpeg.bytes.clone())
            .await
            .map_err(AppError::from_storage)?;

        let (model, duration_ms, suggestion) = match &self.p.recognizer {
            None => (None, None, None),
            Some(r) => {
                let start = Instant::now();
                let result = r.recognize(&jpeg).await;
                let elapsed = start.elapsed();
                let outcome = match &result {
                    Ok(_) => "ok",
                    Err(e) => {
                        tracing::warn!(error = %e, model = r.model(), "recognition failed");
                        e.outcome()
                    }
                };
                metrics::counter!("zaehler_ai_requests_total", "outcome" => outcome).increment(1);
                metrics::histogram!("zaehler_ai_request_duration_seconds")
                    .record(elapsed.as_secs_f64());
                (
                    Some(r.model().to_owned()),
                    Some(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)),
                    result.ok(),
                )
            }
        };

        let now = self.p.clock.now();
        let mut uow = self.p.uow.begin().await?;
        let matched = match &suggestion {
            Some(rec) => {
                let meters = uow.meters().list(t).await?;
                match_meter(uow.as_mut(), t, &meters, rec, now).await?
            }
            None => None,
        };
        let draft = Draft {
            id,
            created_by: t.user_id(),
            photo_key: key,
            model,
            duration_ms,
            status: if suggestion.is_some() {
                DraftStatus::Pending
            } else {
                DraftStatus::Failed
            },
            suggestion,
            matched_meter_id: matched,
            created_at: now,
        };
        uow.drafts().create(t, &draft).await?;
        uow.commit().await?;
        Ok(id)
    }
}

/// Bestätigungsformular eines Entwurfs.
#[derive(Debug, Clone)]
pub struct DraftView {
    pub draft: Draft,
    /// Auswahl der Messstellen (ohne archivierte)
    pub meters: Vec<MeterSummary>,
    /// Gewählte Messstelle: Auswahl des Nutzers, sonst Zuordnung
    pub meter: Option<MeterDetail>,
    pub register: Option<RegisterId>,
    /// Plausibilität des Vorschlags für Messstelle und Register
    pub plausibility: Option<Plausibility>,
}

impl GetDraft {
    // @impl SPEC_AI_DRAFTS
    pub async fn execute(
        &self,
        t: &Tenant,
        id: DraftId,
        meter: Option<MeterId>,
    ) -> Result<DraftView, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let draft = uow
            .drafts()
            .get_open(t, id)
            .await?
            .ok_or(AppError::NotFound)?;
        let meters: Vec<MeterSummary> = uow
            .meters()
            .list(t)
            .await?
            .into_iter()
            .filter(|m| !m.meter.archived)
            .collect();
        let chosen = meter
            .or(draft.matched_meter_id)
            .filter(|m| meters.iter().any(|s| s.meter.id == *m));
        let mut view = DraftView {
            draft,
            meters,
            meter: None,
            register: None,
            plausibility: None,
        };
        let Some(meter_id) = chosen else {
            return Ok(view);
        };
        let aggregate = uow
            .meters()
            .get(t, meter_id)
            .await?
            .ok_or(AppError::NotFound)?;
        let readings = uow.readings().list_for_meter(t, meter_id).await?;
        let suggestion = view.draft.suggestion.as_ref();
        view.register = choose_register(&aggregate, suggestion.and_then(|s| s.register.as_deref()));
        if let (Some(register), Some(value)) = (view.register, suggestion.and_then(|s| s.value)) {
            let input = ReadingsInput {
                meter_id,
                at: self.p.clock.now(),
                values: vec![(register, value)],
                note: None,
            };
            view.plausibility = check_all(&aggregate, &readings, &input)
                .ok()
                .and_then(|c| c.into_iter().next())
                .map(|c| c.plausibility);
        }
        view.meter = Some(MeterDetail {
            aggregate,
            readings,
        });
        Ok(view)
    }
}

impl ConfirmDraft {
    /// Speichert die bestätigte Ablesung mit Foto (`source = photo`) und
    /// schließt den Entwurf ab. Reihenfolge: Foto kopieren, Ablesung und
    /// Entwurf in einer Transaktion, danach Entwurfsfoto löschen.
    // @impl SPEC_AI_DRAFTS, SPEC_RDG_PLAUSIBILITY
    pub async fn execute(
        &self,
        t: &Tenant,
        id: DraftId,
        input: &ReadingsInput,
        confirmed: bool,
    ) -> Result<RecordOutcome, AppError> {
        t.require(Role::Member)?;
        if input.values.len() != 1 {
            return Err(AppError::field("value", "reading.values_required"));
        }
        let mut uow = self.p.uow.begin().await?;
        let draft = uow
            .drafts()
            .get_open(t, id)
            .await?
            .ok_or(AppError::NotFound)?;
        let reading_id = ReadingId(self.p.ids.new_uuid());
        let photo = BlobKey::reading_photo(t.household_id(), reading_id);
        let suggestion = draft.suggestion.as_ref();
        let extras = ReadingExtras {
            id: Some(reading_id),
            photo_key: Some(photo.clone()),
            ai_confidence: suggestion.map(|s| s.confidence),
        };
        let outcome = record_in(
            &self.p,
            uow.as_mut(),
            t,
            input,
            confirmed,
            ReadingSource::Photo,
            extras,
        )
        .await?;
        if !matches!(outcome, RecordOutcome::Saved(_)) {
            return Ok(outcome);
        }
        let corrected = suggestion.map(|s| s.value != input.values.first().map(|(_, v)| *v));
        self.p
            .blobs
            .copy(&draft.photo_key, &photo)
            .await
            .map_err(AppError::from_storage)?;
        uow.drafts()
            .finish(
                t,
                id,
                DraftOutcome::Confirmed {
                    reading: reading_id,
                    corrected,
                },
                self.p.clock.now(),
            )
            .await?;
        uow.commit().await?;
        metrics::counter!("zaehler_readings_created_total", "source" => ReadingSource::Photo.as_str())
            .increment(1);
        if let Some(c) = corrected {
            metrics::counter!("zaehler_ai_corrections_total", "corrected" => c.to_string())
                .increment(1);
        }
        if let Err(e) = self.p.blobs.delete(&draft.photo_key).await {
            tracing::warn!(error = %e, draft = %id, "draft photo not deleted");
        }
        Ok(outcome)
    }
}

impl DiscardDraft {
    // @impl SPEC_AI_DRAFTS
    pub async fn execute(&self, t: &Tenant, id: DraftId) -> Result<(), AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let draft = uow
            .drafts()
            .get_open(t, id)
            .await?
            .ok_or(AppError::NotFound)?;
        uow.drafts()
            .finish(t, id, DraftOutcome::Discarded, self.p.clock.now())
            .await?;
        uow.commit().await?;
        if let Err(e) = self.p.blobs.delete(&draft.photo_key).await {
            tracing::warn!(error = %e, draft = %id, "draft photo not deleted");
        }
        Ok(())
    }
}

impl GetDraftPhoto {
    // @impl SPEC_AI_DRAFTS, SPEC_SEC_UPLOADS
    pub async fn execute(&self, t: &Tenant, id: DraftId) -> Result<Bytes, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let draft = uow
            .drafts()
            .get_open(t, id)
            .await?
            .ok_or(AppError::NotFound)?;
        drop(uow);
        if draft.photo_key.household() != t.household_id() {
            return Err(AppError::NotFound);
        }
        self.p
            .blobs
            .get(&draft.photo_key)
            .await
            .map_err(AppError::from_storage)
    }
}
