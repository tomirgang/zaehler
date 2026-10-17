//! Port-Tests für API-Tokens und täglich verdichtete Ablesungen
//! (SPEC_API_AUTH, SPEC_API_DAILY_READINGS): Hash-Suche, Widerruf,
//! Tenant-Isolation, Ersetzen und Verwerfen je Tag.

use chrono::Duration;
use rust_decimal::Decimal;
use zaehler_domain::reading::ReadingSource;

use super::{id, meter, reading, t0, tenant};
use crate::{
    PortError, Role,
    ids::{ApiTokenId, ReadingId},
    model::{DailyOutcome, NewApiToken, Reading},
    port::UnitOfWorkFactory,
    secret,
};

fn new_token(name: &str) -> (NewApiToken, String) {
    let s = secret::generate_api_token();
    (
        NewApiToken {
            id: ApiTokenId(id()),
            name: name.into(),
            role: Role::Member,
            token_hash: s.hash,
            prefix: s.token.chars().take(8).collect(),
            created_at: t0(),
            expires_at: Some(t0() + Duration::days(30)),
        },
        s.token,
    )
}

/// Führt alle Prüfungen zu API-Tokens aus.
// @test SPEC_API_AUTH, SPEC_API_DAILY_READINGS, SPEC_SEC_TENANT_SCOPE
#[allow(clippy::too_many_lines)] // ein zusammenhängender Ablauf
pub(super) async fn api(f: &dyn UnitOfWorkFactory) {
    let t = tenant(f, "API").await;
    let other = tenant(f, "Fremd API").await;
    let (tok, plain) = new_token("Home Assistant");
    let mut uow = f.begin().await.expect("begin");
    uow.api_tokens().create(&t, &tok).await.expect("create");
    uow.commit().await.expect("commit");

    let mut uow = f.begin().await.expect("begin");
    let found = uow
        .api_tokens()
        .find_by_hash(&secret::hash(&plain))
        .await
        .expect("find")
        .expect("token");
    assert_eq!(found.id, tok.id);
    assert_eq!(found.household_id, t.household_id());
    assert_eq!(found.user_id, t.user_id());
    assert_eq!(found.user_name, "API");
    assert_eq!(found.role, Role::Member);
    assert_eq!(found.prefix, tok.prefix);
    assert_eq!(found.expires_at, tok.expires_at);
    assert!(found.last_used_at.is_none());
    assert!(
        uow.api_tokens()
            .find_by_hash(&secret::hash("zt_unbekannt"))
            .await
            .expect("find")
            .is_none()
    );
    assert_eq!(uow.api_tokens().list(&t).await.expect("list").len(), 1);
    assert!(
        uow.api_tokens()
            .list(&other)
            .await
            .expect("list")
            .is_empty()
    );
    assert!(matches!(
        uow.api_tokens().revoke(&other, tok.id, t0()).await,
        Err(PortError::NotFound)
    ));
    uow.api_tokens()
        .touch(tok.id, t0() + Duration::hours(1))
        .await
        .expect("touch");
    uow.commit().await.expect("commit");

    // Tagesverdichtung: Stand, späterer ersetzt, früherer verworfen
    let (_, device, register) = meter(f, &t, "API Zähler").await;
    let (_, foreign_device, foreign_register) = meter(f, &other, "Fremd API Zähler").await;
    let day = t0() + Duration::days(2);
    let (from, until) = (day - Duration::hours(12), day + Duration::hours(12));
    let api = |at: chrono::DateTime<chrono::Utc>, value: i64| Reading {
        id: ReadingId(id()),
        register_id: register,
        device_id: device,
        read_at: at,
        value: Decimal::new(value, 1),
        source: ReadingSource::Api,
        note: None,
        photo_key: None,
        ai_confidence: None,
        created_by: Some(t.user_id()),
    };
    let mut uow = f.begin().await.expect("begin");
    let manual = reading(&t, register, device, 2, 100);
    uow.readings()
        .insert(&t, std::slice::from_ref(&manual))
        .await
        .expect("manual");
    let first = api(day - Duration::hours(2), 101);
    assert_eq!(
        uow.readings()
            .upsert_daily(&t, &first, tok.id, from, until)
            .await
            .expect("first"),
        DailyOutcome::Stored(first.id)
    );
    let later = api(day + Duration::hours(3), 105);
    assert_eq!(
        uow.readings()
            .upsert_daily(&t, &later, tok.id, from, until)
            .await
            .expect("later"),
        DailyOutcome::Replaced(first.id)
    );
    let earlier = api(day + Duration::hours(1), 103);
    assert_eq!(
        uow.readings()
            .upsert_daily(&t, &earlier, tok.id, from, until)
            .await
            .expect("earlier"),
        DailyOutcome::Ignored
    );
    // gleicher Zeitpunkt wie die manuelle Ablesung
    let clash = api(manual.read_at, 100);
    assert!(matches!(
        uow.readings()
            .upsert_daily(&t, &clash, ApiTokenId(id()), from, until)
            .await,
        Err(PortError::Conflict(_))
    ));
    let mut foreign = api(day, 1);
    foreign.register_id = foreign_register;
    foreign.device_id = foreign_device;
    assert!(matches!(
        uow.readings()
            .upsert_daily(&t, &foreign, tok.id, from, until)
            .await,
        Err(PortError::NotFound)
    ));
    uow.commit().await.expect("commit");

    let mut uow = f.begin().await.expect("begin");
    let meter_id = uow
        .readings()
        .get(&t, manual.id)
        .await
        .expect("get")
        .expect("manual")
        .0;
    let all = uow
        .readings()
        .list_for_meter(&t, meter_id)
        .await
        .expect("list");
    assert_eq!(all.len(), 2, "manuelle und eine API-Ablesung");
    let kept = all.iter().find(|r| r.id == first.id).expect("api");
    assert_eq!(kept.read_at, later.read_at);
    assert_eq!(kept.value, later.value);
    assert_eq!(kept.source, ReadingSource::Api);
    assert!(
        all.iter()
            .any(|r| r.id == manual.id && r.value == manual.value)
    );

    // Widerruf genau einmal
    uow.api_tokens()
        .revoke(&t, tok.id, t0() + Duration::days(1))
        .await
        .expect("revoke");
    assert!(matches!(
        uow.api_tokens().revoke(&t, tok.id, t0()).await,
        Err(PortError::NotFound)
    ));
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    let listed = uow.api_tokens().list(&t).await.expect("list");
    assert_eq!(listed[0].revoked_at, Some(t0() + Duration::days(1)));
    assert_eq!(listed[0].last_used_at, Some(t0() + Duration::hours(1)));

    // Löschen mit dem Haushalt
    uow.households().delete(&t).await.expect("delete");
    uow.commit().await.expect("commit");
    let mut uow = f.begin().await.expect("begin");
    assert!(
        uow.api_tokens()
            .find_by_hash(&secret::hash(&plain))
            .await
            .expect("find")
            .is_none()
    );
}
