//! Rechnungsimport: Dokument hochladen, Vorschlag prüfen und ausgewählte
//! Teile übernehmen (SPEC_DOC_UPLOADS, SPEC_AI_INVOICE).

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{Duration, NaiveDate, NaiveTime};
use rust_decimal::Decimal;
use serde::Deserialize;
use zaehler_app::{
    AppError, FieldErrors,
    ids::{ContractId, DocumentId, MeterId, PositionId, RegisterId},
    model::{
        Contract, Document, DocumentKind, DocumentStatus, DocumentType, InvoiceExtraction,
        MeterAggregate,
    },
    usecase::{
        contract::{
            ComponentInput, ContractInput, InstallmentInput, PositionInput, SettlementInput,
        },
        document::{ConfirmOutcome, ContractChoice, InvoiceConfirmation},
        reading::ReadingsInput,
    },
};
use zaehler_domain::{
    calendar::Calendar,
    meter::Direction,
    tariff::{BasePriceInterval, UnitPricing},
};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format, parse},
    pages::{
        FormData,
        contracts::{date, date_input, decimal, interval_options, money, plain, required_date},
        readings::hint,
        render, render_status, see_other,
    },
    security::constant_time_eq,
    state::WebState,
};

/// Höchstzahl übernommener Zeilen je Art (Ablesungen, Preise).
const MAX_ROWS: usize = 32;

pub struct DocumentRow {
    pub id: String,
    pub name: String,
    pub created: String,
    pub status: String,
}

#[derive(Template)]
#[template(path = "pages/documents/upload.html")]
struct UploadPage {
    ctx: PageCtx,
    ai_enabled: bool,
    file_hint: String,
    documents: Vec<DocumentRow>,
    errors: Vec<String>,
    /// Vorauswahl „Beleg“ (`?kind=receipt`)
    receipt: bool,
}

async fn upload_page(
    ctx: &Ctx,
    state: &WebState,
    errors: Vec<String>,
) -> Result<UploadPage, Response> {
    let lang = ctx.page.lang;
    let documents = state
        .uc
        .list_documents
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?
        .into_iter()
        .map(|d| DocumentRow {
            id: d.id.to_string(),
            created: format::datetime_in(lang, d.created_at, ctx.page.tz),
            status: lang.t(&format!("document-status-{}", d.status.as_str())),
            name: d.original_name,
        })
        .collect();
    Ok(UploadPage {
        ctx: ctx.page.clone(),
        ai_enabled: state.uc.analyse_document.ai_enabled(),
        file_hint: lang.ta(
            "document-file-hint",
            &[(
                "max",
                state
                    .cfg
                    .max_document_bytes
                    .div_ceil(1024 * 1024)
                    .to_string(),
            )],
        ),
        documents,
        errors,
        receipt: false,
    })
}

/// Upload-Formular mit den offenen Dokumenten.
// @impl SPEC_DOC_UPLOADS
#[derive(Deserialize)]
pub struct KindQuery {
    kind: Option<String>,
}

pub async fn list(
    State(state): State<WebState>,
    ctx: Ctx,
    Query(q): Query<KindQuery>,
) -> Result<Response, Response> {
    if !ctx.page.can_write {
        return Err(ctx.fail(AppError::Forbidden));
    }
    let mut page = upload_page(&ctx, &state, Vec::new()).await?;
    page.receipt = q.kind.as_deref() == Some("receipt");
    Ok(render(&page))
}

async fn upload_failed(
    ctx: &Ctx,
    state: &WebState,
    status: StatusCode,
    errors: Vec<String>,
) -> Result<Response, Response> {
    Ok(render_status(
        status,
        &upload_page(ctx, state, errors).await?,
    ))
}

