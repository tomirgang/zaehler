//! Abfragen der Jobs ohne Haushaltsbezug: Outbox und `job_runs`
//! (SPEC_JOB_OUTBOX, SPEC_JOB_SCHEDULER).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, OnConflict},
};
use zaehler_app::{
    PortError, Role,
    ids::{HouseholdId, OutboxId, UserId},
    model::{
        JobRun, NotificationKind, NotificationLogEntry, OutboxEntry, OutboxResult, OutboxStatus,
        Recipient,
    },
    port::{JobRepository, PortResult},
};

use crate::{
    entity::{job_run, membership, meter, notification_log, notification_settings, outbox, user},
    uow::{SeaOrmStore, data_err, db_err},
};

fn to_entry(m: outbox::Model) -> PortResult<OutboxEntry> {
    Ok(OutboxEntry {
        id: OutboxId(m.id),
        household_id: HouseholdId(m.household_id),
        kind: m.kind,
        payload: m.payload,
        attempts: u32::try_from(m.attempts).map_err(data_err)?,
        created_at: m.created_at,
    })
}

// @impl SPEC_JOB_OUTBOX, SPEC_JOB_SCHEDULER
#[async_trait]
impl JobRepository for SeaOrmStore {
    async fn claim_outbox(&self, now: DateTime<Utc>, limit: u32) -> PortResult<Vec<OutboxEntry>> {
        outbox::Entity::find()
            .filter(outbox::Column::Status.eq(OutboxStatus::Pending.as_str()))
            .filter(outbox::Column::NextAttemptAt.lte(now))
            .order_by_asc(outbox::Column::CreatedAt)
            .order_by_asc(outbox::Column::Id)
            .limit(u64::from(limit))
            .all(self.conn())
            .await
            .map_err(db_err)?
            .into_iter()
            .map(to_entry)
            .collect()
    }

    async fn finish_outbox(&self, id: OutboxId, result: OutboxResult) -> PortResult<()> {
        let update = outbox::Entity::update_many().filter(outbox::Column::Id.eq(id.0));
        let update = match result {
            OutboxResult::Done { at } => update
                .col_expr(
                    outbox::Column::Status,
                    Expr::value(OutboxStatus::Done.as_str()),
                )
                .col_expr(outbox::Column::FinishedAt, Expr::value(Some(at))),
            OutboxResult::Retry {
                attempts,
                next_attempt_at,
            } => update
                .col_expr(
                    outbox::Column::Attempts,
                    Expr::value(i32::try_from(attempts).map_err(data_err)?),
                )
                .col_expr(outbox::Column::NextAttemptAt, Expr::value(next_attempt_at)),
            OutboxResult::Failed { attempts, at } => update
                .col_expr(
                    outbox::Column::Status,
                    Expr::value(OutboxStatus::Failed.as_str()),
                )
                .col_expr(
                    outbox::Column::Attempts,
                    Expr::value(i32::try_from(attempts).map_err(data_err)?),
                )
                .col_expr(outbox::Column::FinishedAt, Expr::value(Some(at))),
        };
        update.exec(self.conn()).await.map_err(db_err)?;
        Ok(())
    }

    async fn pending_outbox(&self) -> PortResult<u64> {
        outbox::Entity::find()
            .filter(outbox::Column::Status.eq(OutboxStatus::Pending.as_str()))
            .count(self.conn())
            .await
            .map_err(db_err)
    }

