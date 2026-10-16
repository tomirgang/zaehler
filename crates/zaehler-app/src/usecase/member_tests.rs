//! Tests: Mitglieder, Rollen und Einladungen (SPEC_TEN_ROLES, SPEC_TEN_INVITES).

#![allow(clippy::many_single_char_names)]

use bytes::Bytes;
use chrono::{Duration, Utc};
use uuid::Uuid;
use zaehler_domain::{
    meter::{MeterKind, MeterPurpose},
    reading::ReadingSource,
};

use crate::{
    AppError, Role, Tenant,
    ids::{BlobKey, DraftId, HouseholdId, InvitationId, MeterId, ReadingId, UserId},
    model::{HouseholdData, Identity},
    testing::memory_ports,
    usecase::{
        UseCases,
        import::{DateFormat, DecimalSeparator, ImportMapping},
        member::InvitationInput,
        meter::{MeterInput, SwapInput},
        reading::ReadingsInput,
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

async fn user(uc: &UseCases, name: &str) -> UserId {
    uc.sign_in.execute(&identity(name)).await.unwrap().user.id
}

/// Lädt `name` mit `role` in den Haushalt von `t` ein und liefert seinen Tenant.
async fn join(uc: &UseCases, t: &Tenant, name: &str, role: Role) -> Tenant {
    let u = user(uc, name).await;
    let inv = uc
        .create_invitation
        .execute(t, InvitationInput { role, email: None })
        .await
        .unwrap();
    let household = uc.accept_invitation.execute(u, &inv.token).await.unwrap();
    assert_eq!(household, t.household_id());
    uc.resolve_tenant
        .execute(u, Some(household))
        .await
        .unwrap()
        .tenant
}

#[allow(clippy::needless_pass_by_value)] // nimmt das Ergebnis des Aufrufs direkt
fn forbidden<T: std::fmt::Debug>(r: Result<T, AppError>, what: &str) {
    assert!(
        matches!(r, Err(AppError::Forbidden)),
        "{what}: Forbidden erwartet, erhalten {r:?}"
    );
}

fn meter_input() -> MeterInput {
    MeterInput {
        name: "Strom".into(),
        kind: MeterKind::Electricity,
        unit: String::new(),
        schedule: None,
        purpose: MeterPurpose::General,
        targets: crate::model::MeterTargets::default(),
        parent_meter_id: None,
        max_daily_rate: None,
        billing_start: None,
        serial_number: None,
        rollover_digits: None,
        decimal_places: 0,
        registers: Vec::new(),
        start_at: Utc::now(),
    }
}

/// Jeder schreibende Anwendungsfall mit `Viewer` und jeder Owner-Fall mit
/// `Member` ergibt `Forbidden`, bevor Daten gelesen werden.
// @test SPEC_TEN_ROLES
#[tokio::test]
#[allow(clippy::too_many_lines)] // eine Zeile je Anwendungsfall
async fn role_matrix() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let o = owner(&uc, "owner").await;
    let (h, u) = (o.household_id(), o.user_id());
    let m = MeterId(Uuid::now_v7());
    let r = ReadingId(Uuid::now_v7());
    let d = DraftId(Uuid::now_v7());
    let key = BlobKey::tmp(h, Uuid::now_v7());
    let mapping = ImportMapping {
        meter_id: m,
        has_header: false,
        date: 0,
        time: None,
        value: 1,
        register: None,
        serial: None,
        note: None,
        date_format: DateFormat::Iso8601,
        decimal: DecimalSeparator::Comma,
        default_register: None,
    };
    let readings = ReadingsInput {
        meter_id: m,
        at: Utc::now(),
        values: Vec::new(),
        note: None,
    };
    let swap = SwapInput {
        at: Utc::now(),
        end_values: Vec::new(),
        serial_number: None,
        rollover_digits: None,
        decimal_places: 0,
        start_values: Vec::new(),
        parent_meter_id: None,
    };
    let data = HouseholdData {
        name: "x".into(),
        timezone: chrono_tz::Europe::Berlin,
    };
    let invite = InvitationInput {
        role: Role::Viewer,
        email: None,
    };
    let viewer = Tenant::new(u, h, Role::Viewer);
    let member = Tenant::new(u, h, Role::Member);

    // nur Owner
    for t in [&viewer, &member] {
        let role = t.role().as_str();
        let at = |what: &str| format!("{what} als {role}");
        forbidden(
            uc.create_meter.execute(t, meter_input()).await,
            &at("CreateMeter"),
        );
        forbidden(
            uc.update_meter.execute(t, m, meter_input()).await,
            &at("UpdateMeter"),
        );
        forbidden(
            uc.swap_device.execute(t, m, swap.clone()).await,
            &at("SwapDevice"),
        );
        forbidden(
            uc.update_household.execute(t, data.clone()).await,
            &at("UpdateHousehold"),
        );
        forbidden(
            uc.delete_household.execute(t, "x").await,
            &at("DeleteHousehold"),
        );
        forbidden(
            uc.export_household
                .execute(t, std::io::Cursor::new(Vec::new()))
                .await
                .map(|_| ()),
            &at("ExportHousehold"),
        );
        forbidden(
            uc.change_role.execute(t, u, Role::Owner).await,
            &at("ChangeRole"),
        );
        forbidden(uc.remove_member.execute(t, u).await, &at("RemoveMember"));
        forbidden(uc.list_invitations.execute(t).await, &at("ListInvitations"));
        forbidden(
            uc.create_invitation.execute(t, invite.clone()).await,
            &at("CreateInvitation"),
        );
        forbidden(
            uc.revoke_invitation
                .execute(t, InvitationId(Uuid::now_v7()))
                .await,
            &at("RevokeInvitation"),
        );
    }

    // ab Member
    let t = &viewer;
    forbidden(
        uc.check_readings.execute(t, &readings).await,
        "CheckReadings",
    );
    forbidden(
        uc.record_readings
            .execute(t, &readings, false, ReadingSource::Manual)
            .await,
        "RecordReadings",
    );
    forbidden(uc.delete_reading.execute(t, r).await, "DeleteReading");
    forbidden(
        uc.queue_photos
            .execute(t, vec![Bytes::from_static(b"x")])
            .await,
        "QueuePhotos",
    );
    forbidden(
        uc.analyse_photo.execute(t, Bytes::from_static(b"x")).await,
        "AnalysePhoto",
    );
    forbidden(uc.analyse_queued.next(t).await, "AnalyseQueued");
    forbidden(uc.get_draft.execute(t, d, None).await, "GetDraft");
    forbidden(
        uc.confirm_draft.execute(t, d, &readings, false).await,
        "ConfirmDraft",
    );
    forbidden(uc.discard_draft.execute(t, d).await, "DiscardDraft");
    forbidden(uc.get_draft_photo.execute(t, d).await, "GetDraftPhoto");
    forbidden(
        uc.upload_csv.execute(t, Bytes::from_static(b"a;b\n")).await,
        "UploadCsv",
    );
    forbidden(
        uc.preview_import.execute(t, &key, &mapping).await,
        "PreviewImport",
    );
    forbidden(
        uc.commit_import.execute(t, &key, &mapping, false).await,
        "CommitImport",
    );

    // lesend und Austritt bleiben für Viewer erlaubt
    assert!(uc.list_members.execute(&viewer).await.is_ok());
    assert!(uc.dashboard.execute(&viewer).await.is_ok());
}

// @test SPEC_TEN_ROLES
#[tokio::test]
async fn the_last_owner_stays() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let o = owner(&uc, "anna").await;
    let viewer = join(&uc, &o, "bea", Role::Viewer).await;
    assert_eq!(viewer.role(), Role::Viewer);
    let names: Vec<(String, Role)> = uc
        .list_members
        .execute(&viewer)
        .await
        .unwrap()
        .into_iter()
        .map(|m| (m.display_name, m.role))
        .collect();
    assert_eq!(
        names,
        [
            ("anna".to_owned(), Role::Owner),
            ("bea".into(), Role::Viewer)
        ]
    );

    let conflict = |r: Result<(), AppError>| {
        assert!(
            matches!(r, Err(AppError::Conflict("member.last_owner"))),
            "{r:?}"
        );
    };
    conflict(uc.change_role.execute(&o, o.user_id(), Role::Member).await);
    conflict(uc.remove_member.execute(&o, o.user_id()).await);
    conflict(uc.leave_household.execute(&o).await);

    // mit zweitem Owner sind alle drei möglich
    uc.change_role
        .execute(&o, viewer.user_id(), Role::Owner)
        .await
        .unwrap();
    uc.change_role
        .execute(&o, o.user_id(), Role::Member)
        .await
        .unwrap();
    let bea = uc
        .resolve_tenant
        .execute(viewer.user_id(), Some(o.household_id()))
        .await
        .unwrap()
        .tenant;
    assert_eq!(bea.role(), Role::Owner);
    uc.remove_member.execute(&bea, o.user_id()).await.unwrap();
    // unbekanntes Mitglied
    assert!(matches!(
        uc.remove_member.execute(&bea, o.user_id()).await,
        Err(AppError::NotFound)
    ));
    // Anna hat danach nur noch einen eigenen Haushalt
    let anna = uc
        .resolve_tenant
        .execute(o.user_id(), Some(o.household_id()))
        .await
        .unwrap();
    assert_ne!(anna.tenant.household_id(), o.household_id());
}

