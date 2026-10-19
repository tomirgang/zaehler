//! Mehrstufiger Setup-Assistent (SPEC_SETUP_FLOW).

use std::collections::BTreeMap;

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, State},
    response::Response,
};
use serde_json::{Map, Value, json};
use zaehler_app::{AppError, FieldError, model::SetupDraft};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::Lang,
    pages::{render, see_other},
    security::{constant_time_eq, csrf_failed},
    state::WebState,
};

const STEPS: [&str; 7] = [
    "grid",
    "heat_pump",
    "pv",
    "vehicle",
    "media",
    "contracts",
    "summary",
];

#[derive(Debug, Clone)]
struct Field {
    name: String,
    value: String,
    input_type: &'static str,
    options: Vec<SelectOption>,
    /// Sichtbarkeitsregel für `app.js`, z.B. `enabled;existing_grid=`
    show_if: String,
    required: bool,
}

#[derive(Debug, Clone)]
struct SelectOption {
    value: String,
    label: String,
}

#[derive(Debug, Clone)]
struct PhotoSlot {
    key: &'static str,
    label: String,
    saved: bool,
    confidence: Option<String>,
    show_if: String,
}

/// Vorhandene Messstelle des Haushalts als Auswahlziel.
#[derive(Debug, Clone)]
struct ExistingMeter {
    id: String,
    name: String,
    electricity: bool,
}

#[derive(Debug)]
struct SetupError {
    step: String,
    message: String,
}

#[derive(Template)]
#[template(path = "pages/setup/step.html")]
struct SetupPage {
    ctx: PageCtx,
    step: String,
    title: String,
    progress: String,
    index: usize,
    selected: bool,
    fields: Vec<Field>,
    has_required: bool,
    photos: Vec<PhotoSlot>,
    notice: Option<String>,
    enabled_steps: Vec<String>,
    errors: Vec<SetupError>,
}

fn step_index(step: &str) -> Option<usize> {
    STEPS.iter().position(|s| *s == step)
}

fn draft_object<'a>(draft: &'a SetupDraft, key: &str) -> Option<&'a Map<String, Value>> {
    draft.objects.get(key).and_then(Value::as_object)
}

fn string_value(draft: &SetupDraft, key: &str, field: &str) -> String {
    draft_object(draft, key)
        .and_then(|fields| {
            fields
                .get(field)
                .filter(|value| {
                    !value.is_null() && !value.as_str().is_some_and(|value| value.trim().is_empty())
                })
                .or_else(|| {
                    let recognized = match field {
                        "serial_number" => "serial_number",
                        "start_value" => "value",
                        "register" => "register",
                        "unit" => "unit",
                        "kind" => "meter_kind",
                        _ => return None,
                    };
                    fields
                        .get("recognition")
                        .and_then(Value::as_object)
                        .and_then(|recognition| recognition.get(recognized))
                        .filter(|value| !value.is_null())
                })
        })
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .or_else(|| (!value.is_null()).then(|| value.to_string()))
        })
        .unwrap_or_default()
}

/// Feld des Assistenten: Name, Eingabetyp, Objektschlüssel im Entwurf,
/// Sichtbarkeitsregel (zusätzlich zu `enabled`) und Pflichtfeld.
///
/// Regeln: durch `;` getrennte Bedingungen, die alle gelten müssen; `feld`
/// (angehakt bzw. nicht leer), `feld=a|b` oder `feld!=a|b`. Ohne JavaScript
/// sind alle Felder sichtbar (SPEC_SETUP_FLOW).
type FieldSpec = (&'static str, &'static str, &'static str, &'static str, bool);

const NEW_GRID: &str = "existing_grid=";
const NEW_HEAT_PUMP: &str = "existing_heat_pump=";
const NEW_GENERATION: &str = "existing_generation=";
const NEW_BATTERY: &str = "battery_meter=new";
const NEW_MEDIA: &str = "existing_media=";
const NEW_WALLBOX: &str = "drive_type!=combustion;home_meter=wallbox";
const ACQ_TIME: &str = "has_acquisition;acquisition_mode=time";
const ACQ_MILEAGE: &str = "has_acquisition;acquisition_mode=mileage";