/// Dokument hochladen und sofort analysieren; danach 303 zur Prüfung. Das
/// CSRF-Token steht im Feld `_csrf` des Multipart-Formulars
/// (SPEC_SEC_SESSION_CSRF).
// @impl SPEC_DOC_UPLOADS, SPEC_AI_INVOICE, SPEC_SEC_UPLOADS, SPEC_SEC_SESSION_CSRF
pub async fn upload(
    State(state): State<WebState>,
    ctx: Ctx,
    mut multipart: Multipart,
) -> Result<Response, Response> {
    let lang = ctx.page.lang;
    let mut token = None;
    let mut file = None;
    let mut kind = DocumentKind::Invoice;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => {
                let key = if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    "document-too-large"
                } else {
                    "err-document-type_invalid"
                };
                return upload_failed(&ctx, &state, e.status(), vec![lang.t(key)]).await;
            }
        };
        match field.name() {
            Some("_csrf") => token = field.text().await.ok(),
            Some("kind") => {
                kind = field
                    .text()
                    .await
                    .ok()
                    .and_then(|k| k.parse().ok())
                    .unwrap_or_default();
            }
            Some("file") => {
                let name = field.file_name().unwrap_or_default().to_owned();
                match field.bytes().await {
                    Ok(b) => file = Some((b, name)),
                    Err(e) => {
                        return upload_failed(
                            &ctx,
                            &state,
                            e.status(),
                            vec![lang.t("document-too-large")],
                        )
                        .await;
                    }
                }
            }
            _ => {}
        }
    }
    if !token.is_some_and(|t| constant_time_eq(&t, &ctx.page.csrf)) {
        return Err(crate::security::csrf_failed(lang));
    }
    let (bytes, name) = file.unwrap_or_default();
    let id = match state
        .uc
        .upload_document
        .execute(&ctx.tenant, bytes, &name, kind)
        .await
    {
        Ok(id) => id,
        Err(AppError::Validation(e)) => {
            let messages = e.0.iter().map(|e| lang.err(e)).collect();
            return upload_failed(&ctx, &state, StatusCode::UNPROCESSABLE_ENTITY, messages).await;
        }
        Err(e) => return Err(ctx.fail(e)),
    };
    analyse_now(&state, &ctx, id).await?;
    Ok(see_other(&format!("/documents/{id}")))
}

/// Erkennung; Fehler der AI hinterlassen ein Dokument zur manuellen
/// Übernahme und sind kein Fehler der Seite.
async fn analyse_now(state: &WebState, ctx: &Ctx, id: DocumentId) -> Result<(), Response> {
    state
        .uc
        .analyse_document
        .execute(&ctx.tenant, id)
        .await
        .map(|_| ())
        .map_err(|e| ctx.fail(e))
}

/// Erneute Erkennung, etwa nach einem Zeitüberschreitungsfehler.
// @impl SPEC_AI_INVOICE
pub async fn analyse(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_id(&id).map_err(|e| ctx.fail(e))?;
    analyse_now(&state, &ctx, id).await?;
    Ok(see_other(&format!("/documents/{id}")))
}

fn parse_id(id: &str) -> Result<DocumentId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

/// Datei nur als Download und mit `sandbox`, damit ein eingebettetes Skript
/// eines PDFs nicht im Kontext der Anwendung läuft (SPEC_DOC_UPLOADS).
// @impl SPEC_DOC_UPLOADS, SPEC_SEC_HEADERS
pub async fn file(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_id(&id).map_err(|e| ctx.fail(e))?;
    let (kind, bytes, name) = state
        .uc
        .get_document_file
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let (mime, ext) = match kind {
        DocumentType::Pdf => ("application/pdf", "pdf"),
        DocumentType::Jpeg => ("image/jpeg", "jpg"),
    };
    // nur ASCII im Header; der Anzeigename steht auf der Seite
    let stem: String = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(s, _)| s)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{stem}.{ext}\""))
        .unwrap_or(HeaderValue::from_static("attachment"));
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(mime)),
            (header::CONTENT_DISPOSITION, disposition),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("sandbox; default-src 'none'"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store"),
            ),
        ],
        bytes,
    )
        .into_response())
}

// @impl SPEC_DOC_UPLOADS
pub async fn discard(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let id = parse_id(&id).map_err(|e| ctx.fail(e))?;
    state
        .uc
        .discard_document
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    ctx.flash("flash-document-discarded").await;
    Ok(see_other("/documents"))
}

/// Ablesung im Formular.
#[derive(Debug, Clone, Default)]
pub struct ReadingRow {
    pub key: String,
    pub selected: bool,
    pub serial: String,
    pub register_hint: String,
    pub register: String,
    pub at: String,
    pub value: String,
}

/// Preisstand im Formular.
#[derive(Debug, Clone, Default)]
pub struct PriceRow {
    pub key: String,
    pub selected: bool,
    pub name: String,
    pub position: String,
    pub valid_from: String,
    pub base: String,
    pub interval: String,
    pub price: String,
}

