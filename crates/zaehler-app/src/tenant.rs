//! Berechtigungsnachweis für Zugriffe auf Daten eines Haushalts.

use crate::{
    AppError,
    ids::{HouseholdId, UserId},
};

/// Rolle im Haushalt; die Reihenfolge entspricht den Rechten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Member,
    Owner,
}

impl Role {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Member => "member",
            Self::Owner => "owner",
        }
    }
}

impl std::str::FromStr for Role {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "viewer" => Ok(Self::Viewer),
            "member" => Ok(Self::Member),
            "owner" => Ok(Self::Owner),
            other => Err(format!("unbekannte Rolle: {other}")),
        }
    }
}

/// Geprüfte Kombination aus Nutzer, Haushalt und Rolle.
///
/// Die Felder sind privat; ein `Tenant` entsteht nur in diesem Crate nach
/// Prüfung der Mitgliedschaft (`usecase::auth::ResolveTenant`).
// @impl SPEC_SEC_TENANT_SCOPE
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    user_id: UserId,
    household_id: HouseholdId,
    role: Role,
}

impl Tenant {
    pub(crate) fn new(user_id: UserId, household_id: HouseholdId, role: Role) -> Self {
        Self {
            user_id,
            household_id,
            role,
        }
    }

    /// Tenant eines Jobs für einen Haushalt: Rolle `Viewer`, ohne Nutzer
    /// (Nil-UUID). Nur innerhalb dieses Crates verfügbar; Jobs lesen damit
    /// fachliche Daten über die normalen Ports (SPEC_JOB_OUTBOX).
    // @impl SPEC_JOB_OUTBOX
    pub(crate) fn for_job(household_id: HouseholdId) -> Self {
        Self {
            user_id: UserId(uuid::Uuid::nil()),
            household_id,
            role: Role::Viewer,
        }
    }

    /// Nutzer des Tenants; beim Job-Tenant die Nil-UUID.
    #[must_use]
    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    #[must_use]
    pub fn household_id(&self) -> HouseholdId {
        self.household_id
    }

    #[must_use]
    pub fn role(&self) -> Role {
        self.role
    }

    /// Prüft, ob die Rolle mindestens `min` ist.
    pub fn require(&self, min: Role) -> Result<(), AppError> {
        if self.role >= min {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }
}
