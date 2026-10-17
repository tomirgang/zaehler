//! Abbildung des In-Memory-Zustands auf Archiv-Datensätze (SPEC_BACKUP_DATA).
//! Felder, die der In-Memory-Adapter nicht führt (z.B. `created_at` von
//! Ablesungen), erhalten feste Ersatzwerte; verlustfrei ist der
//! Datenbank-Adapter.

use rust_decimal::{Decimal, prelude::ToPrimitive};

use super::{
    BlobKey, DateTime, Device, DeviceData, DeviceId, Draft, DraftId, Household, HouseholdData,
    HouseholdId, InMemoryStore, InvitationId, Meter, MeterData, MeterId, NewInvitation, PortError,
    PortResult, Reading, ReadingId, Register, RegisterData, RegisterId, Role, State, StoredDraft,
    StoredInvitation, StoredUser, User, UserId, Utc, async_trait, poisoned,
};
use crate::{
    backup::{
        BackupData, DeviceRecord, DraftRecord, HouseholdRecord, InvitationRecord, MembershipRecord,
        MeterRecord, NotificationSettingRecord, ReadingRecord, RegisterRecord, RevisionRecord,
        UserRecord,
    },
    ids::RevisionId,
    model::{
        MeterTargets, MonthDay, Revision, schedule_columns, schedule_from_columns,
        season_weights_from_json, season_weights_to_json,
    },
    port::BackupRepository,
    secret,
};

fn invalid<E>(_: E) -> PortError {
    PortError::Invalid("backup.invalid")
}

fn milli(d: Decimal) -> i64 {
    (d * Decimal::from(1000)).trunc().to_i64().unwrap_or(0)
}

