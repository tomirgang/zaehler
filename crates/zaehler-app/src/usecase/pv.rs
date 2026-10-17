//! Photovoltaik: Anlagen mit Zuordnung, Sammelerfassung je Hausanschluss,
//! Kennzahlen des Hausanschlusses und Anlagenbericht mit Erwartung und
//! Wirtschaftlichkeit (SPEC_PV_ASSIGNMENT, SPEC_PV_BULK_CAPTURE,
//! SPEC_PV_KPI, SPEC_PV_YIELD_EXPECTATION, SPEC_PV_ECONOMICS).

use chrono::{DateTime, Datelike, Months, NaiveDate, Utc};
use rust_decimal::Decimal;
use zaehler_domain::{
    assets::{AssetCategory, PvCostCategory},
    calendar::Calendar,
    consumption::Interval,
    meter::{Direction, MeterKind},
    period::Period,
    pv::{
        BATTERY_CHARGE, BATTERY_DISCHARGE, BatteryKpi, Economics, Flows, GridSetup, Kpi, PvKind,
        PvRole, PvSystemData, YieldMonth, battery_kpis, break_even, economics, expectation,
        expectation_in, kpis, simple_return, yield_months,
    },
    reading::ReadingSource,
    series::{MonthValue, covered_until, months_between, total_in},
};

use crate::{
    AppError, FieldErrors, Role, Tenant,
    ids::{BlobKey, DraftId, MeterId, PvSystemId, ReadingId, RegisterId},
    model::{AssetCost, AssetRef, Draft, DraftOutcome, MeterAggregate, PvSystem},
    port::UnitOfWork,
    usecase::{
        analysis::register_intervals,
        capture::choose_register,
        contract::{money, text_opt},
        cost::{MeterPricing, calendar_today, load_pricing},
        reading::{
            ReadingExtras, ReadingsInput, RecordOutcome, RegisterCheck, enqueue_readings, record_in,
        },
        use_case, validate,
    },
};

pub const MAX_NAME: usize = 80;
pub const MAX_INVERTER: usize = 80;
/// Jahre der Fortschreibung für den Amortisationszeitpunkt.
pub const PROJECTION_YEARS: u32 = 30;

use_case!(
    /// PV-Anlagen des Haushalts (Viewer).
    ListPvSystems,
    /// Eine Anlage (Viewer).
    GetPvSystem,
    /// Anlage mit Zuordnung anlegen (Owner).
    CreatePvSystem,
    /// Anlagendaten und Zuordnung ändern (Owner).
    UpdatePvSystem,
    /// Anlage mit ihrem Kostenbuch löschen (Owner).
    DeletePvSystem,
    /// Stände aller Messstellen eines Hausanschlusses mit gemeinsamem
    /// Zeitpunkt (Member).
    RecordBulkReadings,
    /// Energiekennzahlen eines Hausanschlusses (Viewer).
    GridAnalysis,
    /// Erwartung, Ampel und Wirtschaftlichkeit einer Anlage (Viewer).
    PvSystemReport,
);

/// Anlagendaten mit Zuordnung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PvSystemInput {
    pub data: PvSystemData,
    pub meters: Vec<(MeterId, PvRole)>,
}

// @impl SPEC_PV_ASSIGNMENT
fn validate_data(e: &mut FieldErrors, d: &PvSystemData) {
    validate::name(e, "name", &d.name, MAX_NAME, "pv.name_invalid");
    text_opt(e, "inverter", d.inverter.as_ref(), MAX_INVERTER);
    if d.kwp <= Decimal::ZERO || d.kwp > Decimal::from(10_000) {
        e.add("kwp", "pv.kwp_invalid");
    } else {
        money(e, "kwp", d.kwp, 3);
    }
    if d.specific_yield <= Decimal::ZERO || d.specific_yield > Decimal::from(3000) {
        e.add("specific_yield", "pv.yield_invalid");
    } else {
        money(e, "specific_yield", d.specific_yield, 3);
    }
    if d.degradation_pct < Decimal::ZERO || d.degradation_pct > Decimal::from(10) {
        e.add("degradation_pct", "pv.degradation_invalid");
    } else {
        money(e, "degradation_pct", d.degradation_pct, 3);
    }
    if d.alarm_threshold_pct == 0 || d.alarm_threshold_pct > 100 {
        e.add("alarm_threshold_pct", "pv.threshold_invalid");
    }
    if d.azimuth.is_some_and(|a| !(0..=360).contains(&a)) {
        e.add("azimuth", "pv.azimuth_invalid");
    }
    if d.tilt.is_some_and(|t| !(0..=90).contains(&t)) {
        e.add("tilt", "pv.tilt_invalid");
    }
    if d.battery_kwh.is_some_and(|b| b <= Decimal::ZERO) {
        e.add("battery_kwh", "pv.capacity_invalid");
    }
    if let Some(shares) = d.month_shares {
        let sum: Decimal = shares.iter().sum();
        if shares.iter().any(|s| *s < Decimal::ZERO)
            || (sum - Decimal::ONE_HUNDRED).abs() > Decimal::ONE
        {
            e.add("month_shares", "pv.shares_invalid");
        }
    }
}

