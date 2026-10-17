//! Zustand des Web-Adapters; wird in der Composition Root erzeugt.

use std::sync::Arc;

use zaehler_app::{port::Readiness, usecase::UseCases};

use crate::{auth::OidcClient, queue::AnalysisRunner};

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
    /// Grenze für Haushaltsarchive in Byte (SPEC_BACKUP_HOUSEHOLD)
    pub max_backup_bytes: usize,
    /// Grenze für Dokumente in Byte (SPEC_DOC_UPLOADS)
    pub max_document_bytes: usize,
    /// Übernimmt den Trace-Context eingehender Requests (SPEC_OPS_OTEL)
    pub trace_parent: Option<TraceParent>,
}

/// Setzt den Eltern-Span des Request-Spans aus den Request-Headern.
pub type TraceParent = fn(&axum::http::HeaderMap, &tracing::Span);

/// Standard für `upload.max_backup_bytes`: 1 GiB.
pub const DEFAULT_MAX_BACKUP_BYTES: usize = 1024 * 1024 * 1024;

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
            max_backup_bytes: DEFAULT_MAX_BACKUP_BYTES,
            max_document_bytes: zaehler_app::usecase::DEFAULT_MAX_DOCUMENT_BYTES,
            trace_parent: None,
        }
    }

    #[must_use]
    pub fn with_trace_parent(mut self, f: TraceParent) -> Self {
        self.trace_parent = Some(f);
        self
    }

    #[must_use]
    pub fn with_max_backup_bytes(mut self, bytes: usize) -> Self {
        self.max_backup_bytes = bytes;
        self
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

    #[must_use]
    pub fn with_max_document_bytes(mut self, bytes: usize) -> Self {
        self.max_document_bytes = bytes;
        self
    }
}

#[derive(Clone)]
pub struct WebState {
    pub uc: Arc<UseCases>,
    pub readiness: Arc<dyn Readiness>,
    pub cfg: Arc<WebConfig>,
    pub oidc: Option<Arc<OidcClient>>,
    /// Hintergrundanalyse der Foto-Warteschlange (SPEC_AI_MULTI)
    pub analysis: Arc<AnalysisRunner>,
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
