//! Fehler der Anwendungsfälle und der Ports.

use thiserror::Error;

/// Fehler eines Driven-Port-Adapters.
#[derive(Debug, Error)]
pub enum PortError {
    #[error("nicht gefunden")]
    NotFound,
    /// Verletzung einer Eindeutigkeit (z.B. doppelte Ablesung).
    #[error("Konflikt: {0}")]
    Conflict(String),
    /// Eingabe vom Adapter abgelehnt (z.B. Bildformat); der Code ist ein
    /// Übersetzungsschlüssel.
    #[error("ungültige Eingabe: {0}")]
    Invalid(&'static str),
    #[error("Adapterfehler: {0}")]
    Backend(String),
}

/// Fehler zu einem Eingabefeld; `code` ist ein Übersetzungsschlüssel, den
/// der Driving-Adapter in Text umsetzt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: String,
    pub code: &'static str,
    pub args: Vec<(&'static str, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldErrors(pub Vec<FieldError>);

impl FieldErrors {
    pub fn add(&mut self, field: impl Into<String>, code: &'static str) {
        self.0.push(FieldError {
            field: field.into(),
            code,
            args: Vec::new(),
        });
    }

    pub fn add_with(
        &mut self,
        field: impl Into<String>,
        code: &'static str,
        args: Vec<(&'static str, String)>,
    ) {
        self.0.push(FieldError {
            field: field.into(),
            code,
            args,
        });
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn for_field(&self, field: &str) -> Vec<&FieldError> {
        self.0.iter().filter(|e| e.field == field).collect()
    }

    /// `Ok(())` ohne Fehler, sonst `AppError::Validation`.
    pub fn into_result(self) -> Result<(), AppError> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(AppError::Validation(self))
        }
    }
}

/// Fehler eines Anwendungsfalls.
// @impl SPEC_ERR_HANDLING
#[derive(Debug, Error)]
pub enum AppError {
    /// Nicht vorhanden oder gehört zu einem anderen Haushalt.
    #[error("nicht gefunden")]
    NotFound,
    #[error("keine Berechtigung")]
    Forbidden,
    #[error("nicht angemeldet")]
    Unauthenticated,
    #[error("ungültige Eingaben")]
    Validation(FieldErrors),
    /// Fachlicher Konflikt; der Code ist ein Übersetzungsschlüssel.
    #[error("Konflikt: {0}")]
    Conflict(&'static str),
    #[error("externer Dienst nicht verfügbar: {0}")]
    Upstream(&'static str),
    #[error("interner Fehler: {0}")]
    Internal(String),
}

impl From<PortError> for AppError {
    fn from(e: PortError) -> Self {
        match e {
            PortError::NotFound => Self::NotFound,
            PortError::Conflict(_) => Self::Conflict("conflict"),
            PortError::Invalid(code) => Self::field("file", code),
            PortError::Backend(msg) => Self::Internal(msg),
        }
    }
}

impl AppError {
    /// Fehler des Objektspeichers: `NotFound` bleibt erhalten, Adapterfehler
    /// werden zu `Upstream("storage")`.
    #[must_use]
    pub fn from_storage(e: PortError) -> Self {
        match e {
            PortError::Backend(msg) => {
                tracing::error!(error = %msg, "object storage failed");
                Self::Upstream("storage")
            }
            other => other.into(),
        }
    }

    /// Einzelner Feldfehler als `Validation`.
    #[must_use]
    pub fn field(field: impl Into<String>, code: &'static str) -> Self {
        let mut errors = FieldErrors::default();
        errors.add(field, code);
        Self::Validation(errors)
    }
}
