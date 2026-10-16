//! Jobs des Schedulers (SPEC_JOB_SCHEDULER, SPEC_JOB_OUTBOX). Sie laufen nur
//! auf der Instanz mit Leader-Lock und ohne angemeldeten Nutzer.

use chrono::{DateTime, Duration, Timelike, Utc};
use zaehler_domain::{
    calendar::Calendar,
    schedule::{self, Due},
};

use crate::{
    AppError, Tenant,
    ids::{HouseholdId, InvitationId, MeterId, ReadingId},
    model::{
        DueMeter, Notification, NotificationKind, NotificationLogEntry, OutboxEntry, OutboxEvent,
        OutboxResult,
    },
    usecase::{
        Ports,
        analysis::{check_alarm, load_for_job},
        use_case,
    },
};

/// Erinnerungen frühestens ab dieser Stunde (lokal, SPEC_NTF_REMINDERS).
pub const REMINDER_HOUR: u32 = 8;

/// Einträge je Lauf von `DispatchOutbox`.
pub const OUTBOX_BATCH: u32 = 100;
/// Nach so vielen Versuchen wird ein Eintrag `failed`.
pub const OUTBOX_MAX_ATTEMPTS: u32 = 10;
/// Erledigte Einträge entfernt das Housekeeping nach dieser Zeit.
pub const OUTBOX_RETENTION: Duration = Duration::days(30);

/// Wartezeit vor dem Versuch `attempts + 1`: 1 min, verdoppelt je Versuch,
/// höchstens 6 h.
// @impl SPEC_JOB_OUTBOX
#[must_use]
pub fn backoff(attempts: u32) -> Duration {
    let minutes = 1_i64 << attempts.saturating_sub(1).min(16);
    Duration::minutes(minutes).min(Duration::hours(6))
}

/// Ergebnis eines Laufs von `DispatchOutbox`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DispatchReport {
    pub done: u32,
    pub retried: u32,
    pub failed: u32,
}

use_case!(
    /// Offene Outbox-Einträge verarbeiten (Job).
    DispatchOutbox,
    /// Ablese-Erinnerungen versenden (Job, stündlich).
    SendReminders,
);

/// Versendet eine Benachrichtigung an alle Empfänger, je Empfänger und
/// Schlüssel höchstens einmal. Ein Fehler entfernt den Eintrag im
/// Versandprotokoll wieder, damit ein späterer Lauf es erneut versucht;
/// bereits versendete Empfänger bleiben eingetragen.
// @impl SPEC_NTF_EMAIL, SPEC_NTF_REMINDERS, SPEC_NTF_ALARMS
async fn deliver(
    p: &Ports,
    household: HouseholdId,
    key: &str,
    notification: &Notification,
    recipients: Vec<crate::model::Recipient>,
) -> Result<u32, AppError> {
    let kind = notification.kind();
    let mut sent = 0;
    let mut failure = None;
    for r in recipients {
        let entry = NotificationLogEntry {
            household_id: household,
            user_id: r.user_id,
            kind,
            subject_key: key.to_owned(),
            sent_at: p.clock.now(),
        };
        if !p.jobs.log_once(&entry).await? {
            continue;
        }
        match p.notifier.send(&r.email, notification, &r.locale).await {
            Ok(()) => {
                sent += 1;
                metrics::counter!("zaehler_notifications_total", "kind" => kind.as_str(), "outcome" => "sent")
                    .increment(1);
            }
            Err(e) => {
                // Adresse nicht protokollieren, nur Nutzer und Art
                tracing::warn!(user = %r.user_id, kind = kind.as_str(), error = %e, "notification not sent");
                metrics::counter!("zaehler_notifications_total", "kind" => kind.as_str(), "outcome" => "error")
                    .increment(1);
                p.jobs.release_log(&entry).await?;
                failure = Some(AppError::Upstream("smtp"));
            }
        }
    }
    match failure {
        Some(e) => Err(e),
        None => Ok(sent),
    }
}

