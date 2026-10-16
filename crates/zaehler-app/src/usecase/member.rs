//! Mitglieder, Rollen und Einladungen (SPEC_TEN_ROLES, SPEC_TEN_INVITES).

use crate::{
    AppError, FieldErrors, Role, Tenant,
    ids::{HouseholdId, InvitationId, OutboxId, UserId},
    model::{Invitation, Member, NewInvitation, OutboxEvent},
    port::UnitOfWork,
    secret,
    usecase::use_case,
};

/// Längste zulässige E-Mail-Adresse (RFC 5321).
pub const MAX_EMAIL: usize = 254;

use_case!(
    /// Mitglieder des aktiven Haushalts.
    ListMembers,
    /// Rolle eines Mitglieds ändern.
    ChangeRole,
    /// Mitglied entfernen.
    RemoveMember,
    /// Aktiven Haushalt verlassen.
    LeaveHousehold,
    /// Offene Einladungen des Haushalts.
    ListInvitations,
    /// Einladung erzeugen.
    CreateInvitation,
    /// Einladung widerrufen.
    RevokeInvitation,
    /// Einladung vor dem Annehmen anzeigen.
    PreviewInvitation,
    /// Einladung einlösen.
    AcceptInvitation,
);

/// Prüft, dass nach der Änderung mindestens ein Owner bleibt.
// @impl SPEC_TEN_ROLES
fn keeps_an_owner(
    members: &[Member],
    user: UserId,
    new_role: Option<Role>,
) -> Result<(), AppError> {
    let owners_after = members
        .iter()
        .filter(|m| {
            let role = if m.user_id == user {
                new_role
            } else {
                Some(m.role)
            };
            role == Some(Role::Owner)
        })
        .count();
    if owners_after == 0 {
        Err(AppError::Conflict("member.last_owner"))
    } else {
        Ok(())
    }
}

/// Liest die Mitglieder unter Sperre und prüft, dass `user` Mitglied ist.
async fn locked_members(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    user: UserId,
) -> Result<Vec<Member>, AppError> {
    uow.memberships().lock_household(t).await?;
    let members = uow.memberships().list(t).await?;
    if members.iter().any(|m| m.user_id == user) {
        Ok(members)
    } else {
        Err(AppError::NotFound)
    }
}

impl ListMembers {
    #[tracing::instrument(name = "ListMembers", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<Vec<Member>, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        Ok(uow.memberships().list(t).await?)
    }
}

impl ChangeRole {
    /// Der letzte Owner kann nicht herabgestuft werden.
    // @impl SPEC_TEN_ROLES
    #[tracing::instrument(name = "ChangeRole", skip_all)]
    pub async fn execute(&self, t: &Tenant, user: UserId, role: Role) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let members = locked_members(uow.as_mut(), t, user).await?;
        keeps_an_owner(&members, user, Some(role))?;
        uow.memberships().set_role(t, user, role).await?;
        uow.commit().await?;
        tracing::info!(household = %t.household_id(), %user, role = role.as_str(), "role changed");
        Ok(())
    }
}

impl RemoveMember {
    /// Der letzte Owner kann nicht entfernt werden.
    // @impl SPEC_TEN_ROLES
    #[tracing::instrument(name = "RemoveMember", skip_all)]
    pub async fn execute(&self, t: &Tenant, user: UserId) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let members = locked_members(uow.as_mut(), t, user).await?;
        keeps_an_owner(&members, user, None)?;
        uow.memberships().remove(t, user).await?;
        uow.commit().await?;
        tracing::info!(household = %t.household_id(), %user, "member removed");
        Ok(())
    }
}

impl LeaveHousehold {
    /// Jede Rolle darf austreten, außer der letzte Owner.
    // @impl SPEC_TEN_ROLES
    #[tracing::instrument(name = "LeaveHousehold", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<(), AppError> {
        t.require(Role::Viewer)?;
        let user = t.user_id();
        let mut uow = self.p.uow.begin().await?;
        let members = locked_members(uow.as_mut(), t, user).await?;
        keeps_an_owner(&members, user, None)?;
        uow.memberships().remove(t, user).await?;
        uow.commit().await?;
        tracing::info!(household = %t.household_id(), %user, "member left");
        Ok(())
    }
}

impl ListInvitations {
    #[tracing::instrument(name = "ListInvitations", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<Vec<Invitation>, AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        Ok(uow.invitations().list_open(t, self.p.clock.now()).await?)
    }
}

/// Eingaben einer Einladung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitationInput {
    pub role: Role,
    /// Optional; mit SMTP wird die Einladung dorthin versendet.
    pub email: Option<String>,
}

