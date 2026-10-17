//! API-Tokens und täglich verdichtete Ablesungen auf SeaORM (SPEC_API_AUTH,
//! SPEC_API_DAILY_READINGS).

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, sea_query::Expr,
};
use uuid::Uuid;
use zaehler_app::{
    PortError, Tenant,
    ids::{ApiTokenId, HouseholdId, ReadingId, UserId},
    model::{ApiToken, DailyOutcome, NewApiToken, Reading},
    port::{ApiTokenRepository, PortResult},
    secret,
};

use super::{SeaOrmUow, data_err, db_err, reading_model};
use crate::{
    entity::{api_token, reading, user},
    fixed,
};

fn to_token(m: api_token::Model, user_name: String) -> PortResult<ApiToken> {
    Ok(ApiToken {
        id: ApiTokenId(m.id),
        household_id: HouseholdId(m.household_id),
        user_id: UserId(m.user_id),
        user_name,
        name: m.name,
        role: m.role.parse().map_err(data_err)?,
        prefix: m.prefix,
        created_at: m.created_at,
        expires_at: m.expires_at,
        revoked_at: m.revoked_at,
        last_used_at: m.last_used_at,
    })
}

impl SeaOrmUow {
    async fn user_names(&self, ids: &[Uuid]) -> PortResult<HashMap<Uuid, String>> {
        Ok(user::Entity::find()
            .filter(user::Column::Id.is_in(ids.to_vec()))
            .all(&self.txn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|u| (u.id, u.display_name))
            .collect())
    }

    /// Siehe [`zaehler_app::port::ReadingRepository::upsert_daily`]; die
    /// Prüfung des Haushalts erfolgt beim Aufrufer. Gleiche Zeitpunkte werden
    /// vorab geprüft, damit kein Unique-Fehler die Transaktion abbricht.
    pub(super) async fn upsert_daily_in(
        &self,
        r: &Reading,
        token: ApiTokenId,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> PortResult<DailyOutcome> {
        let existing = reading::Entity::find()
            .filter(reading::Column::ApiTokenId.eq(token.0))
            .filter(reading::Column::RegisterId.eq(r.register_id.0))
            .filter(reading::Column::ReadAt.gte(from))
            .filter(reading::Column::ReadAt.lt(until))
            .order_by_desc(reading::Column::ReadAt)
            .one(&self.txn)
            .await
            .map_err(db_err)?;
        let clash = reading::Entity::find()
            .filter(reading::Column::RegisterId.eq(r.register_id.0))
            .filter(reading::Column::DeviceId.eq(r.device_id.0))
            .filter(reading::Column::ReadAt.eq(r.read_at))
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|m| m.id);
        match existing {
            None => {
                if clash.is_some() {
                    return Err(PortError::Conflict("reading".into()));
                }
                reading::Entity::insert(reading_model(r, Some(token.0))?)
                    .exec_without_returning(&self.txn)
                    .await
                    .map_err(db_err)?;
                Ok(DailyOutcome::Stored(r.id))
            }
            Some(m) if m.read_at < r.read_at => {
                if clash.is_some_and(|c| c != m.id) {
                    return Err(PortError::Conflict("reading".into()));
                }
                reading::Entity::update_many()
                    .col_expr(reading::Column::ReadAt, Expr::value(r.read_at))
                    .col_expr(reading::Column::DeviceId, Expr::value(r.device_id.0))
                    .col_expr(
                        reading::Column::ValueMilli,
                        Expr::value(fixed::quantity_to_db(r.value).map_err(data_err)?),
                    )
                    .filter(reading::Column::Id.eq(m.id))
                    .exec(&self.txn)
                    .await
                    .map_err(db_err)?;
                Ok(DailyOutcome::Replaced(ReadingId(m.id)))
            }
            Some(_) => Ok(DailyOutcome::Ignored),
        }
    }
}

// @impl SPEC_API_AUTH
#[async_trait]
impl ApiTokenRepository for SeaOrmUow {
    async fn create(&mut self, t: &Tenant, tok: &NewApiToken) -> PortResult<()> {
        api_token::Entity::insert(api_token::ActiveModel {
            id: Set(tok.id.0),
            household_id: Set(t.household_id().0),
            user_id: Set(t.user_id().0),
            name: Set(tok.name.clone()),
            role: Set(tok.role.as_str().to_owned()),
            token_hash: Set(secret::to_hex(&tok.token_hash)),
            prefix: Set(tok.prefix.clone()),
            created_at: Set(tok.created_at),
            expires_at: Set(tok.expires_at),
            revoked_at: Set(None),
            last_used_at: Set(None),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn find_by_hash(&mut self, hash: &[u8; 32]) -> PortResult<Option<ApiToken>> {
        let Some(m) = api_token::Entity::find()
            .filter(api_token::Column::TokenHash.eq(secret::to_hex(hash)))
            .one(&self.txn)
            .await
            .map_err(db_err)?
        else {
            return Ok(None);
        };
        let name = self
            .user_names(&[m.user_id])
            .await?
            .remove(&m.user_id)
            .unwrap_or_default();
        to_token(m, name).map(Some)
    }

    async fn list(&mut self, t: &Tenant) -> PortResult<Vec<ApiToken>> {
        let rows = api_token::Entity::find()
            .filter(api_token::Column::HouseholdId.eq(t.household_id().0))
            .order_by_desc(api_token::Column::CreatedAt)
            .order_by_desc(api_token::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?;
        let users: Vec<Uuid> = rows.iter().map(|m| m.user_id).collect();
        let names = self.user_names(&users).await?;
        rows.into_iter()
            .map(|m| {
                let name = names.get(&m.user_id).cloned().unwrap_or_default();
                to_token(m, name)
            })
            .collect()
    }

    async fn revoke(&mut self, t: &Tenant, id: ApiTokenId, at: DateTime<Utc>) -> PortResult<()> {
        let res = api_token::Entity::update_many()
            .col_expr(api_token::Column::RevokedAt, Expr::value(Some(at)))
            .filter(api_token::Column::Id.eq(id.0))
            .filter(api_token::Column::HouseholdId.eq(t.household_id().0))
            .filter(api_token::Column::RevokedAt.is_null())
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 1 {
            Ok(())
        } else {
            Err(PortError::NotFound)
        }
    }

    async fn touch(&mut self, id: ApiTokenId, at: DateTime<Utc>) -> PortResult<()> {
        api_token::Entity::update_many()
            .col_expr(api_token::Column::LastUsedAt, Expr::value(Some(at)))
            .filter(api_token::Column::Id.eq(id.0))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}
