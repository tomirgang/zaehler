//! Benachrichtigungen per E-Mail (ADR_011).

use async_trait::async_trait;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct Message {
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Error)]
pub enum NotifyError {
    #[error("Kanal nicht implementiert: {0}")]
    NotImplemented(&'static str),
}

/// Ein Benachrichtigungskanal; `recipient` ist kanal-spezifisch (E-Mail-Adresse).
/// Der Trait erlaubt Test-Doubles im Scheduler.
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn send(&self, recipient: &str, message: &Message) -> Result<(), NotifyError>;
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
}

const fn default_smtp_port() -> u16 {
    587
}

/// E-Mail via SMTP (STARTTLS). Umsetzung mit `lettre` folgt mit dem
/// Benachrichtigungs-Feature (REQ_NTF_002).
pub struct EmailNotifier {
    #[allow(dead_code)]
    cfg: SmtpConfig,
}

impl EmailNotifier {
    #[must_use]
    pub fn new(cfg: SmtpConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Notifier for EmailNotifier {
    async fn send(&self, _recipient: &str, _message: &Message) -> Result<(), NotifyError> {
        // TODO(REQ_NTF_002): lettre::AsyncSmtpTransport<Tokio1Executor> anbinden.
        Err(NotifyError::NotImplemented("email"))
    }
}
