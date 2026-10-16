//! Konfiguration: Defaults → TOML-Datei → Umgebungsvariablen.
//!
//! Umgebungsvariablen verwenden das Präfix `ZAEHLER_` und `__` als
//! Trenner für verschachtelte Schlüssel, z.B. `ZAEHLER_DATABASE__URL`.

use std::{net::SocketAddr, path::Path};

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub log: LogConfig,
    pub storage: StorageConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oidc: Option<OidcConfig>,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub image: ImageConfig,
    #[serde(default)]
    pub housekeeping: HousekeepingConfig,
    #[serde(default)]
    pub upload: UploadConfig,
    #[serde(default)]
    pub backup: BackupConfig,
    #[serde(default)]
    pub otel: OtelConfig,
    /// Ohne Abschnitt `ai` ist die Erkennung abgeschaltet (REQ_AI_006).
    #[serde(skip_serializing)]
    pub ai: Option<zaehler_ai::OpenRouterConfig>,
    #[serde(default)]
    pub notify: NotifyConfig,
    #[serde(default)]
    pub invitations: InvitationsConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
}

/// Einladungen (SPEC_TEN_INVITES).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvitationsConfig {
    /// Gültigkeit eines Einladungslinks in Tagen.
    pub ttl_days: u16,
}

impl Default for InvitationsConfig {
    fn default() -> Self {
        Self { ttl_days: 7 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub metrics_listen: SocketAddr,
    /// Öffentliche Basis-URL (für OIDC-Redirects und Links in Benachrichtigungen).
    pub public_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// `postgres://…` (CNPG: Secret `<cluster>-app`, Key `uri`) oder `sqlite://pfad?mode=rwc`.
    pub url: String,
    pub max_connections: u32,
    pub auto_migrate: bool,
    /// Eigene Verbindung für den Leader-Lock, direkt zur Datenbank und nicht
    /// über einen Pooler im Transaktionsmodus; ohne Angabe `url`
    /// (SPEC_OPS_REPLICAS).
    #[serde(default)]
    pub lock_url: Option<String>,
}

/// Scheduler der Jobs (SPEC_JOB_SCHEDULER).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Takt in Sekunden; Outbox je Takt, längere Intervalle je Job.
    pub tick_seconds: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { tick_seconds: 60 }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Pretty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    pub format: LogFormat,
    /// `tracing`-Filter, z.B. `info,zaehler=debug`. `RUST_LOG` hat Vorrang.
    pub filter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum StorageConfig {
    /// Lokales Verzeichnis (docker compose, Volume).
    Fs { path: String },
    /// S3-kompatibler Objektspeicher (k8s: MinIO, Garage, Ceph RGW …).
    S3 {
        bucket: String,
        #[serde(default)]
        endpoint: Option<String>,
        #[serde(default)]
        region: Option<String>,
        /// Ohne Angabe aus `AWS_ACCESS_KEY_ID`.
        #[serde(default)]
        access_key_id: Option<String>,
        /// Ohne Angabe aus `AWS_SECRET_ACCESS_KEY`.
        #[serde(default, skip_serializing)]
        secret_access_key: Option<String>,
        /// Endpunkt ohne TLS zulassen (z.B. Objektspeicher im selben Cluster).
        #[serde(default)]
        allow_http: bool,
    },
}

/// Bildverarbeitung (SPEC_IMG_PROCESSING).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageConfig {
    pub max_pixels: u64,
    pub jpeg_quality: u8,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            max_pixels: 2_000_000,
            jpeg_quality: 85,
        }
    }
}

/// Housekeeping (SPEC_OPS_HOUSEKEEPING).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HousekeepingConfig {
    pub interval_minutes: u64,
    /// Alter, ab dem offene AI-Entwürfe verworfen werden.
    #[serde(default = "default_draft_ttl")]
    pub draft_ttl_hours: i64,
}

const fn default_draft_ttl() -> i64 {
    24
}

impl Default for HousekeepingConfig {
    fn default() -> Self {
        Self {
            interval_minutes: 60,
            draft_ttl_hours: default_draft_ttl(),
        }
    }
}