/// Werte des Übernahmeformulars.
#[derive(Debug, Clone, Default)]
pub struct InvoiceForm {
    pub readings: Vec<ReadingRow>,
    /// `none`, `new` oder `existing`
    pub contract_mode: String,
    pub contract_id: String,
    pub provider: String,
    pub contract_number: String,
    pub starts_on: String,
    pub prices: Vec<PriceRow>,
    pub installment: bool,
    pub inst_amount: String,
    pub inst_every: String,
    pub inst_from: String,
    pub settlement: bool,
    pub settle_from: String,
    pub settle_to: String,
    pub settle_amount: String,
}

pub struct Choice {
    pub id: String,
    pub label: String,
}

pub struct SummaryView {
    pub provider: String,
    pub contract_number: String,
    pub period: String,
    pub total: String,
    pub confidence: String,
}

#[derive(Template)]
#[template(path = "pages/documents/invoice.html")]
#[allow(clippy::struct_excessive_bools)] // Zustände der Seite
struct InvoicePage {
    ctx: PageCtx,
    doc_id: String,
    doc_name: String,
    status: String,
    open: bool,
    failed: bool,
    ai_enabled: bool,
    summary: Option<SummaryView>,
    meters: Vec<Choice>,
    meter_id: String,
    registers: Vec<Choice>,
    contracts: Vec<Choice>,
    positions: Vec<Choice>,
    intervals: Vec<(String, String)>,
    form: InvoiceForm,
    warnings: Vec<String>,
    needs_confirm: bool,
    errors: FieldErrors,
    /// Alle Fehler als Text (auch solche ohne eigenes Feld)
    messages: Vec<String>,
}

impl InvoicePage {
    /// Die Fehlertexte stehen gesammelt über dem Formular; Felder werden
    /// nur markiert.
    fn has_errs(&self, field: &str) -> bool {
        !self.errors.for_field(field).is_empty()
    }

    fn row_errs(&self, key: &str, part: &str) -> bool {
        self.has_errs(&format!("{key}_{part}"))
    }
}

/// Lokaler Tagesbeginn; das Ende des Abrechnungszeitraums gilt als Stand am
/// Folgetag 0:00, damit die Ablesung den letzten Tag einschließt.
fn reading_at(d: NaiveDate, period_end: Option<NaiveDate>) -> String {
    let day = if Some(d) == period_end {
        d + Duration::days(1)
    } else {
        d
    };
    format::datetime_input(day.and_time(NaiveTime::MIN))
}

/// Position eines Vertrags mit gleichem Namen (ohne Groß-/Kleinschreibung).
fn matching_position(contracts: &[Contract], name: &str) -> String {
    contracts
        .iter()
        .rev()
        .flat_map(|c| &c.data.positions)
        .find(|p| p.name.trim().eq_ignore_ascii_case(name.trim()))
        .map(|p| p.id.to_string())
        .unwrap_or_default()
}

