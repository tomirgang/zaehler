//! Benachrichtigungen per E-Mail über SMTP (ADR_011, SPEC_NTF_EMAIL).
//!
//! `SmtpNotifier` implementiert den Port `Notifier` des Kerns: Texte aus
//! `locales/{de,en}/notify.ftl` in der Sprache des Empfängers, Versand mit
//! `lettre`. Empfängeradressen erscheinen weder in Logs noch in Fehlern.

use std::{borrow::Cow, collections::HashMap, fmt, time::Duration};

use async_trait::async_trait;
use fluent_templates::{
    LanguageIdentifier, Loader, fluent_bundle::FluentValue, langid, static_loader,
};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use zaehler_app::{
    PortError, Role,
    model::Notification,
    port::{Notifier, PortResult},
};

static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "de",
        customise: |bundle| bundle.set_use_isolating(false),
    };
}

static DE: LanguageIdentifier = langid!("de");
static EN: LanguageIdentifier = langid!("en");

/// Zeitlimit je SMTP-Verbindung.
pub const SMTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Transportverschlüsselung (`notify.smtp.security`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// STARTTLS, verpflichtend (Port 587)
    #[default]
    Starttls,
    /// TLS ab Verbindungsaufbau (Port 465)
    Tls,
    /// unverschlüsselt; nur für einen Mail-Server auf `localhost`
    None,
}

/// SMTP-Konfiguration (`notify.smtp.*`).
#[derive(Clone, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    #[serde(default)]
    pub security: Security,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
}

impl fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("from", &self.from)
            .finish()
    }
}

const fn default_smtp_port() -> u16 {
    587
}

impl SmtpConfig {
    /// Prüft Host, Absender und die Verschlüsselung.
    pub fn validate(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("notify.smtp.host darf nicht leer sein".into());
        }
        if self.from.parse::<Mailbox>().is_err() {
            return Err("notify.smtp.from ist keine gültige Adresse".into());
        }
        if self.security == Security::None
            && !matches!(self.host.as_str(), "localhost" | "127.0.0.1" | "::1")
        {
            return Err("notify.smtp.security = none ist nur mit host = localhost zulässig".into());
        }
        if self.username.is_some() != self.password.is_some() {
            return Err("notify.smtp.username und notify.smtp.password nur gemeinsam".into());
        }
        Ok(())
    }
}

/// E-Mail-Versand über einen `lettre`-Transport; in Tests ein Stub.
pub struct SmtpNotifier<T = AsyncSmtpTransport<Tokio1Executor>> {
    transport: T,
    from: Mailbox,
}

impl SmtpNotifier {
    /// Baut den SMTP-Transport; die Verbindung entsteht erst beim Versand.
    // @impl SPEC_NTF_EMAIL
    pub fn new(cfg: &SmtpConfig) -> Result<Self, String> {
        cfg.validate()?;
        let builder = match cfg.security {
            Security::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host),
            Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host),
            Security::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
                &cfg.host,
            )),
        }
        .map_err(|e| format!("SMTP-Konfiguration: {e}"))?;
        let mut builder = builder.port(cfg.port).timeout(Some(SMTP_TIMEOUT));
        if let (Some(user), Some(password)) = (&cfg.username, &cfg.password) {
            builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
        }
        Ok(Self {
            transport: builder.build(),
            from: cfg
                .from
                .parse()
                .map_err(|_| "notify.smtp.from".to_owned())?,
        })
    }
}

impl<T> SmtpNotifier<T> {
    /// Mit eigenem Transport (Tests).
    pub fn with_transport(transport: T, from: Mailbox) -> Self {
        Self { transport, from }
    }
}

#[async_trait]
impl<T> Notifier for SmtpNotifier<T>
where
    T: AsyncTransport + Send + Sync,
    T::Error: SmtpFailure + Send,
{
    fn enabled(&self) -> bool {
        true
    }

    // @impl SPEC_NTF_EMAIL
    async fn send(&self, to: &str, notification: &Notification, locale: &str) -> PortResult<()> {
        let to: Mailbox = to
            .parse()
            .map_err(|_| PortError::Backend("ungültige Empfängeradresse".into()))?;
        let (subject, body) = render(notification, locale);
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)
            .map_err(|e| PortError::Backend(format!("E-Mail nicht erzeugbar: {e}")))?;
        self.transport
            .send(message)
            .await
            .map(|_| ())
            .map_err(|e| PortError::Backend(format!("SMTP: {}", e.category())))
    }
}

