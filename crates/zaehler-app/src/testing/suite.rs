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
    meter::{Direction, MeterKind, MeterPurpose},
    reading::ReadingSource,
    schedule::ReadingSchedule,
};

use crate::{
    PortError, Role, Tenant,
    ids::{
        BlobKey, BlobKind, DeviceId, DraftId, HouseholdId, InvitationId, MeterId, OutboxId,
        ReadingId, RegisterId, RevisionId, UserId,
    },
    model::{
        DeviceData, Draft, DraftAnalysis, DraftOutcome, DraftStatus, HouseholdData, Identity,
        JobRun, MeterData, NewInvitation, OutboxEntry, OutboxEvent, OutboxResult, Reading,
        ReadingUpdate, Recognition, RegisterData, Revision, RevisionAction,
    },
    port::{BlobStore, JobRepository, MaintenanceRepository, UnitOfWorkFactory},
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
        schedule: Some(ReadingSchedule::EveryDays(30)),
        purpose: MeterPurpose::General,
        targets: crate::model::MeterTargets::default(),
        max_daily_rate: Some(Decimal::new(25_500, 3)),
        billing_start: None,
    };
    uow.meters().create(t, m, &data, t0()).await.expect("meter");
    let device = DeviceData {
        serial_number: Some(format!("S{}", id().simple())),
        installed_at: t0(),
        rollover_digits: Some(6),
        decimal_places: 1,
        parent_meter_id: None,
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
    members(f).await;
    invitations(f).await;
    revisions(f).await;
    contracts::contracts(f).await;
    vehicles_suite::vehicles(f).await;
    pv_suite::pv(f).await;
}

#[path = "suite_documents.rs"]
mod documents_suite;
pub use documents_suite::documents;

#[path = "suite_contracts.rs"]
mod contracts;

#[path = "suite_vehicles.rs"]
mod vehicles_suite;

#[path = "suite_pv.rs"]
mod pv_suite;

/// Korrektur und Revisionen: Tenant-Isolation, Eindeutigkeit, Erhalt nach
/// dem Löschen der Ablesung.
// @test SPEC_RDG_HISTORY, SPEC_SEC_TENANT_SCOPE
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn revisions(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "Revisionen").await;
    let other = tenant(f, "Fremd R").await;
    let (m, d, r) = meter(f, &t, "Strom R").await;
    let first = reading(&t, r, d, 1, 100);
    let second = reading(&t, r, d, 2, 200);
    let mut uow = f.begin().await.expect("begin");
    uow.readings()
        .insert(&t, &[first.clone(), second.clone()])
        .await
        .expect("insert");
    uow.commit().await.expect("commit");

    let update = ReadingUpdate {
        read_at: first.read_at + Duration::hours(1),
        value: Decimal::new(105, 1),
        note: None,
        updated_at: t0() + Duration::days(3),
    };
    let revision = Revision {
        id: RevisionId(id()),
        reading_id: first.id,
        meter_id: m,
        register_id: r,
        action: RevisionAction::Update,
        changed_by: t.user_id(),
        changed_at: update.updated_at,
        old_read_at: first.read_at,
        old_value: first.value,
        old_note: first.note.clone(),
        new_read_at: Some(update.read_at),
        new_value: Some(update.value),
        new_note: None,
    };
    // fremder Tenant: weder ändern noch Revision schreiben
    let mut uow = f.begin().await.expect("begin");
    assert!(matches!(
        uow.readings().update(&other, first.id, &update).await,
        Err(PortError::NotFound)
    ));
    drop(uow);
    let mut uow = f.begin().await.expect("begin");
    assert!(uow.revisions().append(&other, &revision).await.is_err());
    drop(uow);
    // gleicher Zeitpunkt wie eine andere Ablesung ist ein Konflikt
    let mut uow = f.begin().await.expect("begin");
    let clash = ReadingUpdate {
        read_at: second.read_at,
        ..update.clone()
    };
    assert!(matches!(
        uow.readings().update(&t, first.id, &clash).await,
        Err(PortError::Conflict(_))
    ));
    drop(uow);

    let mut uow = f.begin().await.expect("begin");
    uow.readings()
        .update(&t, first.id, &update)
        .await
        .expect("update");
    uow.revisions()
        .append(&t, &revision)
        .await
        .expect("revision");
    uow.readings().delete(&t, first.id).await.expect("delete");
    let deleted = Revision {
        id: RevisionId(id()),
        action: RevisionAction::Delete,
        changed_at: update.updated_at + Duration::hours(1),
        old_read_at: update.read_at,
        old_value: update.value,
        old_note: None,
        new_read_at: None,
        new_value: None,
        ..revision.clone()
    };
    uow.revisions()
        .append(&t, &deleted)
        .await
        .expect("revision");
    uow.commit().await.expect("commit");

    let mut uow = f.begin().await.expect("begin");
    let history = uow.revisions().for_meter(&t, m).await.expect("history");
    assert_eq!(
        history,
        [deleted, revision],
        "neueste zuerst, nach dem Löschen erhalten"
    );
    assert!(
        uow.revisions()
            .for_meter(&other, m)
            .await
            .expect("history")
            .is_empty()
    );
    let left = uow.readings().list_for_meter(&t, m).await.expect("list");
    assert_eq!(left.len(), 1);
    // Löschen des Haushalts entfernt die Revisionen
    uow.households().delete(&t).await.expect("delete");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert!(
        uow.revisions()
            .for_meter(&t, m)
            .await
            .expect("history")
            .is_empty()
    );
}

