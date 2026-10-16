//! Tests: Korrektur von Ablesungen mit Historie (SPEC_RDG_HISTORY).

use chrono::{DateTime, Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use zaehler_domain::{
    meter::{Direction, MeterKind, MeterPurpose},
    reading::{Plausibility, ReadingSource},
};

use crate::{
    AppError, Tenant,
    ids::{MeterId, ReadingId},
    model::{Identity, RevisionAction},
    testing::memory_ports,
    usecase::{
        UseCases,
        meter::{MeterInput, RegisterInput, SwapInput},
        reading::{CorrectionInput, CorrectionOutcome, ReadingsInput, RecordOutcome},
    },
};

fn day(d: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap() + Duration::days(d)
}

async fn setup() -> (UseCases, Tenant, MeterId) {
    let (ports, _, clock) = memory_ports();
    // Uhr nach allen Ablesungen der Tests
    clock.advance(Duration::days(60));
    let uc = UseCases::new(&ports);
    let identity = Identity {
        issuer: "dev".into(),
        subject: "anna".into(),
        email: None,
        display_name: "Anna".into(),
        locale: "de".into(),
    };
    let user = uc.sign_in.execute(&identity).await.unwrap().user.id;
    let t = uc.resolve_tenant.execute(user, None).await.unwrap().tenant;
    let meter = uc
        .create_meter
        .execute(
            &t,
            MeterInput {
                name: "Strom".into(),
                kind: MeterKind::Electricity,
                unit: String::new(),
                schedule: None,
                purpose: MeterPurpose::General,
                targets: crate::model::MeterTargets::default(),
                parent_meter_id: None,
                max_daily_rate: Some(dec!(20)),
                billing_start: None,
                serial_number: None,
                rollover_digits: None,
                decimal_places: 1,
                registers: vec![RegisterInput {
                    id: None,
                    code: "main".into(),
                    direction: Direction::Import,
                    start_value: Some(dec!(1000)),
                }],
                start_at: day(0),
            },
        )
        .await
        .unwrap();
    (uc, t, meter)
}

async fn record(uc: &UseCases, t: &Tenant, meter: MeterId, d: i64, value: Decimal) -> ReadingId {
    let register = uc
        .get_meter
        .execute(t, meter)
        .await
        .unwrap()
        .aggregate
        .registers[0]
        .id;
    let input = ReadingsInput {
        meter_id: meter,
        at: day(d),
        values: vec![(register, value)],
        note: None,
    };
    match uc
        .record_readings
        .execute(t, &input, true, ReadingSource::Manual)
        .await
        .unwrap()
    {
        RecordOutcome::Saved(ids) => ids[0],
        RecordOutcome::NeedsConfirmation(_) => unreachable!("bestätigt"),
    }
}

fn input(d: i64, value: Decimal, note: Option<&str>) -> CorrectionInput {
    CorrectionInput {
        at: day(d),
        value,
        note: note.map(Into::into),
    }
}

// @test SPEC_RDG_HISTORY, SPEC_RDG_PLAUSIBILITY
#[tokio::test]
async fn correction_writes_a_revision_and_warnings_need_confirmation() {
    let (uc, t, meter) = setup().await;
    let a = record(&uc, &t, meter, 10, dec!(1100)).await;
    record(&uc, &t, meter, 20, dec!(1200)).await;

    // Stand kleiner als der Vorgänger: Warnung ohne Änderung
    let out = uc
        .correct_reading
        .execute(&t, a, &input(10, dec!(900), None), false)
        .await
        .unwrap();
    assert_eq!(
        out,
        CorrectionOutcome::NeedsConfirmation(Plausibility::Decreasing)
    );
    assert!(
        uc.reading_history
            .execute(&t, meter)
            .await
            .unwrap()
            .is_empty()
    );

    // unverändert: keine Revision
    let out = uc
        .correct_reading
        .execute(&t, a, &input(10, dec!(1100), None), false)
        .await
        .unwrap();
    assert_eq!(out, CorrectionOutcome::Saved(meter));
    assert!(
        uc.reading_history
            .execute(&t, meter)
            .await
            .unwrap()
            .is_empty()
    );

    // Korrektur mit Zeit, Wert und Notiz; Plausibilität ohne die Ablesung selbst
    uc.correct_reading
        .execute(&t, a, &input(11, dec!(1110), Some(" Tippfehler ")), false)
        .await
        .unwrap();
    let ctx = uc.get_reading.execute(&t, a).await.unwrap();
    assert_eq!(
        (
            ctx.reading.read_at,
            ctx.reading.value,
            ctx.reading.note.as_deref()
        ),
        (day(11), dec!(1110), Some("Tippfehler"))
    );
    let history = uc.reading_history.execute(&t, meter).await.unwrap();
    assert_eq!(history.len(), 1);
    let r = &history[0].revision;
    assert_eq!(history[0].changed_by_name.as_deref(), Some("Anna"));
    assert_eq!(r.action, RevisionAction::Update);
    assert_eq!(r.changed_by, t.user_id());
    assert_eq!((r.old_read_at, r.old_value), (day(10), dec!(1100)));
    assert_eq!(
        (r.new_read_at, r.new_value, r.new_note.as_deref()),
        (Some(day(11)), Some(dec!(1110)), Some("Tippfehler"))
    );

    // nach dem Löschen bleiben beide Revisionen sichtbar
    uc.delete_reading.execute(&t, a).await.unwrap();
    let history = uc.reading_history.execute(&t, meter).await.unwrap();
    assert_eq!(
        history
            .iter()
            .map(|h| h.revision.action)
            .collect::<Vec<_>>(),
        [RevisionAction::Delete, RevisionAction::Update]
    );
    assert_eq!(history[0].revision.old_value, dec!(1110));
    assert!(matches!(
        uc.get_reading.execute(&t, a).await,
        Err(AppError::NotFound)
    ));
}

// @test SPEC_RDG_HISTORY
#[tokio::test]
async fn correction_rejects_invalid_input_and_duplicates() {
    let (uc, t, meter) = setup().await;
    let a = record(&uc, &t, meter, 10, dec!(1100)).await;
    record(&uc, &t, meter, 20, dec!(1200)).await;
    let validation = |r: Result<CorrectionOutcome, AppError>, field: &str| match r {
        Err(AppError::Validation(e)) => assert!(!e.for_field(field).is_empty(), "{e:?}"),
        other => panic!("Validation für {field} erwartet: {other:?}"),
    };
    validation(
        uc.correct_reading
            .execute(&t, a, &input(10, dec!(1100.25), None), true)
            .await,
        "value",
    );
    validation(
        uc.correct_reading
            .execute(&t, a, &input(400, dec!(1100), None), true)
            .await,
        "at",
    );
    // gleicher Zeitpunkt wie eine andere Ablesung
    assert!(matches!(
        uc.correct_reading
            .execute(&t, a, &input(20, dec!(1100), None), true)
            .await,
        Err(AppError::Conflict("reading.duplicate"))
    ));
}

/// Zeitänderung eines Zählerwechsels verschiebt End- und Startstand sowie
/// Aus- und Einbau gemeinsam; die Verbrauchsreihe bleibt ohne Bruch.
// @test SPEC_RDG_HISTORY, SPEC_CONS_DEVICE_SWAP
#[tokio::test]
async fn swap_readings_move_together() {
    let (uc, t, meter) = setup().await;
    record(&uc, &t, meter, 10, dec!(1100)).await;
    let register = uc
        .get_meter
        .execute(&t, meter)
        .await
        .unwrap()
        .aggregate
        .registers[0]
        .id;
    uc.swap_device
        .execute(
            &t,
            meter,
            SwapInput {
                at: day(20),
                end_values: vec![(register, dec!(1200))],
                serial_number: None,
                rollover_digits: None,
                decimal_places: 1,
                start_values: vec![(register, dec!(0))],
                parent_meter_id: None,
            },
        )
        .await
        .unwrap();
    record(&uc, &t, meter, 30, dec!(100)).await;
    let detail = uc.get_meter.execute(&t, meter).await.unwrap();
    let end = detail
        .readings
        .iter()
        .find(|r| r.read_at == day(20) && r.value == dec!(1200))
        .unwrap()
        .id;
    let ctx = uc.get_reading.execute(&t, end).await.unwrap();
    assert!(ctx.part_of_swap);

    // außerhalb der Nachbarn beider Geräte
    assert!(matches!(
        uc.correct_reading
            .execute(&t, end, &input(31, dec!(1200), None), true)
            .await,
        Err(AppError::Validation(_))
    ));
    uc.correct_reading
        .execute(&t, end, &input(19, dec!(1190), None), false)
        .await
        .unwrap();
    let detail = uc.get_meter.execute(&t, meter).await.unwrap();
    let devices = &detail.aggregate.devices;
    assert_eq!(devices[0].removed_at, Some(day(19)));
    assert_eq!(devices[1].data.installed_at, day(19));
    assert_eq!(
        detail
            .readings
            .iter()
            .filter(|r| r.read_at == day(19))
            .count(),
        2,
        "End- und Startstand"
    );
    assert_eq!(
        uc.reading_history.execute(&t, meter).await.unwrap().len(),
        2,
        "eine Revision je verschobener Ablesung"
    );
    // Verbrauch: 1100 → 1190 und 0 → 100
    let analysis = uc.meter_analysis.execute(&t, meter, None).await.unwrap();
    let total: Decimal = analysis.import.iter().filter_map(|m| m.value).sum();
    assert!(total <= dec!(290), "{total}");
}