/// Erzeugte Einladung; `token` erscheint nur in dieser Antwort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedInvitation {
    pub id: InvitationId,
    pub token: String,
    /// Die Einladung wird per E-Mail versendet (Adresse und SMTP vorhanden)
    pub emailed: bool,
}

/// Einfache Formprüfung; die Zustellbarkeit prüft erst der Versand.
#[must_use]
pub fn email_valid(email: &str) -> bool {
    let len = email.chars().count();
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    (3..=MAX_EMAIL).contains(&len)
        && !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
        && !domain.contains('@')
}

impl CreateInvitation {
    // @impl SPEC_TEN_INVITES
    #[tracing::instrument(name = "CreateInvitation", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        input: InvitationInput,
    ) -> Result<CreatedInvitation, AppError> {
        t.require(Role::Owner)?;
        let email = input
            .email
            .map(|e| e.trim().to_owned())
            .filter(|e| !e.is_empty());
        let mut errors = FieldErrors::default();
        if email.as_deref().is_some_and(|e| !email_valid(e)) {
            errors.add("email", "member.email_invalid");
        }
        errors.into_result()?;
        let now = self.p.clock.now();
        let secret = secret::generate();
        let id = InvitationId(self.p.ids.new_uuid());
        let invitation = NewInvitation {
            id,
            role: input.role,
            token_hash: secret.hash,
            email,
            created_by: t.user_id(),
            created_at: now,
            expires_at: now + self.p.settings.invitation_ttl,
        };
        let emailed = invitation.email.is_some() && self.p.notifier.enabled();
        let mut uow = self.p.uow.begin().await?;
        uow.invitations().create(t, &invitation).await?;
        if emailed {
            // Versand über die Outbox, nie im Request (SPEC_NTF_EMAIL)
            let event = OutboxEvent::InvitationCreated {
                invitation_id: id,
                token: secret.token.clone(),
            };
            uow.outbox()
                .enqueue(t, OutboxId(self.p.ids.new_uuid()), &event, now)
                .await?;
        }
        uow.commit().await?;
        tracing::info!(household = %t.household_id(), invitation = %id, role = input.role.as_str(), "invitation created");
        Ok(CreatedInvitation {
            id,
            token: secret.token,
            emailed,
        })
    }
}

impl RevokeInvitation {
    #[tracing::instrument(name = "RevokeInvitation", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: InvitationId) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        uow.invitations().revoke(t, id, self.p.clock.now()).await?;
        uow.commit().await?;
        Ok(())
    }
}

/// Fehler aller ungültigen Einladungen; der Grund wird nicht genannt.
const INVALID: AppError = AppError::Conflict("invite.invalid");

/// Offene Einladung zum Token.
async fn open_invitation(
    uow: &mut dyn UnitOfWork,
    token: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Invitation, AppError> {
    // Tokens haben eine feste Länge; alles andere wird nicht nachgeschlagen
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(INVALID);
    }
    match uow.invitations().find_by_hash(&secret::hash(token)).await? {
        Some(i) if i.is_open(now) => Ok(i),
        _ => Err(INVALID),
    }
}

impl PreviewInvitation {
    /// Haushalt und Rolle einer offenen Einladung.
    #[tracing::instrument(name = "PreviewInvitation", skip_all)]
    pub async fn execute(&self, user: UserId, token: &str) -> Result<Invitation, AppError> {
        let mut uow = self.p.uow.begin().await?;
        uow.users()
            .get(user)
            .await?
            .ok_or(AppError::Unauthenticated)?;
        open_invitation(uow.as_mut(), token, self.p.clock.now()).await
    }
}

impl AcceptInvitation {
    /// Löst die Einladung ein; gleichzeitiges Einlösen gelingt nur einmal.
    /// Ein bestehendes Mitglied behält die höhere Rolle.
    // @impl SPEC_TEN_INVITES
    #[tracing::instrument(name = "AcceptInvitation", skip_all)]
    pub async fn execute(&self, user: UserId, token: &str) -> Result<HouseholdId, AppError> {
        let now = self.p.clock.now();
        let mut uow = self.p.uow.begin().await?;
        uow.users()
            .get(user)
            .await?
            .ok_or(AppError::Unauthenticated)?;
        let invitation = open_invitation(uow.as_mut(), token, now).await?;
        if !uow
            .invitations()
            .mark_accepted(invitation.id, user, now)
            .await?
        {
            return Err(INVALID);
        }
        uow.memberships()
            .add(invitation.household_id, user, invitation.role)
            .await?;
        uow.commit().await?;
        tracing::info!(household = %invitation.household_id, %user, invitation = %invitation.id, "invitation accepted");
        Ok(invitation.household_id)
    }
}
