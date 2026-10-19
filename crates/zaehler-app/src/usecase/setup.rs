//! Setup-Assistent: persistenter Entwurf, Fotoerkennung und atomarer Abschluss.

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde_json::{Map, Value};
use zaehler_domain::{
    meter::{Direction, MeterKind, MeterPurpose},
    pv::{DEFAULT_ALARM_THRESHOLD_PCT, DEFAULT_DEGRADATION_PCT, PvKind, PvRole, PvSystemData},
    vehicle::{DepreciationMode, DriveType, Holding},
};

use crate::{
    AppError, Role, Tenant,
    ids::{BlobKey, MeterId},
    model::{Recognition, SetupDraft},
    port::UnitOfWork,
    usecase::{
        contract::{ContractInput, CreateContract},
        meter::{CreateMeter, MeterInput, RegisterInput},
        pv::{CreatePvSystem, PvSystemInput},
        use_case,
        vehicle::{CreateVehicle, NewVehicle, SetAcquisition, VehicleInput},
    },
};

use_case!(
    /// Setup-Assistent öffnen oder fortsetzen (Owner).
    StartSetup,
    /// Offenen Setup-Entwurf abfragen (Owner).
    GetSetupDraft,
    /// Einen Schritt des Setup-Assistenten speichern (Owner).
    SaveSetupDraft,
    /// Setup-Entwurf samt Fotos verwerfen (Owner).
    DiscardSetup,
    /// Setup-Foto normalisieren und optional direkt erkennen (Owner).
    AnalyseSetupPhoto,
    /// Alle gewählten Setup-Objekte atomar anlegen (Owner).
    CompleteSetup,
);

const ALLOWED_STEPS: &[&str] = &[
    "grid",
    "heat_pump",
    "pv",
    "vehicle",
    "media",
    "contracts",
    "summary",
];
/// Foto-Schlüssel je Schritt; sie entsprechen den Registern der Objekte.
const STEP_PHOTO_KEYS: &[(&str, &[&str])] = &[
    ("grid", &["grid", "grid_export"]),
    ("heat_pump", &["heat_pump"]),
    (
        "pv",
        &["pv_generation", "pv_battery_charge", "pv_battery_discharge"],
    ),
    ("vehicle", &["wallbox"]),
    ("media", &["media_general"]),
];
const OBJECT_KEY_CHARS: &str = "abcdefghijklmnopqrstuvwxyz0123456789_";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupPhotoResult {
    pub photo_key: BlobKey,
    pub recognition: Option<Recognition>,
}

fn valid_step(step: &str) -> bool {
    ALLOWED_STEPS.contains(&step)
}

fn step_photo_keys(step: &str) -> &'static [&'static str] {
    STEP_PHOTO_KEYS
        .iter()
        .find(|(key, _)| *key == step)
        .map_or(&[], |(_, keys)| keys)
}

/// Foto-Schlüssel der Register eines lokalen Objekts, in Registerreihenfolge.
fn object_photo_keys(key: &str) -> &'static [&'static str] {
    match key {
        "grid" => &["grid", "grid_export"],
        "pv_battery" => &["pv_battery_charge", "pv_battery_discharge"],
        "heat_pump" => &["heat_pump"],
        "pv_generation" => &["pv_generation"],
        "wallbox" => &["wallbox"],
        "media_general" => &["media_general"],
        _ => &[],
    }
}

fn existing_id(object: &Value) -> Option<&str> {
    object_string(object, "existing_id")
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn valid_object_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 48 && key.chars().all(|c| OBJECT_KEY_CHARS.contains(c))
}

impl StartSetup {
    #[tracing::instrument(name = "StartSetup", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<SetupDraft, AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let draft = if let Some(draft) = uow.setup_drafts().get(t).await? {
            draft
        } else {
            let draft = SetupDraft::new(self.p.clock.now());
            uow.setup_drafts().save(t, &draft).await?;
            draft
        };
        uow.commit().await?;
        Ok(draft)
    }
}

impl GetSetupDraft {
    pub async fn execute(&self, t: &Tenant) -> Result<Option<SetupDraft>, AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        uow.setup_drafts().get(t).await.map_err(Into::into)
    }
}

