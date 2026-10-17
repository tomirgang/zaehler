//! API-Tokens und automatisch gelieferte Zählerstände (SPEC_API_AUTH,
//! SPEC_API_DAILY_READINGS).

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use zaehler_domain::{
    calendar::Calendar,
    reading::{Plausibility, ReadingSource},
};

use crate::{
    AppError, FieldErrors, PortError, Role, Tenant,
    ids::{ApiTokenId, MeterId, ReadingId, RegisterId},
    model::{ApiToken, DailyOutcome, MeterAggregate, NewApiToken, Reading},
    port::UnitOfWork,
    secret,
    usecase::{
        reading::{ReadingsInput, check_all, enqueue_readings},
        use_case, validate,
    },
};

/// Längster Name eines Tokens.
pub const MAX_TOKEN_NAME: usize = 80;

/// Höchstzahl der Stände je Anfrage (SPEC_API_REST).
pub const MAX_INGEST: usize = 1000;

/// `last_used_at` wird höchstens einmal je Minute geschrieben.
const TOUCH_INTERVAL: Duration = Duration::minutes(1);

use_case!(
    /// API-Token für den eigenen Nutzer erzeugen (Viewer).
    CreateApiToken,
    /// Eigene Tokens, für Owner alle des Haushalts (Viewer).
    ListApiTokens,
    /// Eigenes Token, als Owner jedes des Haushalts widerrufen (Viewer).
    RevokeApiToken,
    /// Bearer-Token prüfen und `Tenant` erzeugen.
    ResolveApiTenant,
    /// Automatisch gelieferte Stände speichern (Member).
    IngestReadings,
);

/// Eingaben eines neuen Tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiTokenInput {
    pub name: String,
    pub role: Role,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Erzeugtes Token; `token` erscheint nur in dieser Antwort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedApiToken {
    pub id: ApiTokenId,
    pub token: String,
}

/// Geprüfter Aufrufer der REST-API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiPrincipal {
    pub tenant: Tenant,
    pub token: ApiTokenId,
}

impl CreateApiToken {
    /// Die Rolle des Tokens ist höchstens die eigene Rolle.
    // @impl SPEC_API_AUTH
    #[tracing::instrument(name = "CreateApiToken", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        input: ApiTokenInput,
    ) -> Result<CreatedApiToken, AppError> {
        t.require(Role::Viewer)?;
        let now = self.p.clock.now();
        let name = input.name.trim().to_owned();
        let mut e = FieldErrors::default();
        if name.is_empty()
            || name.chars().count() > MAX_TOKEN_NAME
            || name.chars().any(char::is_control)
        {
            e.add_with(
                "name",
                "api.name_invalid",
                vec![("max", MAX_TOKEN_NAME.to_string())],
            );
        }
        if input.role > t.role() {
            e.add("role", "api.role_too_high");
        }
        if input.expires_at.is_some_and(|x| x <= now) {
            e.add("expires_at", "api.expiry_past");
        }
        e.into_result()?;
        let secret = secret::generate_api_token();
        let id = ApiTokenId(self.p.ids.new_uuid());
        let token = NewApiToken {
            id,
            name,
            role: input.role,
            token_hash: secret.hash,
            prefix: secret.token.chars().take(8).collect(),
            created_at: now,
            expires_at: input.expires_at,
        };
        let mut uow = self.p.uow.begin().await?;
        uow.api_tokens().create(t, &token).await?;
        uow.commit().await?;
        tracing::info!(household = %t.household_id(), token = %id, role = input.role.as_str(), "api token created");
        Ok(CreatedApiToken {
            id,
            token: secret.token,
        })
    }
}

/// Tokens, die der Tenant sehen darf.
async fn visible(uow: &mut dyn UnitOfWork, t: &Tenant) -> Result<Vec<ApiToken>, AppError> {
    let all = uow.api_tokens().list(t).await?;
    Ok(if t.role() >= Role::Owner {
        all
    } else {
        all.into_iter()
            .filter(|x| x.user_id == t.user_id())
            .collect()
    })
}

impl ListApiTokens {
    // @impl SPEC_API_AUTH
    #[tracing::instrument(name = "ListApiTokens", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<Vec<ApiToken>, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        visible(uow.as_mut(), t).await
    }
}

impl RevokeApiToken {
    // @impl SPEC_API_AUTH
    #[tracing::instrument(name = "RevokeApiToken", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: ApiTokenId) -> Result<(), AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        if !visible(uow.as_mut(), t).await?.iter().any(|x| x.id == id) {
            return Err(AppError::NotFound);
        }
        uow.api_tokens().revoke(t, id, self.p.clock.now()).await?;
        uow.commit().await?;
        tracing::info!(household = %t.household_id(), token = %id, "api token revoked");
        Ok(())
    }
}