impl DispatchOutbox {
    /// Verarbeitet offene Einträge in Reihenfolge der Erzeugung. Ein Fehler
    /// verschiebt den Eintrag mit Backoff; nach `OUTBOX_MAX_ATTEMPTS`
    /// Versuchen wird er `failed`.
    // @impl SPEC_JOB_OUTBOX
    #[tracing::instrument(name = "DispatchOutbox", skip_all)]
    pub async fn execute(&self) -> Result<DispatchReport, AppError> {
        let now = self.p.clock.now();
        let entries = self.p.jobs.claim_outbox(now, OUTBOX_BATCH).await?;
        let mut report = DispatchReport::default();
        for entry in entries {
            let result = match self.process(&entry).await {
                Ok(()) => {
                    report.done += 1;
                    OutboxResult::Done { at: now }
                }
                Err(e) => {
                    let attempts = entry.attempts + 1;
                    tracing::warn!(
                        outbox = %entry.id,
                        kind = %entry.kind,
                        attempts,
                        error = %e,
                        "outbox entry failed"
                    );
                    retry(attempts, now, &mut report)
                }
            };
            self.p.jobs.finish_outbox(entry.id, result).await?;
        }
        let pending = self.p.jobs.pending_outbox().await?;
        #[allow(clippy::cast_precision_loss)] // Anzahl offener Einträge als Gauge
        metrics::gauge!("zaehler_outbox_pending").set(pending as f64);
        Ok(report)
    }

    async fn process(&self, entry: &OutboxEntry) -> Result<(), AppError> {
        let event: OutboxEvent = serde_json::from_str(&entry.payload)
            .map_err(|e| AppError::Internal(format!("outbox payload: {e}")))?;
        if !self.p.notifier.enabled() {
            // ohne SMTP gibt es keine Folgeverarbeitung
            return Ok(());
        }
        match event {
            OutboxEvent::ReadingsRecorded {
                meter_id,
                reading_ids,
            } => self.alarm(entry.household_id, meter_id, &reading_ids).await,
            OutboxEvent::InvitationCreated {
                invitation_id,
                token,
            } => {
                self.invitation(entry.household_id, invitation_id, &token)
                    .await
            }
        }
    }

    /// Abweichungsalarm zur ersten auffälligen Ablesung des Ereignisses
    /// (SPEC_NTF_ALARMS); Schlüssel `alarm:{reading}`.
    // @impl SPEC_NTF_ALARMS
    async fn alarm(
        &self,
        household: HouseholdId,
        meter: MeterId,
        readings: &[ReadingId],
    ) -> Result<(), AppError> {
        let t = Tenant::for_job(household);
        let mut uow = self.p.uow.begin().await?;
        let (detail, cal) = match load_for_job(uow.as_mut(), &t, meter).await {
            Ok(x) => x,
            // Messstelle oder Haushalt inzwischen gelöscht
            Err(AppError::NotFound) => return Ok(()),
            Err(e) => return Err(e),
        };
        let household_name = uow.households().get(&t).await?.data.name;
        drop(uow);
        if detail.aggregate.meter.archived {
            return Ok(());
        }
        let today = cal.local_date(self.p.clock.now());
        let Some((reading, finding)) = readings
            .iter()
            .find_map(|id| check_alarm(&detail, cal, today, *id).map(|f| (*id, f)))
        else {
            return Ok(());
        };
        let meter_data = &detail.aggregate.meter;
        let notification = Notification::ConsumptionAlarm {
            household: household_name,
            meter: meter_data.data.name.clone(),
            unit: meter_data.data.unit.clone(),
            from: cal.local(finding.from),
            until: cal.local(finding.until),
            consumption: finding.consumption,
            expected_lower: finding.band.lower,
            expected_upper: finding.band.upper,
            link: format!("{}/meters/{meter}", self.p.settings.public_url),
            settings_link: settings_link(&self.p),
        };
        let recipients = self
            .p
            .jobs
            .recipients(household, NotificationKind::Alarm)
            .await?;
        deliver(
            &self.p,
            household,
            &format!("alarm:{reading}"),
            &notification,
            recipients,
        )
        .await?;
        Ok(())
    }

    /// Einladung an die angegebene Adresse in der Sprache der einladenden
    /// Person (SPEC_TEN_INVITES).
    // @impl SPEC_TEN_INVITES, SPEC_NTF_EMAIL
    async fn invitation(
        &self,
        household: HouseholdId,
        id: InvitationId,
        token: &str,
    ) -> Result<(), AppError> {
        let t = Tenant::for_job(household);
        let now = self.p.clock.now();
        let mut uow = self.p.uow.begin().await?;
        let Some(inv) = uow
            .invitations()
            .list_open(&t, now)
            .await?
            .into_iter()
            .find(|i| i.id == id)
        else {
            // angenommen, widerrufen oder abgelaufen
            return Ok(());
        };
        let Some(email) = inv.email.clone() else {
            return Ok(());
        };
        let inviter = uow.users().get(inv.created_by).await?;
        let tz = uow.households().get(&t).await?.data.timezone;
        drop(uow);
        let (inviter_name, locale) = inviter.map_or_else(
            || (String::new(), "de".to_owned()),
            |u| (u.display_name, u.locale),
        );
        let notification = Notification::Invitation {
            household: inv.household_name.clone(),
            inviter: inviter_name,
            role: inv.role,
            link: format!("{}/invite/{token}", self.p.settings.public_url),
            expires_on: Calendar::new(tz).local_date(inv.expires_at),
        };
        let recipient = crate::model::Recipient {
            user_id: inv.created_by,
            email,
            locale,
        };
        deliver(
            &self.p,
            household,
            &format!("invitation:{id}"),
            &notification,
            vec![recipient],
        )
        .await?;
        Ok(())
    }
}