#[allow(clippy::too_many_lines)] // Tabelle, eine Zeile je Feld
fn field_specs(step: &str) -> &'static [FieldSpec] {
    match step {
        "grid" => &[
            ("existing_grid", "select", "grid", "", false),
            ("name", "text", "grid", NEW_GRID, true),
            ("serial_number", "text", "grid", NEW_GRID, false),
            ("register", "text", "grid", NEW_GRID, false),
            ("start_value", "number", "grid", NEW_GRID, false),
            ("has_export", "checkbox", "grid", NEW_GRID, false),
            (
                "export_start",
                "number",
                "grid",
                "existing_grid=;has_export",
                false,
            ),
        ],
        "heat_pump" => &[
            ("existing_heat_pump", "select", "heat_pump", "", false),
            ("mode", "select", "heat_pump", NEW_HEAT_PUMP, true),
            ("name", "text", "heat_pump", NEW_HEAT_PUMP, true),
            ("serial_number", "text", "heat_pump", NEW_HEAT_PUMP, false),
            ("register", "text", "heat_pump", NEW_HEAT_PUMP, false),
            ("start_value", "number", "heat_pump", NEW_HEAT_PUMP, false),
        ],
        "pv" => &[
            ("system_name", "text", "pv", "", true),
            ("kind", "select", "pv", "", true),
            ("kwp", "number", "pv", "", false),
            ("commissioned_on", "date", "pv", "", false),
            ("existing_generation", "select", "pv_generation", "", false),
            (
                "generation_name",
                "text",
                "pv_generation",
                NEW_GENERATION,
                true,
            ),
            (
                "generation_serial_number",
                "text",
                "pv_generation",
                NEW_GENERATION,
                false,
            ),
            (
                "generation_register",
                "text",
                "pv_generation",
                NEW_GENERATION,
                false,
            ),
            (
                "generation_start",
                "number",
                "pv_generation",
                NEW_GENERATION,
                false,
            ),
            ("battery_meter", "select", "pv_battery", "", false),
            ("battery_name", "text", "pv_battery", NEW_BATTERY, true),
            ("battery_charge", "number", "pv_battery", NEW_BATTERY, false),
            (
                "battery_discharge",
                "number",
                "pv_battery",
                NEW_BATTERY,
                false,
            ),
        ],
        "vehicle" => &[
            ("name", "text", "vehicle", "", true),
            ("model", "text", "vehicle", "", true),
            ("plate", "text", "vehicle", "", false),
            ("drive_type", "select", "vehicle", "", true),
            ("odometer", "number", "vehicle", "", false),
            (
                "home_meter",
                "select",
                "vehicle",
                "drive_type!=combustion",
                false,
            ),
            ("wallbox_name", "text", "wallbox", NEW_WALLBOX, true),
            ("wallbox_start", "number", "wallbox", NEW_WALLBOX, false),
            ("has_acquisition", "checkbox", "vehicle", "", false),
            (
                "acquisition_date",
                "date",
                "vehicle",
                "has_acquisition",
                true,
            ),
            (
                "acquisition_price",
                "number",
                "vehicle",
                "has_acquisition",
                true,
            ),
            (
                "acquisition_mode",
                "select",
                "vehicle",
                "has_acquisition",
                true,
            ),
            (
                "acquisition_residual_value",
                "number",
                "vehicle",
                ACQ_TIME,
                true,
            ),
            ("acquisition_planned_end", "date", "vehicle", ACQ_TIME, true),
            (
                "acquisition_odometer",
                "number",
                "vehicle",
                ACQ_MILEAGE,
                true,
            ),
            (
                "acquisition_lifetime_km",
                "number",
                "vehicle",
                ACQ_MILEAGE,
                true,
            ),
            (
                "acquisition_end_value",
                "number",
                "vehicle",
                ACQ_MILEAGE,
                false,
            ),
        ],
        "media" => &[
            ("existing_media", "select", "media_general", "", false),
            ("name", "text", "media_general", NEW_MEDIA, true),
            ("kind", "select", "media_general", NEW_MEDIA, true),
            ("unit", "text", "media_general", NEW_MEDIA, false),
            ("register", "text", "media_general", NEW_MEDIA, false),
            ("start_value", "number", "media_general", NEW_MEDIA, false),
        ],
        "contracts" => &[
            ("provider", "text", "contracts", "", true),
            ("meter", "select", "contracts", "", true),
            ("second_meter", "select", "contracts", "", false),
            ("direction", "select", "contracts", "", true),
            ("starts_on", "date", "contracts", "", true),
            ("position_name", "text", "contracts", "", false),
            ("base_price", "number", "contracts", "", false),
            ("unit_price", "number", "contracts", "", false),
        ],
        _ => &[],
    }
}