/// Fehlerbeschreibung ohne Antworttext des Servers, der Adressen enthalten
/// kann.
pub trait SmtpFailure {
    fn category(&self) -> String;
}

impl SmtpFailure for lettre::transport::smtp::Error {
    fn category(&self) -> String {
        let kind = if self.is_permanent() {
            "permanent"
        } else if self.is_transient() {
            "transient"
        } else if self.is_timeout() {
            "timeout"
        } else if self.is_tls() {
            "tls"
        } else {
            "connection"
        };
        match self.status() {
            Some(code) => format!("{kind} ({code})"),
            None => kind.to_owned(),
        }
    }
}

impl SmtpFailure for lettre::transport::stub::Error {
    fn category(&self) -> String {
        "stub".into()
    }
}

struct Texts {
    lang: &'static LanguageIdentifier,
    de: bool,
}

impl Texts {
    fn new(locale: &str) -> Self {
        if locale.eq_ignore_ascii_case("en") {
            Self {
                lang: &EN,
                de: false,
            }
        } else {
            Self {
                lang: &DE,
                de: true,
            }
        }
    }

    fn t(&self, key: &str, args: &[(&str, String)]) -> String {
        let map: HashMap<Cow<'static, str>, FluentValue> = args
            .iter()
            .map(|(k, v)| (Cow::Owned((*k).to_owned()), FluentValue::from(v.clone())))
            .collect();
        LOCALES.lookup_with_args(self.lang, key, &map)
    }

    fn date(&self, d: chrono::NaiveDate) -> String {
        if self.de {
            d.format("%d.%m.%Y").to_string()
        } else {
            d.format("%Y-%m-%d").to_string()
        }
    }

    fn datetime(&self, d: chrono::NaiveDateTime) -> String {
        if self.de {
            d.format("%d.%m.%Y %H:%M").to_string()
        } else {
            d.format("%Y-%m-%d %H:%M").to_string()
        }
    }

    fn number(&self, d: Decimal) -> String {
        let s = d.round_dp(1).normalize().to_string();
        if self.de { s.replace('.', ",") } else { s }
    }

    fn role(&self, role: Role) -> String {
        let key = match role {
            Role::Owner => "role-owner",
            Role::Member => "role-member",
            Role::Viewer => "role-viewer",
        };
        self.t(key, &[])
    }
}

