//! Foto-Erfassung: Foto normalisieren, Zählerstand erkennen, Entwurf
//! zuordnen, bestätigen oder verwerfen (SPEC_AI_FLOW, SPEC_AI_DRAFTS).

use std::time::Instant;

use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use zaehler_domain::{
    calendar::Calendar,
    reading::{Plausibility, Reading as DomainReading, ReadingSource, check_in_sequence},
};

use crate::{
    AppError, FieldError, FieldErrors, PortError, Role, Tenant,
    ids::{BlobKey, DraftId, MeterId, ReadingId, RegisterId},
    model::{
        Draft, DraftAnalysis, DraftOutcome, DraftStatus, MeterAggregate, MeterSummary, Reading,
        Recognition,
    },
    port::{NormalizedJpeg, PhotoTime, UnitOfWork},
    usecase::{
        Ports,
        meter::{MeterDetail, neighbours, to_domain},
        reading::{
            ReadingExtras, ReadingsInput, RecordOutcome, check_all, enqueue_readings, record_in,
        },
        use_case,
    },
};

use_case!(
    /// Ein Foto analysieren und Entwurf anlegen.
    AnalysePhoto,
    /// Mehrere Fotos als wartende Entwürfe anlegen.
    QueuePhotos,
    /// Wartende Entwürfe nacheinander analysieren.
    AnalyseQueued,
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
        let (prev, next) = neighbours(readings, r.id, device.id, at);
        check_in_sequence(
            prev.map(|x| to_domain(x, device)).as_ref(),
            &candidate,
            next.map(|x| to_domain(x, device)).as_ref(),
            agg.meter.data.max_daily_rate,
        ) == Plausibility::Ok
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

/// Aufnahmezeit des Fotos als Vorschlag für den Ablesezeitpunkt: mit Offset
/// exakt, sonst in der Zeitzone des Haushalts; Zeitpunkte mehr als 5 Minuten
/// in der Zukunft (falsch gestellte Kamera-Uhr) werden verworfen.
// @impl SPEC_IMG_CAPTURE_TIME
fn photo_time(
    taken: Option<PhotoTime>,
    cal: Calendar,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    taken
        .map(|p| p.to_utc(cal))
        .filter(|at| *at <= now + Duration::minutes(5))
}

/// Höchstzahl Fotos je Upload (SPEC_AI_MULTI).
pub const MAX_PHOTOS: usize = 20;
/// Nach dieser Zeit gilt eine laufende Analyse als abgebrochen und wird neu
/// aufgenommen (Instanz beendet während der Analyse).
pub const ANALYSIS_TIMEOUT: Duration = Duration::minutes(5);

/// Ergebnis eines Uploads mehrerer Fotos.
#[derive(Debug, Clone, Default)]
pub struct QueuedPhotos {
    /// Entwürfe in Upload-Reihenfolge
    pub drafts: Vec<DraftId>,
    /// Abgelehnte Fotos mit Position im Upload (ab 0) und Fehler
    pub rejected: Vec<(usize, FieldError)>,
}

impl QueuePhotos {
    /// Normalisiert und speichert die Fotos und legt je Foto einen Entwurf
    /// im Status `queued` an. Nicht lesbare Fotos werden mit Grund
    /// zurückgemeldet, die übrigen trotzdem angelegt.
    // @impl SPEC_AI_MULTI, SPEC_IMG_PROCESSING, SPEC_IMG_CAPTURE_TIME
    #[tracing::instrument(name = "QueuePhotos", skip_all)]
    pub async fn execute(&self, t: &Tenant, photos: Vec<Bytes>) -> Result<QueuedPhotos, AppError> {
        t.require(Role::Member)?;
        if photos.is_empty() {
            return Err(AppError::field("photo", "photo.required"));
        }
        if photos.len() > MAX_PHOTOS {
            return Err(AppError::Validation(FieldErrors(vec![FieldError {
                field: "photo".into(),
                code: "photo.too_many",
                args: vec![("max", MAX_PHOTOS.to_string())],
            }])));
        }
        let now = self.p.clock.now();
        let cal = Calendar::new(
            self.p
                .uow
                .begin()
                .await?
                .households()
                .get(t)
                .await?
                .data
                .timezone,
        );
        let mut out = QueuedPhotos::default();
        let mut drafts = Vec::new();
        for (index, photo) in photos.into_iter().enumerate() {
            let jpeg = match self.p.images.normalize(photo).await {
                Ok(j) => j,
                Err(PortError::Invalid(code)) => {
                    out.rejected.push((
                        index,
                        FieldError {
                            field: "file".into(),
                            code,
                            args: Vec::new(),
                        },
                    ));
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let id = DraftId(self.p.ids.new_uuid());
            let key = BlobKey::draft_photo(t.household_id(), id);
            self.p
                .blobs
                .put(&key, jpeg.bytes.clone())
                .await
                .map_err(AppError::from_storage)?;
            drafts.push(Draft {
                id,
                created_by: t.user_id(),
                photo_key: key,
                model: None,
                duration_ms: None,
                suggestion: None,
                matched_meter_id: None,
                status: DraftStatus::Queued,
                created_at: now,
                taken_at: photo_time(jpeg.taken_at, cal, now),
                analysis_started_at: None,
            });
        }
        let mut uow = self.p.uow.begin().await?;
        for d in &drafts {
            uow.drafts().create(t, d).await?;
        }
        uow.commit().await?;
        out.drafts = drafts.into_iter().map(|d| d.id).collect();
        Ok(out)
    }
}

/// Wartet auf Analyse: in der Warteschlange oder Analyse abgebrochen.
fn waiting(d: &Draft, now: DateTime<Utc>) -> bool {
    d.status == DraftStatus::Queued
        || (d.status == DraftStatus::Analysing
            && d.analysis_started_at
                .is_none_or(|s| s < now - ANALYSIS_TIMEOUT))
}

/// Ruft die Erkennung auf (ohne Konfiguration nicht) und zählt die Metriken.
async fn recognize(
    p: &Ports,
    jpeg: &NormalizedJpeg,
) -> (Option<String>, Option<i64>, Option<Recognition>) {
    let Some(r) = &p.recognizer else {
        return (None, None, None);
    };
    let start = Instant::now();
    let result = r.recognize(jpeg).await;
    let elapsed = start.elapsed();
    let outcome = match &result {
        Ok(_) => "ok",
        Err(e) => {
            tracing::warn!(error = %e, model = r.model(), "recognition failed");
            e.outcome()
        }
    };
    metrics::counter!("zaehler_ai_requests_total", "outcome" => outcome).increment(1);
    metrics::histogram!("zaehler_ai_request_duration_seconds").record(elapsed.as_secs_f64());
    (
        Some(r.model().to_owned()),
        Some(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)),
        result.ok(),
    )
}

impl AnalyseQueued {
    /// Analysiert den ältesten wartenden Entwurf des Haushalts; `None`, wenn
    /// keiner wartet. Die Hintergrundaufgabe ruft dies wiederholt auf, sodass
    /// die Fotos nacheinander analysiert werden.
    // @impl SPEC_AI_MULTI
    #[tracing::instrument(name = "AnalyseQueued::next", skip_all)]
    pub async fn next(&self, t: &Tenant) -> Result<Option<DraftId>, AppError> {
        t.require(Role::Member)?;
        let now = self.p.clock.now();
        let open = self.p.uow.begin().await?.drafts().list_open(t).await?;
        for d in open.iter().filter(|d| waiting(d, now)) {
            if self.claim(t, d.id, now).await? {
                return match self.analyse(t, d).await {
                    // während der Analyse verworfen
                    Ok(()) | Err(AppError::NotFound) => Ok(Some(d.id)),
                    Err(e) => Err(e),
                };
            }
        }
        Ok(None)
    }

    /// Analysiert diesen Entwurf, falls er wartet; `false`, wenn er bereits
    /// analysiert ist oder ein anderer Lauf ihn hat.
    // @impl SPEC_AI_MULTI
    #[tracing::instrument(name = "AnalyseQueued::draft", skip_all)]
    pub async fn draft(&self, t: &Tenant, id: DraftId) -> Result<bool, AppError> {
        t.require(Role::Member)?;
        let now = self.p.clock.now();
        let draft = self
            .p
            .uow
            .begin()
            .await?
            .drafts()
            .get_open(t, id)
            .await?
            .ok_or(AppError::NotFound)?;
        if !waiting(&draft, now) || !self.claim(t, id, now).await? {
            return Ok(false);
        }
        self.analyse(t, &draft).await?;
        Ok(true)
    }

    async fn claim(&self, t: &Tenant, id: DraftId, now: DateTime<Utc>) -> Result<bool, AppError> {
        let mut uow = self.p.uow.begin().await?;
        let claimed = uow
            .drafts()
            .claim(t, id, now, now - ANALYSIS_TIMEOUT)
            .await?;
        uow.commit().await?;
        Ok(claimed)
    }

    /// Erkennung auf dem gespeicherten (bereits normalisierten) Foto,
    /// Zuordnung und Speicherung des Ergebnisses.
    // @impl SPEC_AI_FLOW, SPEC_AI_DRAFTS, SPEC_AI_MULTI
    async fn analyse(&self, t: &Tenant, draft: &Draft) -> Result<(), AppError> {
        let bytes = self
            .p
            .blobs
            .get(&draft.photo_key)
            .await
            .map_err(AppError::from_storage)?;
        // Maße werden für die Erkennung nicht benötigt
        let jpeg = NormalizedJpeg {
            bytes,
            width: 0,
            height: 0,
            taken_at: None,
        };
        let (model, duration_ms, suggestion) = recognize(&self.p, &jpeg).await;
        let now = self.p.clock.now();
        let mut uow = self.p.uow.begin().await?;
        let matched = match &suggestion {
            Some(rec) => {
                let meters = uow.meters().list(t).await?;
                match_meter(uow.as_mut(), t, &meters, rec, now).await?
            }
            None => None,
        };
        let analysis = DraftAnalysis {
            model,
            duration_ms,
            status: if suggestion.is_some() {
                DraftStatus::Pending
            } else {
                DraftStatus::Failed
            },
            suggestion,
            matched_meter_id: matched,
        };
        uow.drafts().record_analysis(t, draft.id, &analysis).await?;
        uow.commit().await?;
        Ok(())
    }
}

impl AnalysePhoto {
    /// Erkennung ist konfiguriert.
    #[must_use]
    pub fn ai_enabled(&self) -> bool {
        self.p.recognizer.is_some()
    }

    /// Ein Foto: Entwurf anlegen und sofort analysieren. Ohne
    /// AI-Konfiguration oder bei Fehlern der Erkennung entsteht ein Entwurf
    /// mit Status `failed` zur manuellen Eingabe.
    // @impl SPEC_AI_FLOW, SPEC_AI_DRAFTS, SPEC_IMG_PROCESSING
    #[tracing::instrument(name = "AnalysePhoto", skip_all)]
    pub async fn execute(&self, t: &Tenant, photo: Bytes) -> Result<DraftId, AppError> {
        let queued = QueuePhotos::new(self.p.clone())
            .execute(t, vec![photo])
            .await?;
        let Some(&id) = queued.drafts.first() else {
            return Err(AppError::Validation(FieldErrors(
                queued.rejected.into_iter().map(|(_, e)| e).collect(),
            )));
        };
        AnalyseQueued::new(self.p.clone()).draft(t, id).await?;
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
    /// Offene Entwürfe des Haushalts in Prüfreihenfolge (SPEC_AI_MULTI)
    pub queue: Vec<DraftId>,
}

impl DraftView {
    /// Position dieses Entwurfs in der Warteschlange (ab 1) und Anzahl.
    #[must_use]
    pub fn position(&self) -> (usize, usize) {
        let pos = self
            .queue
            .iter()
            .position(|d| *d == self.draft.id)
            .map_or(1, |p| p + 1);
        (pos, self.queue.len())
    }
}

impl GetDraft {
    /// Nächster offener Entwurf nach dem Abschluss von `done` (SPEC_AI_MULTI).
    // @impl SPEC_AI_MULTI
    #[tracing::instrument(name = "GetDraft::next_open", skip_all)]
    pub async fn next_open(&self, t: &Tenant, done: DraftId) -> Result<Option<DraftId>, AppError> {
        t.require(Role::Member)?;
        let open = self.p.uow.begin().await?.drafts().list_open(t).await?;
        Ok(open.into_iter().map(|d| d.id).find(|d| *d != done))
    }
}

impl GetDraft {
    // @impl SPEC_AI_DRAFTS
    #[tracing::instrument(name = "GetDraft", skip_all)]
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
        let queue = uow
            .drafts()
            .list_open(t)
            .await?
            .into_iter()
            .map(|d| d.id)
            .collect();
        let mut view = DraftView {
            draft,
            meters,
            meter: None,
            register: None,
            plausibility: None,
            queue,
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
    #[tracing::instrument(name = "ConfirmDraft", skip_all)]
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
        if !draft.status.is_ready() {
            return Err(AppError::Conflict("capture.not_ready"));
        }
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
        let RecordOutcome::Saved(ids) = &outcome else {
            return Ok(outcome);
        };
        enqueue_readings(&self.p, uow.as_mut(), t, input.meter_id, ids).await?;
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
    #[tracing::instrument(name = "DiscardDraft", skip_all)]
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
    #[tracing::instrument(name = "GetDraftPhoto", skip_all)]
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