/// Zuordnung gemäß SPEC_PV_ASSIGNMENT: genau ein Hausanschluss, mindestens
/// ein Erzeugungszähler, alle Messstellen Strom im Haushalt, Speicher mit
/// den Registern `charge` und `discharge`, Erzeugung und Speicher in keiner
/// weiteren Anlage, Einspeiseregister außer bei Steckersolargeräten.
// @impl SPEC_PV_ASSIGNMENT
async fn check_assignment(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    own: Option<PvSystemId>,
    input: &PvSystemInput,
) -> Result<(), AppError> {
    let mut e = FieldErrors::default();
    let count = |role| input.meters.iter().filter(|(_, r)| *r == role).count();
    if count(PvRole::Grid) != 1 {
        e.add("grid", "pv.grid_required");
    }
    if count(PvRole::Generation) == 0 {
        e.add("generation", "pv.generation_required");
    }
    let mut seen = std::collections::HashSet::new();
    for (m, _) in &input.meters {
        if !seen.insert(*m) {
            e.add("meters", "pv.meter_duplicate");
        }
    }
    for (m, role) in &input.meters {
        let agg = uow.meters().get(t, *m).await?.ok_or(AppError::NotFound)?;
        if agg.meter.data.kind != MeterKind::Electricity {
            e.add(role.as_str(), "pv.meter_kind_invalid");
            continue;
        }
        match role {
            PvRole::Battery => {
                let ok = agg.registers.len() == 2
                    && agg.registers.iter().any(|r| {
                        r.data.code == BATTERY_CHARGE && r.data.direction == Direction::Import
                    })
                    && agg.registers.iter().any(|r| {
                        r.data.code == BATTERY_DISCHARGE && r.data.direction == Direction::Export
                    });
                if !ok {
                    e.add("battery", "pv.battery_registers");
                }
            }
            PvRole::Grid => {
                let export = agg
                    .registers
                    .iter()
                    .any(|r| r.data.direction == Direction::Export);
                if input.data.kind.needs_export() && !export {
                    e.add("kind", "pv.export_required");
                }
            }
            PvRole::Generation => {}
        }
    }
    e.into_result()?;
    for other in uow.pv().list(t).await? {
        if Some(other.id) == own {
            continue;
        }
        let taken = input.meters.iter().any(|(m, role)| {
            *role != PvRole::Grid
                && other
                    .meters
                    .iter()
                    .any(|(x, r)| x == m && *r != PvRole::Grid)
        });
        if taken {
            return Err(AppError::Conflict("pv.meter_assigned"));
        }
    }
    Ok(())
}

fn system_of(id: PvSystemId, input: PvSystemInput) -> PvSystem {
    let mut data = input.data;
    data.name = data.name.trim().to_owned();
    data.inverter = data
        .inverter
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    PvSystem {
        id,
        data,
        meters: input.meters,
    }
}

impl ListPvSystems {
    #[tracing::instrument(name = "ListPvSystems", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<Vec<PvSystem>, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        Ok(uow.pv().list(t).await?)
    }
}

impl GetPvSystem {
    #[tracing::instrument(name = "GetPvSystem", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: PvSystemId) -> Result<PvSystem, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        uow.pv().get(t, id).await?.ok_or(AppError::NotFound)
    }
}

impl CreatePvSystem {
    // @impl SPEC_PV_ASSIGNMENT
    #[tracing::instrument(name = "CreatePvSystem", skip_all)]
    pub async fn execute(&self, t: &Tenant, input: PvSystemInput) -> Result<PvSystemId, AppError> {
        t.require(Role::Owner)?;
        let mut e = FieldErrors::default();
        validate_data(&mut e, &input.data);
        e.into_result()?;
        let mut uow = self.p.uow.begin().await?;
        check_assignment(uow.as_mut(), t, None, &input).await?;
        let id = PvSystemId(self.p.ids.new_uuid());
        uow.pv().create(t, &system_of(id, input)).await?;
        uow.commit().await?;
        Ok(id)
    }
}

impl UpdatePvSystem {
    // @impl SPEC_PV_ASSIGNMENT
    #[tracing::instrument(name = "UpdatePvSystem", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        id: PvSystemId,
        input: PvSystemInput,
    ) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut e = FieldErrors::default();
        validate_data(&mut e, &input.data);
        e.into_result()?;
        let mut uow = self.p.uow.begin().await?;
        uow.pv().get(t, id).await?.ok_or(AppError::NotFound)?;
        check_assignment(uow.as_mut(), t, Some(id), &input).await?;
        uow.pv().update(t, &system_of(id, input)).await?;
        uow.commit().await?;
        Ok(())
    }
}

impl DeletePvSystem {
    /// Löscht auch die Kostenbuch-Einträge der Anlage.
    #[tracing::instrument(name = "DeletePvSystem", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: PvSystemId) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        uow.pv().get(t, id).await?.ok_or(AppError::NotFound)?;
        for c in uow.asset_costs().list(t, AssetRef::pv(id)).await? {
            uow.asset_costs().delete(t, c.id).await?;
        }
        uow.pv().delete(t, id).await?;
        uow.commit().await?;
        Ok(())
    }
}

/// Anlagen am Hausanschluss `grid`.
async fn systems_at(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    grid: MeterId,
) -> Result<Vec<PvSystem>, AppError> {
    Ok(uow
        .pv()
        .list(t)
        .await?
        .into_iter()
        .filter(|s| s.grid() == Some(grid))
        .collect())
}

/// Stände der Sammelerfassung: Messstelle, Register und Wert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkInput {
    pub at: DateTime<Utc>,
    pub values: Vec<(MeterId, RegisterId, Decimal)>,
    pub note: Option<String>,
    pub confirm_warnings: bool,
    /// Foto-Entwürfe und die Messstelle, deren Stände sie belegen
    pub drafts: Vec<(DraftId, MeterId)>,
}

/// Ergebnis der Sammelerfassung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkOutcome {
    Saved(usize),
    /// Warnungen je Messstelle; nichts gespeichert
    NeedsConfirmation(Vec<(MeterId, Vec<RegisterCheck>)>),
}

/// Messstellen eines Hausanschlusses mit Rolle, Hausanschluss zuerst.
async fn grid_meters(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    grid: MeterId,
) -> Result<Vec<(MeterAggregate, PvRole)>, AppError> {
    let systems = systems_at(uow, t, grid).await?;
    if systems.is_empty() {
        return Err(AppError::NotFound);
    }
    let mut ids: Vec<(MeterId, PvRole)> = vec![(grid, PvRole::Grid)];
    for s in &systems {
        for (m, r) in &s.meters {
            if !ids.iter().any(|(x, _)| x == m) {
                ids.push((*m, *r));
            }
        }
    }
    let mut out = Vec::new();
    for (id, role) in ids {
        if let Some(agg) = uow.meters().get(t, id).await? {
            out.push((agg, role));
        }
    }
    Ok(out)
}