/// Upload-Grenzen (SPEC_SEC_UPLOADS).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_field_names)] // Namen entsprechen den Konfigurationsschlüsseln
pub struct UploadConfig {
    #[serde(default = "default_max_photo_bytes")]
    pub max_photo_bytes: usize,
    #[serde(default = "default_max_csv_bytes")]
    pub max_csv_bytes: usize,
    /// Haushaltsarchive inklusive Fotos (SPEC_BACKUP_HOUSEHOLD)
    #[serde(default = "default_max_backup_bytes")]
    pub max_backup_bytes: usize,
}

const fn default_max_backup_bytes() -> usize {
    zaehler_web::state::DEFAULT_MAX_BACKUP_BYTES
}

const fn default_max_photo_bytes() -> usize {
    zaehler_web::state::DEFAULT_MAX_PHOTO_BYTES
}

const fn default_max_csv_bytes() -> usize {
    zaehler_web::state::DEFAULT_MAX_CSV_BYTES
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            max_photo_bytes: default_max_photo_bytes(),
            max_csv_bytes: default_max_csv_bytes(),
            max_backup_bytes: default_max_backup_bytes(),
        }
    }
}

/// Sicherung vor Migrationen (SPEC_OPS_UPGRADE).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConfig {
    /// Vor ausstehenden Migrationen ein vollständiges Archiv schreiben.
    #[serde(default = "default_true")]
    pub before_migrate: bool,
    /// Verzeichnis der Archive vor Migrationen.
    #[serde(default = "default_backup_dir")]
    pub dir: String,
    /// Anzahl aufbewahrter Archive vor Migrationen.
    #[serde(default = "default_backup_keep")]
    pub keep: usize,
}

const fn default_true() -> bool {
    true
}

fn default_backup_dir() -> String {
    "data/backups".into()
}

const fn default_backup_keep() -> usize {
    3
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            before_migrate: true,
            dir: default_backup_dir(),
            keep: default_backup_keep(),
        }
    }
}

/// Trace-Export per OTLP (SPEC_OPS_OTEL).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtelConfig {
    /// Basis-URL des Collectors (OTLP/HTTP, z.B. `http://collector:4318`);
    /// ohne Angabe gilt `OTEL_EXPORTER_OTLP_ENDPOINT`, sonst kein Export.
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default = "default_service_name")]
    pub service_name: String,
}

fn default_service_name() -> String {
    "zaehler".into()
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            endpoint: None,
            service_name: default_service_name(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    pub issuer_url: String,
    pub client_id: String,
    #[serde(skip_serializing)]
    pub client_secret: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Entwicklungs-Login ohne Identity Provider (SPEC_TEN_DEV_LOGIN).
    #[serde(default)]
    pub dev_login: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    #[serde(default = "default_session_ttl")]
    pub ttl_hours: i64,
    /// `None`: abgeleitet aus dem Schema von `server.public_url`.
    #[serde(default)]
    pub cookie_secure: Option<bool>,
}

const fn default_session_ttl() -> i64 {
    720
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ttl_hours: default_session_ttl(),
            cookie_secure: None,
        }
    }
}

/// E-Mail-Versand; ohne `smtp` sind Benachrichtigungen deaktiviert
/// (SPEC_NTF_EMAIL).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotifyConfig {
    #[serde(skip_serializing)]
    pub smtp: Option<zaehler_notify::SmtpConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                listen: ([0, 0, 0, 0], 8080).into(),
                metrics_listen: ([0, 0, 0, 0], 9090).into(),
                public_url: "http://localhost:8080".into(),
            },
            database: DatabaseConfig {
                url: "sqlite://data/zaehler.db?mode=rwc".into(),
                max_connections: 10,
                auto_migrate: true,
                lock_url: None,
            },
            log: LogConfig {
                format: LogFormat::Json,
                filter: "info".into(),
            },
            storage: StorageConfig::Fs {
                path: "data/files".into(),
            },
            oidc: None,
            auth: AuthConfig::default(),
            session: SessionConfig::default(),
            image: ImageConfig::default(),
            housekeeping: HousekeepingConfig::default(),
            upload: UploadConfig::default(),
            backup: BackupConfig::default(),
            otel: OtelConfig::default(),
            ai: None,
            notify: NotifyConfig::default(),
            invitations: InvitationsConfig::default(),
            scheduler: SchedulerConfig::default(),
        }
    }
}

