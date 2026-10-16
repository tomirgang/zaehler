//! Port-Testsuite: jede Implementierung von `UnitOfWorkFactory` (In-Memory,
//! SQLite, PostgreSQL) muss sie bestehen. Aufruf aus den Tests des Adapters:
//! `zaehler_app::testing::suite::run(&factory).await`.

#![allow(clippy::many_single_char_names)]

use std::task::Poll;

use bytes::Bytes;
use chrono::{DateTime, Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use zaehler_domain::{
    meter::{Direction, MeterKind},
    reading::ReadingSource,
};

use crate::{
    PortError, Role, Tenant,
    ids::{
        BlobKey, BlobKind, DeviceId, DraftId, HouseholdId, MeterId, ReadingId, RegisterId, UserId,
    },
    model::{
        DeviceData, Draft, DraftOutcome, DraftStatus, HouseholdData, Identity, MeterData, Reading,
        Recognition, RegisterData,
    },
    port::{BlobStore, MaintenanceRepository, UnitOfWorkFactory},
};

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .single()
        .unwrap_or_default()
}

fn id() -> Uuid {
    Uuid::now_v7()
}

fn identity(name: &str) -> Identity {
    Identity {
        issuer: "https://idp.test".into(),
        subject: id().to_string(),
        email: Some(format!("{name}@example.test")),
        display_name: name.into(),
        locale: "de".into(),
    }
}

async fn tenant(f: &dyn UnitOfWorkFactory, name: &str) -> Tenant {
    let mut uow = f.begin().await.expect("begin");
    let user = uow
        .users()
        .upsert_oidc(UserId(id()), &identity(name), t0())
        .await
        .expect("user");
    let household = HouseholdId(id());
    let data = HouseholdData {
        name: name.into(),
        timezone: chrono_tz::Europe::Berlin,
    };
    uow.households()
        .create(household, user.id, &data, t0())
        .await
        .expect("household");
    uow.commit().await.expect("commit");
    Tenant::new(user.id, household, Role::Owner)
}

async fn meter(
    f: &dyn UnitOfWorkFactory,
    t: &Tenant,
    name: &str,
) -> (MeterId, DeviceId, RegisterId) {
    let (m, d, r) = (MeterId(id()), DeviceId(id()), RegisterId(id()));
    let mut uow = f.begin().await.expect("begin");
    let data = MeterData {
        name: name.into(),
        kind: MeterKind::Electricity,
        unit: "kWh".into(),
        reading_interval_days: Some(30),
        max_daily_rate: Some(Decimal::new(25_500, 3)),
        billing_start: None,
    };
    uow.meters().create(t, m, &data, t0()).await.expect("meter");
    let device = DeviceData {
        serial_number: Some(format!("S{}", id().simple())),
        installed_at: t0(),
        rollover_digits: Some(6),
        decimal_places: 1,
    };
    uow.meters()
        .add_device(t, m, d, &device)
        .await
        .expect("device");
    let register = RegisterData {
        code: "main".into(),
        direction: Direction::Import,
    };
    uow.meters()
        .add_register(t, m, r, &register)
        .await
        .expect("register");
    uow.commit().await.expect("commit");
    (m, d, r)
}

fn reading(t: &Tenant, register: RegisterId, device: DeviceId, days: i64, value: i64) -> Reading {
    Reading {
        id: ReadingId(id()),
        register_id: register,
        device_id: device,
        read_at: t0() + Duration::days(days),
        value: Decimal::new(value, 1),
        source: ReadingSource::Manual,
        note: Some("Notiz".into()),
        photo_key: None,
        ai_confidence: None,
        created_by: Some(t.user_id()),
    }
}

/// Führt alle Prüfungen aus; schlägt per `panic` fehl.
// @test SPEC_ARCH_UOW, SPEC_SEC_TENANT_SCOPE
pub async fn run(f: &dyn UnitOfWorkFactory) {
    users(f).await;
    households(f).await;
    rollback_without_commit(f).await;
    tenant_isolation(f).await;
    readings(f).await;
    devices(f).await;
    household_delete_cascades(f).await;
}

async fn users(f: &dyn UnitOfWorkFactory) {
    let ident = identity("anna");
    let mut uow = f.begin().await.expect("begin");
    let first = uow
        .users()
        .upsert_oidc(UserId(id()), &ident, t0())
        .await
        .expect("upsert");
    let renamed = Identity {
        display_name: "Anna B.".into(),
        ..ident.clone()
    };
    let second = uow
        .users()
        .upsert_oidc(UserId(id()), &renamed, t0())
        .await
        .expect("upsert");
    assert_eq!(
        first.id, second.id,
        "upsert ist idempotent für (issuer, subject)"
    );
    assert_eq!(second.display_name, "Anna B.");
    uow.users()
        .set_locale(first.id, "en")
        .await
        .expect("locale");
    let loaded = uow
        .users()
        .get(first.id)
        .await
        .expect("get")
        .expect("vorhanden");
    assert_eq!(loaded.locale, "en");
    assert!(uow.users().get(UserId(id())).await.expect("get").is_none());
    uow.commit().await.expect("commit");
}