/// Regel inklusive Schalter `enabled` des Schritts.
fn show_rule(rule: &str) -> String {
    if rule.is_empty() {
        "enabled".to_owned()
    } else {
        format!("enabled;{rule}")
    }
}

/// Wert eines Auswahlfelds, das zwischen „neu“ und vorhandener Messstelle
/// unterscheidet, aus dem Entwurf.
fn derived_value(name: &str, draft: &SetupDraft) -> Option<String> {
    let has = |key: &str, field: &str| !string_value(draft, key, field).is_empty();
    match name {
        "battery_meter" => draft_object(draft, "pv_battery").map(|_| {
            let id = string_value(draft, "pv_battery", "existing_id");
            if id.is_empty() { "new".to_owned() } else { id }
        }),
        "has_acquisition" => Some(
            (has("vehicle", "acquisition_date") || has("vehicle", "acquisition_price")).to_string(),
        ),
        _ => None,
    }
}

fn fields(step: &str, draft: &SetupDraft, lang: Lang, existing: &[ExistingMeter]) -> Vec<Field> {
    field_specs(step)
        .iter()
        .map(|(name, input_type, key, rule, required)| {
            let object_field = match *name {
                "system_name" | "wallbox_name" | "generation_name" | "battery_name" => "name",
                "generation_serial_number" => "serial_number",
                "generation_register" => "register",
                "generation_start" | "wallbox_start" => "start_value",
                "battery_charge" => "charge_start",
                "battery_discharge" => "discharge_start",
                "existing_grid"
                | "existing_generation"
                | "existing_heat_pump"
                | "existing_media" => "existing_id",
                _ => name,
            };
            let mut value = derived_value(name, draft)
                .unwrap_or_else(|| string_value(draft, key, object_field));
            if value.is_empty()
                && let Some(photo_object) = recognition_object(name)
            {
                value = string_value(draft, photo_object, "start_value");
            }
            Field {
                name: (*name).to_owned(),
                value,
                input_type,
                options: options_for(step, name, draft, lang, existing),
                show_if: show_rule(rule),
                required: *required,
            }
        })
        .collect()
}

fn recognition_object(name: &str) -> Option<&'static str> {
    match name {
        "export_start" => Some("grid_export"),
        "battery_charge" => Some("pv_battery_charge"),
        "battery_discharge" => Some("pv_battery_discharge"),
        _ => None,
    }
}

fn option(value: &str, label: String) -> SelectOption {
    SelectOption {
        value: value.to_owned(),
        label,
    }
}

