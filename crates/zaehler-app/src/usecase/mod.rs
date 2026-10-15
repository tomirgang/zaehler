//! Anwendungsfälle (Driving Ports). Jeder Anwendungsfall ist eine Struktur mit
//! den benötigten Ports und einer Methode `execute`.

use std::sync::Arc;

use crate::port::{
    BlobStore, Clock, IdGenerator, ImageNormalizer, MeterRecognizer, UnitOfWorkFactory,
};

pub mod auth;
pub mod capture;
pub mod household;
pub mod housekeeping;
pub mod meter;
pub mod reading;
mod validate;

#[cfg(test)]
mod tests;

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
}

macro_rules! use_case {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$doc])*
        #[derive(Clone)]
        pub struct $name {
            p: crate::usecase::Ports,
        }

        impl $name {
            #[must_use]
            pub fn new(p: crate::usecase::Ports) -> Self {
                Self { p }
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
    pub check_readings: reading::CheckReadings,
    pub record_readings: reading::RecordReadings,
    pub delete_reading: reading::DeleteReading,
    pub get_reading_photo: reading::GetReadingPhoto,
    pub analyse_photo: capture::AnalysePhoto,
    pub get_draft: capture::GetDraft,
    pub confirm_draft: capture::ConfirmDraft,
    pub discard_draft: capture::DiscardDraft,
    pub get_draft_photo: capture::GetDraftPhoto,
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
            check_readings: reading::CheckReadings::new(p.clone()),
            record_readings: reading::RecordReadings::new(p.clone()),
            delete_reading: reading::DeleteReading::new(p.clone()),
            get_reading_photo: reading::GetReadingPhoto::new(p.clone()),
            analyse_photo: capture::AnalysePhoto::new(p.clone()),
            get_draft: capture::GetDraft::new(p.clone()),
            confirm_draft: capture::ConfirmDraft::new(p.clone()),
            discard_draft: capture::DiscardDraft::new(p.clone()),
            get_draft_photo: capture::GetDraftPhoto::new(p.clone()),
        }
    }
}