#[allow(clippy::too_many_lines)] // eine Abbildung je Datensatzart
fn export(s: &State) -> BackupData {
    let epoch = DateTime::<Utc>::UNIX_EPOCH;
    let mut data = BackupData {
        users: s
            .users
            .iter()
            .map(|u| UserRecord {
                id: u.user.id.0,
                oidc_issuer: u.issuer.clone(),
                oidc_subject: u.subject.clone(),
                email: u.user.email.clone(),
                display_name: u.user.display_name.clone(),
                locale: u.user.locale.clone(),
                created_at: epoch,
                last_login_at: None,
            })
            .collect(),
        households: s
            .households
            .iter()
            .map(|h| HouseholdRecord {
                id: h.id.0,
                name: h.data.name.clone(),
                timezone: h.data.timezone.name().into(),
                created_at: epoch,
            })
            .collect(),
        memberships: s
            .memberships
            .iter()
            .map(|(h, u, r)| MembershipRecord {
                household_id: h.0,
                user_id: u.0,
                role: r.as_str().into(),
            })
            .collect(),
        meters: s
            .meters
            .iter()
            .map(|(h, m)| {
                let (kind, value, legacy) = schedule_columns(m.data.schedule);
                MeterRecord {
                    id: m.id.0,
                    household_id: h.0,
                    name: m.data.name.clone(),
                    kind: m.data.kind.as_str().into(),
                    unit: m.data.unit.clone(),
                    reading_interval_days: legacy.map(i32::from),
                    max_daily_rate_milli: m.data.max_daily_rate.map(milli),
                    billing_start_month: m.data.billing_start.map(|b| i16::from(b.month)),
                    billing_start_day: m.data.billing_start.map(|b| i16::from(b.day)),
                    archived: m.archived,
                    created_at: m.created_at,
                    purpose: m.data.purpose.as_str().into(),
                    schedule_kind: kind.map(Into::into),
                    schedule_value: value.and_then(|v| i16::try_from(v).ok()),
                    archived_at: m.archived_at,
                    season_weights: m
                        .data
                        .targets
                        .season_weights
                        .as_ref()
                        .map(season_weights_to_json),
                    annual_target_milli: m.data.targets.annual_target.map(milli),
                    target_tolerance_pct: m.data.targets.tolerance_pct.map(i16::from),
                }
            })
            .collect(),
        devices: s
            .devices
            .iter()
            .map(|(m, d)| DeviceRecord {
                id: d.id.0,
                meter_id: m.0,
                serial_number: d.data.serial_number.clone(),
                installed_at: d.data.installed_at,
                removed_at: d.removed_at,
                rollover_digits: d.data.rollover_digits.map(i16::from),
                decimal_places: i16::from(d.data.decimal_places),
                parent_meter_id: d.data.parent_meter_id.map(|p| p.0),
            })
            .collect(),
        registers: s
            .registers
            .iter()
            .map(|(m, r)| RegisterRecord {
                id: r.id.0,
                meter_id: m.0,
                code: r.data.code.clone(),
                direction: r.data.direction.as_str().into(),
            })
            .collect(),
        readings: s
            .readings
            .iter()
            .map(|r| ReadingRecord {
                id: r.id.0,
                register_id: r.register_id.0,
                device_id: r.device_id.0,
                read_at: r.read_at,
                value_milli: milli(r.value),
                source: r.source.as_str().into(),
                photo_key: r.photo_key.as_ref().map(ToString::to_string),
                ai_confidence: r.ai_confidence.and_then(|c| c.to_f64()),
                note: r.note.clone(),
                created_by: r.created_by.map(|u| u.0),
                created_at: r.read_at,
                updated_at: None,
                api_token_id: None,
            })
            .collect(),
        drafts: s
            .drafts
            .iter()
            .map(|d| DraftRecord {
                id: d.draft.id.0,
                household_id: d.household.0,
                created_by: d.draft.created_by.0,
                photo_key: d.draft.photo_key.to_string(),
                model: d.draft.model.clone(),
                duration_ms: d.draft.duration_ms,
                suggestion: d
                    .draft
                    .suggestion
                    .as_ref()
                    .and_then(|x| serde_json::to_string(x).ok()),
                matched_meter_id: d.draft.matched_meter_id.map(|m| m.0),
                status: d.draft.status.as_str().into(),
                confirmed_reading_id: None,
                corrected: d.corrected,
                created_at: d.draft.created_at,
                finished_at: None,
                taken_at: d.draft.taken_at,
                analysis_started_at: d.draft.analysis_started_at,
            })
            .collect(),
        invitations: s
            .invitations
            .iter()
            .map(|x| InvitationRecord {
                id: x.invitation.id.0,
                household_id: x.household.0,
                role: x.invitation.role.as_str().into(),
                token_hash: secret::to_hex(&x.invitation.token_hash),
                email: x.invitation.email.clone(),
                created_by: x.invitation.created_by.0,
                created_at: x.invitation.created_at,
                expires_at: x.invitation.expires_at,
                accepted_by: x.accepted_by.map(|u| u.0),
                accepted_at: x.accepted_at,
                revoked_at: x.revoked_at,
            })
            .collect(),
        revisions: s
            .revisions
            .iter()
            .map(|(h, r)| RevisionRecord {
                id: r.id.0,
                household_id: h.0,
                reading_id: r.reading_id.0,
                meter_id: r.meter_id.0,
                register_id: r.register_id.0,
                action: r.action.as_str().into(),
                changed_by: r.changed_by.0,
                changed_at: r.changed_at,
                old_read_at: r.old_read_at,
                old_value_milli: milli(r.old_value),
                old_note: r.old_note.clone(),
                new_read_at: r.new_read_at,
                new_value_milli: r.new_value.map(milli),
                new_note: r.new_note.clone(),
            })
            .collect(),
        notification_settings: s
            .notification_settings
            .iter()
            .map(|(u, (n, at))| NotificationSettingRecord {
                user_id: u.0,
                email: n.email.clone(),
                reminder: n.reminder,
                alarm: n.alarm,
                updated_at: *at,
            })
            .collect(),
        ..BackupData::default()
    };
    super::contracts_impl::export(s, &mut data);
    super::documents_impl::export(s, &mut data);
    super::vehicles_impl::export(s, &mut data);
    super::pv_impl::export(s, &mut data);
    super::api_impl::export(s, &mut data);
    data
}

