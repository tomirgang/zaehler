//! Tests: API-Tokens und automatisch gelieferte Zählerstände
//! (SPEC_API_AUTH, SPEC_API_DAILY_READINGS).

use chrono::Duration;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use zaehler_domain::{
    meter::{Direction, MeterKind},
    reading::ReadingSource,
};

use super::contract_tests::{berlin, field, meter_input, record};
use crate::{
    AppError, Role, Tenant,
    ids::{MeterId, RegisterId},
    model::{Identity, OutboxEvent},
    testing::memory_ports,
    usecase::{
        UseCases,
        api::{ApiTokenInput, IngestItem, IngestResult, IngestTarget},
        member::InvitationInput,
    },
};

fn identity(subject: &str) -> Identity {
    Identity {
        issuer: "dev".into(),
        subject: subject.into(),
        email: None,
        display_name: subject.into(),
        locale: "de".into(),
    }
}

async fn owner(uc: &UseCases, name: &str) -> Tenant {
    let signed = uc.sign_in.execute(&identity(name)).await.unwrap();
    uc.resolve_tenant
        .execute(signed.user.id, None)
        .await
        .unwrap()
        .tenant
}

async fn join(uc: &UseCases, t: &Tenant, name: &str, role: Role) -> Tenant {
    let u = uc.sign_in.execute(&identity(name)).await.unwrap().user.id;
    let inv = uc
        .create_invitation
        .execute(t, InvitationInput { role, email: None })
        .await
        .unwrap();
    let household = uc.accept_invitation.execute(u, &inv.token).await.unwrap();
    uc.resolve_tenant
        .execute(u, Some(household))
        .await
        .unwrap()
        .tenant
}

fn input(name: &str, role: Role) -> ApiTokenInput {
    ApiTokenInput {
        name: name.into(),
        role,
        expires_at: None,
    }
}

#[allow(clippy::needless_pass_by_value)] // nimmt das Ergebnis des Aufrufs direkt
fn unauthenticated<T: std::fmt::Debug>(r: Result<T, AppError>) {
    assert!(matches!(r, Err(AppError::Unauthenticated)), "{r:?}");
}

// @test SPEC_API_AUTH
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn tokens_bind_household_and_role() {
    let (ports, _, clock) = memory_ports();
    let uc = UseCases::new(&ports);
    let anna = owner(&uc, "anna").await;
    let bea = join(&uc, &anna, "bea", Role::Member).await;

    // Eingaben
    field(
        uc.create_api_token
            .execute(&bea, input("HA", Role::Owner))
            .await,
        "role",
        "api.role_too_high",
    );
    field(
        uc.create_api_token
            .execute(&bea, input("  ", Role::Viewer))
            .await,
        "name",
        "api.name_invalid",
    );
    let past = ApiTokenInput {
        expires_at: Some(crate::port::Clock::now(clock.as_ref()) - Duration::hours(1)),
        ..input("HA", Role::Member)
    };
    field(
        uc.create_api_token.execute(&bea, past).await,
        "expires_at",
        "api.expiry_past",
    );

    // Erzeugen und Auflösen
    let own = uc
        .create_api_token
        .execute(&bea, input("Home Assistant", Role::Member))
        .await
        .unwrap();
    assert!(own.token.starts_with("zt_"));
    let p = uc.resolve_api_tenant.execute(&own.token).await.unwrap();
    assert_eq!(p.token, own.id);
    assert_eq!(p.tenant.household_id(), anna.household_id());
    assert_eq!(p.tenant.user_id(), bea.user_id());
    assert_eq!(p.tenant.role(), Role::Member);
    unauthenticated(uc.resolve_api_tenant.execute("zt_kurz").await);
    unauthenticated(
        uc.resolve_api_tenant
            .execute(&format!("zt_{}", "A".repeat(43)))
            .await,
    );

    // last_used_at höchstens einmal je Minute
    let used = |list: Vec<crate::model::ApiToken>| {
        list.into_iter()
            .find(|x| x.id == own.id)
            .and_then(|x| x.last_used_at)
    };
    let first = used(uc.list_api_tokens.execute(&bea).await.unwrap()).unwrap();
    clock.advance(Duration::seconds(30));
    uc.resolve_api_tenant.execute(&own.token).await.unwrap();
    assert_eq!(
        used(uc.list_api_tokens.execute(&bea).await.unwrap()),
        Some(first)
    );
    clock.advance(Duration::seconds(31));
    uc.resolve_api_tenant.execute(&own.token).await.unwrap();
    assert!(used(uc.list_api_tokens.execute(&bea).await.unwrap()).unwrap() > first);

    // Rolle sinkt mit der Mitgliedsrolle
    uc.change_role
        .execute(&anna, bea.user_id(), Role::Viewer)
        .await
        .unwrap();
    let p = uc.resolve_api_tenant.execute(&own.token).await.unwrap();
    assert_eq!(p.tenant.role(), Role::Viewer);

    // Sichtbarkeit und Widerruf
    let annas = uc
        .create_api_token
        .execute(&anna, input("Skript", Role::Owner))
        .await
        .unwrap();
    assert_eq!(uc.list_api_tokens.execute(&bea).await.unwrap().len(), 1);
    assert_eq!(uc.list_api_tokens.execute(&anna).await.unwrap().len(), 2);
    assert!(matches!(
        uc.revoke_api_token.execute(&bea, annas.id).await,
        Err(AppError::NotFound)
    ));
    uc.revoke_api_token.execute(&anna, own.id).await.unwrap();
    unauthenticated(uc.resolve_api_tenant.execute(&own.token).await);
    assert!(matches!(
        uc.revoke_api_token.execute(&anna, own.id).await,
        Err(AppError::NotFound)
    ));

    // Ablauf
    let expiring = uc
        .create_api_token
        .execute(
            &anna,
            ApiTokenInput {
                expires_at: Some(crate::port::Clock::now(clock.as_ref()) + Duration::hours(1)),
                ..input("Kurz", Role::Viewer)
            },
        )
        .await
        .unwrap();
    uc.resolve_api_tenant
        .execute(&expiring.token)
        .await
        .unwrap();
    clock.advance(Duration::hours(2));
    unauthenticated(uc.resolve_api_tenant.execute(&expiring.token).await);

    // ohne Mitgliedschaft
    let carl = join(&uc, &anna, "carl", Role::Member).await;
    let carls = uc
        .create_api_token
        .execute(&carl, input("Carl", Role::Member))
        .await
        .unwrap();
    uc.remove_member
        .execute(&anna, carl.user_id())
        .await
        .unwrap();
    unauthenticated(uc.resolve_api_tenant.execute(&carls.token).await);
    assert!(
        uc.resolve_api_tenant
            .execute(&annas.token)
            .await
            .is_ok_and(|p| p.tenant.role() == Role::Owner)
    );
}