/// Formular aus dem Vorschlag; ohne Vorschlag leer mit einer Ablesung.
fn from_extraction(
    lang: Lang,
    x: Option<&InvoiceExtraction>,
    agg: Option<&MeterAggregate>,
    contracts: &[Contract],
    is_owner: bool,
) -> InvoiceForm {
    let register_for = |code: Option<&str>| {
        agg.and_then(|a| {
            code.and_then(|c| a.registers.iter().find(|r| r.data.code == c.trim()))
                .or_else(|| a.registers.first())
        })
        .map(|r| r.id.to_string())
        .unwrap_or_default()
    };
    let Some(x) = x else {
        return InvoiceForm {
            readings: vec![ReadingRow {
                key: "r0".into(),
                selected: true,
                register: register_for(None),
                ..ReadingRow::default()
            }],
            contract_mode: "none".into(),
            inst_every: "1".into(),
            ..InvoiceForm::default()
        };
    };
    let readings = x
        .readings
        .iter()
        .take(MAX_ROWS)
        .enumerate()
        .map(|(i, r)| ReadingRow {
            key: format!("r{i}"),
            selected: true,
            serial: r.serial_number.clone().unwrap_or_default(),
            register_hint: r.register.clone().unwrap_or_default(),
            register: register_for(r.register.as_deref()),
            at: reading_at(r.date, x.period_end),
            value: plain(lang, r.value),
        })
        .collect();
    let prices = x
        .prices
        .iter()
        .take(MAX_ROWS)
        .enumerate()
        .map(|(i, p)| PriceRow {
            key: format!("price{i}"),
            selected: is_owner,
            name: p.position.clone(),
            position: matching_position(contracts, &p.position),
            valid_from: p.valid_from.map(date_input).unwrap_or_default(),
            base: p.base_price.map(|v| plain(lang, v)).unwrap_or_default(),
            interval: p
                .base_interval
                .unwrap_or(BasePriceInterval::Year)
                .as_str()
                .to_owned(),
            price: p.unit_price.map(|v| plain(lang, v)).unwrap_or_default(),
        })
        .collect();
    let existing = contracts.last();
    let contract_mode = match (existing, is_owner) {
        (Some(_), _) => "existing",
        (None, true) if !x.prices.is_empty() => "new",
        _ => "none",
    };
    let starts_on = x
        .period_start
        .or_else(|| x.prices.iter().filter_map(|p| p.valid_from).min());
    InvoiceForm {
        readings,
        contract_mode: contract_mode.into(),
        contract_id: existing.map(|c| c.id.to_string()).unwrap_or_default(),
        provider: x.provider.clone().unwrap_or_default(),
        contract_number: x.contract_number.clone().unwrap_or_default(),
        starts_on: starts_on.map(date_input).unwrap_or_default(),
        prices,
        installment: is_owner && x.installment.is_some(),
        inst_amount: x
            .installment
            .as_ref()
            .map(|i| plain(lang, i.amount))
            .unwrap_or_default(),
        inst_every: x
            .installment
            .as_ref()
            .map_or(1, |i| i.every_months)
            .to_string(),
        inst_from: x
            .installment
            .as_ref()
            .and_then(|i| i.valid_from)
            .or_else(|| x.period_end.map(|d| d + Duration::days(1)))
            .map(date_input)
            .unwrap_or_default(),
        settlement: x.total_amount.is_some() && x.period_start.is_some() && x.period_end.is_some(),
        settle_from: x.period_start.map(date_input).unwrap_or_default(),
        settle_to: x.period_end.map(date_input).unwrap_or_default(),
        settle_amount: x.total_amount.map(|v| plain(lang, v)).unwrap_or_default(),
    }
}

fn read_form(form: &FormData) -> InvoiceForm {
    let count = |key: &str| form.get(key).parse::<usize>().unwrap_or(0).min(MAX_ROWS);
    let readings = (0..count("readings"))
        .map(|i| {
            let key = format!("r{i}");
            let f = |part: &str| form.get(&format!("{key}_{part}")).to_owned();
            ReadingRow {
                selected: form.has(&format!("{key}_use")),
                serial: f("serial"),
                register_hint: f("hint"),
                register: f("register"),
                at: f("at"),
                value: f("value"),
                key,
            }
        })
        .collect();
    let prices = (0..count("prices"))
        .map(|i| {
            let key = format!("price{i}");
            let f = |part: &str| form.get(&format!("{key}_{part}")).to_owned();
            PriceRow {
                selected: form.has(&format!("{key}_use")),
                name: f("name"),
                position: f("position"),
                valid_from: f("valid_from"),
                base: f("base"),
                interval: f("interval"),
                price: f("price"),
                key,
            }
        })
        .collect();
    let g = |k: &str| form.get(k).to_owned();
    InvoiceForm {
        readings,
        contract_mode: g("contract_mode"),
        contract_id: g("contract_id"),
        provider: g("provider"),
        contract_number: g("contract_number"),
        starts_on: g("starts_on"),
        prices,
        installment: form.has("installment"),
        inst_amount: g("inst_amount"),
        inst_every: g("inst_every"),
        inst_from: g("inst_from"),
        settlement: form.has("settlement"),
        settle_from: g("settle_from"),
        settle_to: g("settle_to"),
        settle_amount: g("settle_amount"),
    }
}

fn required_decimal(e: &mut FieldErrors, field: &str, value: &str) -> Option<Decimal> {
    if value.trim().is_empty() {
        e.add(field, "form.number_required");
        return None;
    }
    decimal(e, field, value)
}