async fn households(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "Haushalt A").await;
    let mut uow = f.begin().await.expect("begin");
    assert_eq!(
        uow.memberships()
            .role(t.user_id(), t.household_id())
            .await
            .expect("role"),
        Some(Role::Owner)
    );
    let list = uow
        .memberships()
        .households_of(t.user_id())
        .await
        .expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Haushalt A");
    assert_eq!(list[0].timezone, chrono_tz::Europe::Berlin);
    let updated = HouseholdData {
        name: "Haushalt B".into(),
        timezone: chrono_tz::Europe::Vienna,
    };
    uow.households().update(&t, &updated).await.expect("update");
    let h = uow.households().get(&t).await.expect("get");
    assert_eq!(h.data, updated);
    uow.commit().await.expect("commit");
}

async fn rollback_without_commit(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "Rollback").await;
    {
        let mut uow = f.begin().await.expect("begin");
        let data = MeterData {
            name: "verworfen".into(),
            kind: MeterKind::Water,
            unit: "m³".into(),
            reading_interval_days: None,
            max_daily_rate: None,
            billing_start: None,
        };
        uow.meters()
            .create(&t, MeterId(id()), &data, t0())
            .await
            .expect("create");
        // Drop ohne commit
    }
    let mut uow = f.begin().await.expect("begin");
    assert!(
        uow.meters().list(&t).await.expect("list").is_empty(),
        "Rollback ohne commit"
    );
}

async fn tenant_isolation(f: &dyn UnitOfWorkFactory) {
    let a = tenant(f, "A").await;
    let b = tenant(f, "B").await;
    let (m, d, r) = meter(f, &a, "Strom A").await;
    let ra = reading(&a, r, d, 1, 100);
    {
        let mut uow = f.begin().await.expect("begin");
        uow.readings()
            .insert(&a, std::slice::from_ref(&ra))
            .await
            .expect("insert");
        uow.commit().await.expect("commit");
    }
    let mut uow = f.begin().await.expect("begin");
    assert!(uow.meters().list(&b).await.expect("list").is_empty());
    assert!(uow.meters().get(&b, m).await.expect("get").is_none());
    assert!(
        uow.readings()
            .list_for_meter(&b, m)
            .await
            .expect("list")
            .is_empty()
    );
    assert!(uow.readings().get(&b, ra.id).await.expect("get").is_none());
    assert!(
        uow.meters()
            .find_by_name(&b, "Strom A")
            .await
            .expect("name")
            .is_none()
    );
    assert_eq!(
        uow.memberships()
            .role(b.user_id(), a.household_id())
            .await
            .expect("role"),
        None
    );
    assert_eq!(
        uow.households().get(&b).await.expect("get").id,
        b.household_id()
    );
    drop(uow);

    // schreibende Zugriffe über fremden Tenant schlagen fehl (je eigene Transaktion)
    let mut uow = f.begin().await.expect("begin");
    let foreign = reading(&b, r, d, 2, 110);
    assert!(uow.readings().insert(&b, &[foreign]).await.is_err());
    drop(uow);
    let mut uow = f.begin().await.expect("begin");
    let reg = RegisterData {
        code: "HT".into(),
        direction: Direction::Import,
    };
    assert!(
        uow.meters()
            .add_register(&b, m, RegisterId(id()), &reg)
            .await
            .is_err()
    );
    drop(uow);
    let mut uow = f.begin().await.expect("begin");
    assert!(uow.readings().delete(&b, ra.id).await.is_err());
    drop(uow);
    let mut uow = f.begin().await.expect("begin");
    assert!(uow.meters().retire_device(&b, d, t0()).await.is_err());
    drop(uow);

    let mut uow = f.begin().await.expect("begin");
    assert_eq!(
        uow.readings()
            .list_for_meter(&a, m)
            .await
            .expect("list")
            .len(),
        1,
        "Daten von A unverändert"
    );
}