impl Config {
    // @impl SPEC_OPS_CONFIG
    pub fn load(file: Option<&Path>) -> Result<Self, Box<figment::Error>> {
        let mut figment = Figment::from(Serialized::defaults(Self::default()));
        if let Some(file) = file {
            figment = figment.merge(Toml::file_exact(file));
        }
        figment
            .merge(Env::prefixed("ZAEHLER_").split("__").ignore(&["config"]))
            .extract()
            .map_err(Box::new)
    }

    /// JSON-Darstellung ohne Geheimnisse (Secrets sind `skip_serializing`).
    pub fn redacted(&self) -> String {
        let mut c = self.clone();
        c.database.url = redact_url(&c.database.url);
        c.database.lock_url = c.database.lock_url.as_deref().map(redact_url);
        c.otel.endpoint = c.otel.endpoint.as_deref().map(redact_url);
        serde_json::to_string_pretty(&c).unwrap_or_default()
    }

    /// Host von `server.public_url` ist `localhost` oder eine Loopback-Adresse.
    #[must_use]
    pub fn public_url_is_local(&self) -> bool {
        let rest = self
            .server
            .public_url
            .split_once("://")
            .map_or(self.server.public_url.as_str(), |(_, r)| r);
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        matches!(host, "localhost" | "127.0.0.1" | "[::1]")
    }

    /// `Secure`-Flag des Sitzungscookies: explizit oder aus dem Schema der
    /// öffentlichen URL (SPEC_SEC_SESSION_CSRF).
    #[must_use]
    pub fn cookie_secure(&self) -> bool {
        self.session
            .cookie_secure
            .unwrap_or_else(|| self.server.public_url.starts_with("https://"))
    }

    /// Prüft sicherheitsrelevante Kombinationen vor dem Start.
    // @impl SPEC_TEN_DEV_LOGIN, SPEC_SEC_SESSION_CSRF
    pub fn validate(&self) -> Result<(), String> {
        if self.auth.dev_login && self.oidc.is_some() {
            return Err("auth.dev_login und oidc schließen sich aus".into());
        }
        if self.auth.dev_login && !self.public_url_is_local() {
            return Err(
                "auth.dev_login ist nur mit server.public_url auf localhost zulässig".into(),
            );
        }
        if !self.auth.dev_login && self.oidc.is_none() {
            return Err("keine Anmeldung konfiguriert: oidc einrichten oder auth.dev_login für lokale Entwicklung".into());
        }
        if !self.cookie_secure() && !self.public_url_is_local() {
            tracing::warn!("Sitzungscookie ohne Secure-Flag bei nicht lokaler public_url");
        }
        if self.session.ttl_hours <= 0 {
            return Err("session.ttl_hours muss größer 0 sein".into());
        }
        if self.image.max_pixels == 0 {
            return Err("image.max_pixels muss größer 0 sein".into());
        }
        if !(1..=100).contains(&self.image.jpeg_quality) {
            return Err("image.jpeg_quality muss zwischen 1 und 100 liegen".into());
        }
        if self.housekeeping.interval_minutes == 0 {
            return Err("housekeeping.interval_minutes muss größer 0 sein".into());
        }
        if self.housekeeping.draft_ttl_hours <= 0 {
            return Err("housekeeping.draft_ttl_hours muss größer 0 sein".into());
        }
        if self.upload.max_photo_bytes == 0 {
            return Err("upload.max_photo_bytes muss größer 0 sein".into());
        }
        if self.upload.max_csv_bytes == 0 {
            return Err("upload.max_csv_bytes muss größer 0 sein".into());
        }
        if self.upload.max_backup_bytes == 0 {
            return Err("upload.max_backup_bytes muss größer 0 sein".into());
        }
        if self.scheduler.tick_seconds == 0 {
            return Err("scheduler.tick_seconds muss größer 0 sein".into());
        }
        if self.invitations.ttl_days == 0 {
            return Err("invitations.ttl_days muss größer 0 sein".into());
        }
        if let Some(smtp) = &self.notify.smtp {
            smtp.validate()?;
        }
        if self.backup.keep == 0 {
            return Err("backup.keep muss mindestens 1 sein".into());
        }
        if let Some(e) = self.otel.endpoint.as_deref().filter(|e| !e.is_empty())
            && !(e.starts_with("http://") || e.starts_with("https://"))
        {
            return Err("otel.endpoint muss mit http:// oder https:// beginnen".into());
        }
        Ok(())
    }
}