fn blob_key(k: &str) -> PortResult<BlobKey> {
    BlobKey::parse(k).ok_or(PortError::Invalid("backup.invalid"))
}

fn small(v: i16) -> PortResult<u8> {
    u8::try_from(v).map_err(invalid)
}

fn import_meters(s: &mut State, d: &BackupData) -> PortResult<()> {
    for m in &d.meters {
        let billing_start = match (m.billing_start_month, m.billing_start_day) {
            (Some(month), Some(day)) => Some(MonthDay {
                month: small(month)?,
                day: small(day)?,
            }),
            _ => None,
        };
        s.meters.push((
            HouseholdId(m.household_id),
            Meter {
                id: MeterId(m.id),
                data: MeterData {
                    name: m.name.clone(),
                    kind: m.kind.parse().map_err(invalid)?,
                    unit: m.unit.clone(),
                    schedule: schedule_from_columns(
                        m.schedule_kind.as_deref(),
                        m.schedule_value.map(i64::from),
                        m.reading_interval_days.map(i64::from),
                    ),
                    max_daily_rate: m.max_daily_rate_milli.map(|v| Decimal::new(v, 3)),
                    billing_start,
                    purpose: m.purpose.parse().map_err(invalid)?,
                    targets: MeterTargets {
                        season_weights: m
                            .season_weights
                            .as_deref()
                            .map(|w| {
                                season_weights_from_json(w)
                                    .ok_or(PortError::Invalid("backup.invalid"))
                            })
                            .transpose()?,
                        annual_target: m.annual_target_milli.map(|v| Decimal::new(v, 3)),
                        tolerance_pct: m.target_tolerance_pct.map(small).transpose()?,
                    },
                },
                archived: m.archived,
                archived_at: m.archived_at,
                created_at: m.created_at,
            },
        ));
    }
    for x in &d.devices {
        s.devices.push((
            MeterId(x.meter_id),
            Device {
                id: DeviceId(x.id),
                data: DeviceData {
                    serial_number: x.serial_number.clone(),
                    installed_at: x.installed_at,
                    rollover_digits: x.rollover_digits.map(small).transpose()?,
                    decimal_places: small(x.decimal_places)?,
                    parent_meter_id: x.parent_meter_id.map(MeterId),
                },
                removed_at: x.removed_at,
            },
        ));
    }
    for r in &d.registers {
        s.registers.push((
            MeterId(r.meter_id),
            Register {
                id: RegisterId(r.id),
                data: RegisterData {
                    code: r.code.clone(),
                    direction: r.direction.parse().map_err(invalid)?,
                },
            },
        ));
    }
    Ok(())
}

fn import(s: &mut State, d: &BackupData) -> PortResult<()> {
    for u in &d.users {
        s.users.push(StoredUser {
            user: User {
                id: UserId(u.id),
                display_name: u.display_name.clone(),
                email: u.email.clone(),
                locale: u.locale.clone(),
            },
            issuer: u.oidc_issuer.clone(),
            subject: u.oidc_subject.clone(),
        });
    }
    for h in &d.households {
        s.households.push(Household {
            id: HouseholdId(h.id),
            data: HouseholdData {
                name: h.name.clone(),
                timezone: h.timezone.parse().map_err(invalid)?,
            },
        });
    }
    for m in &d.memberships {
        let role: Role = m.role.parse().map_err(invalid)?;
        s.memberships
            .push((HouseholdId(m.household_id), UserId(m.user_id), role));
    }
    import_meters(s, d)?;
    for r in &d.readings {
        s.readings.push(Reading {
            id: ReadingId(r.id),
            register_id: RegisterId(r.register_id),
            device_id: DeviceId(r.device_id),
            read_at: r.read_at,
            value: Decimal::new(r.value_milli, 3),
            source: r.source.parse().map_err(invalid)?,
            note: r.note.clone(),
            photo_key: r.photo_key.as_deref().map(blob_key).transpose()?,
            ai_confidence: r.ai_confidence.and_then(|c| Decimal::try_from(c).ok()),
            created_by: r.created_by.map(UserId),
        });
    }
    for x in &d.drafts {
        s.drafts.push(StoredDraft {
            household: HouseholdId(x.household_id),
            draft: Draft {
                id: DraftId(x.id),
                created_by: UserId(x.created_by),
                photo_key: blob_key(&x.photo_key)?,
                model: x.model.clone(),
                duration_ms: x.duration_ms,
                suggestion: x
                    .suggestion
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(invalid)?,
                matched_meter_id: x.matched_meter_id.map(MeterId),
                status: x.status.parse().map_err(invalid)?,
                created_at: x.created_at,
                taken_at: x.taken_at,
                analysis_started_at: x.analysis_started_at,
            },
            corrected: x.corrected,
        });
    }
    import_v1(s, d)
}

