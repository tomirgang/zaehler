//! Tests: Einheiten mit Steuerzeichen (SPEC_MTR_MANAGE).

use rust_decimal_macros::dec;
use zaehler_domain::meter::{Direction, MeterKind};

use super::contract_tests::{berlin, field, meter_input, record};
use crate::{
    model::Identity,
    testing::memory_ports,
    usecase::{UseCases, meter::RegisterInput},
};

/// Eine Einheit mit Steuerzeichen (z.B. „m“ + U+0017 statt „m³“) wird
/// abgelehnt; eine bereits gespeicherte bleibt trotz Ablesungen
/// korrigierbar, eine gültige nicht.
// @test SPEC_MTR_MANAGE
#[tokio::test]
async fn units_with_control_characters_are_rejected_and_correctable() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let user = uc
        .sign_in
        .execute(&Identity {
            issuer: "dev".into(),
            subject: "anna".into(),
            email: None,
            display_name: "Anna".into(),
            locale: "de".into(),
        })
        .await
        .unwrap()
        .user
        .id;
    let t = uc.resolve_tenant.execute(user, None).await.unwrap().tenant;
    let mut input = meter_input(
        "Wasser",
        MeterKind::Water,
        &[("main", Direction::Import)],
        berlin(2026, 1, 1),
    );
    input.unit = "m\u{17}".into();
    field(
        uc.create_meter.execute(&t, input.clone()).await,
        "unit",
        "meter.unit_control",
    );
    input.unit = String::new();
    let id = uc.create_meter.execute(&t, input.clone()).await.unwrap();
    record(&uc, &t, id, berlin(2026, 1, 10), dec!(5)).await;

    // beschädigte Einheit wie in Altdaten, am Anwendungsfall vorbei gespeichert
    let detail = uc.get_meter.execute(&t, id).await.unwrap();
    let mut data = detail.aggregate.meter.data.clone();
    data.unit = "m\u{17}".into();
    let mut uow = ports.uow.begin().await.unwrap();
    uow.meters().update(&t, id, &data).await.unwrap();
    uow.commit().await.unwrap();

    let register = &detail.aggregate.registers[0];
    let mut update = input.clone();
    update.registers = vec![RegisterInput {
        id: Some(register.id),
        code: register.data.code.clone(),
        direction: register.data.direction,
        start_value: None,
    }];
    update.unit = "m³".into();
    uc.update_meter
        .execute(&t, id, update.clone())
        .await
        .unwrap();
    assert_eq!(
        uc.get_meter
            .execute(&t, id)
            .await
            .unwrap()
            .aggregate
            .meter
            .data
            .unit,
        "m³"
    );
    // gültige Einheit bleibt nach der ersten Ablesung gesperrt
    update.unit = "l".into();
    field(
        uc.update_meter.execute(&t, id, update).await,
        "unit",
        "meter.unit_locked",
    );
}