fn redact_url(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(scheme_end), Some(at)) if at > scheme_end => {
            format!("{}://***{}", &url[..scheme_end], &url[at..])
        }
        _ => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // @test SPEC_OPS_CONFIG
    #[test]
    #[allow(clippy::result_large_err)]
    fn env_overrides_defaults() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("ZAEHLER_DATABASE__URL", "postgres://u:p@db/zaehler");
            jail.set_env("ZAEHLER_SERVER__LISTEN", "127.0.0.1:3000");
            let c = Config::load(None).map_err(|e| e.to_string())?;
            assert!(c.database.url.starts_with("postgres://"));
            assert_eq!(c.server.listen.port(), 3000);
            assert!(!c.redacted().contains("u:p"));
            Ok(())
        });
    }

    // @test SPEC_OPS_CONFIG, SPEC_STORE_BLOBS
    #[test]
    #[allow(clippy::result_large_err)]
    fn s3_storage_from_env_without_secret_in_output() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("ZAEHLER_STORAGE__KIND", "s3");
            jail.set_env("ZAEHLER_STORAGE__BUCKET", "zaehler");
            jail.set_env("ZAEHLER_STORAGE__ENDPOINT", "http://minio:9000");
            jail.set_env("ZAEHLER_STORAGE__ALLOW_HTTP", "true");
            jail.set_env("ZAEHLER_STORAGE__SECRET_ACCESS_KEY", "geheim-123");
            jail.set_env("ZAEHLER_HOUSEKEEPING__INTERVAL_MINUTES", "5");
            let c = Config::load(None).map_err(|e| e.to_string())?;
            let StorageConfig::S3 {
                bucket,
                allow_http,
                secret_access_key,
                ..
            } = &c.storage
            else {
                panic!("S3 erwartet");
            };
            assert_eq!(bucket, "zaehler");
            assert!(*allow_http);
            assert_eq!(secret_access_key.as_deref(), Some("geheim-123"));
            assert_eq!(c.housekeeping.interval_minutes, 5);
            assert_eq!(c.image.jpeg_quality, 85);
            assert!(!c.redacted().contains("geheim-123"));
            Ok(())
        });
    }

    // @test SPEC_OPS_CONFIG
    #[test]
    fn image_and_housekeeping_values_are_validated() {
        let mut c = Config::default();
        c.auth.dev_login = true;
        c.image.jpeg_quality = 0;
        assert!(c.validate().is_err());
        c.image.jpeg_quality = 85;
        c.housekeeping.interval_minutes = 0;
        assert!(c.validate().is_err());
        c.housekeeping.interval_minutes = 60;
        assert!(c.validate().is_ok());
    }

    // @test SPEC_TEN_DEV_LOGIN, SPEC_SEC_SESSION_CSRF
    #[test]
    fn auth_combinations_are_validated() {
        let mut c = Config::default();
        assert!(c.validate().is_err(), "ohne Anmeldung kein Start");
        c.auth.dev_login = true;
        assert!(c.validate().is_ok());
        assert!(!c.cookie_secure(), "http://localhost");
        c.server.public_url = "https://zaehler.example.com".into();
        assert!(c.validate().is_err(), "Dev-Login nur lokal");
        c.auth.dev_login = false;
        c.oidc = Some(OidcConfig {
            issuer_url: "https://idp".into(),
            client_id: "z".into(),
            client_secret: "s".into(),
        });
        assert!(c.validate().is_ok());
        assert!(c.cookie_secure());
        c.auth.dev_login = true;
        assert!(
            c.validate().is_err(),
            "Dev-Login und OIDC schließen sich aus"
        );
    }
}