impl RecordBulkReadings {
    /// Messstellen der Sammelerfassung (Hausanschluss, Erzeugung, Speicher).
    #[tracing::instrument(name = "RecordBulkReadings::meters", skip_all)]
    pub async fn meters(
        &self,
        t: &Tenant,
        grid: MeterId,
    ) -> Result<Vec<(MeterAggregate, PvRole)>, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        grid_meters(uow.as_mut(), t, grid).await
    }

    /// Alle Stände mit gemeinsamem Zeitpunkt in einer Transaktion; Prüfung
    /// wie bei der Einzelerfassung, bei Warnungen wird nichts gespeichert
    /// (SPEC_PV_BULK_CAPTURE).
    // @impl SPEC_PV_BULK_CAPTURE, SPEC_RDG_PLAUSIBILITY
    #[tracing::instrument(name = "RecordBulkReadings", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        grid: MeterId,
        input: BulkInput,
    ) -> Result<BulkOutcome, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let meters = grid_meters(uow.as_mut(), t, grid).await?;
        if input.values.is_empty() {
            return Err(AppError::field("values", "reading.values_required"));
        }
        if input
            .values
            .iter()
            .any(|(m, _, _)| !meters.iter().any(|(a, _)| a.meter.id == *m))
        {
            return Err(AppError::NotFound);
        }
        // Fotos: offene, analysierte Entwürfe je Messstelle des Hausanschlusses
        let mut drafts = Vec::new();
        for (draft, meter) in &input.drafts {
            if !meters.iter().any(|(a, _)| a.meter.id == *meter) {
                return Err(AppError::NotFound);
            }
            let d = uow
                .drafts()
                .get_open(t, *draft)
                .await?
                .ok_or(AppError::NotFound)?;
            if !d.status.is_ready() {
                return Err(AppError::Conflict("capture.not_ready"));
            }
            drafts.push((d, *meter));
        }
        let mut warnings = Vec::new();
        let mut saved: Vec<(MeterId, RegisterId, ReadingId)> = Vec::new();
        for (agg, _) in &meters {
            let values: Vec<(RegisterId, Decimal)> = input
                .values
                .iter()
                .filter(|(m, _, _)| *m == agg.meter.id)
                .map(|(_, r, v)| (*r, *v))
                .collect();
            if values.is_empty() {
                continue;
            }
            let photo = drafts.iter().any(|(_, m)| *m == agg.meter.id);
            // mit Foto je Register eine eigene Ablesung mit eigenem Beleg
            let batches: Vec<Vec<(RegisterId, Decimal)>> = if photo {
                values.iter().map(|v| vec![*v]).collect()
            } else {
                vec![values]
            };
            for batch in batches {
                match self
                    .record_batch(uow.as_mut(), t, agg.meter.id, &batch, &input, photo)
                    .await?
                {
                    RecordOutcome::Saved(ids) => saved.extend(
                        batch
                            .iter()
                            .zip(ids)
                            .map(|((r, _), id)| (agg.meter.id, *r, id)),
                    ),
                    RecordOutcome::NeedsConfirmation(checks) => {
                        warnings.push((agg.meter.id, checks));
                    }
                }
            }
        }
        if !warnings.is_empty() {
            return Ok(BulkOutcome::NeedsConfirmation(warnings));
        }
        // Alarme wie bei der Einzelerfassung (E14)
        for (agg, _) in &meters {
            let ids: Vec<ReadingId> = saved
                .iter()
                .filter(|(m, _, _)| *m == agg.meter.id)
                .map(|(_, _, id)| *id)
                .collect();
            if !ids.is_empty() {
                enqueue_readings(&self.p, uow.as_mut(), t, agg.meter.id, &ids).await?;
            }
        }
        for (d, meter) in &drafts {
            self.attach_photo(uow.as_mut(), t, d, *meter, &saved, &input)
                .await?;
        }
        uow.commit().await?;
        for (d, _) in &drafts {
            if let Err(e) = self.p.blobs.delete(&d.photo_key).await {
                tracing::warn!(error = %e, draft = %d.id, "draft photo not deleted");
            }
        }
        Ok(BulkOutcome::Saved(saved.len()))
    }

    /// Stände einer Messstelle; mit Foto als `source = photo` mit eigenem
    /// Belegschlüssel. Feldfehler erhalten die Messstelle im Feldnamen.
    async fn record_batch(
        &self,
        uow: &mut dyn UnitOfWork,
        t: &Tenant,
        meter: MeterId,
        values: &[(RegisterId, Decimal)],
        input: &BulkInput,
        photo: bool,
    ) -> Result<RecordOutcome, AppError> {
        let (extras, source) = if photo {
            let id = ReadingId(self.p.ids.new_uuid());
            let extras = ReadingExtras {
                id: Some(id),
                photo_key: Some(BlobKey::reading_photo(t.household_id(), id)),
                ai_confidence: None,
            };
            (extras, ReadingSource::Photo)
        } else {
            (ReadingExtras::default(), ReadingSource::Manual)
        };
        let readings = ReadingsInput {
            meter_id: meter,
            at: input.at,
            values: values.to_vec(),
            note: input.note.clone(),
        };
        record_in(
            &self.p,
            uow,
            t,
            &readings,
            input.confirm_warnings,
            source,
            extras,
        )
        .await
        .map_err(|e| match e {
            AppError::Validation(e) => AppError::Validation(FieldErrors(
                e.0.into_iter()
                    .map(|x| crate::FieldError {
                        field: format!("{meter}_{}", x.field),
                        ..x
                    })
                    .collect(),
            )),
            other => other,
        })
    }

    /// Belegfoto je Ablesung der Messstelle, Entwurf abschließen.
    async fn attach_photo(
        &self,
        uow: &mut dyn UnitOfWork,
        t: &Tenant,
        d: &Draft,
        meter: MeterId,
        saved: &[(MeterId, RegisterId, ReadingId)],
        input: &BulkInput,
    ) -> Result<(), AppError> {
        let mine: Vec<&(MeterId, RegisterId, ReadingId)> =
            saved.iter().filter(|(m, _, _)| *m == meter).collect();
        let Some(first) = mine.first() else {
            return Err(AppError::field("values", "reading.values_required"));
        };
        for (_, _, id) in &mine {
            self.p
                .blobs
                .copy(&d.photo_key, &BlobKey::reading_photo(t.household_id(), *id))
                .await
                .map_err(AppError::from_storage)?;
        }
        let value = input
            .values
            .iter()
            .find(|(m, r, _)| *m == meter && *r == first.1)
            .map(|(_, _, v)| *v);
        let corrected = d.suggestion.as_ref().map(|s| s.value != value);
        uow.drafts()
            .finish(
                t,
                d.id,
                DraftOutcome::Confirmed {
                    reading: first.2,
                    corrected,
                },
                self.p.clock.now(),
            )
            .await?;
        Ok(())
    }

    /// Offene Foto-Entwürfe mit Vorschlag für Messstelle und Register des
    /// Hausanschlusses (SPEC_PV_BULK_CAPTURE).
    #[tracing::instrument(name = "RecordBulkReadings::drafts", skip_all)]
    pub async fn drafts(&self, t: &Tenant, grid: MeterId) -> Result<Vec<BulkDraft>, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        let meters = grid_meters(uow.as_mut(), t, grid).await?;
        Ok(uow
            .drafts()
            .list_open(t)
            .await?
            .into_iter()
            .map(|draft| {
                let agg = meters
                    .iter()
                    .map(|(a, _)| a)
                    .find(|a| Some(a.meter.id) == draft.matched_meter_id);
                let register = agg.and_then(|a| {
                    choose_register(
                        a,
                        draft
                            .suggestion
                            .as_ref()
                            .and_then(|s| s.register.as_deref()),
                    )
                });
                BulkDraft {
                    meter: agg.map(|a| a.meter.id),
                    register,
                    draft,
                }
            })
            .collect())
    }
}