/// Ablesungen je Zeitpunkt.
fn parse_readings(
    e: &mut FieldErrors,
    rows: &[ReadingRow],
    meter_id: MeterId,
    cal: Calendar,
) -> Vec<ReadingsInput> {
    let mut out: Vec<ReadingsInput> = Vec::new();
    for r in rows.iter().filter(|r| r.selected) {
        let k = &r.key;
        let register = r.register.parse::<RegisterId>().map_err(|_| {
            e.add(format!("{k}_register"), "capture.register_required");
        });
        let value = required_decimal(e, &format!("{k}_value"), &r.value);
        let at = parse::datetime_local(&r.at)
            .map(|l| cal.to_utc(l))
            .map_err(|_| e.add(format!("{k}_at"), "form.datetime_invalid"));
        let (Ok(register), Some(value), Ok(at)) = (register, value, at) else {
            continue;
        };
        match out.iter_mut().find(|i| i.at == at) {
            Some(i) => i.values.push((register, value)),
            None => out.push(ReadingsInput {
                meter_id,
                at,
                values: vec![(register, value)],
                note: None,
            }),
        }
    }
    out
}

fn parse_price(
    e: &mut FieldErrors,
    p: &PriceRow,
    default_from: Option<NaiveDate>,
) -> Option<ComponentInput> {
    let k = &p.key;
    let valid_from = if p.valid_from.trim().is_empty() {
        if default_from.is_none() {
            e.add(format!("{k}_valid_from"), "form.date_required");
        }
        default_from
    } else {
        date(e, &format!("{k}_valid_from"), &p.valid_from)
    };
    let base = decimal(e, &format!("{k}_base"), &p.base).unwrap_or(Decimal::ZERO);
    let price = decimal(e, &format!("{k}_price"), &p.price).unwrap_or(Decimal::ZERO);
    Some(ComponentInput {
        id: None,
        valid_from: valid_from?,
        base_price: base,
        base_interval: p.interval.parse().unwrap_or(BasePriceInterval::Year),
        unit_pricing: UnitPricing::Flat { price },
        factor: None,
    })
}

/// Vertrag aus den ausgewählten Preiszeilen; gleiche Namen bilden eine
/// Position mit mehreren Preisständen.
fn parse_contract(e: &mut FieldErrors, form: &InvoiceForm) -> ContractChoice {
    let selected = || form.prices.iter().filter(|p| p.selected);
    match form.contract_mode.as_str() {
        "new" => {
            let starts_on = required_date(e, "starts_on", &form.starts_on);
            let mut positions: Vec<PositionInput> = Vec::new();
            for p in selected() {
                let name = p.name.trim();
                if name.is_empty() {
                    e.add(format!("{}_name", p.key), "document.position_name_required");
                    continue;
                }
                let Some(c) = parse_price(e, p, starts_on) else {
                    continue;
                };
                match positions.iter_mut().find(|x| x.name == name) {
                    Some(x) => x.components.push(c),
                    None => positions.push(PositionInput {
                        id: None,
                        name: name.to_owned(),
                        register_id: None,
                        factor: Decimal::ONE,
                        vat_pct: None,
                        components: vec![c],
                    }),
                }
            }
            match starts_on {
                Some(starts_on) => ContractChoice::Create(ContractInput {
                    direction: Direction::Import,
                    provider: form.provider.clone(),
                    contract_number: Some(form.contract_number.clone()),
                    starts_on,
                    ends_on: None,
                    billing_start: None,
                    note: None,
                    positions,
                }),
                None => ContractChoice::None,
            }
        }
        "existing" => {
            let Ok(contract) = form.contract_id.parse::<ContractId>() else {
                e.add("contract_id", "document.contract_required");
                return ContractChoice::None;
            };
            let components = selected()
                .filter_map(|p| {
                    let position = p.position.parse::<PositionId>().map_err(|_| {
                        e.add(format!("{}_position", p.key), "document.position_required");
                    });
                    let c = parse_price(e, p, None);
                    Some((position.ok()?, c?))
                })
                .collect();
            ContractChoice::Existing {
                contract,
                components,
            }
        }
        _ => ContractChoice::None,
    }
}

