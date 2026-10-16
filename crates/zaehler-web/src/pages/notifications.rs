//! Eigene Benachrichtigungseinstellungen (SPEC_NTF_EMAIL).

use askama::Template;
use axum::{extract::State, http::StatusCode, response::Response};
use zaehler_app::{AppError, FieldErrors, model::NotificationSettings};

use crate::{
    extract::{Ctx, PageCtx},
    pages::{FormData, field_errors, render_status, see_other},
    state::WebState,
};

#[derive(Template)]
#[template(path = "pages/settings/notifications.html")]
struct NotificationsPage {
    ctx: PageCtx,
    /// E-Mail-Versand konfiguriert
    enabled: bool,
    email: String,
    login_email: String,
    email_hint: String,
    reminder: bool,
    alarm: bool,
    errors: FieldErrors,
}

impl NotificationsPage {
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
    form: Option<(&NotificationSettings, FieldErrors)>,
) -> Response {
    let view = match state
        .uc
        .get_notification_settings
        .execute(&ctx.tenant)
        .await
    {
        Ok(v) => v,
        Err(e) => return ctx.fail(e),
    };
    let (settings, errors, status) = match form {
        Some((s, e)) => (s.clone(), e, StatusCode::UNPROCESSABLE_ENTITY),
        None => (view.settings, FieldErrors::default(), StatusCode::OK),
    };
    let lang = ctx.page.lang;
    let login_email = view.login_email.unwrap_or_default();
    let page = NotificationsPage {
        enabled: view.enabled,
        email: settings.email.unwrap_or_default(),
        email_hint: if login_email.is_empty() {
            lang.t("notifications-email-hint-none")
        } else {
            lang.ta(
                "notifications-email-hint",
                &[("login", login_email.clone())],
            )
        },
        login_email,
        reminder: settings.reminder,
        alarm: settings.alarm,
        errors,
        ctx: ctx.page.clone(),
    };
    render_status(status, &page)
}

// @impl SPEC_NTF_EMAIL
pub async fn show(State(state): State<WebState>, ctx: Ctx) -> Response {
    page(&state, &ctx, None).await
}

// @impl SPEC_NTF_EMAIL, SPEC_NTF_REMINDERS, SPEC_NTF_ALARMS
pub async fn update(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let form = FormData::new(pairs);
    let settings = NotificationSettings {
        email: form.opt("email"),
        reminder: form.has("reminder"),
        alarm: form.has("alarm"),
    };
    match state
        .uc
        .update_notification_settings
        .execute(&ctx.tenant, settings.clone())
        .await
    {
        Ok(()) => {
            ctx.flash("flash-notifications-saved").await;
            see_other("/settings/notifications")
        }
        Err(AppError::Validation(errors)) => page(&state, &ctx, Some((&settings, errors))).await,
        Err(e) => ctx.fail(e),
    }
}