async fn user(f: &dyn UnitOfWorkFactory, name: &str) -> UserId {
    let mut uow = f.begin().await.expect("begin");
    let u = uow
        .users()
        .upsert_oidc(UserId(id()), &identity(name), t0())
        .await
        .expect("user");
    uow.commit().await.expect("commit");
    u.id
}

// @test SPEC_TEN_ROLES, SPEC_SEC_TENANT_SCOPE
async fn members(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "Mitglieder").await;
    let other = tenant(f, "Fremd").await;
    let bea = user(f, "Bea").await;
    let mut uow = f.begin().await.expect("begin");
    uow.memberships()
        .add(t.household_id(), bea, Role::Viewer)
        .await
        .expect("add");
    // erneutes Hinzufügen mit niedrigerer Rolle behält die höhere
    uow.memberships()
        .add(t.household_id(), bea, Role::Member)
        .await
        .expect("add");
    uow.memberships()
        .add(t.household_id(), bea, Role::Viewer)
        .await
        .expect("add");
    uow.memberships().lock_household(&t).await.expect("lock");
    let list = uow.memberships().list(&t).await.expect("list");
    assert_eq!(
        list.iter()
            .map(|m| (m.display_name.as_str(), m.role))
            .collect::<Vec<_>>(),
        [("Bea", Role::Member), ("Mitglieder", Role::Owner)]
    );
    uow.commit().await.expect("commit");

    let mut uow = f.begin().await.expect("begin");
    // fremder Tenant sieht und ändert nichts
    assert_eq!(uow.memberships().list(&other).await.expect("list").len(), 1);
    assert!(matches!(
        uow.memberships().set_role(&other, bea, Role::Owner).await,
        Err(PortError::NotFound)
    ));
    assert!(matches!(
        uow.memberships().remove(&other, bea).await,
        Err(PortError::NotFound)
    ));
    uow.memberships()
        .set_role(&t, bea, Role::Owner)
        .await
        .expect("role");
    assert_eq!(
        uow.memberships()
            .role(bea, t.household_id())
            .await
            .expect("role"),
        Some(Role::Owner)
    );
    uow.memberships().remove(&t, bea).await.expect("remove");
    assert_eq!(
        uow.memberships()
            .role(bea, t.household_id())
            .await
            .expect("role"),
        None
    );
    assert!(matches!(
        uow.memberships().remove(&t, bea).await,
        Err(PortError::NotFound)
    ));
    uow.commit().await.expect("commit");
}

fn new_invitation(t: &Tenant, expires_in: Duration) -> NewInvitation {
    NewInvitation {
        id: InvitationId(id()),
        role: Role::Member,
        token_hash: crate::secret::generate().hash,
        email: Some("bea@example.test".into()),
        created_by: t.user_id(),
        created_at: t0(),
        expires_at: t0() + expires_in,
    }
}