impl ResolveApiTenant {
    /// Unbekannte, abgelaufene oder widerrufene Tokens und Tokens ohne
    /// Mitgliedschaft ergeben `Unauthenticated`; die wirksame Rolle ist das
    /// Minimum aus Token- und Mitgliedsrolle.
    // @impl SPEC_API_AUTH
    #[tracing::instrument(name = "ResolveApiTenant", skip_all)]
    pub async fn execute(&self, token: &str) -> Result<ApiPrincipal, AppError> {
        if !secret::is_api_token(token) {
            return Err(AppError::Unauthenticated);
        }
        let now = self.p.clock.now();
        let mut uow = self.p.uow.begin().await?;
        let tok = uow
            .api_tokens()
            .find_by_hash(&secret::hash(token))
            .await?
            .filter(|x| x.is_active(now))
            .ok_or(AppError::Unauthenticated)?;
        let member = uow
            .memberships()
            .role(tok.user_id, tok.household_id)
            .await?
            .ok_or(AppError::Unauthenticated)?;
        if tok.last_used_at.is_none_or(|l| now - l >= TOUCH_INTERVAL) {
            uow.api_tokens().touch(tok.id, now).await?;
            uow.commit().await?;
        }
        Ok(ApiPrincipal {
            tenant: Tenant::new(tok.user_id, tok.household_id, member.min(tok.role)),
            token: tok.id,
        })
    }
}

/// Ziel eines automatisch gelieferten Stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestTarget {
    Register(RegisterId),
    /// Zählernummer des Geräts und Code des Registers
    Serial {
        serial: String,
        register: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestItem {
    pub target: IngestTarget,
    pub at: DateTime<Utc>,
    pub value: Decimal,
}

/// Ergebnis je Eintrag in der Reihenfolge der Anfrage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestResult {
    Stored(ReadingId),
    Replaced(ReadingId),
    Ignored,
    /// Fehlercode (nicht übersetzt)
    Error(&'static str),
}

impl IngestResult {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stored(_) => "stored",
            Self::Replaced(_) => "replaced",
            Self::Ignored => "ignored",
            Self::Error(_) => "error",
        }
    }
}

/// Messstellen des Haushalts mit ihren Ablesungen, einmal je Anfrage
/// geladen und nach jedem Eintrag fortgeschrieben.
struct Meters {
    list: Vec<(MeterAggregate, Vec<Reading>)>,
}

impl Meters {
    async fn load(uow: &mut dyn UnitOfWork, t: &Tenant) -> Result<Self, AppError> {
        let mut list = Vec::new();
        for s in uow.meters().list(t).await? {
            if let Some(agg) = uow.meters().get(t, s.meter.id).await? {
                let readings = uow.readings().list_for_meter(t, s.meter.id).await?;
                list.push((agg, readings));
            }
        }
        Ok(Self { list })
    }

    fn find(&self, target: &IngestTarget) -> Result<(usize, RegisterId), &'static str> {
        match target {
            IngestTarget::Register(id) => self
                .list
                .iter()
                .position(|(a, _)| a.register(*id).is_some())
                .map(|i| (i, *id))
                .ok_or("api.register_unknown"),
            IngestTarget::Serial { serial, register } => {
                let serial = &validate::normalize_serial(serial);
                let i = self
                    .list
                    .iter()
                    .position(|(a, _)| {
                        a.devices
                            .iter()
                            .any(|d| d.data.serial_number.as_deref() == Some(serial.as_str()))
                    })
                    .ok_or("api.serial_unknown")?;
                let agg = &self.list[i].0;
                let active = agg
                    .active_device()
                    .and_then(|d| d.data.serial_number.as_deref());
                if active != Some(serial.as_str()) {
                    return Err("api.device_inactive");
                }
                agg.registers
                    .iter()
                    .find(|r| r.data.code == *register)
                    .map(|r| (i, r.id))
                    .ok_or("api.register_unknown")
            }
        }
    }
}

/// Code des ersten Feldfehlers.
fn first_code(e: &AppError) -> &'static str {
    match e {
        AppError::Validation(f) => f.0.first().map_or("api.invalid", |x| x.code),
        AppError::Conflict(c) => c,
        AppError::NotFound => "api.register_unknown",
        _ => "api.invalid",
    }
}

