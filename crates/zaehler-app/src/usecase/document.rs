//! Dokumente hochladen, Rechnungen erkennen und Vorschläge übernehmen
//! (SPEC_DOC_UPLOADS, SPEC_AI_INVOICE).

use std::time::Instant;

use bytes::Bytes;
use zaehler_domain::reading::ReadingSource;

use crate::{
    AppError, PortError, Role, Tenant,
    ids::{BlobKey, ContractId, DocumentId, MeterId, PositionId},
    model::{Document, DocumentAnalysis, DocumentStatus, DocumentType},
    port::{DocumentContent, UnitOfWork},
    usecase::{
        capture::normalize_serial,
        contract::{
            ComponentInput, ContractInput, InstallmentInput, SettlementInput, add_component_in,
            create_in, load_contract, record_settlement_in, set_installment_in,
        },
        reading::{ReadingExtras, ReadingsInput, RecordOutcome, RegisterCheck, record_in},
        use_case,
    },
};

use_case!(
    /// PDF oder Foto speichern (Member).
    UploadDocument,
    /// Rechnung erkennen und Messstelle zuordnen (Member).
    AnalyseDocument,
    /// Offene Dokumente des Haushalts (Member).
    ListDocuments,
    /// Dokument mit Vorschlag (Viewer).
    GetDocument,
    /// Datei eines Dokuments zum Herunterladen (Viewer).
    GetDocumentFile,
    /// Dokument verwerfen (Member).
    DiscardDocument,
    /// Ausgewählte Vorschläge einer Rechnung übernehmen (Member; Vertrag,
    /// Preisstände und Abschlag Owner).
    ConfirmInvoice,
);

/// Höchstlänge des angezeigten Dateinamens.
const MAX_NAME: usize = 120;

/// Dateiname zur Anzeige: letzter Pfadteil ohne Steuerzeichen, gekürzt.
fn display_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = last
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    let clean = clean.trim().to_owned();
    if clean.is_empty() {
        "dokument".into()
    } else {
        clean
    }
}

async fn load_open(
    uow: &mut dyn UnitOfWork,
    t: &Tenant,
    id: DocumentId,
) -> Result<Document, AppError> {
    uow.documents()
        .get(t, id)
        .await?
        .filter(|d| d.status.is_open())
        .ok_or(AppError::NotFound)
}

impl UploadDocument {
    /// PDFs (Magic Bytes `%PDF-`) werden unverändert gespeichert, Fotos
    /// normalisiert; alles andere wird abgelehnt (SPEC_DOC_UPLOADS).
    // @impl SPEC_DOC_UPLOADS, SPEC_SEC_UPLOADS
    #[tracing::instrument(name = "UploadDocument", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        bytes: Bytes,
        original_name: &str,
    ) -> Result<DocumentId, AppError> {
        t.require(Role::Member)?;
        if bytes.is_empty() {
            return Err(AppError::field("file", "document.required"));
        }
        let max = self.p.settings.max_document_bytes;
        if bytes.len() > max {
            let mut e = crate::FieldErrors::default();
            e.add_with(
                "file",
                "document.too_large",
                vec![("max", (max / (1024 * 1024)).max(1).to_string())],
            );
            return Err(AppError::Validation(e));
        }
        let (media_type, content) = if bytes.starts_with(b"%PDF-") {
            (DocumentType::Pdf, bytes)
        } else {
            let jpeg = self.p.images.normalize(bytes).await.map_err(|e| match e {
                PortError::Invalid(_) => AppError::field("file", "document.type_invalid"),
                other => other.into(),
            })?;
            (DocumentType::Jpeg, jpeg.bytes)
        };
        let id = DocumentId(self.p.ids.new_uuid());
        let key = BlobKey::document(t.household_id(), id, media_type == DocumentType::Pdf);
        let size = i64::try_from(content.len()).unwrap_or(i64::MAX);
        self.p
            .blobs
            .put(&key, content)
            .await
            .map_err(AppError::from_storage)?;
        let doc = Document {
            id,
            blob_key: key.clone(),
            media_type,
            bytes: size,
            original_name: display_name(original_name),
            created_by: t.user_id(),
            created_at: self.p.clock.now(),
            status: DocumentStatus::Uploaded,
            model: None,
            extraction: None,
            meter_id: None,
            analysed_at: None,
        };
        let saved = async {
            let mut uow = self.p.uow.begin().await?;
            uow.documents().create(t, &doc).await?;
            uow.commit().await
        }
        .await;
        if let Err(e) = saved {
            // Datei ohne Datensatz entfernen; Fehler dabei nur protokollieren
            if let Err(e) = self.p.blobs.delete(&key).await {
                tracing::warn!(error = %e, "document file not removed after failed insert");
            }
            return Err(e.into());
        }
        Ok(id)
    }
}

