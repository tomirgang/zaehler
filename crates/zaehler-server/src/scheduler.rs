//! Periodische Aufgaben der Instanz.

use std::time::Duration;

use tokio::{task::JoinHandle, time::MissedTickBehavior};
use zaehler_app::usecase::housekeeping::Housekeeping;

/// Startet das Housekeeping-Intervall; der erste Lauf erfolgt sofort. Jede
/// Instanz läuft unabhängig, der Anwendungsfall ist idempotent
/// (SPEC_OPS_HOUSEKEEPING).
// @impl SPEC_OPS_HOUSEKEEPING
pub fn spawn_housekeeping(hk: Housekeeping, every: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match hk.execute().await {
                Ok(r) => tracing::info!(
                    drafts = r.drafts,
                    tmp = r.tmp,
                    sessions = r.sessions,
                    "housekeeping done"
                ),
                Err(e) => tracing::warn!(error = %e, "housekeeping failed"),
            }
        }
    })
}