impl SaveSetupDraft {
    /// Speichert die Auswahl und JSON-Nutzdaten des angegebenen Schritts.
    #[tracing::instrument(name = "SaveSetupDraft", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        step: &str,
        selected: bool,
        object: Option<Value>,
    ) -> Result<SetupDraft, AppError> {
        t.require(Role::Owner)?;
        if !valid_step(step) {
            return Err(AppError::field("step", "setup.step_invalid"));
        }
        let mut uow = self.p.uow.begin().await?;
        let now = self.p.clock.now();
        let mut draft = uow
            .setup_drafts()
            .get(t)
            .await?
            .unwrap_or_else(|| SetupDraft::new(now));
        draft.version = 1;
        draft.steps.insert(step.to_owned(), selected);
        if let Some(object) = object {
            if !object.is_object() {
                return Err(AppError::field("object", "setup.object_invalid"));
            }
            draft.objects.insert(step.to_owned(), object);
        } else if !selected {
            draft.objects.remove(step);
        }
        draft.updated_at = now;
        uow.setup_drafts().save(t, &draft).await?;
        uow.commit().await?;
        Ok(draft)
    }

    /// Speichert die Auswahl des Schritts und mehrere zugehörige lokale
    /// Objekte atomar, etwa Erzeugungszähler plus PV-Anlage. Nicht mehr
    /// benötigte Objekte und deren Fotos (Schritt abgewählt, Objekt
    /// weggelassen oder durch eine vorhandene Messstelle ersetzt) werden
    /// entfernt; die Zahl der gelöschten Fotos wird zurückgegeben.
    pub async fn execute_objects(
        &self,
        t: &Tenant,
        step: &str,
        selected: bool,
        objects: Vec<(String, Value)>,
    ) -> Result<(SetupDraft, usize), AppError> {
        t.require(Role::Owner)?;
        if !valid_step(step)
            || objects
                .iter()
                .any(|(key, value)| !valid_object_key(key) || !value.is_object())
        {
            return Err(AppError::field("object", "setup.object_invalid"));
        }
        let mut uow = self.p.uow.begin().await?;
        let now = self.p.clock.now();
        let mut draft = uow
            .setup_drafts()
            .get(t)
            .await?
            .unwrap_or_else(|| SetupDraft::new(now));
        draft.steps.insert(step.to_owned(), selected);
        let mut keep_photos = BTreeSet::new();
        let mut keep_objects = BTreeSet::new();
        if selected {
            for (key, value) in objects {
                keep_objects.insert(key.clone());
                if existing_id(&value).is_none() {
                    keep_photos.extend(object_photo_keys(&key).iter().copied());
                }
                draft.objects.insert(key, value);
            }
        }
        for key in step_object_keys(step) {
            if !keep_objects.contains(*key) {
                draft.objects.remove(*key);
            }
        }
        let mut removed = Vec::new();
        for key in step_photo_keys(step) {
            if !keep_photos.contains(key)
                && let Some(photo) = draft.photos.remove(*key)
                && let Some(photo) = BlobKey::parse(&photo)
            {
                removed.push(photo);
            }
        }
        draft.updated_at = now;
        uow.setup_drafts().save(t, &draft).await?;
        uow.commit().await?;
        for photo in &removed {
            if let Err(error) = self.p.blobs.delete(photo).await {
                tracing::warn!(%error, key = %photo, "unused setup photo could not be deleted");
            }
        }
        Ok((draft, removed.len()))
    }
}

fn step_object_keys(step: &str) -> &'static [&'static str] {
    match step {
        "pv" => &[
            "pv",
            "pv_generation",
            "pv_battery",
            "pv_battery_charge",
            "pv_battery_discharge",
        ],
        "vehicle" => &["vehicle", "wallbox"],
        "grid" => &["grid", "grid_export"],
        "heat_pump" => &["heat_pump"],
        "media" => &["media_general"],
        "contracts" => &["contracts"],
        _ => &[],
    }
}

impl DiscardSetup {
    #[tracing::instrument(name = "DiscardSetup", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let photos = uow.setup_drafts().delete(t).await?;
        uow.commit().await?;
        for photo in photos {
            if let Err(error) = self.p.blobs.delete(&photo).await {
                tracing::warn!(%error, key = %photo, "setup photo could not be deleted");
            }
        }
        Ok(())
    }
}