/// Offener Foto-Entwurf in der Sammelerfassung.
#[derive(Debug, Clone)]
pub struct BulkDraft {
    pub draft: Draft,
    /// Erkannte Messstelle, falls sie zum Hausanschluss gehört
    pub meter: Option<MeterId>,
    pub register: Option<RegisterId>,
}

/// Intervalle der Register einer Messstelle, gefiltert.
fn groups(
    agg: &MeterAggregate,
    readings: &[crate::model::Reading],
    keep: impl Fn(&crate::model::Register) -> bool,
) -> Vec<Vec<Interval>> {
    register_intervals(agg, readings)
        .into_iter()
        .filter(|r| keep(&r.register))
        .map(|r| r.intervals.unwrap_or_default())
        .collect()
}

fn sum(groups: &[Vec<Interval>], from: DateTime<Utc>, to: DateTime<Utc>) -> Option<Decimal> {
    let refs: Vec<&[Interval]> = groups.iter().map(Vec::as_slice).collect();
    total_in(&refs, from, to)
}

/// Messreihen eines Hausanschlusses mit seinen Anlagen.
pub(crate) struct GridSource {
    pub grid: MeterAggregate,
    pub systems: Vec<PvSystem>,
    pub setup: GridSetup,
    pub cal: Calendar,
    import: Vec<Vec<Interval>>,
    export: Vec<Vec<Interval>>,
    generation: Vec<(PvSystemId, Vec<Vec<Interval>>)>,
    charge: Vec<Vec<Interval>>,
    discharge: Vec<Vec<Interval>>,
    /// Erster Tag mit Ablesung einer beteiligten Messstelle
    pub first: Option<NaiveDate>,
}

impl GridSource {
    /// `None`, wenn keine Anlage den Hausanschluss verwendet.
    pub(crate) async fn load(
        uow: &mut dyn UnitOfWork,
        t: &Tenant,
        grid: MeterId,
        cal: Calendar,
    ) -> Result<Option<Self>, AppError> {
        let systems = systems_at(uow, t, grid).await?;
        if systems.is_empty() {
            return Ok(None);
        }
        let agg = uow.meters().get(t, grid).await?.ok_or(AppError::NotFound)?;
        let readings = uow.readings().list_for_meter(t, grid).await?;
        let mut first = readings.first().map(|r| cal.local_date(r.read_at));
        let import = groups(&agg, &readings, |r| r.data.direction == Direction::Import);
        let export = groups(&agg, &readings, |r| r.data.direction == Direction::Export);
        let mut generation = Vec::new();
        let (mut charge, mut discharge) = (Vec::new(), Vec::new());
        let mut done = Vec::new();
        for s in &systems {
            let mut own = Vec::new();
            for (m, role) in &s.meters {
                if *role == PvRole::Grid || done.contains(m) {
                    continue;
                }
                done.push(*m);
                let Some(a) = uow.meters().get(t, *m).await? else {
                    continue;
                };
                let rs = uow.readings().list_for_meter(t, *m).await?;
                if let Some(f) = rs.first().map(|r| cal.local_date(r.read_at)) {
                    first = Some(first.map_or(f, |x| x.min(f)));
                }
                match role {
                    PvRole::Generation => {
                        // Erzeugung: Einspeiseregister, sonst Bezugsregister
                        let has_export = a
                            .registers
                            .iter()
                            .any(|r| r.data.direction == Direction::Export);
                        let dir = if has_export {
                            Direction::Export
                        } else {
                            Direction::Import
                        };
                        own.extend(groups(&a, &rs, |r| r.data.direction == dir));
                    }
                    PvRole::Battery => {
                        charge.extend(groups(&a, &rs, |r| r.data.code == BATTERY_CHARGE));
                        discharge.extend(groups(&a, &rs, |r| r.data.code == BATTERY_DISCHARGE));
                    }
                    PvRole::Grid => {}
                }
            }
            generation.push((s.id, own));
        }
        let setup = GridSetup {
            has_generation: generation.iter().any(|(_, g)| !g.is_empty()),
            has_export_register: !export.is_empty(),
            has_battery: !charge.is_empty(),
            full_feed_in: systems.iter().all(|s| s.data.kind == PvKind::FullFeedIn),
        };
        Ok(Some(Self {
            grid: agg,
            systems,
            setup,
            cal,
            import,
            export,
            generation,
            charge,
            discharge,
            first,
        }))
    }