/// Betreff und Text einer Benachrichtigung in der Sprache `locale` (`de`
/// oder `en`, sonst Deutsch).
// @impl SPEC_NTF_EMAIL, SPEC_NTF_REMINDERS, SPEC_NTF_ALARMS
#[must_use]
#[allow(clippy::too_many_lines)] // ein Abschnitt je Benachrichtigung
pub fn render(notification: &Notification, locale: &str) -> (String, String) {
    let x = Texts::new(locale);
    let mut lines = Vec::new();
    let settings = |link: &str| x.t("footer-settings", &[("link", link.to_owned())]);
    let subject = match notification {
        Notification::Reminder {
            household,
            meters,
            settings_link,
        } => {
            let h = [("household", household.clone())];
            lines.push(x.t("reminder-intro", &h));
            lines.push(String::new());
            for m in meters {
                let key = if m.overdue {
                    "reminder-overdue"
                } else {
                    "reminder-due"
                };
                lines.push(x.t(
                    key,
                    &[("meter", m.name.clone()), ("date", x.date(m.due_on))],
                ));
                lines.push(x.t("reminder-capture", &[("link", m.link.clone())]));
                lines.push(String::new());
            }
            lines.push(settings(settings_link));
            x.t("reminder-subject", &h)
        }
        Notification::ConsumptionAlarm {
            household,
            meter,
            unit,
            from,
            until,
            consumption,
            expected_lower,
            expected_upper,
            link,
            settings_link,
        } => {
            let unit = ("unit", unit.clone());
            lines.push(x.t(
                "alarm-intro",
                &[("household", household.clone()), ("meter", meter.clone())],
            ));
            lines.push(String::new());
            lines.push(x.t(
                "alarm-period",
                &[("from", x.datetime(*from)), ("until", x.datetime(*until))],
            ));
            lines.push(x.t(
                "alarm-consumption",
                &[("consumption", x.number(*consumption)), unit.clone()],
            ));
            lines.push(x.t(
                "alarm-expected",
                &[
                    ("lower", x.number(*expected_lower)),
                    ("upper", x.number(*expected_upper)),
                    unit,
                ],
            ));
            lines.push(String::new());
            lines.push(x.t("alarm-details", &[("link", link.clone())]));
            lines.push(settings(settings_link));
            x.t("alarm-subject", &[("meter", meter.clone())])
        }
        Notification::PvYieldAlarm {
            household,
            system,
            meter,
            from,
            until,
            generation,
            expected,
            threshold_pct,
            link,
            settings_link,
        } => {
            lines.push(x.t(
                "pv-alarm-intro",
                &[
                    ("household", household.clone()),
                    ("system", system.clone()),
                    ("meter", meter.clone()),
                ],
            ));
            lines.push(String::new());
            lines.push(x.t(
                "alarm-period",
                &[("from", x.datetime(*from)), ("until", x.datetime(*until))],
            ));
            lines.push(x.t(
                "pv-alarm-generation",
                &[
                    ("generation", x.number(*generation)),
                    ("expected", x.number(*expected)),
                    ("threshold", threshold_pct.to_string()),
                ],
            ));
            lines.push(String::new());
            lines.push(x.t("alarm-details", &[("link", link.clone())]));
            lines.push(settings(settings_link));
            x.t("pv-alarm-subject", &[("system", system.clone())])
        }
        Notification::Invitation {
            household,
            inviter,
            role,
            link,
            expires_on,
        } => {
            lines.push(x.t(
                "invite-intro",
                &[
                    ("inviter", inviter.clone()),
                    ("household", household.clone()),
                    ("role", x.role(*role)),
                ],
            ));
            lines.push(String::new());
            lines.push(x.t("invite-accept", &[("link", link.clone())]));
            lines.push(x.t("invite-expires", &[("date", x.date(*expires_on))]));
            x.t("invite-subject", &[("household", household.clone())])
        }
    };
    lines.push(String::new());
    lines.push(x.t("footer-sender", &[]));
    (subject, lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use lettre::transport::stub::AsyncStubTransport;
    use rust_decimal::Decimal;
    use zaehler_app::model::DueMeter;

    use super::*;

    fn keys(text: &str) -> Vec<String> {
        let mut keys: Vec<String> = text
            .lines()
            .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
            .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_owned()))
            .collect();
        keys.sort();
        keys
    }

    // @test SPEC_I18N, SPEC_NTF_EMAIL
    #[test]
    fn german_and_english_have_the_same_keys() {
        assert_eq!(
            keys(include_str!("../locales/de/notify.ftl")),
            keys(include_str!("../locales/en/notify.ftl"))
        );
    }

    fn reminder() -> Notification {
        Notification::Reminder {
            household: "Haus".into(),
            meters: vec![DueMeter {
                name: "Strom".into(),
                due_on: NaiveDate::from_ymd_opt(2026, 1, 14).unwrap(),
                overdue: true,
                link: "https://z.test/meters/1/readings/new".into(),
            }],
            settings_link: "https://z.test/settings/notifications".into(),
        }
    }

    // @test SPEC_NTF_REMINDERS
    #[test]
    fn reminder_in_both_languages() {
        let (subject, body) = render(&reminder(), "de");
        assert_eq!(subject, "Ablesung fällig: Haus");
        assert!(body.contains("Strom: überfällig seit 14.01.2026"), "{body}");
        assert!(body.contains("https://z.test/meters/1/readings/new"));
        assert!(body.contains("https://z.test/settings/notifications"));
        let (subject, body) = render(&reminder(), "en");
        assert_eq!(subject, "Reading due: Haus");
        assert!(body.contains("Strom: overdue since 2026-01-14"), "{body}");
    }

    // @test SPEC_NTF_ALARMS
    #[test]
    fn alarm_with_period_and_band() {
        let at = |d| {
            NaiveDate::from_ymd_opt(2026, 1, d)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap()
        };
        let n = Notification::ConsumptionAlarm {
            household: "Haus".into(),
            meter: "Gas".into(),
            unit: "m³".into(),
            from: at(10),
            until: at(12),
            consumption: Decimal::new(605, 1),
            expected_lower: Decimal::new(16, 0),
            expected_upper: Decimal::new(24, 0),
            link: "https://z.test/meters/1".into(),
            settings_link: "https://z.test/settings/notifications".into(),
        };
        let (subject, body) = render(&n, "de");
        assert_eq!(subject, "Auffälliger Verbrauch: Gas");
        assert!(body.contains("Verbrauch: 60,5 m³"), "{body}");
        assert!(body.contains("Erwartet: 16 bis 24 m³"), "{body}");
        assert!(
            body.contains("10.01.2026 12:00 bis 12.01.2026 12:00"),
            "{body}"
        );
    }

    // @test SPEC_TEN_INVITES
    #[test]
    fn invitation_text() {
        let n = Notification::Invitation {
            household: "Haus".into(),
            inviter: "Anna".into(),
            role: Role::Member,
            link: "https://z.test/invite/abc".into(),
            expires_on: NaiveDate::from_ymd_opt(2026, 1, 22).unwrap(),
        };
        let (subject, body) = render(&n, "en");
        assert_eq!(subject, "Invitation to household \"Haus\"");
        assert!(body.contains("Anna invites you"), "{body}");
        assert!(body.contains("(role: Member)"), "{body}");
        assert!(body.contains("https://z.test/invite/abc"));
        assert!(body.contains("2026-01-22"));
    }

    // @test SPEC_NTF_EMAIL
    #[tokio::test]
    async fn sends_plain_text_mail_and_hides_addresses_in_errors() {
        let from: Mailbox = "Zähler <zaehler@z.test>".parse().unwrap();
        let ok = SmtpNotifier::with_transport(AsyncStubTransport::new_ok(), from.clone());
        ok.send("anna@example.test", &reminder(), "de")
            .await
            .unwrap();
        // Stub-Transport, der den Inhalt behält
        let stub = AsyncStubTransport::new_ok();
        let notifier = SmtpNotifier::with_transport(stub.clone(), from.clone());
        notifier
            .send("anna@example.test", &reminder(), "de")
            .await
            .unwrap();
        let messages = stub.messages().await;
        assert_eq!(messages.len(), 1);
        let (envelope, raw) = &messages[0];
        assert_eq!(envelope.to()[0].to_string(), "anna@example.test");
        assert!(
            raw.contains("Content-Type: text/plain; charset=utf-8"),
            "{raw}"
        );

        let failing = SmtpNotifier::with_transport(AsyncStubTransport::new_error(), from);
        let err = failing
            .send("anna@example.test", &reminder(), "de")
            .await
            .unwrap_err()
            .to_string();
        assert!(!err.contains("anna@example.test"), "{err}");
        let err = notifier
            .send("kein-at", &reminder(), "de")
            .await
            .unwrap_err()
            .to_string();
        assert!(!err.contains("kein-at"), "{err}");
    }

    // @test SPEC_NTF_EMAIL
    #[tokio::test]
    async fn config_validation() {
        let mut cfg = SmtpConfig {
            host: "mail.example.test".into(),
            port: 587,
            security: Security::Starttls,
            username: Some("z".into()),
            password: Some("geheim".into()),
            from: "zaehler@example.test".into(),
        };
        assert!(cfg.validate().is_ok());
        assert!(!format!("{cfg:?}").contains("geheim"));
        assert!(SmtpNotifier::new(&cfg).is_ok());
        cfg.security = Security::None;
        assert!(cfg.validate().is_err(), "unverschlüsselt nur lokal");
        cfg.host = "localhost".into();
        assert!(cfg.validate().is_ok());
        cfg.password = None;
        assert!(cfg.validate().is_err());
        cfg.username = None;
        cfg.from = "kein-at".into();
        assert!(cfg.validate().is_err());
    }

    /// Versand an einen echten Mail-Server, z.B. Mailpit aus
    /// `compose.mail.yaml`: `ZAEHLER_TEST_SMTP_PORT=1025`.
    // @test SPEC_NTF_EMAIL
    #[tokio::test]
    async fn sends_via_smtp_server_when_configured() {
        let Ok(port) = std::env::var("ZAEHLER_TEST_SMTP_PORT") else {
            return;
        };
        let cfg = SmtpConfig {
            host: "localhost".into(),
            port: port.parse().unwrap(),
            security: Security::None,
            username: None,
            password: None,
            from: "zaehler@example.test".into(),
        };
        SmtpNotifier::new(&cfg)
            .unwrap()
            .send("anna@example.test", &reminder(), "de")
            .await
            .unwrap();
    }
}
