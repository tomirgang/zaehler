//! Persönliche API-Tokens (SPEC_API_AUTH).

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};
use chrono::NaiveDate;
use zaehler_app::{
    AppError, FieldErrors, Role,
    ids::ApiTokenId,
    usecase::api::{ApiTokenInput, MAX_TOKEN_NAME},
};
use zaehler_domain::calendar::Calendar;

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format},
    pages::{FormData, field_errors, render_status, see_other},
    state::WebState,
};

pub struct TokenRow {
    pub id: String,
    pub name: String,
    pub user: String,
    pub role_label: String,
    pub prefix: String,
    pub created: String,
    pub expires: String,
    pub last_used: String,
    /// `active`, `expired` oder `revoked`
    pub state: &'static str,
    pub state_label: String,
}

#[derive(Template)]
#[template(path = "pages/settings/tokens.html")]
struct TokensPage {
    ctx: PageCtx,
    tokens: Vec<TokenRow>,
    roles: Vec<(&'static str, String)>,
    /// Soeben erzeugtes Token; erscheint nur einmal
    created: Option<String>,
    name: String,
    role: String,
    expires_on: String,
    max_name: usize,
    api_url: String,
    errors: FieldErrors,
}

impl TokensPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn has_errs(&self, field: &str) -> bool {
        !self.errors.for_field(field).is_empty()
    }

    #[allow(clippy::unused_self)] // Askama ruft Hilfen über `self` auf
    fn same(&self, a: &str, b: &str) -> bool {
        a == b
    }
}

fn role_label(lang: Lang, role: Role) -> String {
    lang.t(&format!("role-{}", role.as_str()))
}

struct Input {
    created: Option<String>,
    name: String,
    role: String,
    expires_on: String,
    errors: FieldErrors,
    status: StatusCode,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            created: None,
            name: String::new(),
            role: Role::Member.as_str().into(),
            expires_on: String::new(),
            errors: FieldErrors::default(),
            status: StatusCode::OK,
        }
    }
}

async fn page(state: &WebState, ctx: &Ctx, input: Input) -> Response {
    let lang = ctx.page.lang;
    let tz = ctx.page.tz;
    let tokens = match state.uc.list_api_tokens.execute(&ctx.tenant).await {
        Ok(t) => t,
        Err(e) => return ctx.fail(e),
    };
    let now = chrono::Utc::now();
    let own = ctx.tenant.role();
    let page = TokensPage {
        tokens: tokens
            .into_iter()
            .map(|t| {
                let state = if t.revoked_at.is_some() {
                    "revoked"
                } else if t.is_active(now) {
                    "active"
                } else {
                    "expired"
                };
                TokenRow {
                    id: t.id.to_string(),
                    name: t.name,
                    user: t.user_name,
                    role_label: role_label(lang, t.role),
                    prefix: t.prefix,
                    created: format::datetime_in(lang, t.created_at, tz),
                    expires: t
                        .expires_at
                        .map(|x| format::datetime_in(lang, x, tz))
                        .unwrap_or_default(),
                    last_used: t
                        .last_used_at
                        .map(|x| format::datetime_in(lang, x, tz))
                        .unwrap_or_default(),
                    state,
                    state_label: lang.t(&format!("token-state-{state}")),
                }
            })
            .collect(),
        roles: [Role::Viewer, Role::Member, Role::Owner]
            .into_iter()
            .filter(|r| *r <= own)
            .map(|r| (r.as_str(), role_label(lang, r)))
            .collect(),
        created: input.created,
        name: input.name,
        role: input.role,
        expires_on: input.expires_on,
        max_name: MAX_TOKEN_NAME,
        api_url: format!("{}/api/v1", state.cfg.public_url),
        errors: input.errors,
        ctx: ctx.page.clone(),
    };
    render_status(input.status, &page)
}

// @impl SPEC_API_AUTH
pub async fn show(State(state): State<WebState>, ctx: Ctx) -> Response {
    page(&state, &ctx, Input::default()).await
}

/// Erzeugt ein Token und zeigt es genau einmal an.
// @impl SPEC_API_AUTH
pub async fn create(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let form = FormData::new(pairs);
    let mut input = Input {
        name: form.get("name").to_owned(),
        role: form.get("role").to_owned(),
        expires_on: form.get("expires_on").to_owned(),
        ..Input::default()
    };
    let mut errors = FieldErrors::default();
    let role = form.get("role").parse::<Role>().ok();
    if role.is_none() {
        errors.add("role", "member.role_invalid");
    }
    // Ablauf zum Beginn des Folgetags in der Zeitzone des Haushalts
    let expires_at = form.opt("expires_on").and_then(|v| {
        let parsed = NaiveDate::parse_from_str(&v, "%Y-%m-%d").ok();
        if parsed.is_none() {
            errors.add("expires_on", "api.expiry_invalid");
        }
        parsed?
            .succ_opt()
            .map(|next| Calendar::new(ctx.page.tz).start_of(next))
    });
    let result = match (role, errors.is_empty()) {
        (Some(role), true) => {
            state
                .uc
                .create_api_token
                .execute(
                    &ctx.tenant,
                    ApiTokenInput {
                        name: input.name.clone(),
                        role,
                        expires_at,
                    },
                )
                .await
        }
        _ => Err(AppError::Validation(errors)),
    };
    match result {
        Ok(created) => {
            page(
                &state,
                &ctx,
                Input {
                    created: Some(created.token),
                    ..Input::default()
                },
            )
            .await
        }
        Err(AppError::Validation(errors)) => {
            input.errors = errors;
            input.status = StatusCode::UNPROCESSABLE_ENTITY;
            page(&state, &ctx, input).await
        }
        Err(e) => ctx.fail(e),
    }
}

// @impl SPEC_API_AUTH
pub async fn revoke(State(state): State<WebState>, ctx: Ctx, Path(id): Path<String>) -> Response {
    let result = match id.parse::<ApiTokenId>() {
        Ok(id) => state.uc.revoke_api_token.execute(&ctx.tenant, id).await,
        Err(_) => Err(AppError::NotFound),
    };
    match result {
        Ok(()) => {
            ctx.flash("flash-token-revoked").await;
            see_other("/settings/tokens")
        }
        Err(e) => ctx.fail(e),
    }
}