    fn bounds(&self, p: Period) -> (DateTime<Utc>, DateTime<Utc>) {
        (self.cal.start_of(p.start), self.cal.start_of(p.end))
    }

    /// Erzeugung aller Anlagen im Zeitraum.
    fn generation_total(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Option<Decimal> {
        let all: Vec<Vec<Interval>> = self
            .generation
            .iter()
            .flat_map(|(_, g)| g.iter().cloned())
            .collect();
        if all.is_empty() {
            None
        } else {
            sum(&all, from, to)
        }
    }

    /// Erzeugung einer Anlage im Zeitraum.
    pub(crate) fn generation_of(&self, id: PvSystemId, p: Period) -> Option<Decimal> {
        let (from, to) = self.bounds(p);
        self.generation
            .iter()
            .find(|(s, _)| *s == id)
            .filter(|(_, g)| !g.is_empty())
            .and_then(|(_, g)| sum(g, from, to))
    }

    /// Kennzahlen des Zeitraums (SPEC_PV_KPI).
    // @impl SPEC_PV_KPI
    pub(crate) fn kpi(&self, p: Period) -> Kpi {
        let (from, to) = self.bounds(p);
        let opt = |g: &[Vec<Interval>]| if g.is_empty() { None } else { sum(g, from, to) };
        kpis(
            self.setup,
            Flows {
                generation: self.generation_total(from, to),
                import: opt(&self.import),
                export: opt(&self.export),
                charge: opt(&self.charge),
                discharge: opt(&self.discharge),
            },
        )
    }

    /// Speicherkennzahlen des Zeitraums (SPEC_PV_BATTERY).
    pub(crate) fn battery(&self, k: &Kpi) -> Option<BatteryKpi> {
        let capacity: Option<Decimal> = self
            .systems
            .iter()
            .filter_map(|s| s.data.battery_kwh)
            .reduce(|a, b| a + b);
        Some(battery_kpis(k.charge?, k.discharge?, capacity))
    }

    /// Letzter Zeitpunkt, den alle Messreihen abdecken.
    pub(crate) fn covered_until(&self) -> Option<NaiveDate> {
        let mut all: Vec<&[Interval]> = Vec::new();
        for g in self
            .import
            .iter()
            .chain(&self.export)
            .chain(&self.charge)
            .chain(&self.discharge)
            .chain(self.generation.iter().flat_map(|(_, g)| g))
        {
            all.push(g);
        }
        covered_until(&all).map(|t| self.cal.local_date(t))
    }

    /// Gesamtverbrauch je Monat, falls Erzeugung gemessen wird
    /// (REQ_PV_006).
    pub(crate) fn consumption_months(&self, months: &[Period]) -> Option<Vec<MonthValue>> {
        if !self.setup.has_generation {
            return None;
        }
        Some(
            months
                .iter()
                .map(|m| MonthValue {
                    month: *m,
                    value: self.kpi(*m).consumption,
                })
                .collect(),
        )
    }

    /// Erzeugung einer Anlage je Monat.
    pub(crate) fn generation_months(&self, id: PvSystemId, months: &[Period]) -> Vec<MonthValue> {
        months
            .iter()
            .map(|m| MonthValue {
                month: *m,
                value: self.generation_of(id, *m),
            })
            .collect()
    }

    /// Autarkiegrad und durchschnittliche Einspeisevergütung `v_ein` im
    /// Zeitraum; `pricing` ist die Kostenrechnung des Hausanschlusses.
    pub(crate) fn autarky_and_rate(
        &self,
        p: Period,
        pricing: &MeterPricing,
    ) -> (Option<Decimal>, Decimal) {
        let k = self.kpi(p);
        (k.autarky, feed_in_rate(&k, pricing, p))
    }
}

/// Vergütung des Einspeisevertrags im Zeitraum (Gutschrift positiv).
fn feed_in_total(pricing: &MeterPricing, p: Period) -> Decimal {
    pricing
        .period_cost(Direction::Export, p)
        .map_or(Decimal::ZERO, |c| -c.total())
}

/// `v_ein = Vergütung / E` (SPEC_PV_FEED_IN).
// @impl SPEC_PV_FEED_IN
fn feed_in_rate(k: &Kpi, pricing: &MeterPricing, p: Period) -> Decimal {
    match k.export {
        Some(e) if e > Decimal::ZERO => feed_in_total(pricing, p) / e,
        _ => Decimal::ZERO,
    }
}

/// Kennzahlen eines Zeitraums.
#[derive(Debug, Clone)]
pub struct GridPeriod {
    pub period: Period,
    /// Zeitraum endet vor seinem Ende (laufendes Jahr bis zur letzten Ablesung)
    pub partial: bool,
    pub kpi: Kpi,
    pub battery: Option<BatteryKpi>,
}

/// Spezifischer Ertrag einer Anlage in einem Jahr.
#[derive(Debug, Clone)]
pub struct SystemYear {
    pub system: PvSystemId,
    pub name: String,
    pub year: i32,
    pub generation: Option<Decimal>,
    /// kWh/kWp
    pub specific: Option<Decimal>,
}

/// Ergebnis von [`GridAnalysis`].
#[derive(Debug, Clone)]
pub struct GridView {
    pub grid: MeterAggregate,
    pub systems: Vec<PvSystem>,
    pub setup: GridSetup,
    /// Monate von der ersten Ablesung bis heute
    pub months: Vec<GridPeriod>,
    /// Kalenderjahre, neuestes zuerst
    pub years: Vec<GridPeriod>,
    pub billing_year: Option<GridPeriod>,
    pub yields: Vec<SystemYear>,
}

impl GridView {
    /// Letzter vollständig gemessener Monat.
    #[must_use]
    pub fn last_complete(&self) -> Option<&GridPeriod> {
        self.months.iter().rev().find(|m| {
            m.kpi.import.is_some() && (m.kpi.generation.is_some() || !self.setup.has_generation)
        })
    }
}

fn period_row(src: &GridSource, p: Period, until: Option<NaiveDate>) -> Option<GridPeriod> {
    let end = until.map_or(p.end, |u| p.end.min(u));
    if end <= p.start {
        return None;
    }
    let clipped = Period::new(p.start, end);
    let kpi = src.kpi(clipped);
    Some(GridPeriod {
        period: clipped,
        partial: end < p.end,
        battery: src.battery(&kpi),
        kpi,
    })
}

impl GridAnalysis {
    /// Kennzahlen je Monat, Kalenderjahr und Abrechnungsjahr; Jahre bis zur
    /// letzten gemeinsamen Ablesung (SPEC_PV_KPI, SPEC_PV_BATTERY).
    // @impl SPEC_PV_KPI, SPEC_PV_BATTERY
    #[tracing::instrument(name = "GridAnalysis", skip_all)]
    pub async fn execute(&self, t: &Tenant, grid: MeterId) -> Result<GridView, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let (cal, today) = calendar_today(&self.p, uow.as_mut(), t).await?;
        let src = GridSource::load(uow.as_mut(), t, grid, cal)
            .await?
            .ok_or(AppError::NotFound)?;
        drop(uow);
        let months = src
            .first
            .map(|f| months_between(f, today))
            .unwrap_or_default();
        let until = src.covered_until();
        let rows = months
            .iter()
            .map(|m| {
                let kpi = src.kpi(*m);
                GridPeriod {
                    period: *m,
                    partial: false,
                    battery: src.battery(&kpi),
                    kpi,
                }
            })
            .collect();
        let years: Vec<i32> = src
            .first
            .map(|f| (f.year()..=today.year()).rev().collect())
            .unwrap_or_default();
        let year_rows = years
            .iter()
            .filter_map(|y| period_row(&src, Period::year(*y), until))
            .collect();
        let billing_year = src.grid.meter.data.billing_start.and_then(|md| {
            zaehler_domain::analysis::current_billing_year(
                today,
                u32::from(md.month),
                u32::from(md.day),
            )
            .and_then(|p| period_row(&src, p, until))
        });
        let mut yields = Vec::new();
        for s in &src.systems {
            for y in &years {
                let Some(p) = until
                    .map(|u| Period::year(*y).end.min(u))
                    .filter(|e| *e > Period::year(*y).start)
                    .map(|e| Period::new(Period::year(*y).start, e))
                else {
                    continue;
                };
                let generation = src.generation_of(s.id, p);
                yields.push(SystemYear {
                    system: s.id,
                    name: s.data.name.clone(),
                    year: *y,
                    specific: generation.map(|g| (g / s.data.kwp).round_dp(1)),
                    generation,
                });
            }
        }
        Ok(GridView {
            months: rows,
            years: year_rows,
            billing_year,
            yields,
            setup: src.setup,
            systems: src.systems.clone(),
            grid: src.grid.clone(),
        })
    }
}