async fn readings(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "Ablesungen").await;
    let (m, d, r) = meter(f, &t, "Wasser").await;
    let mut first = reading(&t, r, d, 10, 1234);
    first.photo_key = Some(BlobKey::reading_photo(t.household_id(), first.id));
    first.source = ReadingSource::Photo;
    first.ai_confidence = Some(Decimal::new(875, 3));
    let second = reading(&t, r, d, 5, 1000);
    {
        let mut uow = f.begin().await.expect("begin");
        uow.readings()
            .insert(&t, &[first.clone(), second.clone()])
            .await
            .expect("insert");
        uow.commit().await.expect("commit");
    }
    {
        let mut uow = f.begin().await.expect("begin");
        let dup = Reading {
            id: ReadingId(id()),
            ..first.clone()
        };
        let err = uow
            .readings()
            .insert(&t, &[dup])
            .await
            .expect_err("Duplikat");
        assert!(
            matches!(err, PortError::Conflict(_)),
            "Duplikat ergibt Conflict: {err:?}"
        );
    }
    let mut uow = f.begin().await.expect("begin");
    let list = uow.readings().list_for_meter(&t, m).await.expect("list");
    assert_eq!(
        list,
        vec![second.clone(), first.clone()],
        "chronologisch, Werte exakt"
    );
    let (meter_id, loaded) = uow
        .readings()
        .get(&t, first.id)
        .await
        .expect("get")
        .expect("vorhanden");
    assert_eq!((meter_id, loaded), (m, first.clone()));
    let summary = uow.meters().list(&t).await.expect("list");
    assert_eq!(summary[0].last_reading_at, Some(first.read_at));
    uow.readings().delete(&t, second.id).await.expect("delete");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert_eq!(
        uow.readings().list_for_meter(&t, m).await.expect("list"),
        vec![first]
    );
}

async fn devices(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "Geräte").await;
    let (m, d, r) = meter(f, &t, "Gas").await;
    let new_device = DeviceId(id());
    let mut uow = f.begin().await.expect("begin");
    let agg = uow
        .meters()
        .get(&t, m)
        .await
        .expect("get")
        .expect("vorhanden");
    let serial = agg.devices[0]
        .data
        .serial_number
        .clone()
        .expect("Seriennummer");
    assert_eq!(
        uow.meters()
            .find_by_serial(&t, &serial)
            .await
            .expect("serial"),
        Some((m, d))
    );
    assert_eq!(agg.registers[0].id, r);
    uow.meters()
        .retire_device(&t, d, t0() + Duration::days(30))
        .await
        .expect("retire");
    let data = DeviceData {
        serial_number: Some("NEU-1".into()),
        installed_at: t0() + Duration::days(30),
        rollover_digits: None,
        decimal_places: 3,
    };
    uow.meters()
        .add_device(&t, m, new_device, &data)
        .await
        .expect("add");
    uow.meters()
        .rename_register(&t, r, "HT")
        .await
        .expect("rename");
    uow.commit().await.expect("commit");

    let mut uow = f.begin().await.expect("begin");
    assert!(
        uow.meters()
            .find_by_serial(&t, &serial)
            .await
            .expect("serial")
            .is_none(),
        "ausgebautes Gerät"
    );
    let agg = uow
        .meters()
        .get(&t, m)
        .await
        .expect("get")
        .expect("vorhanden");
    assert_eq!(agg.devices.len(), 2);
    assert_eq!(agg.devices[0].removed_at, Some(t0() + Duration::days(30)));
    assert_eq!(agg.active_device().map(|x| x.id), Some(new_device));
    assert_eq!(agg.active_device().map(|x| x.data.clone()), Some(data));
    assert_eq!(agg.registers[0].data.code, "HT");
    assert_eq!(agg.meter.data.max_daily_rate, Some(Decimal::new(25_500, 3)));
}

async fn household_delete_cascades(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "Löschen").await;
    let (_, d, r) = meter(f, &t, "Strom").await;
    {
        let mut uow = f.begin().await.expect("begin");
        uow.readings()
            .insert(&t, &[reading(&t, r, d, 1, 5)])
            .await
            .expect("insert");
        uow.households().delete(&t).await.expect("delete");
        uow.commit().await.expect("commit");
    }
    let mut uow = f.begin().await.expect("begin");
    assert!(matches!(
        uow.households().get(&t).await,
        Err(PortError::NotFound)
    ));
    assert!(
        uow.memberships()
            .households_of(t.user_id())
            .await
            .expect("list")
            .is_empty()
    );
    assert!(uow.meters().list(&t).await.expect("list").is_empty());
}

