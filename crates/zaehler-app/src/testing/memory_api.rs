//! API-Tokens und täglich verdichtete Ablesungen im In-Memory-Adapter
//! (SPEC_API_AUTH, SPEC_API_DAILY_READINGS) und ihre Abbildung auf
//! Archiv-Datensätze (SPEC_BACKUP_DATA).

use chrono::{DateTime, Utc};

use super::{MemoryUow, PortError, PortResult, State, Tenant, async_trait};
use crate::{
    backup::{ApiTokenRecord, BackupData},
    ids::{ApiTokenId, HouseholdId, ReadingId, UserId},
    model::{ApiToken, DailyOutcome, NewApiToken, Reading},
    port::ApiTokenRepository,
    secret,
};

#[derive(Debug, Clone)]
pub(super) struct StoredToken {
    household: HouseholdId,
    user: UserId,
    token: NewApiToken,
    revoked_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
}

fn invalid<E>(_: E) -> PortError {
    PortError::Invalid("backup.invalid")
}

fn to_token(s: &State, x: &StoredToken) -> ApiToken {
    ApiToken {
        id: x.token.id,
        household_id: x.household,
        user_id: x.user,
        user_name: s
            .users
            .iter()
            .find(|u| u.user.id == x.user)
            .map(|u| u.user.display_name.clone())
            .unwrap_or_default(),
        name: x.token.name.clone(),
        role: x.token.role,
        prefix: x.token.prefix.clone(),
        created_at: x.token.created_at,
        expires_at: x.token.expires_at,
        revoked_at: x.revoked_at,
        last_used_at: x.last_used_at,
    }
}

pub(super) fn delete_household(s: &mut State, hid: HouseholdId) {
    s.api_tokens.retain(|x| x.household != hid);
    let readings: Vec<ReadingId> = s.readings.iter().map(|r| r.id).collect();
    s.reading_tokens.retain(|r, _| readings.contains(r));
}

pub(super) fn export(s: &State, data: &mut BackupData) {
    for x in &s.api_tokens {
        data.api_tokens.push(ApiTokenRecord {
            id: x.token.id.0,
            household_id: x.household.0,
            user_id: x.user.0,
            name: x.token.name.clone(),
            role: x.token.role.as_str().into(),
            token_hash: secret::to_hex(&x.token.token_hash),
            prefix: x.token.prefix.clone(),
            created_at: x.token.created_at,
            expires_at: x.token.expires_at,
            revoked_at: x.revoked_at,
            last_used_at: x.last_used_at,
        });
    }
    for r in &mut data.readings {
        r.api_token_id = s.reading_tokens.get(&ReadingId(r.id)).map(|t| t.0);
    }
}

pub(super) fn import(s: &mut State, d: &BackupData) -> PortResult<()> {
    for x in &d.api_tokens {
        s.api_tokens.push(StoredToken {
            household: HouseholdId(x.household_id),
            user: UserId(x.user_id),
            token: NewApiToken {
                id: ApiTokenId(x.id),
                name: x.name.clone(),
                role: x.role.parse().map_err(invalid)?,
                token_hash: secret::hash_from_hex(&x.token_hash)
                    .ok_or(PortError::Invalid("backup.invalid"))?,
                prefix: x.prefix.clone(),
                created_at: x.created_at,
                expires_at: x.expires_at,
            },
            revoked_at: x.revoked_at,
            last_used_at: x.last_used_at,
        });
    }
    for r in &d.readings {
        if let Some(t) = r.api_token_id {
            s.reading_tokens.insert(ReadingId(r.id), ApiTokenId(t));
        }
    }
    Ok(())
}

impl MemoryUow {
    /// Siehe [`crate::port::ReadingRepository::upsert_daily`].
    pub(super) fn upsert_daily_in(
        &mut self,
        r: &Reading,
        token: ApiTokenId,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> PortResult<DailyOutcome> {
        let tokens = &self.state.reading_tokens;
        let existing = self
            .state
            .readings
            .iter()
            .find(|x| {
                x.register_id == r.register_id
                    && tokens.get(&x.id) == Some(&token)
                    && x.read_at >= from
                    && x.read_at < until
            })
            .map(|x| (x.id, x.read_at));
        let clash = |s: &State, except: Option<ReadingId>| {
            s.readings.iter().any(|x| {
                Some(x.id) != except
                    && x.register_id == r.register_id
                    && x.device_id == r.device_id
                    && x.read_at == r.read_at
            })
        };
        match existing {
            None => {
                if clash(&self.state, None) {
                    return Err(PortError::Conflict("reading".into()));
                }
                self.state.readings.push(r.clone());
                self.state.reading_tokens.insert(r.id, token);
                Ok(DailyOutcome::Stored(r.id))
            }
            Some((id, at)) if at < r.read_at => {
                if clash(&self.state, Some(id)) {
                    return Err(PortError::Conflict("reading".into()));
                }
                if let Some(x) = self.state.readings.iter_mut().find(|x| x.id == id) {
                    x.read_at = r.read_at;
                    x.device_id = r.device_id;
                    x.value = r.value;
                }
                Ok(DailyOutcome::Replaced(id))
            }
            Some(_) => Ok(DailyOutcome::Ignored),
        }
    }
}

#[async_trait]
impl ApiTokenRepository for MemoryUow {
    async fn create(&mut self, t: &Tenant, tok: &NewApiToken) -> PortResult<()> {
        if self
            .state
            .api_tokens
            .iter()
            .any(|x| x.token.token_hash == tok.token_hash)
        {
            return Err(PortError::Conflict("token".into()));
        }
        self.state.api_tokens.push(StoredToken {
            household: t.household_id(),
            user: t.user_id(),
            token: tok.clone(),
            revoked_at: None,
            last_used_at: None,
        });
        Ok(())
    }

    async fn find_by_hash(&mut self, hash: &[u8; 32]) -> PortResult<Option<ApiToken>> {
        Ok(self
            .state
            .api_tokens
            .iter()
            .find(|x| &x.token.token_hash == hash)
            .map(|x| to_token(&self.state, x)))
    }

    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<ApiToken>> {
        let mut out: Vec<ApiToken> = self
            .state
            .api_tokens
            .iter()
            .filter(|x| x.household == t.household_id())
            .map(|x| to_token(&self.state, x))
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        Ok(out)
    }

    async fn revoke(&mut self, t: &Tenant, id: ApiTokenId, at: DateTime<Utc>) -> PortResult<()> {
        let x = self
            .state
            .api_tokens
            .iter_mut()
            .find(|x| x.household == t.household_id() && x.token.id == id && x.revoked_at.is_none())
            .ok_or(PortError::NotFound)?;
        x.revoked_at = Some(at);
        Ok(())
    }

    async fn touch(&mut self, id: ApiTokenId, at: DateTime<Utc>) -> PortResult<()> {
        if let Some(x) = self.state.api_tokens.iter_mut().find(|x| x.token.id == id) {
            x.last_used_at = Some(at);
        }
        Ok(())
    }
}