/// Einladungen: Tenant-Isolation, Ablauf, Widerruf und einmaliges Einlösen
/// auch bei zwei gleichzeitigen Transaktionen.
// @test SPEC_TEN_INVITES, SPEC_SEC_TENANT_SCOPE
async fn invitations(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "Einladend").await;
    let other = tenant(f, "Andere").await;
    let open = new_invitation(&t, Duration::days(7));
    let hash = open.token_hash;
    let expired = new_invitation(&t, Duration::hours(-1));
    let revoked = new_invitation(&t, Duration::days(7));
    let mut uow = f.begin().await.expect("begin");
    for i in [&open, &expired, &revoked] {
        uow.invitations().create(&t, i).await.expect("create");
    }
    uow.commit().await.expect("commit");
    // gleicher Hash ist ein Konflikt
    let mut uow = f.begin().await.expect("begin");
    let duplicate = NewInvitation {
        id: InvitationId(id()),
        ..open.clone()
    };
    assert!(uow.invitations().create(&t, &duplicate).await.is_err());
    drop(uow);

    let mut uow = f.begin().await.expect("begin");
    assert!(matches!(
        uow.invitations().revoke(&other, revoked.id, t0()).await,
        Err(PortError::NotFound)
    ));
    uow.invitations()
        .revoke(&t, revoked.id, t0())
        .await
        .expect("revoke");
    let listed = uow.invitations().list_open(&t, t0()).await.expect("list");
    assert_eq!(
        listed.iter().map(|i| i.id).collect::<Vec<_>>(),
        [open.id],
        "nur offene"
    );
    assert_eq!(listed[0].household_name, "Einladend");
    assert!(
        uow.invitations()
            .list_open(&other, t0())
            .await
            .expect("list")
            .is_empty()
    );
    let found = uow
        .invitations()
        .find_by_hash(&hash)
        .await
        .expect("find")
        .expect("vorhanden");
    assert_eq!(found.household_id, t.household_id());
    assert_eq!(found.email.as_deref(), Some("bea@example.test"));
    assert!(
        uow.invitations()
            .find_by_hash(&[0; 32])
            .await
            .expect("find")
            .is_none()
    );
    assert!(
        !uow.invitations()
            .mark_accepted(revoked.id, other.user_id(), t0())
            .await
            .expect("mark"),
        "widerrufen"
    );
    uow.commit().await.expect("commit");
    accept_once_concurrently(f, &t, hash, open.id).await;
}

/// Zwei Transaktionen lösen gleichzeitig ein: genau eine gelingt; danach
/// entfernt das Löschen des Haushalts die Einladung.
async fn accept_once_concurrently(
    f: &dyn UnitOfWorkFactory,
    t: &Tenant,
    hash: [u8; 32],
    open: InvitationId,
) {
    let (a, b) = (user(f, "A").await, user(f, "B").await);
    let mut first = f.begin().await.expect("begin");
    let mut second = f.begin().await.expect("begin");
    for uow in [&mut first, &mut second] {
        assert!(
            uow.invitations()
                .find_by_hash(&hash)
                .await
                .expect("find")
                .is_some()
        );
    }
    assert!(
        first
            .invitations()
            .mark_accepted(open, a, t0())
            .await
            .expect("mark")
    );
    first
        .memberships()
        .add(t.household_id(), a, Role::Member)
        .await
        .expect("add");
    first.commit().await.expect("commit");
    let second_ok = match second.invitations().mark_accepted(open, b, t0()).await {
        Ok(true) => second.commit().await.is_ok(),
        // SQLite: veralteter Lesestand kann nicht schreiben
        Ok(false) | Err(_) => false,
    };
    assert!(!second_ok, "zweites Einlösen gelingt nicht");
    let mut uow = f.begin().await.expect("begin");
    let accepted = uow
        .invitations()
        .find_by_hash(&hash)
        .await
        .expect("find")
        .expect("vorhanden");
    assert_eq!(accepted.accepted_by, Some(a));
    assert!(!accepted.is_open(t0()));
    assert_eq!(
        uow.memberships()
            .role(a, t.household_id())
            .await
            .expect("role"),
        Some(Role::Member)
    );
    // Löschen des Haushalts entfernt die Einladungen
    uow.households().delete(t).await.expect("delete");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert!(
        uow.invitations()
            .find_by_hash(&hash)
            .await
            .expect("find")
            .is_none()
    );
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
            schedule: None,
            purpose: MeterPurpose::General,
            targets: crate::model::MeterTargets::default(),
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
        parent_meter_id: None,
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
        analysis_started_at: None,
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
    drafts_queue(f, &t, &other, meter_id).await;
    drafts_expire(f, m, &t, [&open, &failed, &confirmed]).await;
}