/// Wirtschaftlichkeit eines Monats oder Jahres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PvPeriodEconomics {
    pub period: Period,
    pub generation: Option<Decimal>,
    pub economics: Economics,
    /// davon Investitionen und Ersatzinvestitionen
    pub investments: Decimal,
    /// Bilanz kumuliert seit Inbetriebnahme am Ende des Zeitraums
    pub cumulative: Decimal,
    /// fortgeschrieben (Zukunft)
    pub projected: bool,
}

/// Jahresprognose der Erzeugung (SPEC_PV_YIELD_EXPECTATION).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YieldForecast {
    pub year: i32,
    pub measured: Decimal,
    pub expected_rest: Decimal,
}

impl YieldForecast {
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.measured + self.expected_rest
    }
}

/// Ergebnis von [`PvSystemReport`].
#[derive(Debug, Clone)]
pub struct PvReport {
    pub system: PvSystem,
    pub grid_name: Option<String>,
    /// Erzeugung gegen Erwartung je Monat seit Inbetriebnahme
    pub yields: Vec<YieldMonth>,
    pub forecast: Option<YieldForecast>,
    /// Jahre seit Inbetriebnahme (bis heute)
    pub years: Vec<PvPeriodEconomics>,
    /// Monate gemessen und fortgeschrieben (Diagramm der Bilanz)
    pub months: Vec<PvPeriodEconomics>,
    /// Amortisationszeitpunkt und ob er fortgeschrieben ist
    pub break_even: Option<(NaiveDate, bool)>,
    pub simple_return: Option<Decimal>,
    pub investments: Decimal,
}

fn is_investment(c: &AssetCost) -> bool {
    matches!(
        c.category,
        AssetCategory::Pv(PvCostCategory::Investment | PvCostCategory::Replacement)
    )
}

impl PvSystemReport {
    /// Erwartung und Ampel je Monat, Jahresprognose, Vergütung, Ersparnis,
    /// Bilanz, Amortisation und Rendite (SPEC_PV_ECONOMICS).
    // @impl SPEC_PV_YIELD_EXPECTATION, SPEC_PV_MONITORING, SPEC_PV_ECONOMICS
    #[tracing::instrument(name = "PvSystemReport", skip_all)]
    #[allow(clippy::too_many_lines)] // Zusammenstellung aller Kennzahlen
    pub async fn execute(&self, t: &Tenant, id: PvSystemId) -> Result<PvReport, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let (cal, today) = calendar_today(&self.p, uow.as_mut(), t).await?;
        let system = uow.pv().get(t, id).await?.ok_or(AppError::NotFound)?;
        let grid = system.grid().ok_or(AppError::NotFound)?;
        let src = GridSource::load(uow.as_mut(), t, grid, cal)
            .await?
            .ok_or(AppError::NotFound)?;
        let pricing = load_pricing(uow.as_mut(), t, grid, cal, today).await?;
        let costs = uow.asset_costs().list(t, AssetRef::pv(id)).await?;
        drop(uow);