impl AnalyseDocument {
    /// Erkennung ist konfiguriert.
    #[must_use]
    pub fn ai_enabled(&self) -> bool {
        self.p.extractor.is_some()
    }

    /// Erkennung über `DocumentExtractor`; ohne AI-Konfiguration oder bei
    /// Fehlern bleibt das Dokument für die manuelle Übernahme offen
    /// (REQ_AI_006). Zuordnung der Messstelle über die Zählernummer.
    // @impl SPEC_AI_INVOICE, SPEC_AI_EXTRACTION
    #[tracing::instrument(name = "AnalyseDocument", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: DocumentId) -> Result<DocumentStatus, AppError> {
        t.require(Role::Member)?;
        let doc = {
            let mut uow = self.p.uow.begin().await?;
            load_open(uow.as_mut(), t, id).await?
        };
        let now = self.p.clock.now();
        let analysis = if let Some(extractor) = &self.p.extractor {
            let bytes = self
                .p
                .blobs
                .get(&doc.blob_key)
                .await
                .map_err(AppError::from_storage)?;
            let started = Instant::now();
            let result = extractor
                .invoice(&DocumentContent {
                    media_type: doc.media_type,
                    bytes,
                })
                .await;
            let outcome = result
                .as_ref()
                .map_or_else(crate::port::RecognitionError::outcome, |_| "ok");
            metrics::counter!("zaehler_documents_total", "kind" => "invoice", "outcome" => outcome)
                .increment(1);
            metrics::histogram!("zaehler_document_analysis_seconds")
                .record(started.elapsed().as_secs_f64());
            match result {
                Ok(extraction) => DocumentAnalysis {
                    model: Some(extractor.model().to_owned()),
                    meter_id: self.match_meter(t, &extraction).await?,
                    extraction: Some(extraction),
                    status: DocumentStatus::Analysed,
                    at: now,
                },
                Err(e) => {
                    tracing::warn!(outcome, error = %e, "invoice extraction failed");
                    DocumentAnalysis {
                        model: Some(extractor.model().to_owned()),
                        extraction: None,
                        meter_id: None,
                        status: DocumentStatus::Failed,
                        at: now,
                    }
                }
            }
        } else {
            DocumentAnalysis {
                model: None,
                extraction: None,
                meter_id: None,
                status: DocumentStatus::Failed,
                at: now,
            }
        };
        let mut uow = self.p.uow.begin().await?;
        uow.documents().record_analysis(t, id, &analysis).await?;
        uow.commit().await?;
        Ok(analysis.status)
    }

    /// Messstelle zur ersten bekannten Zählernummer der Rechnung.
    async fn match_meter(
        &self,
        t: &Tenant,
        x: &crate::model::InvoiceExtraction,
    ) -> Result<Option<MeterId>, AppError> {
        let mut uow = self.p.uow.begin().await?;
        for serial in x.readings.iter().filter_map(|r| r.serial_number.as_deref()) {
            let serial = normalize_serial(serial);
            if serial.is_empty() {
                continue;
            }
            if let Some((meter, _)) = uow.meters().find_by_serial(t, &serial).await? {
                return Ok(Some(meter));
            }
        }
        Ok(None)
    }
}

impl ListDocuments {
    #[tracing::instrument(name = "ListDocuments", skip_all)]
    pub async fn execute(&self, t: &Tenant) -> Result<Vec<Document>, AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        Ok(uow.documents().list_open(t).await?)
    }
}

impl GetDocument {
    #[tracing::instrument(name = "GetDocument", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: DocumentId) -> Result<Document, AppError> {
        t.require(Role::Viewer)?;
        let mut uow = self.p.uow.begin().await?;
        uow.documents().get(t, id).await?.ok_or(AppError::NotFound)
    }
}

impl GetDocumentFile {
    /// Art, Inhalt und Dateiname; die Auslieferung erfolgt nur als Download
    /// (SPEC_DOC_UPLOADS).
    // @impl SPEC_DOC_UPLOADS
    #[tracing::instrument(name = "GetDocumentFile", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        id: DocumentId,
    ) -> Result<(DocumentType, Bytes, String), AppError> {
        t.require(Role::Viewer)?;
        let doc = {
            let mut uow = self.p.uow.begin().await?;
            uow.documents()
                .get(t, id)
                .await?
                .ok_or(AppError::NotFound)?
        };
        let bytes = self
            .p
            .blobs
            .get(&doc.blob_key)
            .await
            .map_err(AppError::from_storage)?;
        Ok((doc.media_type, bytes, doc.original_name))
    }
}

impl DiscardDocument {
    /// Das Housekeeping entfernt die Datei später (SPEC_DOC_UPLOADS).
    #[tracing::instrument(name = "DiscardDocument", skip_all)]
    pub async fn execute(&self, t: &Tenant, id: DocumentId) -> Result<(), AppError> {
        t.require(Role::Member)?;
        let mut uow = self.p.uow.begin().await?;
        load_open(uow.as_mut(), t, id).await?;
        uow.documents()
            .finish(t, id, DocumentStatus::Discarded, self.p.clock.now())
            .await?;
        uow.commit().await?;
        Ok(())
    }
}

