//! Tests: Erinnerungen, Abweichungsalarme und Einladungen per E-Mail
//! (SPEC_NTF_EMAIL, SPEC_NTF_REMINDERS, SPEC_NTF_ALARMS).

use std::sync::{Arc, atomic::Ordering};

use chrono::{Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use zaehler_domain::{
    meter::{Direction, MeterKind, MeterPurpose},
    reading::ReadingSource,
    schedule::ReadingSchedule,
};

use crate::{
    AppError, Role, Tenant,
    ids::MeterId,
    model::{Identity, MeterTargets, Notification, NotificationSettings},
    testing::{FixedClock, RecordingNotifier, memory_ports},
    usecase::{
        Ports, UseCases,
        jobs::{DispatchOutbox, SendReminders},
        member::InvitationInput,
        meter::{MeterInput, RegisterInput},
        reading::{CorrectionInput, ReadingsInput},
    },
};

struct Env {
    uc: UseCases,
    t: Tenant,
    ports: Ports,
    clock: Arc<FixedClock>,
    mail: Arc<RecordingNotifier>,
}

/// Haushalt in Europe/Berlin, Uhr 2026-01-15 12:00 UTC (13:00 lokal).
async fn env() -> Env {
    let (mut ports, _, clock) = memory_ports();
    let mail = Arc::new(RecordingNotifier::default());
    ports.notifier = mail.clone();
    let uc = UseCases::new(&ports);
    let identity = Identity {
        issuer: "dev".into(),
        subject: "anna".into(),
        email: Some("anna@example.test".into()),
        display_name: "Anna".into(),
        locale: "de".into(),
    };
    let user = uc.sign_in.execute(&identity).await.unwrap().user.id;
    let t = uc.resolve_tenant.execute(user, None).await.unwrap().tenant;
    Env {
        uc,
        t,
        ports,
        clock,
        mail,
    }
}

async fn meter(
    e: &Env,
    days_ago: i64,
    schedule: Option<ReadingSchedule>,
    targets: MeterTargets,
) -> MeterId {
    e.uc.create_meter
        .execute(
            &e.t,
            MeterInput {
                name: format!("Strom {days_ago} {}", schedule.is_some()),
                kind: MeterKind::Electricity,
                unit: String::new(),
                schedule,
                max_daily_rate: None,
                billing_start: None,
                purpose: MeterPurpose::General,
                parent_meter_id: None,
                targets,
                serial_number: None,
                rollover_digits: None,
                decimal_places: 0,
                registers: vec![RegisterInput {
                    id: None,
                    code: "main".into(),
                    direction: Direction::Import,
                    start_value: Some(dec!(0)),
                }],
                start_at: e.ports.clock.now() - Duration::days(days_ago),
            },
        )
        .await
        .unwrap()
}

async fn record(e: &Env, meter: MeterId, days_ago: i64, value: Decimal) -> crate::ids::ReadingId {
    let register =
        e.uc.get_meter
            .execute(&e.t, meter)
            .await
            .unwrap()
            .aggregate
            .registers[0]
            .id;
    let crate::usecase::reading::RecordOutcome::Saved(ids) =
        e.uc.record_readings
            .execute(
                &e.t,
                &ReadingsInput {
                    meter_id: meter,
                    at: e.ports.clock.now() - Duration::days(days_ago),
                    values: vec![(register, value)],
                    note: None,
                },
                true,
                ReadingSource::Manual,
            )
            .await
            .unwrap()
    else {
        panic!("gespeichert erwartet");
    };
    ids[0]
}

// @test SPEC_NTF_REMINDERS, SPEC_RDG_SCHEDULE
#[tokio::test]
async fn reminder_once_per_day_from_eight_local() {
    let e = env().await;
    meter(
        &e,
        40,
        Some(ReadingSchedule::EveryDays(30)),
        MeterTargets::default(),
    )
    .await;
    // Messstelle ohne Plan erscheint nicht
    meter(&e, 40, None, MeterTargets::default()).await;
    let job = SendReminders::new(e.ports.clone());
    let report = job.execute().await.unwrap();
    assert_eq!((report.households, report.sent), (1, 1));
    let sent = e.mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "anna@example.test");
    assert_eq!(sent[0].2, "de");
    let Notification::Reminder { meters, .. } = &sent[0].1 else {
        panic!("Erinnerung erwartet");
    };
    assert_eq!(meters.len(), 1);
    assert!(meters[0].overdue);
    assert!(meters[0].link.ends_with("/readings/new"));
    // am selben Tag kein zweites Mal
    assert_eq!(job.execute().await.unwrap().sent, 0);
    // am Folgetag vor 08:00 lokal nichts, ab 08:00 wieder
    e.clock
        .advance(Utc.with_ymd_and_hms(2026, 1, 16, 6, 59, 0).unwrap() - e.ports.clock.now());
    assert_eq!(job.execute().await.unwrap().sent, 0);
    e.clock.advance(Duration::minutes(1));
    assert_eq!(job.execute().await.unwrap().sent, 1);
    assert_eq!(e.mail.sent().len(), 2);
}

