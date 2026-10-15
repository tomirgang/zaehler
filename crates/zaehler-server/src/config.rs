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
    #[serde(skip_serializing)]
    pub ai: Option<zaehler_ai::OpenRouterConfig>,
    #[serde(default)]
    pub notify: NotifyConfig,
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
        endpoint: Option<String>,
        region: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] // genutzt ab REQ_TEN_002 (OIDC-Login)
pub struct OidcConfig {
    pub issuer_url: String,
    pub client_id: String,
    #[serde(skip_serializing)]
    pub client_secret: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[allow(dead_code)] // genutzt ab REQ_NTF_001 (Scheduler)
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
            },
            log: LogConfig {
                format: LogFormat::Json,
                filter: "info".into(),
            },
            storage: StorageConfig::Fs {
                path: "data/files".into(),
            },
            oidc: None,
            ai: None,
            notify: NotifyConfig::default(),
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
        serde_json::to_string_pretty(&c).unwrap_or_default()
    }

    pub fn db_backend(&self) -> &'static str {
        if self.database.url.starts_with("sqlite:") {
            "sqlite"
        } else {
            "postgres"
        }
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
            assert_eq!(c.db_backend(), "postgres");
            assert_eq!(c.server.listen.port(), 3000);
            assert!(!c.redacted().contains("u:p"));
            Ok(())
        });
    }
}