        let start = system.data.commissioned_on;
        let months = if start <= today {
            months_between(start, today)
        } else {
            Vec::new()
        };
        let series = src.generation_months(id, &months);
        let yields = yield_months(&system.data, &series);
        let until = src.covered_until();

        // Jahresprognose: gemessen bis zur letzten Ablesung, Rest erwartet
        let year = Period::year(today.year());
        let forecast = until.and_then(|u| {
            let from = year.start.max(start);
            let measured_end = u.min(year.end);
            let measured = if measured_end > from {
                src.generation_of(id, Period::new(from, measured_end))?
            } else {
                Decimal::ZERO
            };
            let rest = if measured_end < year.end {
                expectation_in(
                    &system.data,
                    &series,
                    Period::new(measured_end.max(from), year.end),
                )
            } else {
                Decimal::ZERO
            };
            Some(YieldForecast {
                year: today.year(),
                measured,
                expected_rest: rest.round_dp(3),
            })
        });

        // gemessene Monate bis zur letzten gemeinsamen Ablesung
        let last_measured = until.map(|u| Period::month_of(u).start);
        let month_costs = |p: Period| -> (Decimal, Decimal) {
            let all: Decimal = costs.iter().map(|c| c.amount_in(p)).sum();
            let inv: Decimal = costs
                .iter()
                .filter(|c| is_investment(c))
                .map(|c| c.amount_in(p))
                .sum();
            (all, inv)
        };
        let mut rows: Vec<PvPeriodEconomics> = Vec::new();
        let mut cumulative = Decimal::ZERO;
        for m in &months {
            if last_measured.is_none_or(|l| m.start >= l) {
                break;
            }
            let k = src.kpi(*m);
            let own = src.generation_of(id, *m);
            let share = match (own, k.generation) {
                (Some(o), Some(g)) if g > Decimal::ZERO => o / g,
                _ => Decimal::ZERO,
            };
            let (all, inv) = month_costs(*m);
            let price = pricing.unit_price(*m).map(|p| p.value);
            let e = economics(
                share,
                k.self_consumption.unwrap_or(Decimal::ZERO),
                price,
                feed_in_total(&pricing, *m),
                k.export.unwrap_or(Decimal::ZERO),
                all,
            );
            cumulative += e.balance();
            rows.push(PvPeriodEconomics {
                period: *m,
                generation: own,
                economics: e,
                investments: inv,
                cumulative,
                projected: false,
            });
        }

        // Fortschreibung: Erwartung, Eigenverbrauchsquote und Preise der
        // letzten 12 Monate
        let measured_until = rows.last().map_or(start, |r| r.period.end);
        let last12 = Period::new(
            // höchstens 12 Monate, nicht vor der Inbetriebnahme
            measured_until
                .checked_sub_months(Months::new(12))
                .unwrap_or(measured_until)
                .max(start.min(measured_until)),
            measured_until,
        );
        let k12 = src.kpi(last12);
        let ratio = k12
            .self_consumption_ratio
            .unwrap_or(Decimal::ZERO)
            .clamp(Decimal::ZERO, Decimal::ONE);
        let price_now = pricing
            .unit_price(last12)
            .map_or(Decimal::ZERO, |p| p.value);
        let rate_now = feed_in_rate(&k12, &pricing, last12);
        let mut future = Vec::new();
        let mut m = Period::month_of(measured_until.max(start));
        let horizon = start + Months::new(PROJECTION_YEARS * 12);
        let mut history = series.clone();
        while m.start < horizon {
            let (expected, _) = expectation(&system.data, &history, m);
            let (all, inv) = month_costs(m);
            let e = Economics {
                feed_in: (expected * (Decimal::ONE - ratio) * rate_now).round_dp(2),
                savings: (expected * ratio * price_now).round_dp(2),
                advantage: (expected * ratio * (price_now - rate_now)).round_dp(2),
                costs: all,
            };
            cumulative += e.balance();
            future.push(PvPeriodEconomics {
                period: m,
                generation: Some(expected),
                economics: e,
                investments: inv,
                cumulative,
                projected: true,
            });
            history.push(MonthValue {
                month: m,
                value: Some(expected),
            });
            m = m.shift_months(1);
        }

        let all_months: Vec<PvPeriodEconomics> = rows.iter().chain(&future).copied().collect();
        let flows: Vec<(Period, Decimal, Decimal)> = all_months
            .iter()
            .map(|r| {
                (
                    r.period,
                    r.economics.feed_in + r.economics.savings,
                    r.economics.costs,
                )
            })
            .collect();
        let break_even = break_even(Decimal::ZERO, &flows).map(|d| (d, d >= measured_until));
        let investments: Decimal = costs
            .iter()
            .filter(|c| is_investment(c))
            .map(|c| {
                c.amount_in(Period::new(
                    start.min(c.booked_on),
                    today.succ_opt().unwrap_or(today),
                ))
            })
            .sum();
        let simple_return = if rows.is_empty() {
            None
        } else {
            let without: Decimal = rows
                .iter()
                .map(|r| r.economics.balance() + r.investments)
                .sum();
            let annual = without / Decimal::from(rows.len()) * Decimal::from(12);
            simple_return(&[annual], investments)
        };
        let years = aggregate_years(&rows);
        // Diagramm: gemessen und fünf Jahre Fortschreibung
        let chart_end = measured_until + Months::new(60);
        let months = all_months
            .into_iter()
            .filter(|r| !r.projected || r.period.start < chart_end)
            .collect();
        Ok(PvReport {
            grid_name: Some(src.grid.meter.data.name.clone()),
            system,
            yields,
            forecast,
            years,
            months,
            break_even,
            simple_return,
            investments,
        })
    }
}

