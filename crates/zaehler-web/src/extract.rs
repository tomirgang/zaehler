//! Extraktoren: angemeldeter Nutzer mit Tenant und Seitenkontext.

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use chrono_tz::Tz;
use tower_sessions::Session;
use zaehler_app::{AppError, Role, Tenant, ids::HouseholdId, ids::UserId, model::HouseholdSummary};

use crate::{i18n::Lang, pages, security, state::WebState};

pub const SESSION_USER: &str = "uid";
pub const SESSION_HOUSEHOLD: &str = "hid";
pub const SESSION_CSRF: &str = "csrf";
pub const SESSION_FLASH: &str = "flash";
pub const SESSION_OIDC: &str = "oidc";
/// Token einer Einladung, die nach der Anmeldung angezeigt wird (SPEC_TEN_INVITES)
pub const SESSION_INVITE: &str = "invite";

/// Daten, die jede Seite im Layout braucht.
#[derive(Debug, Clone)]
pub struct PageCtx {
    pub lang: Lang,
    pub csrf: String,
    pub user_name: String,
    pub households: Vec<HouseholdSummary>,
    pub active: HouseholdId,
    pub tz: Tz,
    pub is_owner: bool,
    pub can_write: bool,
    /// Navigation „Kosten“ nur mit Verträgen (SPEC_TAR_COST_VIEW)
    pub has_contracts: bool,
    pub flash: Option<String>,
    pub static_prefix: String,
    pub version: &'static str,
}

impl PageCtx {
    /// Name des aktiven Haushalts für die Titelleiste (SPEC_UI_TOPBAR).
    // @impl SPEC_UI_TOPBAR
    #[must_use]
    pub fn active_name(&self) -> &str {
        self.households
            .iter()
            .find(|h| h.id == self.active)
            .map_or("", |h| h.name.as_str())
    }

    /// „Angemeldet als …“ für die Titelleiste.
    #[must_use]
    pub fn signed_in(&self) -> String {
        self.lang
            .ta("nav-signed-in", &[("name", self.user_name.clone())])
    }
}

/// Angemeldeter Nutzer mit geprüftem Tenant.
pub struct Ctx {
    pub tenant: Tenant,
    pub page: PageCtx,
    pub hx: bool,
    pub session: Session,
}

impl Ctx {
    /// Flash-Meldung für die nächste Seite (Fluent-Schlüssel).
    pub async fn flash(&self, key: &str) {
        if let Err(e) = self.session.insert(SESSION_FLASH, key).await {
            tracing::warn!(error = %e, "flash not stored");
        }
    }

    /// Fehler als Antwort (SPEC_ERR_HANDLING).
    #[must_use]
    pub fn fail(&self, err: AppError) -> Response {
        pages::error_response(&self.page, self.hx, err)
    }
}

pub fn is_hx(parts: &Parts) -> bool {
    parts.headers.get("hx-request").is_some_and(|v| v == "true")
}

/// Weiterleitung zum Login; bei HTMX über `HX-Redirect`.
#[must_use]
pub fn login_redirect(hx: bool) -> Response {
    if hx {
        (StatusCode::OK, [("hx-redirect", "/auth/login")]).into_response()
    } else {
        (StatusCode::SEE_OTHER, [(header::LOCATION, "/auth/login")]).into_response()
    }
}

fn internal(e: impl std::fmt::Display) -> Response {
    let reference = uuid::Uuid::now_v7();
    tracing::error!(%reference, error = %e, "request context failed");
    // Sprache des Nutzers ist hier noch unbekannt
    let text = [Lang::De, Lang::En]
        .map(|l| l.ta("error-internal", &[("ref", reference.to_string())]))
        .join("\n");
    (StatusCode::INTERNAL_SERVER_ERROR, text).into_response()
}

impl FromRequestParts<WebState> for Ctx {
    type Rejection = Response;

    // @impl SPEC_SEC_TENANT_SCOPE
    async fn from_request_parts(
        parts: &mut Parts,
        state: &WebState,
    ) -> Result<Self, Self::Rejection> {
        let hx = is_hx(parts);
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|(_, msg)| internal(msg))?;
        let Some(user_id) = session
            .get::<UserId>(SESSION_USER)
            .await
            .map_err(internal)?
        else {
            return Err(login_redirect(hx));
        };
        let preferred = session
            .get::<HouseholdId>(SESSION_HOUSEHOLD)
            .await
            .map_err(internal)?;
        let resolved = match state.uc.resolve_tenant.execute(user_id, preferred).await {
            Ok(r) => r,
            Err(AppError::Unauthenticated) => {
                session.flush().await.map_err(internal)?;
                return Err(login_redirect(hx));
            }
            Err(e) => return Err(internal(e)),
        };
        let active = resolved.tenant.household_id();
        // Log- und Trace-Felder des Request-Spans (SPEC_OPS_OTEL)
        let span = tracing::Span::current();
        span.record("household", tracing::field::display(active));
        span.record("user", tracing::field::display(user_id));
        if preferred != Some(active) {
            session
                .insert(SESSION_HOUSEHOLD, active)
                .await
                .map_err(internal)?;
        }
        let existing = session
            .get::<String>(SESSION_CSRF)
            .await
            .map_err(internal)?;
        let csrf = if let Some(t) = existing {
            t
        } else {
            let t = security::new_token();
            session.insert(SESSION_CSRF, &t).await.map_err(internal)?;
            t
        };
        let flash = session
            .remove::<String>(SESSION_FLASH)
            .await
            .map_err(internal)?;
        let tz = resolved
            .households
            .iter()
            .find(|h| h.id == active)
            .map_or(chrono_tz::Europe::Berlin, |h| h.timezone);
        let page = PageCtx {
            lang: Lang::from_code(&resolved.user.locale),
            csrf,
            user_name: resolved.user.display_name,
            households: resolved.households,
            active,
            tz,
            is_owner: resolved.tenant.role() >= Role::Owner,
            can_write: resolved.tenant.role() >= Role::Member,
            has_contracts: resolved.has_contracts,
            flash,
            static_prefix: format!("/static/{}", state.cfg.version),
            version: state.cfg.version,
        };
        Ok(Self {
            tenant: resolved.tenant,
            page,
            hx,
            session,
        })
    }
}

/// Sprache für nicht angemeldete Seiten aus `Accept-Language`.
pub struct AnonLang(pub Lang);

impl<S: Send + Sync> FromRequestParts<S> for AnonLang {
    type Rejection = std::convert::Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let header = parts
            .headers
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|v| v.to_str().ok());
        std::future::ready(Ok(Self(Lang::from_accept_language(header))))
    }
}