impl AnalyseSetupPhoto {
    #[tracing::instrument(name = "AnalyseSetupPhoto", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        object_key: &str,
        bytes: Bytes,
    ) -> Result<SetupPhotoResult, AppError> {
        t.require(Role::Owner)?;
        let known = STEP_PHOTO_KEYS
            .iter()
            .any(|(_, keys)| keys.contains(&object_key));
        if !known || bytes.is_empty() {
            return Err(AppError::field("photo", "setup.photo_invalid"));
        }
        let jpeg = self
            .p
            .images
            .normalize(bytes)
            .await
            .map_err(AppError::from)?;
        let key = BlobKey::setup_photo(t.household_id(), self.p.ids.new_uuid());
        self.p
            .blobs
            .put(&key, jpeg.bytes.clone())
            .await
            .map_err(AppError::from_storage)?;

        let recognition = if let Some(recognizer) = &self.p.recognizer {
            match recognizer.recognize(&jpeg).await {
                Ok(recognition) => Some(recognition),
                Err(error) => {
                    tracing::warn!(%error, object_key, "setup photo recognition failed");
                    None
                }
            }
        } else {
            None
        };

        let save_result = async {
            let mut uow = self.p.uow.begin().await?;
            let now = self.p.clock.now();
            let mut draft = uow
                .setup_drafts()
                .get(t)
                .await?
                .unwrap_or_else(|| SetupDraft::new(now));
            let old_key = draft.photos.insert(object_key.to_owned(), key.to_string());
            if let Some(recognition) = &recognition {
                let object = draft
                    .objects
                    .entry(object_key.to_owned())
                    .or_insert_with(|| Value::Object(Map::new()));
                if !object.is_object() {
                    *object = Value::Object(Map::new());
                }
                if let Some(fields) = object.as_object_mut() {
                    fields.insert(
                        "recognition".into(),
                        serde_json::to_value(recognition)
                            .map_err(|e| AppError::Internal(e.to_string()))?,
                    );
                }
            }
            draft.updated_at = now;
            uow.setup_drafts().save(t, &draft).await?;
            uow.commit().await?;
            Ok::<_, AppError>(old_key)
        }
        .await;

        match save_result {
            Ok(old_key) => {
                if let Some(old_key) = old_key.and_then(|s| BlobKey::parse(&s))
                    && let Err(error) = self.p.blobs.delete(&old_key).await
                {
                    tracing::warn!(%error, "replaced setup photo could not be deleted");
                }
            }
            Err(error) => {
                let _ = self.p.blobs.delete(&key).await;
                return Err(error);
            }
        }
        Ok(SetupPhotoResult {
            photo_key: key,
            recognition,
        })
    }
}

