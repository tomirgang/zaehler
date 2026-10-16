//! Outbox und Protokoll der Jobs (SPEC_JOB_OUTBOX, SPEC_JOB_SCHEDULER),
//! Versandprotokoll und Benachrichtigungseinstellungen (SPEC_NTF_EMAIL).

#![allow(clippy::wildcard_imports)]

use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261016_000009_jobs"
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum Outbox {
    Table,
    Id,
    HouseholdId,
    Kind,
    Payload,
    Status,
    Attempts,
    NextAttemptAt,
    CreatedAt,
    FinishedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum JobRuns {
    Table,
    Job,
    LastStartedAt,
    LastFinishedAt,
    LastOutcome,
}

#[derive(DeriveIden, Clone, Copy)]
enum NotificationLog {
    Table,
    UserId,
    Kind,
    SubjectKey,
    HouseholdId,
    SentAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum NotificationSettings {
    Table,
    UserId,
    Email,
    Reminder,
    Alarm,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Households {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

// @impl SPEC_JOB_OUTBOX, SPEC_JOB_SCHEDULER
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(Outbox::Table)
                .if_not_exists()
                .col(pk_uuid(Outbox::Id))
                .col(uuid(Outbox::HouseholdId))
                .col(string(Outbox::Kind))
                .col(text(Outbox::Payload))
                .col(string(Outbox::Status))
                .col(integer(Outbox::Attempts).default(0))
                .col(timestamp_with_time_zone(Outbox::NextAttemptAt))
                .col(timestamp_with_time_zone(Outbox::CreatedAt))
                .col(timestamp_with_time_zone_null(Outbox::FinishedAt))
                .foreign_key(
                    ForeignKey::create()
                        .from(Outbox::Table, Outbox::HouseholdId)
                        .to(Households::Table, Households::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_outbox_status_next_attempt_at")
                .table(Outbox::Table)
                .col(Outbox::Status)
                .col(Outbox::NextAttemptAt)
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(JobRuns::Table)
                .if_not_exists()
                .col(string(JobRuns::Job).primary_key())
                .col(timestamp_with_time_zone(JobRuns::LastStartedAt))
                .col(timestamp_with_time_zone(JobRuns::LastFinishedAt))
                .col(string(JobRuns::LastOutcome))
                .to_owned(),
        )
        .await?;
        // Ein Eintrag je Empfänger, Art und Schlüssel verhindert Doppelversand
        // auch bei mehreren Instanzen (SPEC_NTF_REMINDERS, SPEC_NTF_ALARMS).
        m.create_table(
            Table::create()
                .table(NotificationLog::Table)
                .if_not_exists()
                .col(uuid(NotificationLog::UserId))
                .col(string(NotificationLog::Kind))
                .col(string(NotificationLog::SubjectKey))
                .col(uuid(NotificationLog::HouseholdId))
                .col(timestamp_with_time_zone(NotificationLog::SentAt))
                .primary_key(
                    Index::create()
                        .col(NotificationLog::UserId)
                        .col(NotificationLog::Kind)
                        .col(NotificationLog::SubjectKey),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(NotificationLog::Table, NotificationLog::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(NotificationLog::Table, NotificationLog::HouseholdId)
                        .to(Households::Table, Households::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_notification_log_sent_at")
                .table(NotificationLog::Table)
                .col(NotificationLog::SentAt)
                .to_owned(),
        )
        .await?;
        m.create_table(
            Table::create()
                .table(NotificationSettings::Table)
                .if_not_exists()
                .col(pk_uuid(NotificationSettings::UserId))
                .col(string_null(NotificationSettings::Email))
                .col(boolean(NotificationSettings::Reminder).default(true))
                .col(boolean(NotificationSettings::Alarm).default(true))
                .col(timestamp_with_time_zone(NotificationSettings::UpdatedAt))
                .foreign_key(
                    ForeignKey::create()
                        .from(NotificationSettings::Table, NotificationSettings::UserId)
                        .to(Users::Table, Users::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(NotificationSettings::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(NotificationLog::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(JobRuns::Table).to_owned())
            .await?;
        m.drop_table(Table::drop().table(Outbox::Table).to_owned())
            .await
    }
}
