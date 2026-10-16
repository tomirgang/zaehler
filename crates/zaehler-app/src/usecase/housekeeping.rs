//! Housekeeping: entfernt abgelaufene Daten ohne Haushaltsbezug.

use std::sync::Arc;

use chrono::Duration;

use crate::{
    AppError,
    port::{BlobStore, Clock, MaintenanceRepository},
};

/// Alter, ab dem temporäre Objekte (`households/*/tmp/`) gelöscht werden.
pub const TMP_TTL: Duration = Duration::hours(24);

/// Anzahl entfernter Elemente je Art eines Laufs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HousekeepingReport {
    pub drafts: u64,
    pub tmp: u64,
    pub sessions: u64,
}

/// Wird von einem Intervall in der Composition Root in jeder Instanz
/// ausgelöst; jede Operation ist idempotent und verträgt parallele Läufe.
#[derive(Clone)]
pub struct Housekeeping {
    maintenance: Arc<dyn MaintenanceRepository>,
    blobs: Arc<dyn BlobStore>,
    clock: Arc<dyn Clock>,
    draft_ttl: Duration,
}

impl Housekeeping {
    /// `draft_ttl`: Alter, ab dem offene AI-Entwürfe verworfen werden.
    #[must_use]
    pub fn new(
        maintenance: Arc<dyn MaintenanceRepository>,
        blobs: Arc<dyn BlobStore>,
        clock: Arc<dyn Clock>,
        draft_ttl: Duration,
    ) -> Self {
        Self {
            maintenance,
            blobs,
            clock,
            draft_ttl,
        }
    }

    /// Führt alle Schritte aus, auch wenn einer fehlschlägt; liefert dann den
    /// ersten Fehler.
    // @impl SPEC_OPS_HOUSEKEEPING
    #[tracing::instrument(name = "Housekeeping", skip_all)]
    pub async fn execute(&self) -> Result<HousekeepingReport, AppError> {
        let now = self.clock.now();
        let mut report = HousekeepingReport::default();
        let mut first_error = None;

        match self.expire_drafts(now).await {
            Ok(n) => report.drafts = n,
            Err(e) => first_error = first_error.or(Some(e)),
        }
        match self.remove_tmp(now - TMP_TTL).await {
            Ok(n) => report.tmp = n,
            Err(e) => first_error = first_error.or(Some(e)),
        }
        match self.maintenance.delete_expired_sessions(now).await {
            Ok(n) => report.sessions = n,
            Err(e) => first_error = first_error.or(Some(e.into())),
        }

        metrics::counter!("zaehler_housekeeping_removed_total", "kind" => "draft")
            .increment(report.drafts);
        metrics::counter!("zaehler_housekeeping_removed_total", "kind" => "tmp")
            .increment(report.tmp);
        metrics::counter!("zaehler_housekeeping_removed_total", "kind" => "session")
            .increment(report.sessions);
        match first_error {
            Some(e) => Err(e),
            None => Ok(report),
        }
    }

    /// Offene Entwürfe verwerfen, danach ihre Fotos löschen.
    async fn expire_drafts(&self, now: chrono::DateTime<chrono::Utc>) -> Result<u64, AppError> {
        let keys = self
            .maintenance
            .expire_drafts(now - self.draft_ttl, now)
            .await?;
        for key in &keys {
            self.blobs
                .delete(key)
                .await
                .map_err(AppError::from_storage)?;
        }
        Ok(keys.len() as u64)
    }

    async fn remove_tmp(&self, cutoff: chrono::DateTime<chrono::Utc>) -> Result<u64, AppError> {
        let keys = self
            .blobs
            .list_tmp_older_than(cutoff)
            .await
            .map_err(AppError::from_storage)?;
        for key in &keys {
            self.blobs
                .delete(key)
                .await
                .map_err(AppError::from_storage)?;
        }
        Ok(keys.len() as u64)
    }
}