#[allow(clippy::too_many_lines)] // ein Zweig je Auswahlfeld
fn options_for(
    step: &str,
    name: &str,
    draft: &SetupDraft,
    lang: Lang,
    existing: &[ExistingMeter],
) -> Vec<SelectOption> {
    let translated = |prefix: &str, values: &[&str]| {
        values
            .iter()
            .map(|v| option(v, lang.t(&format!("{prefix}{v}"))))
            .collect::<Vec<_>>()
    };
    let existing_options = |electricity_only: bool| {
        existing
            .iter()
            .filter(|m| m.electricity || !electricity_only)
            .map(|m| (m.id.clone(), m.name.clone()))
            .collect::<Vec<_>>()
    };
    match name {
        "kind" if step == "pv" => translated("pv-kind-", &["surplus", "full_feed_in", "plug_in"]),
        "kind" => translated("kind-", &["electricity", "gas", "water", "heat", "custom"]),
        "drive_type" => translated("drive-", &["electric", "plugin_hybrid", "combustion"]),
        "direction" => translated("contract-direction-", &["import", "export"]),
        "mode" => translated("setup-hp-mode-", &["submeter", "own"]),
        "acquisition_mode" => translated("setup-acq-mode-", &["time", "mileage"]),
        "existing_grid" | "existing_generation" | "existing_heat_pump" | "existing_media" => {
            let mut out = vec![option("", lang.t("setup-option-new-meter"))];
            out.extend(
                existing_options(name != "existing_media")
                    .into_iter()
                    .map(|(id, label)| SelectOption { value: id, label }),
            );
            out
        }
        "battery_meter" => {
            let mut out = vec![
                option("", lang.t("setup-option-none")),
                option("new", lang.t("setup-option-new-battery")),
            ];
            out.extend(
                existing_options(true)
                    .into_iter()
                    .map(|(id, label)| SelectOption { value: id, label }),
            );
            out
        }
        "home_meter" => {
            let mut out = vec![
                option("", lang.t("setup-option-none")),
                option("wallbox", lang.t("setup-option-new-wallbox")),
            ];
            out.extend(
                existing_options(true)
                    .into_iter()
                    .map(|(id, label)| SelectOption {
                        value: format!("id:{id}"),
                        label,
                    }),
            );
            out
        }
        "meter" | "second_meter" => {
            let mut out = Vec::new();
            if name == "second_meter" {
                out.push(option("", lang.t("setup-option-none")));
            }
            // Vorhandene Messstellen der Vorschritte erscheinen einmal, unter
            // ihrem lokalen Schlüssel
            let reused = |key: &str| {
                draft_object(draft, key)
                    .and_then(|o| o.get("existing_id"))
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            };
            let grid_existing = reused("grid");
            let grid_label = grid_existing
                .as_deref()
                .and_then(|id| existing.iter().find(|m| m.id == id))
                .map_or_else(
                    || lang.t("setup-ref-grid"),
                    |m| lang.ta("setup-ref-existing", &[("name", m.name.clone())]),
                );
            out.push(option("grid", grid_label));
            let own_heat_pump = draft.steps.get("heat_pump").copied().unwrap_or(false)
                && reused("heat_pump").is_none()
                && draft_object(draft, "heat_pump")
                    .and_then(|o| o.get("mode"))
                    .and_then(Value::as_str)
                    == Some("own");
            if own_heat_pump {
                out.push(option("heat_pump", lang.t("setup-ref-heat_pump")));
            }
            if draft.steps.get("media").copied().unwrap_or(false)
                && reused("media_general").is_none()
            {
                out.push(option("media_general", lang.t("setup-ref-media_general")));
            }
            out.extend(
                existing_options(false)
                    .into_iter()
                    .filter(|(id, _)| grid_existing.as_deref() != Some(id.as_str()))
                    .map(|(id, label)| SelectOption {
                        value: format!("id:{id}"),
                        label,
                    }),
            );
            out
        }
        _ => Vec::new(),
    }
}

fn photo_slots(step: &str, draft: &SetupDraft, lang: Lang) -> Vec<PhotoSlot> {
    let keys: &[(&'static str, &str)] = match step {
        "grid" => &[
            ("grid", NEW_GRID),
            ("grid_export", "existing_grid=;has_export"),
        ],
        "heat_pump" => &[("heat_pump", NEW_HEAT_PUMP)],
        "pv" => &[
            ("pv_generation", NEW_GENERATION),
            ("pv_battery_charge", NEW_BATTERY),
            ("pv_battery_discharge", NEW_BATTERY),
        ],
        "vehicle" => &[("wallbox", NEW_WALLBOX)],
        "media" => &[("media_general", NEW_MEDIA)],
        _ => &[],
    };
    keys.iter()
        .map(|(key, rule)| PhotoSlot {
            key,
            label: lang.t(&format!("setup-photo-{key}")),
            saved: draft.photos.contains_key(*key),
            show_if: show_rule(rule),
            confidence: draft_object(draft, key)
                .and_then(|object| object.get("recognition"))
                .and_then(Value::as_object)
                .and_then(|recognition| recognition.get("confidence"))
                .filter(|value| !value.is_null())
                .map(|value| {
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned)
                }),
        })
        .collect()
}