impl CompleteSetup {
    /// Validiert und legt den Entwurf in einer Transaktion an. Jeder Fehler
    /// beendet die Funktion vor `commit`, sodass keine Teilergebnisse bleiben.
    #[tracing::instrument(name = "CompleteSetup", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<BTreeMap<String, MeterId>, AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let draft = uow.setup_drafts().get(t).await?.ok_or(AppError::NotFound)?;
        let mut local = BTreeMap::<String, MeterId>::new();
        let now = self.p.clock.now();
        let mut copied_photos = Vec::new();
        let photos = async {
            self.create_selected_meters(
                t,
                &draft,
                &mut local,
                &mut copied_photos,
                now,
                uow.as_mut(),
            )
            .await?;
            self.create_pv(t, &draft, &local, now, uow.as_mut()).await?;
            self.create_vehicle_step(t, &draft, &mut local, now, uow.as_mut())
                .await?;
            self.create_contract_step(t, &draft, &local, uow.as_mut())
                .await?;
            uow.setup_drafts().delete(t).await.map_err(Into::into)
        }
        .await;
        let photos = match photos {
            Ok(photos) => photos,
            Err(error) => {
                self.delete_setup_copies(&copied_photos).await;
                return Err(error);
            }
        };
        if let Err(error) = uow.commit().await {
            self.delete_setup_copies(&copied_photos).await;
            return Err(error.into());
        }
        for key in photos {
            if let Err(error) = self.p.blobs.delete(&key).await {
                tracing::warn!(%error, key = %key, "completed setup photo could not be deleted");
            }
        }
        Ok(local)
    }

    async fn create_selected_meters(
        &self,
        t: &Tenant,
        draft: &SetupDraft,
        local: &mut BTreeMap<String, MeterId>,
        copied_photos: &mut Vec<BlobKey>,
        now: DateTime<Utc>,
        uow: &mut dyn UnitOfWork,
    ) -> Result<(), AppError> {
        let selected = |step: &str| draft.steps.get(step).copied().unwrap_or(false);
        let mut used_photos = BTreeSet::new();
        for key in [
            "grid",
            "heat_pump",
            "wallbox",
            "pv_generation",
            "pv_battery",
            "media_general",
        ] {
            let (enabled, required) = match key {
                "grid" | "heat_pump" => (selected(key), true),
                "pv_generation" => (selected("pv"), true),
                "pv_battery" => (selected("pv"), false),
                "media_general" => (selected("media"), true),
                _ => (selected("vehicle"), false),
            };
            if !enabled {
                continue;
            }
            let Some(object) = draft.objects.get(key) else {
                if required {
                    return Err(AppError::field(key, "setup.object_required"));
                }
                continue;
            };
            if let Some(id) = existing_id(object) {
                // Medienzähler dürfen jede Art haben, alle übrigen sind Strom
                let electricity_only = key != "media_general";
                let id = resolve_existing_meter(uow, t, id, key, electricity_only).await?;
                local.insert(key.to_owned(), id);
                continue;
            }
            let photos: Vec<Option<BlobKey>> = object_photo_keys(key)
                .iter()
                .map(|photo| draft.photos.get(*photo).and_then(|v| BlobKey::parse(v)))
                .collect();
            used_photos.extend(object_photo_keys(key).iter().copied());
            let (id, copies) = self
                .create_meter()
                .execute_in_setup(
                    t,
                    input_for_meter(key, object, now, local.get("grid").copied())?,
                    &photos,
                    uow,
                )
                .await?;
            local.insert(key.to_owned(), id);
            copied_photos.extend(copies);
        }
        if let Some(unused) = draft
            .photos
            .keys()
            .find(|key| !used_photos.contains(key.as_str()))
        {
            return Err(AppError::field(
                format!("photo.{unused}"),
                "setup.photo_unassigned",
            ));
        }
        Ok(())
    }

    async fn create_pv(
        &self,
        t: &Tenant,
        draft: &SetupDraft,
        local: &BTreeMap<String, MeterId>,
        now: DateTime<Utc>,
        uow: &mut dyn UnitOfWork,
    ) -> Result<(), AppError> {
        if !draft.steps.get("pv").copied().unwrap_or(false) {
            return Ok(());
        }
        let object = draft
            .objects
            .get("pv")
            .ok_or_else(|| AppError::field("pv", "setup.object_required"))?;
        let grid = referenced_meter(object, "grid", "grid", local)?;
        let generation = referenced_meter(object, "generation", "pv_generation", local)?;
        let mut meters = vec![(grid, PvRole::Grid), (generation, PvRole::Generation)];
        if let Some(battery) = local.get("pv_battery").copied() {
            meters.push((battery, PvRole::Battery));
        }
        self.create_pv_system()
            .execute_in(
                t,
                PvSystemInput {
                    data: pv_data(object, now)?,
                    meters,
                },
                uow,
            )
            .await?;
        Ok(())
    }

    async fn create_vehicle_step(
        &self,
        t: &Tenant,
        draft: &SetupDraft,
        local: &mut BTreeMap<String, MeterId>,
        now: DateTime<Utc>,
        uow: &mut dyn UnitOfWork,
    ) -> Result<(), AppError> {
        if !draft.steps.get("vehicle").copied().unwrap_or(false) {
            return Ok(());
        }
        let object = draft
            .objects
            .get("vehicle")
            .ok_or_else(|| AppError::field("vehicle", "setup.object_required"))?;
        let home_meter = match object_string(object, "home_meter").filter(|s| !s.is_empty()) {
            Some(reference) => {
                Some(resolve_reference(uow, t, local, reference, "vehicle.home_meter").await?)
            }
            None => None,
        };
        let id = self
            .create_vehicle()
            .execute_in(
                t,
                NewVehicle {
                    name: required_text(object, "name", "vehicle")?,
                    odometer: decimal_field(object, "odometer", "vehicle")?
                        .unwrap_or(Decimal::ZERO),
                    start_at: date_time_field(object, "start_at").unwrap_or(now),
                    vehicle: vehicle_input(object, home_meter)?,
                },
                uow,
            )
            .await?;
        local.insert("vehicle".into(), id);
        if let Some(holding) = holding_from(object)? {
            SetAcquisition::new((*self.p).clone())
                .execute_in(t, id, Some(holding), uow)
                .await?;
        }
        Ok(())
    }

    async fn create_contract_step(
        &self,
        t: &Tenant,
        draft: &SetupDraft,
        local: &BTreeMap<String, MeterId>,
        uow: &mut dyn UnitOfWork,
    ) -> Result<(), AppError> {
        if !draft.steps.get("contracts").copied().unwrap_or(false) {
            return Ok(());
        }
        let object = draft
            .objects
            .get("contracts")
            .ok_or_else(|| AppError::field("contracts", "setup.object_required"))?;
        let input: ContractInput = serde_json::from_value(
            object
                .get("contract")
                .cloned()
                .unwrap_or_else(|| object.clone()),
        )
        .map_err(|_| AppError::field("contracts", "setup.contract_invalid"))?;
        let mut targets = Vec::new();
        for field in ["meter", "second_meter"] {
            let reference = match (object_string(object, field), field) {
                (Some(r), _) if !r.is_empty() => r,
                (_, "meter") => "grid",
                _ => continue,
            };
            let id =
                resolve_reference(uow, t, local, reference, &format!("contracts.{field}")).await?;
            if !targets.contains(&id) {
                targets.push(id);
            }
        }
        for id in targets {
            self.create_contract()
                .execute_in(t, id, input.clone(), uow)
                .await?;
        }
        Ok(())
    }

    fn create_meter(&self) -> CreateMeter {
        CreateMeter::new((*self.p).clone())
    }

    fn create_pv_system(&self) -> CreatePvSystem {
        CreatePvSystem::new((*self.p).clone())
    }

    fn create_vehicle(&self) -> CreateVehicle {
        CreateVehicle::new((*self.p).clone())
    }

    fn create_contract(&self) -> CreateContract {
        CreateContract::new((*self.p).clone())
    }

    async fn delete_setup_copies(&self, photos: &[BlobKey]) {
        for photo in photos {
            if let Err(error) = self.p.blobs.delete(photo).await {
                tracing::warn!(%error, key = %photo, "setup reading photo copy could not be deleted");
            }
        }
    }
}