fn parse_confirmation(
    form: &InvoiceForm,
    meter_id: MeterId,
    cal: Calendar,
    confirm_warnings: bool,
) -> Result<InvoiceConfirmation, FieldErrors> {
    let mut e = FieldErrors::default();
    let readings = parse_readings(&mut e, &form.readings, meter_id, cal);
    let contract = parse_contract(&mut e, form);
    let installment = if form.installment {
        let amount = required_decimal(&mut e, "inst_amount", &form.inst_amount);
        let every = if let Ok(Some(n)) = parse::integer::<u8>(&form.inst_every) {
            Some(n)
        } else {
            e.add("inst_every", "form.number_invalid");
            None
        };
        let from = required_date(&mut e, "inst_from", &form.inst_from);
        match (amount, every, from) {
            (Some(amount), Some(every_months), Some(valid_from)) => Some(InstallmentInput {
                valid_from,
                amount,
                every_months,
            }),
            _ => None,
        }
    } else {
        None
    };
    let settlement = if form.settlement {
        let from = required_date(&mut e, "settle_from", &form.settle_from);
        let to = required_date(&mut e, "settle_to", &form.settle_to);
        let amount = required_decimal(&mut e, "settle_amount", &form.settle_amount);
        match (from, to, amount) {
            (Some(period_start), Some(period_end), Some(amount)) => Some(SettlementInput {
                period_start,
                period_end,
                amount,
            }),
            _ => None,
        }
    } else {
        None
    };
    if e.is_empty() {
        Ok(InvoiceConfirmation {
            meter_id,
            readings,
            confirm_warnings,
            contract,
            installment,
            settlement,
        })
    } else {
        Err(e)
    }
}

/// Daten der Seite außer dem Formular.
struct Loaded {
    doc: Document,
    meters: Vec<Choice>,
    meter: Option<MeterAggregate>,
    contracts: Vec<Contract>,
}

async fn load(
    state: &WebState,
    ctx: &Ctx,
    id: DocumentId,
    meter: Option<MeterId>,
) -> Result<Loaded, Response> {
    let doc = state
        .uc
        .get_document
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let meters = state
        .uc
        .list_meters
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?;
    let selected = meter.or(doc.meter_id).or_else(|| {
        // eine einzige Messstelle ist eindeutig
        (meters.len() == 1).then(|| meters[0].meter.id)
    });
    let (meter, contracts) = match selected {
        Some(m) => {
            let detail = state
                .uc
                .get_meter
                .execute(&ctx.tenant, m)
                .await
                .map_err(|e| ctx.fail(e))?;
            let contracts = state
                .uc
                .meter_contracts
                .execute(&ctx.tenant, m)
                .await
                .map_err(|e| ctx.fail(e))?;
            (Some(detail.aggregate), contracts)
        }
        None => (None, Vec::new()),
    };
    Ok(Loaded {
        doc,
        meters: meters
            .into_iter()
            .map(|m| Choice {
                id: m.meter.id.to_string(),
                label: m.meter.data.name,
            })
            .collect(),
        meter,
        contracts,
    })
}

fn contract_label(lang: Lang, c: &Contract) -> String {
    format!(
        "{} ({})",
        c.data.provider,
        format::date(lang, c.data.starts_on)
    )
}

fn summary(lang: Lang, x: &InvoiceExtraction) -> SummaryView {
    let dash = || lang.t("capture-not-recognized");
    let d = |v: Option<NaiveDate>| v.map_or_else(dash, |d| format::date(lang, d));
    SummaryView {
        provider: x.provider.clone().unwrap_or_else(dash),
        contract_number: x.contract_number.clone().unwrap_or_else(dash),
        period: lang.ta(
            "contract-period",
            &[("start", d(x.period_start)), ("end", d(x.period_end))],
        ),
        total: x.total_amount.map_or_else(dash, |v| money(lang, v)),
        confidence: format::decimal(lang, x.confidence * Decimal::ONE_HUNDRED, 0),
    }
}

fn invoice_page(
    ctx: &Ctx,
    state: &WebState,
    l: &Loaded,
    form: Option<InvoiceForm>,
    warnings: Vec<String>,
    needs_confirm: bool,
    errors: FieldErrors,
) -> InvoicePage {
    let lang = ctx.page.lang;
    let x = l.doc.extraction.as_ref();
    let form = form.unwrap_or_else(|| {
        from_extraction(lang, x, l.meter.as_ref(), &l.contracts, ctx.page.is_owner)
    });
    let meter_id = l
        .meter
        .as_ref()
        .map(|m| m.meter.id.to_string())
        .unwrap_or_default();
    InvoicePage {
        doc_id: l.doc.id.to_string(),
        doc_name: l.doc.original_name.clone(),
        status: lang.t(&format!("document-status-{}", l.doc.status.as_str())),
        open: l.doc.status.is_open(),
        failed: l.doc.status == DocumentStatus::Failed,
        ai_enabled: state.uc.analyse_document.ai_enabled(),
        summary: x.map(|x| summary(lang, x)),
        meters: l
            .meters
            .iter()
            .map(|m| Choice {
                id: m.id.clone(),
                label: m.label.clone(),
            })
            .collect(),
        registers: l
            .meter
            .iter()
            .flat_map(|m| &m.registers)
            .map(|r| Choice {
                id: r.id.to_string(),
                label: r.data.code.clone(),
            })
            .collect(),
        contracts: l
            .contracts
            .iter()
            .map(|c| Choice {
                id: c.id.to_string(),
                label: contract_label(lang, c),
            })
            .collect(),
        positions: l
            .contracts
            .iter()
            .flat_map(|c| {
                c.data.positions.iter().map(move |p| Choice {
                    id: p.id.to_string(),
                    label: format!("{}: {}", contract_label(lang, c), p.name),
                })
            })
            .collect(),
        intervals: interval_options(lang),
        messages: errors.0.iter().map(|e| lang.err(e)).collect(),
        meter_id,
        form,
        warnings,
        needs_confirm,
        errors,
        ctx: ctx.page.clone(),
    }
}

