//! Port-Testsuite: jede Implementierung von `UnitOfWorkFactory` (In-Memory,
//! SQLite, PostgreSQL) muss sie bestehen. Aufruf aus den Tests des Adapters:
//! `zaehler_app::testing::suite::run(&factory).await`.

#![allow(clippy::many_single_char_names)]

use chrono::{DateTime, Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use zaehler_domain::{
    meter::{Direction, MeterKind},
    reading::ReadingSource,
};

use crate::{
    PortError, Role, Tenant,
    ids::{DeviceId, HouseholdId, MeterId, ReadingId, RegisterId, UserId},
    model::{DeviceData, HouseholdData, Identity, MeterData, Reading, RegisterData},
    port::UnitOfWorkFactory,
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
    let first = reading(&t, r, d, 10, 1234);
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
