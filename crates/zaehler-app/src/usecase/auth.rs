//! Anmeldung, Auswahl des aktiven Haushalts und Nutzereinstellungen.

use crate::{
    AppError, Tenant,
    ids::{HouseholdId, UserId},
    model::{HouseholdData, HouseholdSummary, Identity, User},
    port::UnitOfWork,
    usecase::{Ports, use_case},
};

/// Unterstützte Sprachen der Oberfläche.
pub const SUPPORTED_LOCALES: [&str; 2] = ["de", "en"];

/// Ergebnis der Anmeldung.
#[derive(Debug, Clone)]
pub struct SignedIn {
    pub user: User,
    pub households: Vec<HouseholdSummary>,
    pub active: HouseholdId,
}

/// Ergebnis der Tenant-Auflösung je Request.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub tenant: Tenant,
    pub user: User,
    pub households: Vec<HouseholdSummary>,
}

use_case!(
    /// Nutzer anmelden; legt beim ersten Login einen Haushalt an.
    SignIn,
    /// Aktiven Haushalt prüfen und `Tenant` erzeugen.
    ResolveTenant,
    /// Sprache des Nutzers speichern.
    SetLocale,
);

/// Legt einen Haushalt mit dem Namen des Nutzers an.
// @impl SPEC_TEN_HOUSEHOLDS
async fn create_default_household(
    uow: &mut dyn UnitOfWork,
    p: &Ports,
    user: &User,
) -> Result<(), AppError> {
    let trimmed = user.display_name.trim();
    let name: String = if trimmed.is_empty() {
        "Haushalt".to_owned()
    } else {
        trimmed.chars().take(80).collect()
    };
    let data = HouseholdData {
        name,
        timezone: chrono_tz::Europe::Berlin,
    };
    uow.households()
        .create(HouseholdId(p.ids.new_uuid()), user.id, &data, p.clock.now())
        .await?;
    Ok(())
}

impl SignIn {
    // @impl SPEC_TEN_HOUSEHOLDS
    pub async fn execute(&self, identity: &Identity) -> Result<SignedIn, AppError> {
        if identity.issuer.is_empty() || identity.subject.is_empty() {
            return Err(AppError::Unauthenticated);
        }
        let mut uow = self.p.uow.begin().await?;
        let user = uow
            .users()
            .upsert_oidc(UserId(self.p.ids.new_uuid()), identity, self.p.clock.now())
            .await?;
        let mut households = uow.memberships().households_of(user.id).await?;
        if households.is_empty() {
            create_default_household(&mut *uow, &self.p, &user).await?;
            households = uow.memberships().households_of(user.id).await?;
        }
        uow.commit().await?;
        let active = households
            .first()
            .ok_or(AppError::Internal("kein Haushalt".into()))?
            .id;
        Ok(SignedIn {
            user,
            households,
            active,
        })
    }
}

impl ResolveTenant {
    /// Wählt `preferred`, falls der Nutzer Mitglied ist, sonst den ersten
    /// Haushalt; ohne Mitgliedschaft wird ein Haushalt angelegt.
    // @impl SPEC_SEC_TENANT_SCOPE, SPEC_TEN_HOUSEHOLDS
    pub async fn execute(
        &self,
        user_id: UserId,
        preferred: Option<HouseholdId>,
    ) -> Result<Resolved, AppError> {
        let mut uow = self.p.uow.begin().await?;
        let user = uow
            .users()
            .get(user_id)
            .await?
            .ok_or(AppError::Unauthenticated)?;
        let mut households = uow.memberships().households_of(user.id).await?;
        if households.is_empty() {
            create_default_household(&mut *uow, &self.p, &user).await?;
            households = uow.memberships().households_of(user.id).await?;
        }
        uow.commit().await?;
        let chosen = preferred
            .and_then(|id| households.iter().find(|h| h.id == id))
            .or_else(|| households.first())
            .ok_or(AppError::Internal("kein Haushalt".into()))?;
        let tenant = Tenant::new(user.id, chosen.id, chosen.role);
        Ok(Resolved {
            tenant,
            user,
            households,
        })
    }
}

impl SetLocale {
    pub async fn execute(&self, user: UserId, locale: &str) -> Result<(), AppError> {
        if !SUPPORTED_LOCALES.contains(&locale) {
            return Err(AppError::field("locale", "settings.locale_invalid"));
        }
        let mut uow = self.p.uow.begin().await?;
        uow.users().set_locale(user, locale).await?;
        uow.commit().await?;
        Ok(())
    }
}
