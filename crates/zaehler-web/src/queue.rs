//! Hintergrundanalyse der Foto-Warteschlange je Haushalt (SPEC_AI_MULTI).

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use zaehler_app::{Tenant, ids::HouseholdId, usecase::UseCases};

/// Startet je Haushalt höchstens eine Aufgabe in dieser Instanz; sie
/// analysiert die wartenden Fotos nacheinander, bis keines mehr wartet.
/// Mehrere Instanzen sichert die Beanspruchung im Datenbank-Adapter ab.
#[derive(Debug, Default)]
pub struct AnalysisRunner {
    running: Mutex<HashSet<HouseholdId>>,
}

/// Gibt den Haushalt beim Ende der Aufgabe frei, auch nach einem Panic.
struct Running {
    runner: Arc<AnalysisRunner>,
    household: HouseholdId,
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Ok(mut r) = self.runner.running.lock() {
            r.remove(&self.household);
        }
    }
}

impl AnalysisRunner {
    /// Startet die Analyse der wartenden Fotos des Haushalts, falls in dieser
    /// Instanz noch keine läuft.
    // @impl SPEC_AI_MULTI
    pub fn start(self: &Arc<Self>, uc: Arc<UseCases>, t: Tenant) {
        let household = t.household_id();
        let started = self.running.lock().is_ok_and(|mut r| r.insert(household));
        if !started {
            return;
        }
        let guard = Running {
            runner: self.clone(),
            household,
        };
        tokio::spawn(async move {
            let _guard = guard;
            loop {
                match uc.analyse_queued.next(&t).await {
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(e) => {
                        tracing::warn!(error = %e, "photo analysis stopped");
                        break;
                    }
                }
            }
        });
    }
}
