//! Zustand des Web-Adapters; wird in der Composition Root erzeugt.

use std::sync::Arc;

use zaehler_app::{port::Readiness, usecase::UseCases};

use crate::auth::OidcClient;

/// Konfiguration des Web-Adapters.
#[derive(Debug, Clone)]
pub struct WebConfig {
    /// Öffentliche Basis-URL ohne abschließenden Schrägstrich.
    pub public_url: String,
    /// `scheme://host[:port]` der öffentlichen URL (Prüfung des `Origin`-Headers).
    pub public_origin: String,
    pub dev_login: bool,
    pub cookie_secure: bool,
    pub session_ttl_hours: i64,
    pub version: &'static str,
    /// Grenze für Foto-Uploads in Byte (SPEC_SEC_UPLOADS)
    pub max_photo_bytes: usize,
    /// Grenze für CSV-Uploads in Byte (SPEC_SEC_UPLOADS)
    pub max_csv_bytes: usize,
}

/// Standard für `upload.max_photo_bytes`: 15 MiB.
pub const DEFAULT_MAX_PHOTO_BYTES: usize = 15 * 1024 * 1024;
/// Standard für `upload.max_csv_bytes`: 5 MiB.
pub const DEFAULT_MAX_CSV_BYTES: usize = 5 * 1024 * 1024;

impl WebConfig {
    #[must_use]
    pub fn new(
        public_url: &str,
        dev_login: bool,
        cookie_secure: bool,
        session_ttl_hours: i64,
        version: &'static str,
    ) -> Self {
        let public_url = public_url.trim_end_matches('/').to_owned();
        let public_origin = match public_url.find("://") {
            Some(i) => {
                let rest = &public_url[i + 3..];
                let host_end = rest.find('/').map_or(public_url.len(), |j| i + 3 + j);
                public_url[..host_end].to_owned()
            }
            None => public_url.clone(),
        };
        Self {
            public_url,
            public_origin,
            dev_login,
            cookie_secure,
            session_ttl_hours,
            version,
            max_photo_bytes: DEFAULT_MAX_PHOTO_BYTES,
            max_csv_bytes: DEFAULT_MAX_CSV_BYTES,
        }
    }

    #[must_use]
    pub fn with_max_photo_bytes(mut self, bytes: usize) -> Self {
        self.max_photo_bytes = bytes;
        self
    }

    #[must_use]
    pub fn with_max_csv_bytes(mut self, bytes: usize) -> Self {
        self.max_csv_bytes = bytes;
        self
    }
}

#[derive(Clone)]
pub struct WebState {
    pub uc: Arc<UseCases>,
    pub readiness: Arc<dyn Readiness>,
    pub cfg: Arc<WebConfig>,
    pub oidc: Option<Arc<OidcClient>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_from_public_url() {
        let c = WebConfig::new("https://zaehler.example.com/app/", false, true, 1, "0");
        assert_eq!(c.public_url, "https://zaehler.example.com/app");
        assert_eq!(c.public_origin, "https://zaehler.example.com");
        let c = WebConfig::new("http://localhost:8080", true, false, 1, "0");
        assert_eq!(c.public_origin, "http://localhost:8080");
    }
}