// @test SPEC_TEN_ROLES
#[tokio::test]
async fn members_can_leave() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let o = owner(&uc, "anna").await;
    let viewer = join(&uc, &o, "bea", Role::Viewer).await;
    uc.leave_household.execute(&viewer).await.unwrap();
    assert_eq!(uc.list_members.execute(&o).await.unwrap().len(), 1);
}

// @test SPEC_TEN_INVITES
#[tokio::test]
async fn invitations_expire_and_are_single_use() {
    let (ports, _, clock) = memory_ports();
    let uc = UseCases::new(&ports);
    let o = owner(&uc, "anna").await;
    let bea = user(&uc, "bea").await;
    let carl = user(&uc, "carl").await;

    // ungültige E-Mail-Adresse
    assert!(matches!(
        uc.create_invitation
            .execute(
                &o,
                InvitationInput {
                    role: Role::Member,
                    email: Some("kein-at".into()),
                },
            )
            .await,
        Err(AppError::Validation(_))
    ));

    let inv = uc
        .create_invitation
        .execute(
            &o,
            InvitationInput {
                role: Role::Member,
                email: Some(" bea@example.org ".into()),
            },
        )
        .await
        .unwrap();
    let open = uc.list_invitations.execute(&o).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].email.as_deref(), Some("bea@example.org"));
    assert_eq!(open[0].expires_at - open[0].created_at, Duration::days(7));
    let preview = uc
        .preview_invitation
        .execute(bea, &inv.token)
        .await
        .unwrap();
    assert_eq!(preview.household_name, "anna");
    assert_eq!(preview.role, Role::Member);

    let invalid = |r: Result<HouseholdId, AppError>| {
        assert!(
            matches!(r, Err(AppError::Conflict("invite.invalid"))),
            "{r:?}"
        );
    };
    // falsches oder verändertes Token
    invalid(uc.accept_invitation.execute(bea, "abc").await);
    invalid(uc.accept_invitation.execute(bea, &"0".repeat(64)).await);

    uc.accept_invitation.execute(bea, &inv.token).await.unwrap();
    // zweites Einlösen schlägt fehl, auch durch einen anderen Nutzer
    invalid(uc.accept_invitation.execute(bea, &inv.token).await);
    invalid(uc.accept_invitation.execute(carl, &inv.token).await);
    assert!(uc.list_invitations.execute(&o).await.unwrap().is_empty());

    // nach 7 Tagen ungültig
    let later = uc
        .create_invitation
        .execute(
            &o,
            InvitationInput {
                role: Role::Viewer,
                email: None,
            },
        )
        .await
        .unwrap();
    clock.advance(Duration::days(7));
    invalid(uc.accept_invitation.execute(carl, &later.token).await);
    assert!(
        uc.preview_invitation
            .execute(carl, &later.token)
            .await
            .is_err()
    );

    // widerrufen
    let revoked = uc
        .create_invitation
        .execute(
            &o,
            InvitationInput {
                role: Role::Viewer,
                email: None,
            },
        )
        .await
        .unwrap();
    uc.revoke_invitation.execute(&o, revoked.id).await.unwrap();
    invalid(uc.accept_invitation.execute(carl, &revoked.token).await);
    assert!(matches!(
        uc.revoke_invitation.execute(&o, revoked.id).await,
        Err(AppError::NotFound)
    ));
}

