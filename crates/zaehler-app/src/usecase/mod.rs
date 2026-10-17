//! Anwendungsfälle (Driving Ports). Jeder Anwendungsfall ist eine Struktur mit
//! den benötigten Ports und einer Methode `execute`.

use std::sync::Arc;

use crate::port::{
    BackupRepository, BlobStore, Clock, DocumentExtractor, IdGenerator, ImageNormalizer,
    JobRepository, MeterRecognizer, Notifier, UnitOfWorkFactory,
};

pub mod analysis;
pub mod asset_cost;
pub mod auth;
pub mod backup;
pub mod capture;
pub mod contract;
pub mod cost;
pub mod document;
pub mod household;
pub mod housekeeping;
pub mod import;
pub mod jobs;
pub mod member;
pub mod meter;
pub mod notify;
pub mod reading;
mod validate;
pub mod vehicle;
pub mod vehicle_report;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod document_tests;
#[cfg(test)]
mod expectation_tests;
#[cfg(test)]
mod jobs_tests;
#[cfg(test)]
mod member_tests;
#[cfg(test)]
mod meter_unit_tests;
#[cfg(test)]
mod meter_v1_tests;
#[cfg(test)]
mod notify_tests;
#[cfg(test)]
mod reading_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vehicle_tests;

/// Driven Ports, die alle Anwendungsfälle benötigen.
#[derive(Clone)]
pub struct Ports {
    pub uow: Arc<dyn UnitOfWorkFactory>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGenerator>,
    pub blobs: Arc<dyn BlobStore>,
    pub images: Arc<dyn ImageNormalizer>,
    /// `None` ohne AI-Konfiguration (REQ_AI_006)
    pub recognizer: Option<Arc<dyn MeterRecognizer>>,
    /// Rechnungserkennung; `None` ohne AI-Konfiguration (REQ_AI_006)
    pub extractor: Option<Arc<dyn DocumentExtractor>>,
    pub backup: Arc<dyn BackupRepository>,
    /// Abfragen der Jobs ohne Haushaltsbezug (SPEC_JOB_OUTBOX)
    pub jobs: Arc<dyn JobRepository>,
    /// E-Mail-Versand; `DisabledNotifier` ohne SMTP (SPEC_NTF_EMAIL)
    pub notifier: Arc<dyn Notifier>,
    pub settings: Settings,
}

/// Einstellungen der Anwendungsfälle aus der Konfiguration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Gültigkeit einer Einladung (`invitations.ttl_days`, SPEC_TEN_INVITES)
    pub invitation_ttl: chrono::Duration,
    /// Öffentliche Basis-URL ohne abschließenden Schrägstrich für Links in
    /// E-Mails (`server.public_url`)
    pub public_url: String,
    /// Größtes Dokument in Bytes (`upload.max_document_bytes`,
    /// SPEC_DOC_UPLOADS)
    pub max_document_bytes: usize,
}

/// Vorgabe für `upload.max_document_bytes` (10 MiB).
pub const DEFAULT_MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;

impl Default for Settings {
    fn default() -> Self {
        Self {
            invitation_ttl: chrono::Duration::days(7),
            public_url: "http://localhost:8080".into(),
            max_document_bytes: DEFAULT_MAX_DOCUMENT_BYTES,
        }
    }
}

macro_rules! use_case {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$doc])*
        #[derive(Clone)]
        pub struct $name {
            p: std::sync::Arc<crate::usecase::Ports>,
        }

        impl $name {
            #[must_use]
            pub fn new(p: crate::usecase::Ports) -> Self {
                Self { p: std::sync::Arc::new(p) }
            }
        }
    )+};
}
pub(crate) use use_case;