fn page(
    ctx: &Ctx,
    step: &str,
    draft: &SetupDraft,
    errors: Vec<SetupError>,
    existing: &[ExistingMeter],
    removed: usize,
) -> Result<Response, Response> {
    let index = step_index(step).ok_or_else(|| ctx.fail(AppError::NotFound))?;
    let lang = ctx.page.lang;
    let fields = fields(step, draft, lang, existing);
    Ok(render(&SetupPage {
        ctx: ctx.page.clone(),
        step: step.to_owned(),
        title: lang.t(&format!("setup-step-{step}")),
        progress: lang.ta(
            "setup-progress",
            &[
                ("step", (index + 1).to_string()),
                ("total", STEPS.len().to_string()),
            ],
        ),
        index: index + 1,
        selected: draft.steps.get(step).copied().unwrap_or(step == "grid"),
        has_required: fields.iter().any(|f| f.required),
        fields,
        photos: photo_slots(step, draft, lang),
        notice: (removed > 0)
            .then(|| lang.ta("setup-photos-removed", &[("count", removed.to_string())])),
        enabled_steps: draft
            .steps
            .iter()
            .filter_map(|(key, enabled)| enabled.then_some(key.clone()))
            .collect(),
        errors,
    }))
}

/// Einstieg: neuer Entwurf oder vorhandenen Entwurf fortsetzen.
pub async fn start(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    state
        .uc
        .start_setup
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?;
    Ok(see_other("/setup/grid"))
}

pub async fn step(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(step): Path<String>,
    axum::extract::Query(query): axum::extract::Query<StepQuery>,
) -> Result<Response, Response> {
    if step_index(&step).is_none() {
        return Err(ctx.fail(AppError::NotFound));
    }
    let draft = match state.uc.get_setup_draft.execute(&ctx.tenant).await {
        Ok(Some(draft)) => draft,
        Ok(None) => state
            .uc
            .start_setup
            .execute(&ctx.tenant)
            .await
            .map_err(|e| ctx.fail(e))?,
        Err(error) => return Err(ctx.fail(error)),
    };
    let existing = existing_meters(&state, &ctx).await?;
    page(
        &ctx,
        &step,
        &draft,
        Vec::new(),
        &existing,
        query.removed.unwrap_or(0),
    )
}

#[derive(serde::Deserialize)]
pub struct StepQuery {
    removed: Option<usize>,
}

async fn existing_meters(state: &WebState, ctx: &Ctx) -> Result<Vec<ExistingMeter>, Response> {
    let meters = state
        .uc
        .list_meters
        .execute(&ctx.tenant)
        .await
        .map_err(|error| ctx.fail(error))?;
    Ok(meters
        .into_iter()
        .filter(|summary| {
            !summary.meter.archived
                && summary.meter.data.kind != zaehler_domain::meter::MeterKind::Vehicle
        })
        .map(|summary| ExistingMeter {
            id: summary.meter.id.to_string(),
            name: summary.meter.data.name,
            electricity: summary.meter.data.kind == zaehler_domain::meter::MeterKind::Electricity,
        })
        .collect())
}

fn value(fields: &BTreeMap<String, String>, name: &str) -> Value {
    if name == "has_export" {
        Value::Bool(fields.contains_key(name))
    } else {
        fields
            .get(name)
            .map_or(Value::Null, |text| Value::String(text.trim().to_owned()))
    }
}

fn object(fields: &BTreeMap<String, String>, names: &[&str]) -> Value {
    let mut object = Map::new();
    for name in names {
        object.insert((*name).to_owned(), value(fields, name));
    }
    Value::Object(object)
}