fn draft(t: &Tenant, created_at: DateTime<Utc>, meter: Option<MeterId>) -> Draft {
    let id = DraftId(id());
    Draft {
        id,
        created_by: t.user_id(),
        photo_key: BlobKey::draft_photo(t.household_id(), id),
        model: Some("test/model".into()),
        duration_ms: Some(1234),
        suggestion: Some(Recognition {
            value: Some(Decimal::new(123_456, 3)),
            serial_number: Some("1EMH-0012".into()),
            register: Some("1.8.0".into()),
            unit: Some("kWh".into()),
            meter_kind: Some(MeterKind::Electricity),
            confidence: Decimal::new(91, 2),
        }),
        matched_meter_id: meter,
        status: DraftStatus::Pending,
        created_at,
        taken_at: Some(created_at - Duration::minutes(3)),
    }
}

/// Entwürfe der Foto-Erfassung und ihr Ablauf; für jede Implementierung von
/// `UnitOfWorkFactory` mit zugehörigem `MaintenanceRepository`.
///
/// # Panics
/// Bei jeder verletzten Erwartung.
// @test SPEC_AI_DRAFTS, SPEC_SEC_TENANT_SCOPE, SPEC_OPS_HOUSEKEEPING
pub async fn drafts(f: &dyn UnitOfWorkFactory, m: &dyn MaintenanceRepository) {
    let t = tenant(f, "Entwürfe").await;
    let other = tenant(f, "Fremd").await;
    let (meter_id, device, register) = meter(f, &t, "Strom").await;
    let saved = reading(&t, register, device, 1, 1000);
    let open = draft(&t, t0(), Some(meter_id));
    let mut failed = draft(&t, t0() + Duration::days(2), None);
    failed.status = DraftStatus::Failed;
    failed.suggestion = None;
    failed.model = None;
    failed.duration_ms = None;
    failed.taken_at = None;
    let confirmed = draft(&t, t0(), None);
    {
        let mut uow = f.begin().await.expect("begin");
        for d in [&open, &failed, &confirmed] {
            uow.drafts().create(&t, d).await.expect("create");
        }
        // Foto unter fremdem Haushalt wird abgelehnt
        let foreign = draft(&other, t0(), None);
        assert!(uow.drafts().create(&t, &foreign).await.is_err());
        uow.commit().await.expect("commit");
    }
    let mut uow = f.begin().await.expect("begin");
    assert_eq!(
        uow.drafts().get_open(&t, open.id).await.expect("get"),
        Some(open.clone()),
        "Vorschlag vollständig gespeichert"
    );
    assert_eq!(
        uow.drafts().get_open(&t, failed.id).await.expect("get"),
        Some(failed.clone())
    );
    assert_eq!(
        uow.drafts().get_open(&other, open.id).await.expect("get"),
        None,
        "Entwurf nur im eigenen Haushalt"
    );
    assert!(matches!(
        uow.drafts()
            .finish(&other, open.id, DraftOutcome::Discarded, t0())
            .await,
        Err(PortError::NotFound)
    ));
    uow.readings()
        .insert(&t, std::slice::from_ref(&saved))
        .await
        .expect("reading");
    uow.drafts()
        .finish(
            &t,
            confirmed.id,
            DraftOutcome::Confirmed {
                reading: saved.id,
                corrected: Some(true),
            },
            t0(),
        )
        .await
        .expect("finish");
    assert!(matches!(
        uow.drafts()
            .finish(&t, confirmed.id, DraftOutcome::Discarded, t0())
            .await,
        Err(PortError::NotFound)
    ));
    assert_eq!(
        uow.drafts().get_open(&t, confirmed.id).await.expect("get"),
        None
    );
    uow.commit().await.expect("commit");
    drafts_expire(f, m, &t, [&open, &failed, &confirmed]).await;
}

/// Ablauf offener Entwürfe und Löschen mit dem Haushalt.
async fn drafts_expire(
    f: &dyn UnitOfWorkFactory,
    m: &dyn MaintenanceRepository,
    t: &Tenant,
    [open, failed, confirmed]: [&Draft; 3],
) {
    // nur offene Entwürfe vor dem Stichtag, idempotent
    let cutoff = t0() + Duration::days(1);
    let (a, b) = join(
        m.expire_drafts(cutoff, t0() + Duration::days(3)),
        m.expire_drafts(cutoff, t0() + Duration::days(3)),
    )
    .await;
    let mut expired = a.expect("expire");
    expired.extend(b.expect("expire"));
    expired.dedup();
    assert!(expired.contains(&open.photo_key));
    assert!(!expired.contains(&failed.photo_key));
    assert!(!expired.contains(&confirmed.photo_key));
    assert!(
        !m.expire_drafts(cutoff, t0())
            .await
            .expect("expire")
            .contains(&open.photo_key)
    );
    let mut uow = f.begin().await.expect("begin");
    assert_eq!(uow.drafts().get_open(t, open.id).await.expect("get"), None);
    assert!(
        uow.drafts()
            .get_open(t, failed.id)
            .await
            .expect("get")
            .is_some()
    );

    // Haushalt löschen entfernt Entwürfe
    uow.households().delete(t).await.expect("delete");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert_eq!(
        uow.drafts().get_open(t, failed.id).await.expect("get"),
        None
    );
}

