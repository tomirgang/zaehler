//! Dokumente auf SeaORM (SPEC_DOC_UPLOADS, SPEC_AI_INVOICE).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, sea_query::Expr,
};
use zaehler_app::{
    PortError, Tenant,
    ids::{BlobKey, DocumentId, MeterId, UserId},
    model::{Document, DocumentAnalysis, DocumentStatus, DocumentType},
    port::{DocumentRepository, PortResult},
};

use super::{SeaOrmStore, SeaOrmUow, data_err, db_err};
use crate::entity::{document, settlement};

fn open_status() -> Vec<&'static str> {
    DocumentStatus::ALL
        .into_iter()
        .filter(|s| s.is_open())
        .map(DocumentStatus::as_str)
        .collect()
}

fn to_document(m: document::Model) -> PortResult<Document> {
    Ok(Document {
        id: DocumentId(m.id),
        blob_key: BlobKey::parse(&m.blob_key).ok_or_else(|| data_err("blob_key"))?,
        media_type: DocumentType::from_media_type(&m.media_type)
            .ok_or_else(|| data_err("media_type"))?,
        bytes: m.bytes,
        original_name: m.original_name,
        created_by: UserId(m.created_by),
        created_at: m.created_at,
        status: m.status.parse().map_err(data_err)?,
        model: m.model,
        extraction: m
            .extraction
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(data_err)?,
        meter_id: m.meter_id.map(MeterId),
        analysed_at: m.analysed_at,
    })
}

impl SeaOrmStore {
    /// Dokumente vor `cutoff` ohne Verweis aus einer Abrechnung löschen
    /// (SPEC_DOC_UPLOADS); liefert ihre Dateien.
    pub(crate) async fn expire_documents_impl(
        &self,
        cutoff: DateTime<Utc>,
    ) -> PortResult<Vec<BlobKey>> {
        let referenced = sea_orm::sea_query::Query::select()
            .column(settlement::Column::DocumentId)
            .from(settlement::Entity)
            .and_where(settlement::Column::DocumentId.is_not_null())
            .to_owned();
        let expired = document::Entity::find()
            .filter(document::Column::CreatedAt.lt(cutoff))
            .filter(document::Column::Id.not_in_subquery(referenced))
            .all(self.conn())
            .await
            .map_err(db_err)?;
        if expired.is_empty() {
            return Ok(Vec::new());
        }
        document::Entity::delete_many()
            .filter(document::Column::Id.is_in(expired.iter().map(|d| d.id)))
            .exec(self.conn())
            .await
            .map_err(db_err)?;
        expired
            .into_iter()
            .map(|d| BlobKey::parse(&d.blob_key).ok_or_else(|| data_err("blob_key")))
            .collect()
    }
}

// @impl SPEC_DOC_UPLOADS, SPEC_SEC_TENANT_SCOPE
#[async_trait]
impl DocumentRepository for SeaOrmUow {
    async fn create(&mut self, t: &Tenant, d: &Document) -> PortResult<()> {
        document::Entity::insert(document::ActiveModel {
            id: Set(d.id.0),
            household_id: Set(t.household_id().0),
            kind: Set("invoice".into()),
            blob_key: Set(d.blob_key.to_string()),
            media_type: Set(d.media_type.media_type().into()),
            bytes: Set(d.bytes),
            original_name: Set(d.original_name.clone()),
            created_by: Set(d.created_by.0),
            created_at: Set(d.created_at),
            status: Set(d.status.as_str().into()),
            model: Set(d.model.clone()),
            extraction: Set(d
                .extraction
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(data_err)?),
            meter_id: Set(d.meter_id.map(|m| m.0)),
            analysed_at: Set(d.analysed_at),
            finished_at: Set(None),
        })
        .exec_without_returning(&self.txn)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get(&mut self, t: &Tenant, id: DocumentId) -> PortResult<Option<Document>> {
        document::Entity::find_by_id(id.0)
            .filter(document::Column::HouseholdId.eq(t.household_id().0))
            .one(&self.txn)
            .await
            .map_err(db_err)?
            .map(to_document)
            .transpose()
    }

    async fn list_open(&mut self, t: &Tenant) -> PortResult<Vec<Document>> {
        document::Entity::find()
            .filter(document::Column::HouseholdId.eq(t.household_id().0))
            .filter(document::Column::Status.is_in(open_status()))
            .order_by_desc(document::Column::CreatedAt)
            .order_by_desc(document::Column::Id)
            .all(&self.txn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(to_document)
            .collect()
    }

    async fn record_analysis(
        &mut self,
        t: &Tenant,
        id: DocumentId,
        a: &DocumentAnalysis,
    ) -> PortResult<()> {
        if let Some(m) = a.meter_id {
            self.require_meter(t, m.0).await?;
        }
        let extraction = a
            .extraction
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(data_err)?;
        let res = document::Entity::update_many()
            .col_expr(document::Column::Model, Expr::value(a.model.clone()))
            .col_expr(document::Column::Extraction, Expr::value(extraction))
            .col_expr(
                document::Column::MeterId,
                Expr::value(a.meter_id.map(|m| m.0)),
            )
            .col_expr(document::Column::Status, Expr::value(a.status.as_str()))
            .col_expr(document::Column::AnalysedAt, Expr::value(Some(a.at)))
            .filter(document::Column::Id.eq(id.0))
            .filter(document::Column::HouseholdId.eq(t.household_id().0))
            .filter(document::Column::Status.is_in(open_status()))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }

    async fn finish(
        &mut self,
        t: &Tenant,
        id: DocumentId,
        status: DocumentStatus,
        at: DateTime<Utc>,
    ) -> PortResult<()> {
        let res = document::Entity::update_many()
            .col_expr(document::Column::Status, Expr::value(status.as_str()))
            .col_expr(document::Column::FinishedAt, Expr::value(Some(at)))
            .filter(document::Column::Id.eq(id.0))
            .filter(document::Column::HouseholdId.eq(t.household_id().0))
            .filter(document::Column::Status.is_in(open_status()))
            .exec(&self.txn)
            .await
            .map_err(db_err)?;
        if res.rows_affected == 0 {
            return Err(PortError::NotFound);
        }
        Ok(())
    }
}
