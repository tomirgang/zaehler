//! Scheduler der Instanz: ein Takt (`scheduler.tick_seconds`), Jobs nur mit
//! Leader-Lock (SPEC_JOB_SCHEDULER, SPEC_OPS_REPLICAS, ADR_010).

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use tokio::{task::JoinHandle, time::MissedTickBehavior};
use zaehler_app::{
    model::JobRun,
    port::{Clock, JobRepository, LeaderLock},
    usecase::{
        housekeeping::Housekeeping,
        jobs::{DispatchOutbox, SendReminders},
    },
};

type JobFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

/// Ein periodischer Job; `run` liefert eine kurze Zusammenfassung für das Log.
pub struct Job {
    pub name: &'static str,
    pub every: chrono::Duration,
    pub run: Box<dyn Fn() -> JobFuture + Send + Sync>,
}

pub struct Scheduler {
    leader: Arc<dyn LeaderLock>,
    runs: Arc<dyn JobRepository>,
    clock: Arc<dyn Clock>,
    jobs: Vec<Job>,
}

impl Scheduler {
    #[must_use]
    pub fn new(
        leader: Arc<dyn LeaderLock>,
        runs: Arc<dyn JobRepository>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            leader,
            runs,
            clock,
            jobs: Vec::new(),
        }
    }

    #[must_use]
    pub fn with(mut self, job: Job) -> Self {
        self.jobs.push(job);
        self
    }

    /// Ein Takt: nur als Leader fällige Jobs ausführen. Fällig ist ein Job,
    /// dessen letzter Lauf laut `job_runs` mindestens `every` zurückliegt;
    /// damit gilt das Intervall auch über Neustarts und Leader-Wechsel.
    /// Liefert die ausgeführten Jobs.
    // @impl SPEC_JOB_SCHEDULER
    pub async fn tick(&self) -> Vec<&'static str> {
        let leader = self.leader.is_leader().await;
        metrics::gauge!("zaehler_leader").set(if leader { 1.0 } else { 0.0 });
        if !leader {
            return Vec::new();
        }
        let mut ran = Vec::new();
        for job in &self.jobs {
            let now = self.clock.now();
            let due = match self.runs.last_run(job.name).await {
                Ok(last) => last.is_none_or(|r| now - r.started_at >= job.every),
                Err(e) => {
                    tracing::warn!(job = job.name, error = %e, "job run not readable");
                    false
                }
            };
            if !due {
                continue;
            }
            let outcome = match (job.run)().await {
                Ok(summary) => {
                    tracing::info!(job = job.name, %summary, "job done");
                    "ok"
                }
                Err(e) => {
                    tracing::warn!(job = job.name, error = %e, "job failed");
                    "error"
                }
            };
            metrics::counter!("zaehler_job_runs_total", "job" => job.name, "outcome" => outcome)
                .increment(1);
            let run = JobRun {
                started_at: now,
                finished_at: self.clock.now(),
                outcome: outcome.to_owned(),
            };
            if let Err(e) = self.runs.record_run(job.name, &run).await {
                tracing::warn!(job = job.name, error = %e, "job run not recorded");
            }
            ran.push(job.name);
        }
        ran
    }

    /// Startet den Takt; der erste Takt erfolgt sofort.
    pub fn spawn(self, every: Duration) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                self.tick().await;
            }
        })
    }
}

/// Housekeeping als Job (bisheriges Intervall `housekeeping.interval_minutes`).
// @impl SPEC_OPS_HOUSEKEEPING
pub fn housekeeping(hk: Housekeeping, every: chrono::Duration) -> Job {
    Job {
        name: "housekeeping",
        every,
        run: Box::new(move || {
            let hk = hk.clone();
            Box::pin(async move {
                hk.execute()
                    .await
                    .map(|r| {
                        format!(
                            "drafts={} tmp={} sessions={} outbox={}",
                            r.drafts, r.tmp, r.sessions, r.outbox
                        )
                    })
                    .map_err(|e| e.to_string())
            })
        }),
    }
}

/// Outbox in jedem Takt verarbeiten (SPEC_JOB_OUTBOX).
// @impl SPEC_JOB_OUTBOX
pub fn dispatch_outbox(job: DispatchOutbox, every: chrono::Duration) -> Job {
    Job {
        name: "dispatch_outbox",
        every,
        run: Box::new(move || {
            let job = job.clone();
            Box::pin(async move {
                job.execute()
                    .await
                    .map(|r| format!("done={} retried={} failed={}", r.done, r.retried, r.failed))
                    .map_err(|e| e.to_string())
            })
        }),
    }
}

/// Ablese-Erinnerungen; stündlich, versendet wird je Tag höchstens einmal
/// (SPEC_NTF_REMINDERS).
// @impl SPEC_NTF_REMINDERS
pub fn send_reminders(job: SendReminders, every: chrono::Duration) -> Job {
    Job {
        name: "send_reminders",
        every,
        run: Box::new(move || {
            let job = job.clone();
            Box::pin(async move {
                job.execute()
                    .await
                    .map(|r| format!("households={} sent={}", r.households, r.sent))
                    .map_err(|e| e.to_string())
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use async_trait::async_trait;
    use zaehler_app::testing::memory_ports;

    use super::*;

    struct Switch(AtomicBool);

    #[async_trait]
    impl LeaderLock for Switch {
        async fn is_leader(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn counting(name: &'static str, every: chrono::Duration, n: Arc<AtomicU32>) -> Job {
        Job {
            name,
            every,
            run: Box::new(move || {
                let n = n.clone();
                Box::pin(async move {
                    n.fetch_add(1, Ordering::SeqCst);
                    Ok(String::new())
                })
            }),
        }
    }

    // @test SPEC_JOB_SCHEDULER
    #[tokio::test]
    async fn jobs_run_only_as_leader_and_respect_their_interval() {
        let (ports, store, clock) = memory_ports();
        let lock = Arc::new(Switch(AtomicBool::new(false)));
        let fast = Arc::new(AtomicU32::new(0));
        let hourly = Arc::new(AtomicU32::new(0));
        let scheduler = Scheduler::new(lock.clone(), store.clone(), ports.clock.clone())
            .with(counting("fast", chrono::Duration::minutes(1), fast.clone()))
            .with(counting(
                "hourly",
                chrono::Duration::hours(1),
                hourly.clone(),
            ));

        assert!(scheduler.tick().await.is_empty(), "ohne Lock keine Jobs");
        lock.0.store(true, Ordering::SeqCst);
        assert_eq!(scheduler.tick().await, ["fast", "hourly"]);
        // im selben Takt nichts erneut
        assert!(scheduler.tick().await.is_empty());
        clock.advance(chrono::Duration::minutes(1));
        assert_eq!(scheduler.tick().await, ["fast"]);
        clock.advance(chrono::Duration::minutes(59));
        assert_eq!(scheduler.tick().await, ["fast", "hourly"]);
        assert_eq!(
            (fast.load(Ordering::SeqCst), hourly.load(Ordering::SeqCst)),
            (3, 2)
        );
        // eine zweite Instanz mit demselben Protokoll übernimmt das Intervall
        let other = Scheduler::new(lock, store.clone(), ports.clock.clone()).with(counting(
            "hourly",
            chrono::Duration::hours(1),
            hourly.clone(),
        ));
        assert!(other.tick().await.is_empty());
        let last = store.last_run("hourly").await.unwrap().unwrap();
        assert_eq!(last.outcome, "ok");
    }
}
