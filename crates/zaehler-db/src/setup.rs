//! Setup-Entwürfe mit explizitem Haushaltsscope (SPEC_SETUP_FLOW).

use async_trait::async_trait;
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, sea_query::OnConflict};
use zaehler_app::{
    Tenant,
    ids::{BlobKey, HouseholdId},
    model::SetupDraft,
    port::{PortResult, SetupDraftRepository},
};

use crate::{
    entity::setup_draft,
    uow::{SeaOrmUow, data_err, db_err},
};

#[async_trait]
impl SetupDraftRepository for SeaOrmUow {
    async fn get(&mut self, t: &Tenant) -> PortResult<Option<SetupDraft>> {
        setup_draft::Entity::find_by_id(t.household_id().0)
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(|row| serde_json::from_str(&row.payload).map_err(data_err))
            .transpose()
    }

    async fn save(&mut self, t: &Tenant, draft: &SetupDraft) -> PortResult<()> {
        let payload = serde_json::to_string(draft).map_err(data_err)?;
        setup_draft::Entity::insert(setup_draft::ActiveModel {
            household_id: Set(t.household_id().0),
            payload: Set(payload),
            created_at: Set(draft.created_at),
            updated_at: Set(draft.updated_at),
        })
        .on_conflict(
            OnConflict::column(setup_draft::Column::HouseholdId)
                .update_columns([setup_draft::Column::Payload, setup_draft::Column::UpdatedAt])
                .to_owned(),
        )
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete(&mut self, t: &Tenant) -> PortResult<Vec<BlobKey>> {
        let row = setup_draft::Entity::find()
            .filter(setup_draft::Column::HouseholdId.eq(t.household_id().0))
            .one(&self.txn)
            .await
            .map_err(db_err)?;
        let Some(row) = row else {
            return Ok(Vec::new());
        };
        let draft: SetupDraft = serde_json::from_str(&row.payload).map_err(data_err)?;
        let photos = draft
            .photos
            .values()
            .map(|key| {
                let key = BlobKey::parse(key).ok_or_else(|| data_err("setup photo key"))?;
                if key.household() != HouseholdId(row.household_id) {
                    return Err(data_err("setup photo household"));
                }
                Ok(key)
            })
            .collect::<PortResult<Vec<_>>>()?;
        setup_draft::Entity::delete_by_id(t.household_id().0)
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        Ok(photos)
    }
}