fn existing_object(form: &BTreeMap<String, String>, field: &str) -> Option<Value> {
    form.get(field)
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(|id| json!({ "existing_id": id }))
}

fn json_for_step(step: &str, form: &BTreeMap<String, String>) -> Vec<(String, Value)> {
    match step {
        "grid" => match existing_object(form, "existing_grid") {
            Some(existing) => vec![("grid".into(), existing)],
            None => vec![(
                "grid".into(),
                object(
                    form,
                    &[
                        "name",
                        "serial_number",
                        "register",
                        "start_value",
                        "has_export",
                        "export_start",
                    ],
                ),
            )],
        },
        "heat_pump" => vec![(
            "heat_pump".into(),
            existing_object(form, "existing_heat_pump").unwrap_or_else(|| {
                object(
                    form,
                    &["mode", "name", "start_value", "serial_number", "register"],
                )
            }),
        )],
        "pv" => pv_objects(form),
        "vehicle" => vehicle_objects(form),
        "media" => vec![(
            "media_general".into(),
            existing_object(form, "existing_media").unwrap_or_else(|| {
                object(form, &["name", "kind", "unit", "register", "start_value"])
            }),
        )],
        "contracts" => contract_objects(form),
        _ => Vec::new(),
    }
}

fn pv_objects(form: &BTreeMap<String, String>) -> Vec<(String, Value)> {
    let mut system = object(
        form,
        &[
            "system_name",
            "kind",
            "kwp",
            "commissioned_on",
            "battery_kwh",
        ],
    );
    if let Some(fields) = system.as_object_mut() {
        let name = fields.remove("system_name").unwrap_or(Value::Null);
        fields.insert("name".into(), name);
        fields.insert("grid".into(), Value::String("grid".into()));
        fields.insert("generation".into(), Value::String("pv_generation".into()));
    }
    let mut generation = object(
        form,
        &[
            "generation_name",
            "generation_serial_number",
            "generation_register",
            "generation_start",
        ],
    );
    if let Some(fields) = generation.as_object_mut() {
        for (from, to) in [
            ("generation_name", "name"),
            ("generation_serial_number", "serial_number"),
            ("generation_register", "register"),
            ("generation_start", "start_value"),
        ] {
            if let Some(value) = fields.remove(from) {
                fields.insert(to.into(), value);
            }
        }
    }
    if let Some(existing) = existing_object(form, "existing_generation") {
        generation = existing;
    }
    let mut out = vec![("pv".into(), system), ("pv_generation".into(), generation)];
    let battery_choice = form.get("battery_meter").map_or("", |v| v.trim());
    // Ohne JavaScript genügt ein Name für einen neuen Speicherzähler
    let new_battery = battery_choice == "new"
        || (battery_choice.is_empty()
            && form
                .get("battery_name")
                .is_some_and(|name| !name.trim().is_empty()));
    if let Some(existing) = existing_object(form, "battery_meter").filter(|_| !new_battery) {
        out.push(("pv_battery".into(), existing));
    } else if new_battery {
        let mut battery = object(
            form,
            &["battery_name", "battery_charge", "battery_discharge"],
        );
        if let Some(fields) = battery.as_object_mut() {
            let name = fields.remove("battery_name").unwrap_or(Value::Null);
            let charge = fields.remove("battery_charge").unwrap_or(Value::Null);
            let discharge = fields.remove("battery_discharge").unwrap_or(Value::Null);
            fields.insert("name".into(), name);
            fields.insert("charge_start".into(), charge);
            fields.insert("discharge_start".into(), discharge);
        }
        out.push(("pv_battery".into(), battery));
    }
    out
}

