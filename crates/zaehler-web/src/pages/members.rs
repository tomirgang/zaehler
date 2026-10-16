//! Mitglieder, Rollen und Einladungen (SPEC_TEN_ROLES, SPEC_TEN_INVITES).

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};
use tower_sessions::Session;
use zaehler_app::{
    AppError, FieldErrors, Role,
    ids::{InvitationId, UserId},
    usecase::member::InvitationInput,
};

use crate::{
    extract::{Ctx, PageCtx, SESSION_HOUSEHOLD, SESSION_INVITE, SESSION_USER},
    i18n::{Lang, format},
    pages::{FormData, field_errors, render, render_status, see_other},
    state::WebState,
};

pub struct MemberRow {
    pub user_id: String,
    pub name: String,
    pub role: &'static str,
    pub role_label: String,
    pub is_self: bool,
}

pub struct InvitationRow {
    pub id: String,
    pub role_label: String,
    pub email: String,
    pub expires: String,
}

#[derive(Template)]
#[template(path = "pages/settings/members.html")]
struct MembersPage {
    ctx: PageCtx,
    members: Vec<MemberRow>,
    invitations: Vec<InvitationRow>,
    roles: Vec<(&'static str, String)>,
    /// Link einer soeben erzeugten Einladung; erscheint nur einmal
    created_link: Option<String>,
    /// Die soeben erzeugte Einladung wird zusätzlich per E-Mail versendet
    created_emailed: bool,
    invite_role: String,
    invite_email: String,
    errors: FieldErrors,
}

impl MembersPage {
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

const ROLES: [Role; 3] = [Role::Viewer, Role::Member, Role::Owner];

fn role_label(lang: Lang, role: Role) -> String {
    lang.t(&format!("role-{}", role.as_str()))
}

fn parse_role(value: &str) -> Result<Role, AppError> {
    value
        .parse::<Role>()
        .map_err(|_| AppError::field("role", "member.role_invalid"))
}

struct PageInput {
    created_link: Option<String>,
    created_emailed: bool,
    invite_role: String,
    invite_email: String,
    errors: FieldErrors,
    status: StatusCode,
}

impl Default for PageInput {
    fn default() -> Self {
        Self {
            created_link: None,
            created_emailed: false,
            invite_role: Role::Member.as_str().into(),
            invite_email: String::new(),
            errors: FieldErrors::default(),
            status: StatusCode::OK,
        }
    }
}

async fn page(state: &WebState, ctx: &Ctx, input: PageInput) -> Response {
    let lang = ctx.page.lang;
    let members = match state.uc.list_members.execute(&ctx.tenant).await {
        Ok(m) => m,
        Err(e) => return ctx.fail(e),
    };
    let invitations = if ctx.page.is_owner {
        match state.uc.list_invitations.execute(&ctx.tenant).await {
            Ok(i) => i,
            Err(e) => return ctx.fail(e),
        }
    } else {
        Vec::new()
    };
    let me = ctx.tenant.user_id();
    let page = MembersPage {
        members: members
            .into_iter()
            .map(|m| MemberRow {
                user_id: m.user_id.to_string(),
                name: m.display_name,
                role: m.role.as_str(),
                role_label: role_label(lang, m.role),
                is_self: m.user_id == me,
            })
            .collect(),
        invitations: invitations
            .into_iter()
            .map(|i| InvitationRow {
                id: i.id.to_string(),
                role_label: role_label(lang, i.role),
                email: i.email.unwrap_or_default(),
                expires: format::datetime_in(lang, i.expires_at, ctx.page.tz),
            })
            .collect(),
        roles: ROLES
            .iter()
            .map(|r| (r.as_str(), role_label(lang, *r)))
            .collect(),
        created_link: input.created_link,
        created_emailed: input.created_emailed,
        invite_role: input.invite_role,
        invite_email: input.invite_email,
        errors: input.errors,
        ctx: ctx.page.clone(),
    };
    render_status(input.status, &page)
}

pub async fn show(State(state): State<WebState>, ctx: Ctx) -> Response {
    page(&state, &ctx, PageInput::default()).await
}

fn parse_user(id: &str) -> Result<UserId, AppError> {
    id.parse().map_err(|_| AppError::NotFound)
}

// @impl SPEC_TEN_ROLES
pub async fn change_role(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(user): Path<String>,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let form = FormData::new(pairs);
    let result = match (parse_user(&user), parse_role(form.get("role"))) {
        (Ok(u), Ok(role)) => state.uc.change_role.execute(&ctx.tenant, u, role).await,
        (Err(e), _) | (_, Err(e)) => Err(e),
    };
    match result {
        Ok(()) => {
            ctx.flash("flash-role-changed").await;
            see_other("/settings/members")
        }
        Err(e) => ctx.fail(e),
    }
}

// @impl SPEC_TEN_ROLES
pub async fn remove(State(state): State<WebState>, ctx: Ctx, Path(user): Path<String>) -> Response {
    let result = match parse_user(&user) {
        Ok(u) => state.uc.remove_member.execute(&ctx.tenant, u).await,
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => {
            ctx.flash("flash-member-removed").await;
            see_other("/settings/members")
        }
        Err(e) => ctx.fail(e),
    }
}

/// Aktiven Haushalt verlassen; danach gilt der nächste Haushalt des Nutzers.
// @impl SPEC_TEN_ROLES
pub async fn leave(State(state): State<WebState>, ctx: Ctx) -> Response {
    match state.uc.leave_household.execute(&ctx.tenant).await {
        Ok(()) => {
            if let Err(e) = ctx
                .session
                .remove::<zaehler_app::ids::HouseholdId>(SESSION_HOUSEHOLD)
                .await
            {
                return ctx.fail(AppError::Internal(e.to_string()));
            }
            ctx.flash("flash-household-left").await;
            see_other("/")
        }
        Err(e) => ctx.fail(e),
    }
}

/// Erzeugt eine Einladung und zeigt den Link genau einmal an.
// @impl SPEC_TEN_INVITES
pub async fn create_invitation(
    State(state): State<WebState>,
    ctx: Ctx,
    axum::Form(pairs): axum::Form<Vec<(String, String)>>,
) -> Response {
    let form = FormData::new(pairs);
    let mut input = PageInput {
        invite_role: form.get("role").to_owned(),
        invite_email: form.get("email").to_owned(),
        ..PageInput::default()
    };
    let role = match parse_role(form.get("role")) {
        Ok(r) => r,
        Err(AppError::Validation(errors)) => {
            input.errors = errors;
            input.status = StatusCode::UNPROCESSABLE_ENTITY;
            return page(&state, &ctx, input).await;
        }
        Err(e) => return ctx.fail(e),
    };
    match state
        .uc
        .create_invitation
        .execute(
            &ctx.tenant,
            InvitationInput {
                role,
                email: form.opt("email"),
            },
        )
        .await
    {
        Ok(created) => {
            let link = format!("{}/invite/{}", state.cfg.public_url, created.token);
            page(
                &state,
                &ctx,
                PageInput {
                    created_link: Some(link),
                    created_emailed: created.emailed,
                    ..PageInput::default()
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

pub async fn revoke_invitation(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
) -> Response {
    let result = match id.parse::<InvitationId>() {
        Ok(id) => state.uc.revoke_invitation.execute(&ctx.tenant, id).await,
        Err(_) => Err(AppError::NotFound),
    };
    match result {
        Ok(()) => {
            ctx.flash("flash-invitation-revoked").await;
            see_other("/settings/members")
        }
        Err(e) => ctx.fail(e),
    }
}

#[derive(Template)]
#[template(path = "pages/invite.html")]
struct InvitePage {
    ctx: PageCtx,
    token: String,
    text: String,
}

#[derive(Template)]
#[template(path = "pages/invite_invalid.html")]
struct InviteInvalidPage {
    ctx: PageCtx,
}

fn invalid(ctx: &Ctx) -> Response {
    render_status(
        StatusCode::NOT_FOUND,
        &InviteInvalidPage {
            ctx: ctx.page.clone(),
        },
    )
}

/// Einladung anzeigen. Ohne Anmeldung wird das Token in der Sitzung
/// gespeichert und nach der Anmeldung hierher zurückgeleitet.
// @impl SPEC_TEN_INVITES
pub async fn show_invitation(
    State(state): State<WebState>,
    session: Session,
    Path(token): Path<String>,
    ctx: Result<Ctx, Response>,
) -> Response {
    let ctx = match ctx {
        Ok(c) => c,
        Err(login) => {
            let anonymous = matches!(session.get::<UserId>(SESSION_USER).await, Ok(None));
            if anonymous && let Err(e) = session.insert(SESSION_INVITE, &token).await {
                tracing::warn!(error = %e, "invitation not stored in session");
            }
            return login;
        }
    };
    match state
        .uc
        .preview_invitation
        .execute(ctx.tenant.user_id(), &token)
        .await
    {
        Ok(inv) => render(&InvitePage {
            token,
            text: ctx.page.lang.ta(
                "invite-accept-text",
                &[
                    ("household", inv.household_name),
                    ("role", role_label(ctx.page.lang, inv.role)),
                ],
            ),
            ctx: ctx.page,
        }),
        Err(AppError::Conflict(_)) => invalid(&ctx),
        Err(e) => ctx.fail(e),
    }
}

/// Einladung annehmen; der Haushalt wird aktiv.
// @impl SPEC_TEN_INVITES
pub async fn accept_invitation(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(token): Path<String>,
) -> Response {
    match state
        .uc
        .accept_invitation
        .execute(ctx.tenant.user_id(), &token)
        .await
    {
        Ok(household) => {
            if let Err(e) = ctx.session.insert(SESSION_HOUSEHOLD, household).await {
                return ctx.fail(AppError::Internal(e.to_string()));
            }
            ctx.flash("flash-invitation-accepted").await;
            see_other("/")
        }
        Err(AppError::Conflict(_)) => invalid(&ctx),
        Err(e) => ctx.fail(e),
    }
}