async fn api_meter(uc: &UseCases, t: &Tenant) -> (MeterId, RegisterId) {
    let mut m = meter_input(
        "Strom",
        MeterKind::Electricity,
        &[("1.8.0", Direction::Import)],
        berlin(2026, 1, 1),
    );
    m.serial_number = Some("Z-1".into());
    let id = uc.create_meter.execute(t, m).await.unwrap();
    let register = uc
        .get_meter
        .execute(t, id)
        .await
        .unwrap()
        .aggregate
        .registers[0]
        .id;
    (id, register)
}

fn by_register(
    register: RegisterId,
    at: chrono::DateTime<chrono::Utc>,
    value: Decimal,
) -> IngestItem {
    IngestItem {
        target: IngestTarget::Register(register),
        at,
        value,
    }
}

// @test SPEC_API_DAILY_READINGS
#[tokio::test]
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
async fn one_reading_per_register_day_and_token() {
    let (ports, store, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let anna = owner(&uc, "anna").await;
    let (meter, register) = api_meter(&uc, &anna).await;
    // manuelle Ablesung am selben Tag
    record(
        &uc,
        &anna,
        meter,
        berlin(2026, 1, 14) + Duration::minutes(7 * 60 + 2),
        dec!(1100),
    )
    .await;
    let token = uc
        .create_api_token
        .execute(&anna, input("HA", Role::Member))
        .await
        .unwrap();
    let p = uc.resolve_api_tenant.execute(&token.token).await.unwrap();
    let before = store.outbox().len();

    // 288 Stände im 5-Minuten-Takt ergeben eine Ablesung, die späteste
    let start = berlin(2026, 1, 14);
    let items: Vec<IngestItem> = (0..288)
        .map(|i| {
            by_register(
                register,
                start + Duration::minutes(5 * i),
                dec!(1050) + Decimal::from(i),
            )
        })
        .collect();
    let results = uc
        .ingest_readings
        .execute(&p.tenant, p.token, &items)
        .await
        .unwrap();
    assert_eq!(results.len(), 288);
    let IngestResult::Stored(id) = results[0] else {
        panic!("{:?}", results[0]);
    };
    assert!(
        results[1..]
            .iter()
            .all(|r| *r == IngestResult::Replaced(id))
    );
    let readings = uc.get_meter.execute(&anna, meter).await.unwrap().readings;
    let api: Vec<_> = readings
        .iter()
        .filter(|r| r.source == ReadingSource::Api)
        .collect();
    assert_eq!(api.len(), 1);
    assert_eq!(api[0].id, id);
    assert_eq!(api[0].read_at, start + Duration::minutes(5 * 287));
    assert_eq!(api[0].value, dec!(1337));
    assert!(
        readings
            .iter()
            .any(|r| r.source == ReadingSource::Manual && r.value == dec!(1100)),
        "manuelle Ablesung bleibt"
    );
    // Alarmprüfung über die Outbox
    let added = &store.outbox()[before..];
    assert_eq!(added.len(), 1);
    let event: OutboxEvent = serde_json::from_str(&added[0].entry.payload).unwrap();
    assert!(matches!(
        event,
        OutboxEvent::ReadingsRecorded { reading_ids, .. } if reading_ids == vec![id]
    ));

    // früher verworfen, später ersetzt, über die Zählernummer
    let earlier = by_register(register, start + Duration::hours(2), dec!(1060));
    assert_eq!(
        uc.ingest_readings
            .execute(&p.tenant, p.token, &[earlier])
            .await
            .unwrap(),
        vec![IngestResult::Ignored]
    );
    let later = IngestItem {
        target: IngestTarget::Serial {
            serial: "Z-1".into(),
            register: "1.8.0".into(),
        },
        at: start + Duration::hours(23) + Duration::minutes(59),
        value: dec!(1340),
    };
    assert_eq!(
        uc.ingest_readings
            .execute(&p.tenant, p.token, &[later])
            .await
            .unwrap(),
        vec![IngestResult::Replaced(id)]
    );

    // neuer Tag, unplausibler Stand: gespeichert, nicht abgelehnt
    let next = by_register(
        register,
        berlin(2026, 1, 15) + Duration::hours(1),
        dec!(900),
    );
    let r = uc
        .ingest_readings
        .execute(&p.tenant, p.token, &[next])
        .await
        .unwrap();
    assert!(matches!(r[0], IngestResult::Stored(_)), "{r:?}");

    // Fehler je Eintrag, die übrigen werden gespeichert
    let other = uc
        .create_api_token
        .execute(&anna, input("Zweite Quelle", Role::Member))
        .await
        .unwrap();
    let q = uc.resolve_api_tenant.execute(&other.token).await.unwrap();
    let mixed = [
        by_register(RegisterId(uuid::Uuid::nil()), start, dec!(1)),
        IngestItem {
            target: IngestTarget::Serial {
                serial: "unbekannt".into(),
                register: "1.8.0".into(),
            },
            at: start,
            value: dec!(1),
        },
        by_register(register, start + Duration::hours(12), dec!(1200.55)),
        by_register(register, start + Duration::days(3), dec!(1400)),
        by_register(register, start + Duration::hours(13), dec!(1210)),
    ];
    let r = uc
        .ingest_readings
        .execute(&q.tenant, q.token, &mixed)
        .await
        .unwrap();
    assert_eq!(r[0], IngestResult::Error("api.register_unknown"));
    assert_eq!(r[1], IngestResult::Error("api.serial_unknown"));
    assert_eq!(r[2], IngestResult::Error("reading.too_many_decimals"));
    assert_eq!(r[3], IngestResult::Error("reading.in_future"));
    assert!(
        matches!(r[4], IngestResult::Stored(_)),
        "zweites Token: eigene Ablesung"
    );
    let api = uc
        .get_meter
        .execute(&anna, meter)
        .await
        .unwrap()
        .readings
        .into_iter()
        .filter(|r| r.source == ReadingSource::Api)
        .count();
    assert_eq!(api, 3);

    // Rolle und Grenzen
    let viewer = uc
        .create_api_token
        .execute(&anna, input("Lesen", Role::Viewer))
        .await
        .unwrap();
    let v = uc.resolve_api_tenant.execute(&viewer.token).await.unwrap();
    assert!(matches!(
        uc.ingest_readings.execute(&v.tenant, v.token, &mixed).await,
        Err(AppError::Forbidden)
    ));
    let too_many = vec![by_register(register, start, dec!(1)); 1001];
    field(
        uc.ingest_readings
            .execute(&p.tenant, p.token, &too_many)
            .await,
        "readings",
        "api.count_invalid",
    );
}
