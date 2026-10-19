//! Einstellungen: Haushalt, Sprache, weitere Haushalte, Löschen
//! (SPEC_TEN_HOUSEHOLDS, SPEC_PRIV_HOUSEHOLD_DELETE).

use askama::Template;
use axum::{extract::State, http::StatusCode, response::Response};
use chrono_tz::Tz;
use zaehler_app::{AppError, FieldErrors, ids::HouseholdId, model::HouseholdData};

use crate::{
    extract::{Ctx, PageCtx, SESSION_HOUSEHOLD},
    pages::{FormData, field_errors, render_status, see_other},
    state::WebState,
};

#[derive(Template)]
#[template(path = "pages/settings.html")]
struct SettingsPage {
    ctx: PageCtx,
    name: String,
    timezone: String,
    new_name: String,
    new_timezone: String,
    /// Hinweis zum Haushaltsarchiv mit Größengrenze
    backup_hint: String,
    /// Offener Setup-Entwurf vorhanden (nur für Owner ermittelt)
    setup_has_draft: bool,
    errors: FieldErrors,
}

impl SettingsPage {
    fn errs(&self, field: &str) -> Vec<String> {
        field_errors(self.ctx.lang, &self.errors, field)
    }

    fn has_errs(&self, field: &str) -> bool {
        !self.errors.for_field(field).is_empty()
    }
}

async fn page(
    state: &WebState,
    ctx: &Ctx,
    errors: FieldErrors,
    form: Option<&FormData>,
    status: StatusCode,
) -> Response {
    let household = match state.uc.get_household.execute(&ctx.tenant).await {
        Ok(h) => h,
        Err(e) => return ctx.fail(e),
    };
    let setup_has_draft = if ctx.page.is_owner {
        match state.uc.get_setup_draft.execute(&ctx.tenant).await {
            Ok(d) => d.is_some(),
            Err(e) => return ctx.fail(e),
        }
    } else {
        false
    };
    let pick = |key: &str, default: String| {
        form.and_then(|f| f.has(key).then(|| f.get(key).to_owned()))
            .unwrap_or(default)
    };
    let page = SettingsPage {
        name: pick("name", household.data.name.clone()),
        timezone: pick("timezone", household.data.timezone.name().to_owned()),
        new_name: pick("new_name", String::new()),
        new_timezone: pick("new_timezone", household.data.timezone.name().to_owned()),
        backup_hint: ctx.page.lang.ta(
            "settings-import-hint",
            &[(
                "max",
                state.cfg.max_backup_bytes.div_ceil(1024 * 1024).to_string(),
            )],
        ),
        setup_has_draft,
        errors,
        ctx: ctx.page.clone(),
    };
    render_status(status, &page)
}

pub async fn show(State(state): State<WebState>, ctx: Ctx) -> Response {
    page(&state, &ctx, FieldErrors::default(), None, StatusCode::OK).await
}

fn parse_household(
    form: &FormData,
    name_key: &str,
    tz_key: &str,
) -> Result<HouseholdData, FieldErrors> {
    let mut e = FieldErrors::default();
    let timezone = form.get(tz_key).trim().parse::<Tz>().unwrap_or_else(|_| {
        e.add(tz_key, "form.timezone_invalid");
        chrono_tz::Europe::Berlin
    });
    if e.is_empty() {
        Ok(HouseholdData {
            name: form.get(name_key).to_owned(),
            timezone,
        })
    } else {
        Err(e)
    }
}

fn rename_fields(errors: FieldErrors, prefix: &str) -> FieldErrors {
    FieldErrors(
        errors
            .0
            .into_iter()
            .map(|mut e| {
                if !e.field.starts_with(prefix) {
                    e.field = format!("{prefix}{}", e.field);
                }
                e
            })
            .collect(),
    )
}

pub async fn update_household(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let form = FormData::new(pairs);
    let data = match parse_household(&form, "name", "timezone") {
        Ok(d) => d,
        Err(e) => {
            return page(
                &state,
                &ctx,
                e,
                Some(&form),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        }
    };
    match state.uc.update_household.execute(&ctx.tenant, data).await {
        Ok(()) => {
            ctx.flash("flash-household-updated").await;
            see_other("/settings")
        }
        Err(AppError::Validation(e)) => {
            page(
                &state,
                &ctx,
                e,
                Some(&form),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
        Err(e) => ctx.fail(e),
    }
}

pub async fn update_locale(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let form = FormData::new(pairs);
    match state
        .uc
        .set_locale
        .execute(ctx.tenant.user_id(), form.get("locale"))
        .await
    {
        Ok(()) => {
            ctx.flash("flash-locale-updated").await;
            see_other("/settings")
        }
        Err(e) => ctx.fail(e),
    }
}

pub async fn create_household(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let form = FormData::new(pairs);
    let data = match parse_household(&form, "new_name", "new_timezone") {
        Ok(d) => d,
        Err(e) => {
            return page(
                &state,
                &ctx,
                e,
                Some(&form),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        }
    };
    match state
        .uc
        .create_household
        .execute(ctx.tenant.user_id(), data)
        .await
    {
        Ok(id) => {
            if let Err(e) = ctx.session.insert(SESSION_HOUSEHOLD, id).await {
                return ctx.fail(AppError::Internal(e.to_string()));
            }
            ctx.flash("flash-household-created").await;
            see_other("/")
        }
        Err(AppError::Validation(e)) => {
            page(
                &state,
                &ctx,
                rename_fields(e, "new_"),
                Some(&form),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
        Err(e) => ctx.fail(e),
    }
}

/// Aktiven Haushalt wechseln; die Mitgliedschaft prüft der nächste Request.
pub async fn switch_household(
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let form = FormData::new(pairs);
    let Ok(id) = form.get("household").parse::<HouseholdId>() else {
        return ctx.fail(AppError::NotFound);
    };
    if !ctx.page.households.iter().any(|h| h.id == id) {
        return ctx.fail(AppError::NotFound);
    }
    if let Err(e) = ctx.session.insert(SESSION_HOUSEHOLD, id).await {
        return ctx.fail(AppError::Internal(e.to_string()));
    }
    see_other("/")
}

pub async fn delete_household(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let form = FormData::new(pairs);
    match state
        .uc
        .delete_household
        .execute(&ctx.tenant, form.get("confirm"))
        .await
    {
        Ok(()) => {
            if let Err(e) = ctx.session.remove::<HouseholdId>(SESSION_HOUSEHOLD).await {
                return ctx.fail(AppError::Internal(e.to_string()));
            }
            ctx.flash("flash-household-deleted").await;
            see_other("/")
        }
        Err(AppError::Validation(e)) => {
            page(
                &state,
                &ctx,
                e,
                Some(&form),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
        Err(e) => ctx.fail(e),
    }
}
