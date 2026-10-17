//! Bearer-Authentifizierung der REST-API (SPEC_API_AUTH). Cookies und
//! Sitzungen werden unter `/api/v1` nicht ausgewertet.

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use zaehler_app::{AppError, Tenant, ids::ApiTokenId};

use super::error::{ApiError, Problem};
use crate::state::WebState;

/// Geprüfter Aufrufer mit Tenant und Token-ID.
pub struct ApiCtx {
    pub tenant: Tenant,
    pub token: ApiTokenId,
}

impl FromRequestParts<WebState> for ApiCtx {
    type Rejection = ApiError;

    // @impl SPEC_API_AUTH, SPEC_SEC_TENANT_SCOPE
    async fn from_request_parts(
        parts: &mut Parts,
        state: &WebState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .ok_or_else(Problem::unauthenticated)?;
        let principal = match state.uc.resolve_api_tenant.execute(token).await {
            Ok(p) => p,
            Err(AppError::Unauthenticated) => {
                tracing::info!("api token rejected");
                return Err(Problem::unauthenticated().into());
            }
            Err(e) => return Err(e.into()),
        };
        // Log- und Trace-Felder: Token-ID, nie das Token (SPEC_API_AUTH)
        let span = tracing::Span::current();
        span.record(
            "household",
            tracing::field::display(principal.tenant.household_id()),
        );
        span.record("user", tracing::field::display(principal.tenant.user_id()));
        span.record("api_token", tracing::field::display(principal.token));
        Ok(Self {
            tenant: principal.tenant,
            token: principal.token,
        })
    }
}