/// Prüfungen für jede Implementierung von `BlobStore` (In-Memory,
/// Dateisystem, S3). Zeitstempel müssen der Systemzeit folgen.
///
/// # Panics
/// Bei jeder verletzten Erwartung.
// @test SPEC_STORE_BLOBS, SPEC_OPS_HOUSEKEEPING
pub async fn blob_store(s: &dyn BlobStore) {
    let a = HouseholdId(id());
    let b = HouseholdId(id());
    let photo = BlobKey::reading_photo(a, ReadingId(id()));
    let copy = BlobKey::draft_photo(a, DraftId(id()));
    let tmp_a = BlobKey::tmp(a, id());
    let tmp_b = BlobKey::tmp(b, id());
    let other = BlobKey::reading_photo(b, ReadingId(id()));

    assert!(matches!(s.get(&photo).await, Err(PortError::NotFound)));
    s.put(&photo, Bytes::from_static(b"jpeg-1"))
        .await
        .expect("put");
    s.put(&photo, Bytes::from_static(b"jpeg-2"))
        .await
        .expect("overwrite");
    assert_eq!(s.get(&photo).await.expect("get"), &b"jpeg-2"[..]);
    s.copy(&photo, &copy).await.expect("copy");
    assert_eq!(s.get(&copy).await.expect("get copy"), &b"jpeg-2"[..]);
    assert!(matches!(
        s.copy(&BlobKey::tmp(a, id()), &copy).await,
        Err(PortError::NotFound)
    ));

    // delete ist idempotent, auch bei nebenläufigem Aufruf
    let (x, y) = join(s.delete(&copy), s.delete(&copy)).await;
    x.expect("delete");
    y.expect("delete concurrently");
    s.delete(&copy).await.expect("delete missing");
    assert!(matches!(s.get(&copy).await, Err(PortError::NotFound)));

    s.put(&tmp_a, Bytes::from_static(b"a;b"))
        .await
        .expect("put tmp");
    s.put(&tmp_b, Bytes::from_static(b"a;b"))
        .await
        .expect("put tmp");
    s.put(&other, Bytes::from_static(b"jpeg"))
        .await
        .expect("put other");
    let listed = s
        .list_tmp_older_than(Utc::now() + Duration::minutes(1))
        .await
        .expect("list");
    assert!(listed.contains(&tmp_a) && listed.contains(&tmp_b));
    assert!(listed.iter().all(|k| k.kind() == BlobKind::Tmp));
    let listed = s
        .list_tmp_older_than(Utc::now() - Duration::hours(1))
        .await
        .expect("list");
    assert!(!listed.contains(&tmp_a) && !listed.contains(&tmp_b));

    // Haushalt löschen entfernt nur dessen Objekte, auch nebenläufig
    let (x, y) = join(s.delete_household(a), s.delete_household(a)).await;
    x.expect("delete household");
    y.expect("delete household concurrently");
    assert!(matches!(s.get(&photo).await, Err(PortError::NotFound)));
    assert!(matches!(s.get(&tmp_a).await, Err(PortError::NotFound)));
    assert_eq!(s.get(&other).await.expect("other household"), &b"jpeg"[..]);
    s.delete_household(a).await.expect("delete empty household");
    s.delete_household(b).await.expect("cleanup");
    assert!(matches!(s.get(&tmp_b).await, Err(PortError::NotFound)));
}

/// Führt zwei Futures nebenläufig aus, ohne Abhängigkeit zu einer Runtime.
///
/// # Panics
/// Nie; beide Ergebnisse liegen beim Abschluss vor.
pub async fn join<A: Future, B: Future>(a: A, b: B) -> (A::Output, B::Output) {
    let mut a = std::pin::pin!(a);
    let mut b = std::pin::pin!(b);
    let mut ra = None;
    let mut rb = None;
    std::future::poll_fn(|cx| {
        if ra.is_none()
            && let Poll::Ready(v) = a.as_mut().poll(cx)
        {
            ra = Some(v);
        }
        if rb.is_none()
            && let Poll::Ready(v) = b.as_mut().poll(cx)
        {
            rb = Some(v);
        }
        if ra.is_some() && rb.is_some() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    (ra.expect("a"), rb.expect("b"))
}