/// Alle Anwendungsfälle; wird von der Composition Root erzeugt und an den
/// Driving-Adapter übergeben.
#[derive(Clone)]
pub struct UseCases {
    pub sign_in: auth::SignIn,
    pub resolve_tenant: auth::ResolveTenant,
    pub set_locale: auth::SetLocale,
    pub create_household: household::CreateHousehold,
    pub get_household: household::GetHousehold,
    pub update_household: household::UpdateHousehold,
    pub delete_household: household::DeleteHousehold,
    pub create_meter: meter::CreateMeter,
    pub update_meter: meter::UpdateMeter,
    pub list_meters: meter::ListMeters,
    pub get_meter: meter::GetMeter,
    pub swap_device: meter::SwapDevice,
    pub archive_meter: meter::ArchiveMeter,
    pub restore_meter: meter::RestoreMeter,
    pub check_readings: reading::CheckReadings,
    pub record_readings: reading::RecordReadings,
    pub delete_reading: reading::DeleteReading,
    pub get_reading_photo: reading::GetReadingPhoto,
    pub get_reading: reading::GetReading,
    pub correct_reading: reading::CorrectReading,
    pub reading_history: reading::ReadingHistory,
    pub analyse_photo: capture::AnalysePhoto,
    pub queue_photos: capture::QueuePhotos,
    pub analyse_queued: capture::AnalyseQueued,
    pub get_draft: capture::GetDraft,
    pub confirm_draft: capture::ConfirmDraft,
    pub discard_draft: capture::DiscardDraft,
    pub get_draft_photo: capture::GetDraftPhoto,
    pub meter_analysis: analysis::MeterAnalysis,
    pub meter_chart_data: analysis::MeterChartData,
    pub dashboard: analysis::Dashboard,
    pub upload_csv: import::UploadCsv,
    pub preview_import: import::PreviewImport,
    pub commit_import: import::CommitImport,
    pub export_installation: backup::ExportInstallation,
    pub import_installation: backup::ImportInstallation,
    pub export_household: backup::ExportHousehold,
    pub import_household: backup::ImportHousehold,
    pub list_members: member::ListMembers,
    pub change_role: member::ChangeRole,
    pub remove_member: member::RemoveMember,
    pub leave_household: member::LeaveHousehold,
    pub list_invitations: member::ListInvitations,
    pub create_invitation: member::CreateInvitation,
    pub revoke_invitation: member::RevokeInvitation,
    pub preview_invitation: member::PreviewInvitation,
    pub accept_invitation: member::AcceptInvitation,
    pub get_notification_settings: notify::GetNotificationSettings,
    pub update_notification_settings: notify::UpdateNotificationSettings,
    pub create_contract: contract::CreateContract,
    pub update_contract: contract::UpdateContract,
    pub delete_contract: contract::DeleteContract,
    pub get_contract: contract::GetContract,
    pub meter_contracts: contract::MeterContracts,
    pub add_price_component: contract::AddPriceComponent,
    pub set_installment: contract::SetInstallment,
    pub add_adjustment: contract::AddAdjustment,
    pub record_settlement: contract::RecordSettlement,
    pub delete_contract_entry: contract::DeleteContractEntry,
    pub contract_templates: contract::ContractTemplates,
    pub meter_costs: cost::MeterCosts,
    pub household_costs: cost::HouseholdCosts,
    pub upload_document: document::UploadDocument,
    pub analyse_document: document::AnalyseDocument,
    pub list_documents: document::ListDocuments,
    pub get_document: document::GetDocument,
    pub get_document_file: document::GetDocumentFile,
    pub discard_document: document::DiscardDocument,
    pub confirm_invoice: document::ConfirmInvoice,
    pub confirm_receipt: document::ConfirmReceipt,
    pub list_vehicles: vehicle::ListVehicles,
    pub get_vehicle: vehicle::GetVehicle,
    pub create_vehicle: vehicle::CreateVehicle,
    pub update_vehicle: vehicle::UpdateVehicle,
    pub set_consumption_profile: vehicle::SetConsumptionProfile,
    pub delete_consumption_profile: vehicle::DeleteConsumptionProfile,
    pub set_acquisition: vehicle::SetAcquisition,
    pub record_charging: vehicle::RecordCharging,
    pub delete_charging: vehicle::DeleteCharging,
    pub list_asset_costs: asset_cost::ListAssetCosts,
    pub get_asset_cost: asset_cost::GetAssetCost,
    pub add_asset_cost: asset_cost::AddAssetCost,
    pub update_asset_cost: asset_cost::UpdateAssetCost,
    pub delete_asset_cost: asset_cost::DeleteAssetCost,
    pub vehicle_report: vehicle_report::VehicleReport,
}

