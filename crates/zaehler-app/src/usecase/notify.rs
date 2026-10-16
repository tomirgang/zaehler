//! Benachrichtigungseinstellungen des angemeldeten Nutzers (SPEC_NTF_EMAIL).

use crate::{
    AppError, FieldErrors, Role, Tenant,
    model::NotificationSettings,
    usecase::{member::email_valid, use_case},
};

/// Einstellungen mit der Adresse aus der Anmeldung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationView {
    pub settings: NotificationSettings,
    /// E-Mail aus dem OIDC-Login, falls vorhanden
    pub login_email: Option<String>,
    /// E-Mail-Versand ist konfiguriert
    pub enabled: bool,
}

use_case!(
    /// Eigene Benachrichtigungseinstellungen lesen.
    GetNotificationSettings,
    /// Eigene Benachrichtigungseinstellungen speichern.
    UpdateNotificationSettings,
);

impl GetNotificationSettings {
    // @impl SPEC_NTF_EMAIL
    #[tracing::instrument(name = "GetNotificationSettings", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<NotificationView, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        let user = uow
            .users()
            .get(t.user_id())
            .await?
            .ok_or(AppError::Unauthenticated)?;
        let settings = uow
            .notification_settings()
            .get(t.user_id())
            .await?
            .unwrap_or_default();
        Ok(NotificationView {
            settings,
            login_email: user.email,
            enabled: self.p.notifier.enabled(),
        })
    }
}

impl UpdateNotificationSettings {
    /// Gilt nur für den eigenen Nutzer; eine leere Adresse bedeutet die
    /// Adresse aus der Anmeldung.
    // @impl SPEC_NTF_EMAIL
    #[tracing::instrument(name = "UpdateNotificationSettings", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        settings: NotificationSettings,
    ) -> Result<(), AppError> {
        t.require(Role::Viewer)?;
        let email = settings
            .email
            .map(|e| e.trim().to_owned())
            .filter(|e| !e.is_empty());
        let mut errors = FieldErrors::default();
        if email.as_deref().is_some_and(|e| !email_valid(e)) {
            errors.add("email", "member.email_invalid");
        }
        errors.into_result()?;
        let settings = NotificationSettings { email, ..settings };
        let mut uow = self.p.uow.begin().await?;
        uow.notification_settings()
            .set(t.user_id(), &settings, self.p.clock.now())
            .await?;
        uow.commit().await?;
        Ok(())
    }
}