impl IngestReadings {
    /// Je Register, Kalendertag (Zeitzone des Haushalts) und Token höchstens
    /// eine Ablesung, die späteste. Plausibilitätswarnungen führen nicht zur
    /// Ablehnung, sondern zu Log und Metrik.
    // @impl SPEC_API_DAILY_READINGS
    #[tracing::instrument(name = "IngestReadings", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        token: ApiTokenId,
        items: &[IngestItem],
    ) -> Result<Vec<IngestResult>, AppError> {
        t.require(Role::Member)?;
        if items.is_empty() || items.len() > MAX_INGEST {
            return Err(AppError::field("readings", "api.count_invalid"));
        }
        let now = self.p.clock.now();
        let mut uow = self.p.uow.begin().await?;
        let cal = Calendar::new(uow.households().get(t).await?.data.timezone);
        let mut meters = Meters::load(uow.as_mut(), t).await?;
        let mut results = Vec::with_capacity(items.len());
        let mut changed: BTreeMap<MeterId, Vec<ReadingId>> = BTreeMap::new();
        for item in items {
            let r = self
                .ingest_one(uow.as_mut(), t, token, cal, now, &mut meters, item)
                .await?;
            if let (IngestResult::Stored(id) | IngestResult::Replaced(id), Ok((i, _))) =
                (&r, meters.find(&item.target))
            {
                let ids = changed.entry(meters.list[i].0.meter.id).or_default();
                if !ids.contains(id) {
                    ids.push(*id);
                }
            }
            metrics::counter!("zaehler_api_readings_total", "result" => r.as_str()).increment(1);
            results.push(r);
        }
        // Alarme wie bei der Erfassung (E14)
        for (meter, ids) in &changed {
            enqueue_readings(&self.p, uow.as_mut(), t, *meter, ids).await?;
        }
        uow.commit().await?;
        tracing::info!(household = %t.household_id(), %token, count = items.len(), "api readings ingested");
        Ok(results)
    }

    #[allow(clippy::too_many_arguments)] // Zustand der laufenden Anfrage
    async fn ingest_one(
        &self,
        uow: &mut dyn UnitOfWork,
        t: &Tenant,
        token: ApiTokenId,
        cal: Calendar,
        now: DateTime<Utc>,
        meters: &mut Meters,
        item: &IngestItem,
    ) -> Result<IngestResult, AppError> {
        let (i, register_id) = match meters.find(&item.target) {
            Ok(x) => x,
            Err(code) => return Ok(IngestResult::Error(code)),
        };
        let (agg, readings) = &meters.list[i];
        if agg.meter.archived {
            return Ok(IngestResult::Error("meter.archived"));
        }
        let Some(device) = agg.active_device() else {
            return Ok(IngestResult::Error("meter.no_active_device"));
        };
        if item.at > now + Duration::minutes(5) {
            return Ok(IngestResult::Error("reading.in_future"));
        }
        if item.at < device.data.installed_at {
            return Ok(IngestResult::Error("reading.before_installation"));
        }
        if let Some(same) = readings.iter().find(|r| {
            r.register_id == register_id && r.device_id == device.id && r.read_at == item.at
        }) {
            return Ok(if same.value == item.value {
                IngestResult::Ignored
            } else {
                IngestResult::Error("reading.duplicate")
            });
        }
        let input = ReadingsInput {
            meter_id: agg.meter.id,
            at: item.at,
            values: vec![(register_id, item.value)],
            note: None,
        };
        let checks = match check_all(agg, readings, &input) {
            Ok(c) => c,
            Err(e) => return Ok(IngestResult::Error(first_code(&e))),
        };
        for c in checks.iter().filter(|c| c.plausibility != Plausibility::Ok) {
            let kind = match c.plausibility {
                Plausibility::Decreasing => "decreasing",
                Plausibility::RateTooHigh => "rate_too_high",
                Plausibility::OutOfOrder => "out_of_order",
                Plausibility::Ok => "ok",
            };
            tracing::warn!(household = %t.household_id(), meter = %agg.meter.id, register = %register_id, %token, kind, "api reading implausible");
            metrics::counter!("zaehler_api_reading_warnings_total", "kind" => kind).increment(1);
        }
        let day = cal.local_date(item.at);
        let from = cal.start_of(day);
        let until = cal.start_of(day.succ_opt().unwrap_or(day));
        let reading = Reading {
            id: ReadingId(self.p.ids.new_uuid()),
            register_id,
            device_id: device.id,
            read_at: item.at,
            value: item.value,
            source: ReadingSource::Api,
            note: None,
            photo_key: None,
            ai_confidence: None,
            created_by: Some(t.user_id()),
        };
        let outcome = uow
            .readings()
            .upsert_daily(t, &reading, token, from, until)
            .await
            .map_err(|e| match e {
                PortError::Conflict(_) => AppError::Conflict("reading.duplicate"),
                other => other.into(),
            })?;
        let list = &mut meters.list[i].1;
        Ok(match outcome {
            DailyOutcome::Stored(id) => {
                list.push(Reading { id, ..reading });
                list.sort_by_key(|r| r.read_at);
                IngestResult::Stored(id)
            }
            DailyOutcome::Replaced(id) => {
                if let Some(old) = list.iter_mut().find(|r| r.id == id) {
                    *old = Reading { id, ..reading };
                }
                list.sort_by_key(|r| r.read_at);
                IngestResult::Replaced(id)
            }
            DailyOutcome::Ignored => IngestResult::Ignored,
        })
    }
}