fn vehicle_objects(form: &BTreeMap<String, String>) -> Vec<(String, Value)> {
    const ACQUISITION: [&str; 8] = [
        "acquisition_date",
        "acquisition_price",
        "acquisition_mode",
        "acquisition_residual_value",
        "acquisition_planned_end",
        "acquisition_odometer",
        "acquisition_lifetime_km",
        "acquisition_end_value",
    ];
    let mut names = vec![
        "name",
        "model",
        "plate",
        "drive_type",
        "odometer",
        "home_meter",
    ];
    // Ohne Haken bleibt die Anschaffung leer; ohne JavaScript genügen Datum
    // oder Preis
    let filled = |name: &str| form.get(name).is_some_and(|v| !v.trim().is_empty());
    if form.contains_key("has_acquisition")
        || filled("acquisition_date")
        || filled("acquisition_price")
    {
        names.extend(ACQUISITION);
    }
    let mut vehicle = object(form, &names);
    let combustion = form.get("drive_type").is_some_and(|d| d == "combustion");
    let home = form.get("home_meter").map_or("", |v| v.trim());
    let has_wallbox = !combustion
        && (home == "wallbox"
            || (home.is_empty()
                && form
                    .get("wallbox_name")
                    .is_some_and(|name| !name.trim().is_empty())));
    if let Some(fields) = vehicle.as_object_mut() {
        if has_wallbox {
            fields.insert("home_meter".into(), Value::String("wallbox".into()));
        } else if combustion {
            fields.insert("home_meter".into(), Value::Null);
        }
    }
    let mut out = vec![("vehicle".into(), vehicle)];
    if has_wallbox {
        out.push((
            "wallbox".into(),
            json!({
                "name": form.get("wallbox_name"),
                "start_value": form.get("wallbox_start"),
            }),
        ));
    }
    out
}

fn contract_objects(form: &BTreeMap<String, String>) -> Vec<(String, Value)> {
    let contract = json!({
        "direction": form.get("direction").map_or("import", String::as_str),
        "provider": form.get("provider"),
        "starts_on": form.get("starts_on"),
        "positions": [{
            "name": form.get("position_name").map_or("Energie", String::as_str),
            "factor": "1",
            "components": [{
                "valid_from": form.get("starts_on"),
                "base_price": form.get("base_price").map_or("0", String::as_str),
                "base_interval": "month",
                "unit_pricing": {
                    "kind": "flat",
                    "price": form.get("unit_price").map_or("0", String::as_str),
                }
            }]
        }]
    });
    vec![(
        "contracts".into(),
        json!({
            "meter": form.get("meter").map_or("grid", String::as_str),
            "second_meter": form.get("second_meter"),
            "contract": contract,
        }),
    )]
}

pub async fn save_step(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(step): Path<String>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response, Response> {
    let Some(index) = step_index(&step) else {
        return Err(ctx.fail(AppError::NotFound));
    };
    let action = form.get("action").map_or("next", String::as_str);
    let selected = action != "skip" && (step == "grid" || form.contains_key("enabled"));
    let objects = json_for_step(&step, &form);
    let (_, removed) = state
        .uc
        .save_setup_draft
        .execute_objects(&ctx.tenant, &step, selected, objects)
        .await
        .map_err(|error| ctx.fail(error))?;
    if action == "back" {
        return Ok(see_other(&format!(
            "/setup/{}",
            STEPS[index.saturating_sub(1)]
        )));
    }
    let next = (index + 1).min(STEPS.len() - 1);
    let suffix = if removed > 0 {
        format!("?removed={removed}")
    } else {
        String::new()
    };
    Ok(see_other(&format!("/setup/{}{suffix}", STEPS[next])))
}

pub async fn photo(
    State(state): State<WebState>,
    ctx: Ctx,
    mut multipart: Multipart,
) -> Result<Response, Response> {
    let mut csrf = None;
    let mut object_key = None;
    let mut return_step = None;
    let mut image = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ctx.fail(AppError::field("photo", "setup.photo_invalid")))?
    {
        let name = field.name().unwrap_or_default().to_owned();
        if name == "photo" {
            let bytes = field
                .bytes()
                .await
                .map_err(|_| ctx.fail(AppError::field("photo", "setup.photo_invalid")))?;
            if bytes.len() > state.cfg.max_photo_bytes {
                return Err(ctx.fail(AppError::field("photo", "photo.too_large")));
            }
            image = Some(bytes);
        } else {
            let text = field
                .text()
                .await
                .map_err(|_| ctx.fail(AppError::field("photo", "setup.photo_invalid")))?;
            match name.as_str() {
                "_csrf" => csrf = Some(text),
                "object_key" => object_key = Some(text),
                "return_step" => return_step = Some(text),
                _ => {}
            }
        }
    }
    if !csrf
        .as_deref()
        .is_some_and(|token| constant_time_eq(token, &ctx.page.csrf))
    {
        return Err(csrf_failed(ctx.page.lang));
    }
    let object_key =
        object_key.ok_or_else(|| ctx.fail(AppError::field("photo", "setup.photo_invalid")))?;
    state
        .uc
        .analyse_setup_photo
        .execute(
            &ctx.tenant,
            &object_key,
            image.ok_or_else(|| ctx.fail(AppError::field("photo", "setup.photo_invalid")))?,
        )
        .await
        .map_err(|error| ctx.fail(error))?;
    let return_step = return_step
        .filter(|step| step_index(step).is_some())
        .unwrap_or_else(|| "grid".into());
    Ok(see_other(&format!("/setup/{return_step}")))
}