// @test SPEC_TEN_INVITES
#[tokio::test]
async fn an_existing_member_keeps_the_higher_role() {
    let (ports, _, _) = memory_ports();
    let uc = UseCases::new(&ports);
    let o = owner(&uc, "anna").await;
    let bea = join(&uc, &o, "bea", Role::Member).await;
    let inv = uc
        .create_invitation
        .execute(
            &o,
            InvitationInput {
                role: Role::Viewer,
                email: None,
            },
        )
        .await
        .unwrap();
    uc.accept_invitation
        .execute(bea.user_id(), &inv.token)
        .await
        .unwrap();
    let again = uc
        .resolve_tenant
        .execute(bea.user_id(), Some(o.household_id()))
        .await
        .unwrap();
    assert_eq!(again.tenant.role(), Role::Member);
    assert_eq!(
        again.households.len(),
        2,
        "eigener und eingeladener Haushalt"
    );
}

#[test]
fn email_format() {
    use crate::usecase::member::email_valid;
    for ok in ["a@b.de", "anna.m+zaehler@example.org"] {
        assert!(email_valid(ok), "{ok}");
    }
    for bad in [
        "", "a", "a@b", "@b.de", "a@.de", "a@b.", "a b@c.de", "a@b@c.de",
    ] {
        assert!(!email_valid(bad), "{bad}");
    }
}