/// Monate zu Kalenderjahren.
fn aggregate_years(rows: &[PvPeriodEconomics]) -> Vec<PvPeriodEconomics> {
    let mut out: Vec<PvPeriodEconomics> = Vec::new();
    for r in rows {
        let y = r.period.start.year();
        match out.last_mut().filter(|x| x.period.start.year() == y) {
            Some(x) => {
                x.period = Period::new(x.period.start, r.period.end);
                x.generation = x.generation.zip(r.generation).map(|(a, b)| a + b);
                x.economics.feed_in += r.economics.feed_in;
                x.economics.savings += r.economics.savings;
                x.economics.advantage += r.economics.advantage;
                x.economics.costs += r.economics.costs;
                x.investments += r.investments;
                x.cumulative = r.cumulative;
            }
            None => out.push(*r),
        }
    }
    out.reverse();
    out
}

/// Ladepreis mit PV für den Hausanschluss `grid` im Zeitraum
/// (SPEC_CAR_ENERGY_COST): PV-Anteil am Fahrzeug, sonst Autarkiegrad;
/// `None` ohne Anlage am Hausanschluss.
// @impl SPEC_CAR_ENERGY_COST
pub(crate) fn pv_charging_price(
    src: &GridSource,
    pricing: &MeterPricing,
    p: Period,
    pv_share_pct: Option<Decimal>,
    house_price: Decimal,
) -> Option<Decimal> {
    let (autarky, rate) = src.autarky_and_rate(p, pricing);
    let share = pv_share_pct.map(|s| s / Decimal::ONE_HUNDRED).or(autarky)?;
    Some(zaehler_domain::pv::charging_price(share, rate, house_price))
}

/// Gesamtverbrauch je Monat für die Auswertung des Hausanschlusses
/// (REQ_PV_006); `None` ohne Anlage mit Erzeugungszähler.
// @impl SPEC_PV_KPI
pub(crate) async fn consumption_series(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    grid: MeterId,
    cal: Calendar,
    months: &[Period],
) -> Result<Option<Vec<MonthValue>>, AppError> {
    match GridSource::load(uow, t, grid, cal).await {
        Ok(Some(src)) => Ok(src.consumption_months(months)),
        Ok(None) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Erzeugung und Erwartung einer Anlage im Zeitraum einer neuen Ablesung
/// ihres Erzeugungszählers (SPEC_PV_MONITORING).
pub(crate) struct YieldCheck {
    pub system: PvSystem,
    pub generation: Decimal,
    pub expected: Decimal,
}

/// Anlagen mit `meter` als Erzeugungszähler und ihre Erzeugung im Intervall
/// `[from, to)` gegen die Erwartung.
// @impl SPEC_PV_MONITORING
pub(crate) async fn yield_checks(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    meter: MeterId,
    cal: Calendar,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<YieldCheck>, AppError> {
    let mut out = Vec::new();
    for s in uow.pv().list(t).await? {
        if !s.with_role(PvRole::Generation).any(|m| m == meter) {
            continue;
        }
        let Some(grid) = s.grid() else { continue };
        let Some(src) = GridSource::load(uow, t, grid, cal).await? else {
            continue;
        };
        let Some((_, groups)) = src.generation.iter().find(|(id, _)| *id == s.id) else {
            continue;
        };
        let Some(generation) = sum(groups, from, to) else {
            continue;
        };
        let (a, b) = (cal.local_date(from), cal.local_date(to));
        if b <= a {
            continue;
        }
        let months = months_between(s.data.commissioned_on.min(a), b);
        let series = src.generation_months(s.id, &months);
        let expected = expectation_in(&s.data, &series, Period::new(a, b));
        out.push(YieldCheck {
            system: s,
            generation,
            expected,
        });
    }
    Ok(out)
}

use_case!(
    /// PV-Karten der Übersicht je Hausanschluss (Viewer).
    PvDashboard,
);

/// Karte eines Hausanschlusses in der Übersicht (SPEC_PV_CHARTS).
#[derive(Debug, Clone)]
pub struct PvCard {
    pub grid: MeterId,
    pub name: String,
    /// Letzter vollständig gemessener Monat
    pub month: Option<Period>,
    pub generation: Option<Decimal>,
    pub autarky: Option<Decimal>,
    pub self_consumption_ratio: Option<Decimal>,
    /// Ungünstigste Ertragsampel der Anlagen in diesem Monat
    pub status: Option<zaehler_domain::expectation::Status>,
}

impl PvDashboard {
    /// Je Hausanschluss mit Anlage: Erzeugung des letzten vollständigen
    /// Monats, Autarkiegrad, Eigenverbrauchsquote und Ampel.
    // @impl SPEC_PV_CHARTS, SPEC_PV_MONITORING
    #[tracing::instrument(name = "PvDashboard", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<Vec<PvCard>, AppError> {
        use zaehler_domain::expectation::Status;
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let (cal, today) = calendar_today(&self.p, uow.as_mut(), t).await?;
        let mut grids: Vec<MeterId> = Vec::new();
        for s in uow.pv().list(t).await? {
            if let Some(g) = s.grid()
                && !grids.contains(&g)
            {
                grids.push(g);
            }
        }
        let mut cards = Vec::new();
        for grid in grids {
            let Some(src) = GridSource::load(uow.as_mut(), t, grid, cal).await? else {
                continue;
            };
            let months = src
                .first
                .map(|f| months_between(f, today))
                .unwrap_or_default();
            let last = months
                .iter()
                .rev()
                .map(|m| (*m, src.kpi(*m)))
                .find(|(_, k)| k.generation.is_some());
            let status = last.and_then(|(m, _)| {
                src.systems
                    .iter()
                    .filter_map(|s| {
                        let series = src.generation_months(s.id, &months);
                        yield_months(&s.data, &series)
                            .into_iter()
                            .find(|y| y.month == m)
                            .and_then(|y| y.status)
                    })
                    .max_by_key(|s| match s {
                        Status::Green => 0,
                        Status::Yellow => 1,
                        Status::Red => 2,
                    })
            });
            cards.push(PvCard {
                grid,
                name: src.grid.meter.data.name.clone(),
                month: last.map(|(m, _)| m),
                generation: last.and_then(|(_, k)| k.generation),
                autarky: last.and_then(|(_, k)| k.autarky),
                self_consumption_ratio: last.and_then(|(_, k)| k.self_consumption_ratio),
                status,
            });
        }
        Ok(cards)
    }
}