fn settings_link(p: &Ports) -> String {
    format!("{}/settings/notifications", p.settings.public_url)
}

/// Ergebnis eines Laufs von `SendReminders`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReminderReport {
    pub households: u32,
    pub sent: u32,
}

impl SendReminders {
    /// Je Haushalt mit fälligen oder überfälligen Messstellen höchstens eine
    /// E-Mail je Empfänger und Kalendertag, frühestens ab 08:00 Uhr lokal
    /// (SPEC_NTF_REMINDERS).
    // @impl SPEC_NTF_REMINDERS, SPEC_RDG_SCHEDULE
    #[tracing::instrument(name = "SendReminders", skip_all)]
    pub async fn execute(&self) -> Result<ReminderReport, AppError> {
        let mut report = ReminderReport::default();
        if !self.p.notifier.enabled() {
            return Ok(report);
        }
        let mut failure = None;
        for household in self.p.jobs.households_with_schedules().await? {
            match self.household(household).await {
                Ok(Some(sent)) => {
                    report.households += 1;
                    report.sent += sent;
                }
                Ok(None) => {}
                Err(e) => failure = Some(e),
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(report),
        }
    }

    async fn household(&self, household: HouseholdId) -> Result<Option<u32>, AppError> {
        let t = Tenant::for_job(household);
        let mut uow = self.p.uow.begin().await?;
        let data = uow.households().get(&t).await?.data;
        let cal = Calendar::new(data.timezone);
        let now = self.p.clock.now();
        if cal.local(now).hour() < REMINDER_HOUR {
            return Ok(None);
        }
        let today = cal.local_date(now);
        let meters: Vec<DueMeter> = uow
            .meters()
            .list(&t)
            .await?
            .into_iter()
            .filter(|m| !m.meter.archived)
            .filter_map(|m| {
                let due = schedule::due(
                    m.meter.data.schedule?,
                    m.last_reading_at.map(|at| cal.local_date(at)),
                    today,
                );
                due.needs_reading().then(|| DueMeter {
                    name: m.meter.data.name.clone(),
                    due_on: due.date(),
                    overdue: matches!(due, Due::Overdue(_)),
                    link: format!(
                        "{}/meters/{}/readings/new",
                        self.p.settings.public_url, m.meter.id
                    ),
                })
            })
            .collect();
        drop(uow);
        if meters.is_empty() {
            return Ok(None);
        }
        let notification = Notification::Reminder {
            household: data.name,
            meters,
            settings_link: settings_link(&self.p),
        };
        let recipients = self
            .p
            .jobs
            .recipients(household, NotificationKind::Reminder)
            .await?;
        let sent = deliver(
            &self.p,
            household,
            &format!("reminder:{household}:{today}"),
            &notification,
            recipients,
        )
        .await?;
        Ok(Some(sent))
    }
}

fn retry(attempts: u32, now: DateTime<Utc>, report: &mut DispatchReport) -> OutboxResult {
    if attempts >= OUTBOX_MAX_ATTEMPTS {
        report.failed += 1;
        metrics::counter!("zaehler_outbox_failed_total").increment(1);
        OutboxResult::Failed { attempts, at: now }
    } else {
        report.retried += 1;
        OutboxResult::Retry {
            attempts,
            next_attempt_at: now + backoff(attempts),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // @test SPEC_JOB_OUTBOX
    #[test]
    fn backoff_doubles_up_to_six_hours() {
        assert_eq!(backoff(1), Duration::minutes(1));
        assert_eq!(backoff(2), Duration::minutes(2));
        assert_eq!(backoff(5), Duration::minutes(16));
        assert_eq!(backoff(9), Duration::hours(4) + Duration::minutes(16));
        assert_eq!(backoff(10), Duration::hours(6));
        assert_eq!(backoff(40), Duration::hours(6));
    }
}