/// Vertrag, auf den sich Preisstände, Abschlag und Abrechnung beziehen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractChoice {
    /// Kein Vertrag; nur Ablesungen
    None,
    /// Neuer Vertrag aus der Rechnung
    Create(ContractInput),
    /// Bestehender Vertrag, optional um Preisstände ergänzt
    Existing {
        contract: ContractId,
        components: Vec<(PositionId, ComponentInput)>,
    },
}

/// Ausgewählte Vorschläge einer Rechnung (SPEC_AI_INVOICE).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceConfirmation {
    pub meter_id: MeterId,
    /// Ablesungen je Zeitpunkt
    pub readings: Vec<ReadingsInput>,
    /// Plausibilitätswarnungen bestätigt
    pub confirm_warnings: bool,
    pub contract: ContractChoice,
    pub installment: Option<InstallmentInput>,
    pub settlement: Option<SettlementInput>,
}

/// Ergebnis von [`ConfirmInvoice`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmOutcome {
    /// Gespeichert; Vertrag, auf den sich die Übernahme bezieht
    Saved { contract: Option<ContractId> },
    /// Warnungen je Zeitpunkt; gespeichert wird erst nach Bestätigung
    NeedsConfirmation(Vec<(chrono::DateTime<chrono::Utc>, Vec<RegisterCheck>)>),
}

impl ConfirmInvoice {
    /// Speichert alle ausgewählten Vorschläge in einer Unit of Work:
    /// Ablesungen (Quelle `invoice`, Plausibilität wie bei der Erfassung),
    /// Vertrag oder Preisstände, Abschlag und Abrechnung mit Verweis auf das
    /// Dokument (SPEC_AI_INVOICE).
    // @impl SPEC_AI_INVOICE, SPEC_RDG_PLAUSIBILITY
    #[tracing::instrument(name = "ConfirmInvoice", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        id: DocumentId,
        c: InvoiceConfirmation,
    ) -> Result<ConfirmOutcome, AppError> {
        t.require(Role::Member)?;
        let owner_parts = match &c.contract {
            ContractChoice::None => false,
            ContractChoice::Create(_) => true,
            ContractChoice::Existing { components, .. } => !components.is_empty(),
        } || c.installment.is_some();
        if owner_parts {
            t.require(Role::Owner)?;
        }
        let mut uow = self.p.uow.begin().await?;
        load_open(uow.as_mut(), t, id).await?;
        let meter = uow
            .meters()
            .get(t, c.meter_id)
            .await?
            .ok_or(AppError::NotFound)?;

        let mut warnings = Vec::new();
        for input in &c.readings {
            let input = ReadingsInput {
                meter_id: c.meter_id,
                ..input.clone()
            };
            let outcome = record_in(
                &self.p,
                uow.as_mut(),
                t,
                &input,
                c.confirm_warnings,
                ReadingSource::Invoice,
                ReadingExtras::default(),
            )
            .await?;
            if let RecordOutcome::NeedsConfirmation(checks) = outcome {
                warnings.push((input.at, checks));
            }
        }
        if !warnings.is_empty() {
            return Ok(ConfirmOutcome::NeedsConfirmation(warnings));
        }

        let contract = match &c.contract {
            ContractChoice::None => None,
            ContractChoice::Create(input) => {
                Some(create_in(&self.p, uow.as_mut(), t, &meter, input).await?)
            }
            ContractChoice::Existing {
                contract,
                components,
            } => {
                let existing = load_contract(uow.as_mut(), t, *contract).await?;
                if existing.meter_id != c.meter_id {
                    return Err(AppError::NotFound);
                }
                for (i, (position, input)) in components.iter().enumerate() {
                    add_component_in(
                        &self.p,
                        uow.as_mut(),
                        t,
                        &existing,
                        *position,
                        input.clone(),
                        &format!("price{i}"),
                    )
                    .await?;
                }
                Some(*contract)
            }
        };
        if c.installment.is_some() || c.settlement.is_some() {
            let Some(target) = contract else {
                return Err(AppError::field("contract", "document.contract_required"));
            };
            if let Some(input) = &c.installment {
                let current = load_contract(uow.as_mut(), t, target).await?;
                set_installment_in(&self.p, uow.as_mut(), t, &current, input).await?;
            }
            if let Some(input) = &c.settlement {
                record_settlement_in(&self.p, uow.as_mut(), t, target, input, Some(id)).await?;
            }
        }
        uow.documents()
            .finish(t, id, DocumentStatus::Confirmed, self.p.clock.now())
            .await?;
        uow.commit().await?;
        Ok(ConfirmOutcome::Saved { contract })
    }
}