fn object_string<'a>(object: &'a Value, field: &str) -> Option<&'a str> {
    object.get(field).and_then(Value::as_str)
}

fn required_text(object: &Value, field: &str, prefix: &str) -> Result<String, AppError> {
    object_string(object, field)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AppError::field(format!("{prefix}.{field}"), "setup.value_required"))
}

fn decimal_field(object: &Value, field: &str, prefix: &str) -> Result<Option<Decimal>, AppError> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    if value.is_null() || value.as_str().is_some_and(str::is_empty) {
        return Ok(None);
    }
    let parsed = value
        .as_str()
        .and_then(|v| v.parse().ok())
        .or_else(|| value.as_number().and_then(|v| v.to_string().parse().ok()));
    parsed
        .map(Some)
        .ok_or_else(|| AppError::field(format!("{prefix}.{field}"), "setup.number_invalid"))
}

fn date_time_field(object: &Value, field: &str) -> Option<DateTime<Utc>> {
    object
        .get(field)
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

fn input_for_meter(
    key: &str,
    object: &Value,
    now: DateTime<Utc>,
    grid: Option<MeterId>,
) -> Result<MeterInput, AppError> {
    let prefix = key.to_owned();
    let (kind, purpose, unit) = match key {
        "grid" | "wallbox" | "pv_generation" | "pv_battery" => (
            MeterKind::Electricity,
            if key == "wallbox" {
                MeterPurpose::Wallbox
            } else {
                MeterPurpose::General
            },
            "kWh",
        ),
        "heat_pump" => (MeterKind::Electricity, MeterPurpose::HeatPump, "kWh"),
        _ => {
            let kind = object_string(object, "kind")
                .and_then(|s| s.parse::<MeterKind>().ok())
                .ok_or_else(|| AppError::field(format!("{key}.kind"), "setup.kind_invalid"))?;
            let purpose = object_string(object, "purpose")
                .and_then(|s| s.parse().ok())
                .unwrap_or_default();
            (kind, purpose, kind.default_unit())
        }
    };
    let name = required_text(object, "name", &prefix)?;
    let start_at = date_time_field(object, "start_at").unwrap_or(now);
    let mut registers = Vec::new();
    if key == "pv_battery" {
        for (field, code, direction) in [
            ("charge_start", "charge", Direction::Import),
            ("discharge_start", "discharge", Direction::Export),
        ] {
            registers.push(RegisterInput {
                id: None,
                code: code.to_owned(),
                direction,
                start_value: decimal_field(object, field, &prefix)?,
            });
        }
    } else {
        let code = object_string(object, "register").unwrap_or("main");
        registers.push(RegisterInput {
            id: None,
            code: code.to_owned(),
            direction: Direction::Import,
            start_value: decimal_field(object, "start_value", &prefix)?,
        });
        let export = key == "grid"
            && object
                .get("has_export")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        if export {
            registers.push(RegisterInput {
                id: None,
                code: "export".into(),
                direction: Direction::Export,
                start_value: decimal_field(object, "export_start", &prefix)?,
            });
        }
    }
    let own_heat_pump = key == "heat_pump" && object_string(object, "mode") == Some("own");
    let parent = match key {
        "heat_pump" if own_heat_pump => None,
        "heat_pump" | "wallbox" | "pv_battery" => grid,
        _ => None,
    };
    Ok(MeterInput {
        name,
        kind,
        unit: object_string(object, "unit").unwrap_or(unit).to_owned(),
        schedule: None,
        max_daily_rate: None,
        billing_start: None,
        purpose,
        parent_meter_id: parent,
        targets: crate::model::MeterTargets::default(),
        serial_number: object_string(object, "serial_number").map(str::to_owned),
        rollover_digits: None,
        decimal_places: object
            .get("decimal_places")
            .and_then(Value::as_u64)
            .and_then(|v| u8::try_from(v).ok())
            .unwrap_or(1),
        registers,
        start_at,
    })
}

fn referenced_meter(
    object: &Value,
    field: &str,
    default_key: &str,
    local: &BTreeMap<String, MeterId>,
) -> Result<MeterId, AppError> {
    let key = object_string(object, field).unwrap_or(default_key);
    local
        .get(key)
        .copied()
        .ok_or_else(|| AppError::field(field.to_owned(), "setup.reference_invalid"))
}

fn pv_data(object: &Value, now: DateTime<Utc>) -> Result<PvSystemData, AppError> {
    let kind = object_string(object, "kind")
        .unwrap_or("surplus")
        .parse::<PvKind>()
        .map_err(|_| AppError::field("pv.kind", "setup.kind_invalid"))?;
    let commissioned_on = object_string(object, "commissioned_on")
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .unwrap_or_else(|| now.date_naive());
    Ok(PvSystemData {
        name: required_text(object, "name", "pv")?,
        kind,
        kwp: decimal_field(object, "kwp", "pv")?.unwrap_or(Decimal::ZERO),
        commissioned_on,
        azimuth: object
            .get("azimuth")
            .and_then(Value::as_i64)
            .and_then(|v| i16::try_from(v).ok()),
        tilt: object
            .get("tilt")
            .and_then(Value::as_i64)
            .and_then(|v| i16::try_from(v).ok()),
        inverter: object_string(object, "inverter").map(str::to_owned),
        battery_kwh: decimal_field(object, "battery_kwh", "pv")?,
        specific_yield: decimal_field(object, "specific_yield", "pv")?
            .unwrap_or(Decimal::from(1000)),
        degradation_pct: decimal_field(object, "degradation_pct", "pv")?
            .unwrap_or(DEFAULT_DEGRADATION_PCT),
        month_shares: None,
        alarm_threshold_pct: object
            .get("alarm_threshold_pct")
            .and_then(Value::as_u64)
            .and_then(|v| u8::try_from(v).ok())
            .unwrap_or(DEFAULT_ALARM_THRESHOLD_PCT),
    })
}

fn vehicle_input(object: &Value, home_meter_id: Option<MeterId>) -> Result<VehicleInput, AppError> {
    let drive_type = object_string(object, "drive_type")
        .unwrap_or("electric")
        .parse::<DriveType>()
        .map_err(|_| AppError::field("vehicle.drive_type", "setup.kind_invalid"))?;
    Ok(VehicleInput {
        plate: object_string(object, "plate")
            .unwrap_or_default()
            .to_owned(),
        model: required_text(object, "model", "vehicle")?,
        drive_type,
        home_charging_meter_id: home_meter_id,
        pv_share_pct: decimal_field(object, "pv_share_pct", "vehicle")?,
        home_charging_price: decimal_field(object, "home_charging_price", "vehicle")?,
    })
}

/// Löst einen lokalen Schlüssel oder `id:<uuid>` einer vorhandenen Messstelle
/// des Haushalts auf.
async fn resolve_reference(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    local: &BTreeMap<String, MeterId>,
    reference: &str,
    field: &str,
) -> Result<MeterId, AppError> {
    if let Some(id) = reference.strip_prefix("id:") {
        return resolve_existing_meter(uow, t, id, field, false).await;
    }
    local
        .get(reference)
        .copied()
        .ok_or_else(|| AppError::field(field.to_owned(), "setup.reference_invalid"))
}

/// Eine vorhandene Messstelle des Mandanten (optional nur Strom); fremde
/// oder falsche IDs sind ungültig.
async fn resolve_existing_meter(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    id: &str,
    field: &str,
    electricity_only: bool,
) -> Result<MeterId, AppError> {
    let invalid = || AppError::field(field.to_owned(), "setup.reference_invalid");
    let id: MeterId = id.parse().map_err(|_| invalid())?;
    let ok = uow
        .meters()
        .get(t, id)
        .await?
        .is_some_and(|m| !electricity_only || m.meter.data.kind == MeterKind::Electricity);
    if ok { Ok(id) } else { Err(invalid()) }
}

/// Anschaffung aus den optionalen Feldern `acquisition_*`; ohne Datum und
/// Preis entfällt sie.
fn holding_from(object: &Value) -> Result<Option<Holding>, AppError> {
    let date = |field: &str| -> Result<Option<NaiveDate>, AppError> {
        match object_string(object, field)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            None => Ok(None),
            Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map(Some)
                .map_err(|_| AppError::field(format!("vehicle.{field}"), "form.date_required")),
        }
    };
    let purchased_on = date("acquisition_date")?;
    let price = decimal_field(object, "acquisition_price", "vehicle")?;
    let (purchased_on, price) = match (purchased_on, price) {
        (None, None) => return Ok(None),
        (Some(d), Some(p)) => (d, p),
        (None, _) => {
            return Err(AppError::field(
                "vehicle.acquisition_date",
                "form.date_required",
            ));
        }
        (_, None) => {
            return Err(AppError::field(
                "vehicle.acquisition_price",
                "form.number_required",
            ));
        }
    };
    let by_mileage = object
        .get("acquisition_by_mileage")
        .is_some_and(|v| v.as_bool() == Some(true) || v.as_str() == Some("true"));
    let mode = match object_string(object, "acquisition_mode").filter(|s| !s.is_empty()) {
        _ if by_mileage => DepreciationMode::Mileage,
        None | Some("time") => DepreciationMode::Time,
        Some("mileage") => DepreciationMode::Mileage,
        Some(_) => {
            return Err(AppError::field(
                "vehicle.acquisition_mode",
                "setup.kind_invalid",
            ));
        }
    };
    Ok(Some(Holding {
        purchased_on,
        price,
        mode,
        first_registration: date("acquisition_first_registration")?,
        residual_value: decimal_field(object, "acquisition_residual_value", "vehicle")?,
        planned_end: date("acquisition_planned_end")?,
        odometer_at_purchase: decimal_field(object, "acquisition_odometer", "vehicle")?,
        lifetime_km: decimal_field(object, "acquisition_lifetime_km", "vehicle")?,
        end_value: decimal_field(object, "acquisition_end_value", "vehicle")?,
        sold_on: None,
        sale_price: None,
    }))
}