pub async fn complete(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    match state.uc.complete_setup.execute(&ctx.tenant).await {
        Ok(_) => {
            ctx.flash("setup-done").await;
            Ok(see_other("/"))
        }
        Err(AppError::Validation(errors)) => {
            let draft = state
                .uc
                .get_setup_draft
                .execute(&ctx.tenant)
                .await
                .map_err(|error| ctx.fail(error))?
                .ok_or_else(|| ctx.fail(AppError::NotFound))?;
            let errors = errors
                .0
                .iter()
                .map(|error| setup_error(&ctx, error))
                .collect();
            page(&ctx, "summary", &draft, errors, &[], 0)
        }
        Err(error) => Err(ctx.fail(error)),
    }
}

fn setup_error(ctx: &Ctx, error: &FieldError) -> SetupError {
    let mut parts = error.field.split('.');
    let first = parts.next().unwrap_or_default();
    let step = if first == "photo" {
        match parts.next().unwrap_or_default() {
            "grid" | "grid_export" => "grid",
            "heat_pump" => "heat_pump",
            "pv_generation" | "pv_battery_charge" | "pv_battery_discharge" => "pv",
            "wallbox" => "vehicle",
            "media_general" => "media",
            _ => "summary",
        }
    } else if step_index(first).is_some() {
        first
    } else {
        "summary"
    };
    SetupError {
        step: ctx.page.lang.t(&format!("setup-step-{step}")),
        message: ctx.page.lang.t(error.code),
    }
}

pub async fn discard(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    state
        .uc
        .discard_setup
        .execute(&ctx.tenant)
        .await
        .map_err(|error| ctx.fail(error))?;
    Ok(see_other("/"))
}

#[cfg(test)]
mod tests {
    use super::string_value;
    use serde_json::json;
    use zaehler_app::model::SetupDraft;

    #[test]
    fn recognition_prefills_blank_meter_fields_without_overwriting_values() {
        let mut draft = SetupDraft::new(chrono::Utc::now());
        draft.objects.insert(
            "grid".into(),
            json!({
                "serial_number": "",
                "start_value": "",
                "recognition": {
                    "serial_number": "SN-1",
                    "value": "100.5",
                    "register": "1.8.0",
                    "unit": "kWh",
                    "meter_kind": "electricity"
                }
            }),
        );
        assert_eq!(string_value(&draft, "grid", "serial_number"), "SN-1");
        assert_eq!(string_value(&draft, "grid", "start_value"), "100.5");
        assert_eq!(string_value(&draft, "grid", "register"), "1.8.0");
        assert_eq!(string_value(&draft, "grid", "unit"), "kWh");
        assert_eq!(string_value(&draft, "grid", "kind"), "electricity");

        draft.objects.get_mut("grid").unwrap()["start_value"] = json!("101");
        assert_eq!(string_value(&draft, "grid", "start_value"), "101");
    }
}