/// Warteschlange der Mehrfach-Foto-Erfassung: Reihenfolge, Beanspruchung,
/// abgebrochene Analyse, Ergebnis.
// @test SPEC_AI_MULTI
async fn drafts_queue(f: &dyn UnitOfWorkFactory, t: &Tenant, other: &Tenant, meter: MeterId) {
    let queued = |hours| {
        let mut d = draft(t, t0() + Duration::hours(hours), None);
        d.status = DraftStatus::Queued;
        d.suggestion = None;
        d.model = None;
        d.duration_ms = None;
        d
    };
    let (first, second) = (queued(1), queued(2));
    {
        let mut uow = f.begin().await.expect("begin");
        uow.drafts().create(t, &second).await.expect("create");
        uow.drafts().create(t, &first).await.expect("create");
        uow.commit().await.expect("commit");
    }
    let mut uow = f.begin().await.expect("begin");
    let open: Vec<DraftId> = uow
        .drafts()
        .list_open(t)
        .await
        .expect("list")
        .iter()
        .map(|d| d.id)
        .collect();
    let pos = |id| open.iter().position(|x| *x == id).expect("offen");
    assert!(pos(first.id) < pos(second.id), "älteste zuerst");
    assert!(
        uow.drafts()
            .list_open(other)
            .await
            .expect("list")
            .is_empty()
    );

    let now = t0() + Duration::hours(3);
    let stale = now - Duration::minutes(5);
    let claim = |id, at: DateTime<Utc>| (id, at, at - Duration::minutes(5));
    let (id, at, before) = claim(first.id, now);
    assert!(uow.drafts().claim(t, id, at, before).await.expect("claim"));
    assert!(
        !uow.drafts()
            .claim(t, first.id, now, stale)
            .await
            .expect("claim"),
        "bereits in Analyse"
    );
    assert!(
        !uow.drafts()
            .claim(other, second.id, now, stale)
            .await
            .expect("claim"),
        "fremder Haushalt"
    );
    // abgebrochene Analyse wird nach Ablauf erneut übernommen
    let (id, at, before) = claim(first.id, now + Duration::minutes(10));
    assert!(uow.drafts().claim(t, id, at, before).await.expect("claim"));

    let analysis = DraftAnalysis {
        model: Some("test/model".into()),
        duration_ms: Some(1500),
        suggestion: draft(t, t0(), None).suggestion,
        matched_meter_id: Some(meter),
        status: DraftStatus::Pending,
    };
    uow.drafts()
        .record_analysis(t, first.id, &analysis)
        .await
        .expect("record");
    let done = uow
        .drafts()
        .get_open(t, first.id)
        .await
        .expect("get")
        .expect("offen");
    assert_eq!(
        (done.status, done.matched_meter_id, done.model.as_deref()),
        (DraftStatus::Pending, Some(meter), Some("test/model"))
    );
    assert_eq!(done.suggestion, analysis.suggestion);
    for id in [first.id, second.id] {
        assert!(
            matches!(
                uow.drafts().record_analysis(t, id, &analysis).await,
                Err(PortError::NotFound)
            ),
            "nur im Status analysing"
        );
    }
    uow.commit().await.expect("commit");
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

/// Outbox und Job-Protokoll: Eintrag nur mit Commit, Abholen in Reihenfolge,
/// Wiederholung, Abschluss und Aufräumen (SPEC_JOB_OUTBOX); danach
/// Empfänger und Versandprotokoll der Benachrichtigungen.
///
/// # Panics
/// Bei jeder verletzten Erwartung.
// @test SPEC_JOB_OUTBOX, SPEC_JOB_SCHEDULER
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
pub async fn jobs(f: &dyn UnitOfWorkFactory, j: &dyn JobRepository, m: &dyn MaintenanceRepository) {
    let t = tenant(f, "Outbox").await;
    let (meter_id, _, _) = meter(f, &t, "Strom O").await;
    let event = OutboxEvent::ReadingsRecorded {
        meter_id,
        reading_ids: vec![ReadingId(id())],
    };
    let now = Utc::now();
    // ohne Commit kein Eintrag
    {
        let mut uow = f.begin().await.expect("begin");
        uow.outbox()
            .enqueue(&t, OutboxId(id()), &event, now)
            .await
            .expect("enqueue");
    }
    let before = j.claim_outbox(now, 1000).await.expect("claim").len();
    let (first, second) = (OutboxId(id()), OutboxId(id()));
    let mut uow = f.begin().await.expect("begin");
    uow.outbox()
        .enqueue(&t, first, &event, now - Duration::seconds(2))
        .await
        .expect("enqueue");
    uow.outbox()
        .enqueue(&t, second, &event, now - Duration::seconds(1))
        .await
        .expect("enqueue");
    uow.commit().await.expect("commit");
    let ids = |v: Vec<OutboxEntry>| v.into_iter().map(|e| e.id).collect::<Vec<_>>();
    let claimed: Vec<OutboxId> = ids(j.claim_outbox(now, 1000).await.expect("claim"))
        .into_iter()
        .filter(|e| *e == first || *e == second)
        .collect();
    assert_eq!(claimed, [first, second], "älteste zuerst");
    assert_eq!(
        j.claim_outbox(now, 1000).await.expect("claim").len(),
        before + 2
    );
    let entry = j
        .claim_outbox(now, 1000)
        .await
        .expect("claim")
        .into_iter()
        .find(|e| e.id == first)
        .expect("erster");
    assert_eq!(entry.kind, "readings_recorded");
    assert_eq!(entry.household_id, t.household_id());
    assert_eq!(
        serde_json::from_str::<OutboxEvent>(&entry.payload).expect("json"),
        event
    );

    // Wiederholung später, Abschluss, Fehlschlag
    j.finish_outbox(
        first,
        OutboxResult::Retry {
            attempts: 1,
            next_attempt_at: now + Duration::minutes(1),
        },
    )
    .await
    .expect("retry");
    assert!(!ids(j.claim_outbox(now, 1000).await.expect("claim")).contains(&first));
    let later = j
        .claim_outbox(now + Duration::minutes(1), 1000)
        .await
        .expect("claim");
    assert_eq!(
        later.iter().find(|e| e.id == first).map(|e| e.attempts),
        Some(1)
    );
    j.finish_outbox(first, OutboxResult::Done { at: now })
        .await
        .expect("done");
    j.finish_outbox(
        second,
        OutboxResult::Failed {
            attempts: 10,
            at: now,
        },
    )
    .await
    .expect("failed");
    let open = ids(j
        .claim_outbox(now + Duration::days(1), 1000)
        .await
        .expect("claim"));
    assert!(!open.contains(&first) && !open.contains(&second));
    // erledigte Einträge nach der Aufbewahrung entfernen, fehlgeschlagene nicht
    assert!(
        m.purge_outbox(now + Duration::seconds(1))
            .await
            .expect("purge")
            >= 1
    );
    assert_eq!(
        m.purge_outbox(now + Duration::seconds(1))
            .await
            .expect("purge"),
        0
    );

    // Protokoll der Läufe
    let run = JobRun {
        started_at: now,
        finished_at: now + Duration::seconds(1),
        outcome: "ok".into(),
    };
    j.record_run("suite", &run).await.expect("run");
    let again = JobRun {
        outcome: "error".into(),
        ..run.clone()
    };
    j.record_run("suite", &again).await.expect("run");
    let last = j.last_run("suite").await.expect("last").expect("vorhanden");
    assert_eq!(last.outcome, "error");
    assert!((last.started_at - now).num_milliseconds().abs() < 1);
    assert!(j.last_run("unbekannt").await.expect("last").is_none());
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