// @test SPEC_NTF_REMINDERS, SPEC_NTF_EMAIL
#[tokio::test]
async fn reminder_respects_settings_and_roles() {
    let e = env().await;
    meter(
        &e,
        40,
        Some(ReadingSchedule::EveryDays(30)),
        MeterTargets::default(),
    )
    .await;
    e.uc.update_notification_settings
        .execute(
            &e.t,
            NotificationSettings {
                email: Some("  anna@privat.test ".into()),
                reminder: false,
                alarm: true,
            },
        )
        .await
        .unwrap();
    let view = e.uc.get_notification_settings.execute(&e.t).await.unwrap();
    assert!(view.enabled);
    assert_eq!(view.settings.email.as_deref(), Some("anna@privat.test"));
    assert_eq!(view.login_email.as_deref(), Some("anna@example.test"));
    assert_eq!(
        SendReminders::new(e.ports.clone())
            .execute()
            .await
            .unwrap()
            .sent,
        0
    );
    match e
        .uc
        .update_notification_settings
        .execute(
            &e.t,
            NotificationSettings {
                email: Some("kein-at".into()),
                ..NotificationSettings::default()
            },
        )
        .await
    {
        Err(AppError::Validation(f)) => assert!(!f.for_field("email").is_empty()),
        other => panic!("{other:?}"),
    }
    // Leserechte erhalten keine Erinnerung
    let viewer = Tenant::new(e.t.user_id(), e.t.household_id(), Role::Viewer);
    e.uc.update_notification_settings
        .execute(&viewer, NotificationSettings::default())
        .await
        .unwrap();
    assert_eq!(
        SendReminders::new(e.ports.clone())
            .execute()
            .await
            .unwrap()
            .sent,
        1
    );
}

/// Jahresziel 3650 (10 je Tag), Toleranz 20 %: 50 in 5 Tagen ist grün, 60
/// in 2 Tagen rot.
// @test SPEC_NTF_ALARMS, SPEC_JOB_OUTBOX
#[tokio::test]
async fn alarm_once_per_reading_when_red() {
    let e = env().await;
    let targets = MeterTargets {
        season_weights: None,
        annual_target: Some(dec!(3650)),
        tolerance_pct: Some(20),
    };
    let m = meter(&e, 10, None, targets).await;
    let dispatch = DispatchOutbox::new(e.ports.clone());
    record(&e, m, 5, dec!(50)).await;
    assert_eq!(dispatch.execute().await.unwrap().done, 1);
    assert!(e.mail.sent().is_empty(), "grün: kein Alarm");

    let red = record(&e, m, 3, dec!(110)).await;
    assert_eq!(dispatch.execute().await.unwrap().done, 1);
    let sent = e.mail.sent();
    assert_eq!(sent.len(), 1);
    let Notification::ConsumptionAlarm {
        consumption,
        expected_upper,
        link,
        ..
    } = &sent[0].1
    else {
        panic!("Alarm erwartet");
    };
    assert_eq!(*consumption, dec!(60));
    assert!(*expected_upper < dec!(60));
    assert!(link.ends_with(&format!("/meters/{m}")));

    // Korrektur derselben Ablesung: kein zweiter Alarm
    e.uc.correct_reading
        .execute(
            &e.t,
            red,
            &CorrectionInput {
                at: e.ports.clock.now() - Duration::days(3),
                value: dec!(111),
                note: None,
            },
            true,
        )
        .await
        .unwrap();
    assert_eq!(dispatch.execute().await.unwrap().done, 1);
    assert_eq!(e.mail.sent().len(), 1);
}

/// Ein SMTP-Fehler verschiebt den Eintrag; der nächste Versuch versendet.
// @test SPEC_NTF_ALARMS, SPEC_JOB_OUTBOX
#[tokio::test]
async fn alarm_retried_after_smtp_error() {
    let e = env().await;
    let targets = MeterTargets {
        season_weights: None,
        annual_target: Some(dec!(3650)),
        tolerance_pct: Some(20),
    };
    let m = meter(&e, 10, None, targets).await;
    record(&e, m, 2, dec!(200)).await;
    e.mail.fail.store(true, Ordering::SeqCst);
    let dispatch = DispatchOutbox::new(e.ports.clone());
    assert_eq!(dispatch.execute().await.unwrap().retried, 1);
    e.mail.fail.store(false, Ordering::SeqCst);
    e.clock.advance(Duration::minutes(1));
    assert_eq!(dispatch.execute().await.unwrap().done, 1);
    assert_eq!(e.mail.sent().len(), 1);
}

// @test SPEC_TEN_INVITES, SPEC_NTF_EMAIL
#[tokio::test]
async fn invitation_emailed_with_link() {
    let e = env().await;
    let created =
        e.uc.create_invitation
            .execute(
                &e.t,
                InvitationInput {
                    role: Role::Member,
                    email: Some("ben@example.test".into()),
                },
            )
            .await
            .unwrap();
    assert!(created.emailed);
    DispatchOutbox::new(e.ports.clone())
        .execute()
        .await
        .unwrap();
    let sent = e.mail.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "ben@example.test");
    let Notification::Invitation {
        link,
        inviter,
        role,
        ..
    } = &sent[0].1
    else {
        panic!("Einladung erwartet");
    };
    assert_eq!(
        link,
        &format!("http://localhost:8080/invite/{}", created.token)
    );
    assert_eq!(inviter, "Anna");
    assert_eq!(*role, Role::Member);

    // widerrufene Einladung wird nicht mehr versendet
    let second =
        e.uc.create_invitation
            .execute(
                &e.t,
                InvitationInput {
                    role: Role::Viewer,
                    email: Some("cleo@example.test".into()),
                },
            )
            .await
            .unwrap();
    e.uc.revoke_invitation
        .execute(&e.t, second.id)
        .await
        .unwrap();
    DispatchOutbox::new(e.ports.clone())
        .execute()
        .await
        .unwrap();
    assert_eq!(e.mail.sent().len(), 1);
}

/// Ohne SMTP: keine Erinnerungen, Einladungen nur als Link.
// @test SPEC_NTF_EMAIL
#[tokio::test]
async fn without_smtp_nothing_is_sent() {
    let (ports, _, _) = memory_ports();
    assert!(!ports.notifier.enabled());
    let report = SendReminders::new(ports.clone()).execute().await.unwrap();
    assert_eq!(report.sent, 0);
}