impl UseCases {
    #[must_use]
    pub fn new(p: &Ports) -> Self {
        Self {
            sign_in: auth::SignIn::new(p.clone()),
            resolve_tenant: auth::ResolveTenant::new(p.clone()),
            set_locale: auth::SetLocale::new(p.clone()),
            create_household: household::CreateHousehold::new(p.clone()),
            get_household: household::GetHousehold::new(p.clone()),
            update_household: household::UpdateHousehold::new(p.clone()),
            delete_household: household::DeleteHousehold::new(p.clone()),
            create_meter: meter::CreateMeter::new(p.clone()),
            update_meter: meter::UpdateMeter::new(p.clone()),
            list_meters: meter::ListMeters::new(p.clone()),
            get_meter: meter::GetMeter::new(p.clone()),
            swap_device: meter::SwapDevice::new(p.clone()),
            archive_meter: meter::ArchiveMeter::new(p.clone()),
            restore_meter: meter::RestoreMeter::new(p.clone()),
            check_readings: reading::CheckReadings::new(p.clone()),
            record_readings: reading::RecordReadings::new(p.clone()),
            delete_reading: reading::DeleteReading::new(p.clone()),
            get_reading_photo: reading::GetReadingPhoto::new(p.clone()),
            get_reading: reading::GetReading::new(p.clone()),
            correct_reading: reading::CorrectReading::new(p.clone()),
            reading_history: reading::ReadingHistory::new(p.clone()),
            analyse_photo: capture::AnalysePhoto::new(p.clone()),
            queue_photos: capture::QueuePhotos::new(p.clone()),
            analyse_queued: capture::AnalyseQueued::new(p.clone()),
            get_draft: capture::GetDraft::new(p.clone()),
            confirm_draft: capture::ConfirmDraft::new(p.clone()),
            discard_draft: capture::DiscardDraft::new(p.clone()),
            get_draft_photo: capture::GetDraftPhoto::new(p.clone()),
            meter_analysis: analysis::MeterAnalysis::new(p.clone()),
            meter_chart_data: analysis::MeterChartData::new(p.clone()),
            dashboard: analysis::Dashboard::new(p.clone()),
            upload_csv: import::UploadCsv::new(p.clone()),
            preview_import: import::PreviewImport::new(p.clone()),
            commit_import: import::CommitImport::new(p.clone()),
            export_installation: backup::ExportInstallation::new(p.clone()),
            import_installation: backup::ImportInstallation::new(p.clone()),
            export_household: backup::ExportHousehold::new(p.clone()),
            import_household: backup::ImportHousehold::new(p.clone()),
            list_members: member::ListMembers::new(p.clone()),
            change_role: member::ChangeRole::new(p.clone()),
            remove_member: member::RemoveMember::new(p.clone()),
            leave_household: member::LeaveHousehold::new(p.clone()),
            list_invitations: member::ListInvitations::new(p.clone()),
            create_invitation: member::CreateInvitation::new(p.clone()),
            revoke_invitation: member::RevokeInvitation::new(p.clone()),
            preview_invitation: member::PreviewInvitation::new(p.clone()),
            accept_invitation: member::AcceptInvitation::new(p.clone()),
            get_notification_settings: notify::GetNotificationSettings::new(p.clone()),
            update_notification_settings: notify::UpdateNotificationSettings::new(p.clone()),
            create_contract: contract::CreateContract::new(p.clone()),
            update_contract: contract::UpdateContract::new(p.clone()),
            delete_contract: contract::DeleteContract::new(p.clone()),
            get_contract: contract::GetContract::new(p.clone()),
            meter_contracts: contract::MeterContracts::new(p.clone()),
            add_price_component: contract::AddPriceComponent::new(p.clone()),
            set_installment: contract::SetInstallment::new(p.clone()),
            add_adjustment: contract::AddAdjustment::new(p.clone()),
            record_settlement: contract::RecordSettlement::new(p.clone()),
            delete_contract_entry: contract::DeleteContractEntry::new(p.clone()),
            contract_templates: contract::ContractTemplates::new(p.clone()),
            meter_costs: cost::MeterCosts::new(p.clone()),
            household_costs: cost::HouseholdCosts::new(p.clone()),
            upload_document: document::UploadDocument::new(p.clone()),
            analyse_document: document::AnalyseDocument::new(p.clone()),
            list_documents: document::ListDocuments::new(p.clone()),
            get_document: document::GetDocument::new(p.clone()),
            get_document_file: document::GetDocumentFile::new(p.clone()),
            discard_document: document::DiscardDocument::new(p.clone()),
            confirm_invoice: document::ConfirmInvoice::new(p.clone()),
            confirm_receipt: document::ConfirmReceipt::new(p.clone()),
            list_vehicles: vehicle::ListVehicles::new(p.clone()),
            get_vehicle: vehicle::GetVehicle::new(p.clone()),
            create_vehicle: vehicle::CreateVehicle::new(p.clone()),
            update_vehicle: vehicle::UpdateVehicle::new(p.clone()),
            set_consumption_profile: vehicle::SetConsumptionProfile::new(p.clone()),
            delete_consumption_profile: vehicle::DeleteConsumptionProfile::new(p.clone()),
            set_acquisition: vehicle::SetAcquisition::new(p.clone()),
            record_charging: vehicle::RecordCharging::new(p.clone()),
            delete_charging: vehicle::DeleteCharging::new(p.clone()),
            list_asset_costs: asset_cost::ListAssetCosts::new(p.clone()),
            get_asset_cost: asset_cost::GetAssetCost::new(p.clone()),
            add_asset_cost: asset_cost::AddAssetCost::new(p.clone()),
            update_asset_cost: asset_cost::UpdateAssetCost::new(p.clone()),
            delete_asset_cost: asset_cost::DeleteAssetCost::new(p.clone()),
            vehicle_report: vehicle_report::VehicleReport::new(p.clone()),
        }
    }
}
