//! Dokumente im In-Memory-Adapter (SPEC_DOC_UPLOADS) und ihre Abbildung
//! auf Archiv-Datensätze.

use chrono::{DateTime, Utc};

use super::{BlobKey, HouseholdId, MemoryUow, PortError, PortResult, State, Tenant, async_trait};
use crate::{
    backup::{BackupData, DocumentRecord},
    ids::{DocumentId, MeterId, UserId},
    model::{Document, DocumentAnalysis, DocumentStatus, DocumentType, InvoiceExtraction},
    port::{DocumentContent, DocumentExtractor, DocumentRepository, RecognitionError},
};

/// Entfernt Dokumente vor `cutoff` ohne Verweis aus einer Abrechnung, einem
/// Ladevorgang oder einem Kostenbuch-Eintrag und liefert ihre Dateien.
pub(super) fn expire(s: &mut State, cutoff: DateTime<Utc>) -> Vec<BlobKey> {
    let mut referenced: Vec<uuid::Uuid> = s
        .contracts
        .iter()
        .flat_map(|c| c.settlements.iter().filter_map(|x| x.document_id))
        .collect();
    referenced.extend(super::vehicles_impl::referenced_documents(s));
    let mut keys = Vec::new();
    s.documents.retain(|(_, d)| {
        let expired = d.created_at < cutoff && !referenced.contains(&d.id.0);
        if expired {
            keys.push(d.blob_key.clone());
        }
        !expired
    });
    keys
}

pub(super) fn export(s: &State, data: &mut BackupData) {
    for (h, d) in &s.documents {
        data.documents.push(DocumentRecord {
            id: d.id.0,
            household_id: h.0,
            kind: d.kind.as_str().into(),
            blob_key: d.blob_key.to_string(),
            media_type: d.media_type.media_type().into(),
            bytes: d.bytes,
            original_name: d.original_name.clone(),
            created_by: d.created_by.0,
            created_at: d.created_at,
            status: d.status.as_str().into(),
            model: d.model.clone(),
            extraction: crate::model::extraction_json(d.extraction.as_ref(), d.receipt.as_ref()),
            meter_id: d.meter_id.map(|m| m.0),
            analysed_at: d.analysed_at,
            finished_at: None,
        });
    }
}

pub(super) fn import(s: &mut State, d: &BackupData) -> PortResult<()> {
    let invalid = |_| PortError::Invalid("backup.invalid");
    for x in &d.documents {
        let kind: crate::model::DocumentKind = x
            .kind
            .parse()
            .map_err(|_| PortError::Invalid("backup.invalid"))?;
        let (extraction, receipt) =
            crate::model::parse_extraction(kind, x.extraction.as_deref()).map_err(invalid)?;
        s.documents.push((
            HouseholdId(x.household_id),
            Document {
                id: DocumentId(x.id),
                blob_key: BlobKey::parse(&x.blob_key)
                    .ok_or(PortError::Invalid("backup.invalid"))?,
                media_type: DocumentType::from_media_type(&x.media_type)
                    .ok_or(PortError::Invalid("backup.invalid"))?,
                bytes: x.bytes,
                original_name: x.original_name.clone(),
                created_by: UserId(x.created_by),
                created_at: x.created_at,
                status: x
                    .status
                    .parse()
                    .map_err(|_| PortError::Invalid("backup.invalid"))?,
                model: x.model.clone(),
                extraction,
                receipt,
                kind,
                meter_id: x.meter_id.map(MeterId),
                analysed_at: x.analysed_at,
            },
        ));
    }
    Ok(())
}

impl MemoryUow {
    fn document_mut(&mut self, t: &Tenant, id: DocumentId) -> PortResult<&mut Document> {
        self.state
            .documents
            .iter_mut()
            .find(|(h, d)| *h == t.household_id() && d.id == id && d.status.is_open())
            .map(|(_, d)| d)
            .ok_or(PortError::NotFound)
    }
}

#[async_trait]
impl DocumentRepository for MemoryUow {
    async fn create(&mut self, t: &Tenant, d: &Document) -> PortResult<()> {
        if self.state.documents.iter().any(|(_, x)| x.id == d.id) {
            return Err(PortError::Conflict("documents.id".into()));
        }
        self.state.documents.push((t.household_id(), d.clone()));
        Ok(())
    }

    async fn get(&mut self, t: &Tenant, id: DocumentId) -> PortResult<Option<Document>> {
        Ok(self
            .state
            .documents
            .iter()
            .find(|(h, d)| *h == t.household_id() && d.id == id)
            .map(|(_, d)| d.clone()))
    }

    async fn list_open(&mut self, t: &Tenant) -> PortResult<Vec<Document>> {
        let mut out: Vec<Document> = self
            .state
            .documents
            .iter()
            .filter(|(h, d)| *h == t.household_id() && d.status.is_open())
            .map(|(_, d)| d.clone())
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        Ok(out)
    }

    async fn record_analysis(
        &mut self,
        t: &Tenant,
        id: DocumentId,
        a: &DocumentAnalysis,
    ) -> PortResult<()> {
        if let Some(m) = a.meter_id
            && !self.state.owns_meter(t, m)
        {
            return Err(PortError::NotFound);
        }
        let d = self.document_mut(t, id)?;
        d.model.clone_from(&a.model);
        d.extraction.clone_from(&a.extraction);
        d.receipt.clone_from(&a.receipt);
        d.meter_id = a.meter_id;
        d.status = a.status;
        d.analysed_at = Some(a.at);
        Ok(())
    }

    async fn finish(
        &mut self,
        t: &Tenant,
        id: DocumentId,
        status: DocumentStatus,
        _at: DateTime<Utc>,
    ) -> PortResult<()> {
        self.document_mut(t, id)?.status = status;
        Ok(())
    }
}

/// Ersatz für die Rechnungserkennung mit festem Ergebnis.
pub struct StubExtractor(pub Result<InvoiceExtraction, RecognitionError>);

impl StubExtractor {
    /// Mit Vorschlag für Belege.
    #[must_use]
    pub fn with_receipt(
        self,
        receipt: Result<crate::model::ReceiptExtraction, RecognitionError>,
    ) -> StubExtractors {
        StubExtractors {
            invoice: self.0,
            receipt,
        }
    }
}

#[async_trait]
impl DocumentExtractor for StubExtractor {
    fn model(&self) -> &'static str {
        "stub"
    }

    async fn invoice(&self, _: &DocumentContent) -> Result<InvoiceExtraction, RecognitionError> {
        self.0.clone()
    }

    async fn receipt(
        &self,
        _: &DocumentContent,
    ) -> Result<crate::model::ReceiptExtraction, RecognitionError> {
        Err(RecognitionError::Failed("kein Beleg".into()))
    }
}

/// Erkennung mit festen Ergebnissen für Rechnungen und Belege.
pub struct StubExtractors {
    pub invoice: Result<InvoiceExtraction, RecognitionError>,
    pub receipt: Result<crate::model::ReceiptExtraction, RecognitionError>,
}

#[async_trait]
impl DocumentExtractor for StubExtractors {
    fn model(&self) -> &'static str {
        "stub"
    }

    async fn invoice(&self, _: &DocumentContent) -> Result<InvoiceExtraction, RecognitionError> {
        self.invoice.clone()
    }

    async fn receipt(
        &self,
        _: &DocumentContent,
    ) -> Result<crate::model::ReceiptExtraction, RecognitionError> {
        self.receipt.clone()
    }
}