#[derive(Deserialize)]
pub struct MeterQuery {
    meter: Option<String>,
}

/// Prüfseite mit dem Vorschlag; `?meter=` wählt eine andere Messstelle.
// @impl SPEC_AI_INVOICE, SPEC_DOC_UPLOADS
pub async fn show(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<MeterQuery>,
) -> Result<Response, Response> {
    let id = parse_id(&id).map_err(|e| ctx.fail(e))?;
    let doc = state
        .uc
        .get_document
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    if doc.kind == DocumentKind::Receipt {
        return super::receipts::show(&state, &ctx, doc).await;
    }
    let meter = q.meter.as_deref().and_then(|m| m.parse().ok());
    let l = load(&state, &ctx, id, meter).await?;
    Ok(render(&invoice_page(
        &ctx,
        &state,
        &l,
        None,
        Vec::new(),
        false,
        FieldErrors::default(),
    )))
}

/// Übernimmt die ausgewählten Teile; bei Plausibilitätswarnungen erneut mit
/// Bestätigungsfeld.
// @impl SPEC_AI_INVOICE, SPEC_RDG_PLAUSIBILITY
pub async fn confirm(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Result<Response, Response> {
    let id = parse_id(&id).map_err(|e| ctx.fail(e))?;
    let data = FormData::new(pairs);
    let meter_id = data
        .get("meter_id")
        .parse::<MeterId>()
        .map_err(|_| ctx.fail(AppError::NotFound))?;
    let l = load(&state, &ctx, id, Some(meter_id)).await?;
    let form = read_form(&data);
    let rerender = |warnings, needs_confirm, errors| {
        render_status(
            StatusCode::UNPROCESSABLE_ENTITY,
            &invoice_page(
                &ctx,
                &state,
                &l,
                Some(form.clone()),
                warnings,
                needs_confirm,
                errors,
            ),
        )
    };
    let confirmed = data.get("confirm_warnings") == "true";
    let input = match parse_confirmation(&form, meter_id, Calendar::new(ctx.page.tz), confirmed) {
        Ok(i) => i,
        Err(errors) => return Ok(rerender(Vec::new(), false, errors)),
    };
    let lang = ctx.page.lang;
    match state
        .uc
        .confirm_invoice
        .execute(&ctx.tenant, id, input)
        .await
    {
        Ok(ConfirmOutcome::Saved { contract }) => {
            ctx.flash("flash-invoice-saved").await;
            Ok(see_other(&contract.map_or_else(
                || format!("/meters/{meter_id}"),
                |c| format!("/contracts/{c}"),
            )))
        }
        Ok(ConfirmOutcome::NeedsConfirmation(checks)) => {
            let warnings = checks
                .iter()
                .flat_map(|(at, regs)| {
                    regs.iter().filter_map(move |r| {
                        hint(lang, &r.plausibility).map(|h| {
                            format!("{}: {h}", format::datetime_in(lang, *at, ctx.page.tz))
                        })
                    })
                })
                .collect();
            Ok(rerender(warnings, true, FieldErrors::default()))
        }
        Err(AppError::Validation(errors)) => Ok(rerender(Vec::new(), false, errors)),
        Err(AppError::Conflict(code)) => {
            let mut errors = FieldErrors::default();
            errors.add("contract", code);
            Ok(rerender(Vec::new(), false, errors))
        }
        Err(e) => Err(ctx.fail(e)),
    }
}