    async fn record_run(&self, job: &str, run: &JobRun) -> PortResult<()> {
        job_run::Entity::insert(job_run::ActiveModel {
            job: Set(job.to_owned()),
            last_started_at: Set(run.started_at),
            last_finished_at: Set(run.finished_at),
            last_outcome: Set(run.outcome.clone()),
        })
        .on_conflict(
            OnConflict::column(job_run::Column::Job)
                .update_columns([
                    job_run::Column::LastStartedAt,
                    job_run::Column::LastFinishedAt,
                    job_run::Column::LastOutcome,
                ])
                .to_owned(),
        )
        .exec_without_returning(self.conn())
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn last_run(&self, job: &str) -> PortResult<Option<JobRun>> {
        Ok(job_run::Entity::find_by_id(job.to_owned())
            .one(self.conn())
            .await
            .map_err(db_err)?
            .map(|m| JobRun {
                started_at: m.last_started_at,
                finished_at: m.last_finished_at,
                outcome: m.last_outcome,
            }))
    }

    async fn households_with_schedules(&self) -> PortResult<Vec<HouseholdId>> {
        let ids: Vec<uuid::Uuid> = meter::Entity::find()
            .select_only()
            .column(meter::Column::HouseholdId)
            .distinct()
            .filter(meter::Column::Archived.eq(false))
            .filter(
                Condition::any()
                    .add(meter::Column::ScheduleKind.is_not_null())
                    .add(meter::Column::ReadingIntervalDays.is_not_null()),
            )
            .into_tuple()
            .all(self.conn())
            .await
            .map_err(db_err)?;
        let mut ids: Vec<HouseholdId> = ids.into_iter().map(HouseholdId).collect();
        ids.sort();
        Ok(ids)
    }

    // @impl SPEC_NTF_EMAIL, SPEC_NTF_REMINDERS, SPEC_NTF_ALARMS
    async fn recipients(
        &self,
        household: HouseholdId,
        kind: NotificationKind,
    ) -> PortResult<Vec<Recipient>> {
        let members = membership::Entity::find()
            .filter(membership::Column::HouseholdId.eq(household.0))
            .all(self.conn())
            .await
            .map_err(db_err)?;
        let mut out = Vec::new();
        for m in members {
            let role: Role = m.role.parse().map_err(data_err)?;
            if kind == NotificationKind::Reminder && role < Role::Member {
                continue;
            }
            let Some(u) = user::Entity::find_by_id(m.user_id)
                .one(self.conn())
                .await
                .map_err(db_err)?
            else {
                continue;
            };
            let settings = notification_settings::Entity::find_by_id(m.user_id)
                .one(self.conn())
                .await
                .map_err(db_err)?;
            let wanted = match (kind, &settings) {
                (NotificationKind::Reminder, Some(s)) => s.reminder,
                (NotificationKind::Alarm, Some(s)) => s.alarm,
                _ => true,
            };
            let email = settings.and_then(|s| s.email).or(u.email);
            if let (true, Some(email)) = (wanted, email) {
                out.push(Recipient {
                    user_id: UserId(u.id),
                    email,
                    locale: u.locale,
                });
            }
        }
        out.sort_by_key(|r| r.user_id);
        Ok(out)
    }

    // @impl SPEC_NTF_REMINDERS, SPEC_NTF_ALARMS
    async fn log_once(&self, entry: &NotificationLogEntry) -> PortResult<bool> {
        let inserted = notification_log::Entity::insert(notification_log::ActiveModel {
            user_id: Set(entry.user_id.0),
            kind: Set(entry.kind.as_str().to_owned()),
            subject_key: Set(entry.subject_key.clone()),
            household_id: Set(entry.household_id.0),
            sent_at: Set(entry.sent_at),
        })
        .on_conflict(
            OnConflict::columns([
                notification_log::Column::UserId,
                notification_log::Column::Kind,
                notification_log::Column::SubjectKey,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(self.conn())
        .await;
        match inserted {
            Ok(n) => Ok(n == 1),
            Err(e) => match db_err(e) {
                PortError::Conflict(_) => Ok(false),
                other => Err(other),
            },
        }
    }

    async fn release_log(&self, entry: &NotificationLogEntry) -> PortResult<()> {
        notification_log::Entity::delete_many()
            .filter(notification_log::Column::UserId.eq(entry.user_id.0))
            .filter(notification_log::Column::Kind.eq(entry.kind.as_str()))
            .filter(notification_log::Column::SubjectKey.eq(entry.subject_key.as_str()))
            .exec(self.conn())
            .await
            .map_err(db_err)?;
        Ok(())
    }
}