/// Datensätze ab v1: Einladungen und Revisionen.
fn import_v1(s: &mut State, d: &BackupData) -> PortResult<()> {
    for x in &d.invitations {
        s.invitations.push(StoredInvitation {
            invitation: NewInvitation {
                id: InvitationId(x.id),
                role: x.role.parse().map_err(invalid)?,
                token_hash: secret::hash_from_hex(&x.token_hash)
                    .ok_or(PortError::Invalid("backup.invalid"))?,
                email: x.email.clone(),
                created_by: UserId(x.created_by),
                created_at: x.created_at,
                expires_at: x.expires_at,
            },
            household: HouseholdId(x.household_id),
            accepted_by: x.accepted_by.map(UserId),
            accepted_at: x.accepted_at,
            revoked_at: x.revoked_at,
        });
    }
    for r in &d.revisions {
        s.revisions.push((
            HouseholdId(r.household_id),
            Revision {
                id: RevisionId(r.id),
                reading_id: ReadingId(r.reading_id),
                meter_id: MeterId(r.meter_id),
                register_id: RegisterId(r.register_id),
                action: r.action.parse().map_err(invalid)?,
                changed_by: UserId(r.changed_by),
                changed_at: r.changed_at,
                old_read_at: r.old_read_at,
                old_value: Decimal::new(r.old_value_milli, 3),
                old_note: r.old_note.clone(),
                new_read_at: r.new_read_at,
                new_value: r.new_value_milli.map(|v| Decimal::new(v, 3)),
                new_note: r.new_note.clone(),
            },
        ));
    }
    for x in &d.notification_settings {
        s.notification_settings.insert(
            UserId(x.user_id),
            (
                crate::model::NotificationSettings {
                    email: x.email.clone(),
                    reminder: x.reminder,
                    alarm: x.alarm,
                },
                x.updated_at,
            ),
        );
    }
    super::contracts_impl::import(s, d)?;
    super::documents_impl::import(s, d)?;
    super::vehicles_impl::import(s, d)?;
    super::pv_impl::import(s, d)?;
    super::api_impl::import(s, d)
}

#[async_trait]
impl BackupRepository for InMemoryStore {
    async fn schema(&self) -> PortResult<Vec<String>> {
        Ok(vec!["memory".into()])
    }

    async fn export(&self) -> PortResult<BackupData> {
        let state = self.shared.lock().map_err(poisoned)?;
        Ok(export(&state))
    }

    async fn is_empty(&self) -> PortResult<bool> {
        let s = self.shared.lock().map_err(poisoned)?;
        Ok(s.users.is_empty() && s.households.is_empty())
    }

    async fn import(&self, data: &BackupData) -> PortResult<()> {
        let mut s = self.shared.lock().map_err(poisoned)?;
        // alles oder nichts wie in einer Transaktion
        let mut next = s.clone();
        import(&mut next, data)?;
        *s = next;
        Ok(())
    }
}
