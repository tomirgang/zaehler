//! Typisierte IDs; verhindern Verwechslungen zwischen Entitäten.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            #[must_use]
            pub fn into_uuid(self) -> Uuid {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    )+};
}

id_type!(
    /// Haushalt (Mandant)
    HouseholdId,
    /// Nutzer
    UserId,
    /// Messstelle
    MeterId,
    /// Physisches Zählergerät
    DeviceId,
    /// Register (Zählwerk)
    RegisterId,
    /// Ablesung
    ReadingId,
    /// AI-Entwurf der Foto-Erfassung
    DraftId,
    /// Einladung in einen Haushalt
    InvitationId,
    /// Änderung an einer Ablesung
    RevisionId,
    /// Eintrag der Outbox
    OutboxId,
    /// Vertrag einer Messstelle
    ContractId,
    /// Kostenposition eines Vertrags
    PositionId,
    /// Preisstand einer Kostenposition
    PriceComponentId,
    /// Abschlagsstand eines Vertrags
    InstallmentId,
    /// Bonus oder Gutschrift
    AdjustmentId,
    /// Erfasste Jahresabrechnung
    SettlementId,
    /// Hochgeladenes Dokument (Rechnung)
    DocumentId,
    /// Verbrauchsprofil eines Fahrzeugs
    ProfileId,
    /// Ladevorgang eines Fahrzeugs
    ChargingId,
    /// Eintrag im Kostenbuch
    AssetCostId,
    /// PV-Anlage
    PvSystemId,
);

/// Schlüssel im Objektspeicher, immer unter `households/{household_id}/`.
///
/// Entsteht nur über die Konstruktoren je Verwendungszweck oder über
/// [`BlobKey::parse`], das genau diese Formen akzeptiert. Damit sind
/// Pfadmanipulationen und Zugriffe außerhalb des Haushaltspräfixes
/// ausgeschlossen.
// @impl SPEC_STORE_BLOBS
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobKey {
    key: String,
    household: HouseholdId,
    kind: BlobKind,
}

/// Verwendungszweck eines Objekts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlobKind {
    /// Belegfoto einer Ablesung (`readings/{id}.jpg`)
    ReadingPhoto,
    /// Foto eines AI-Entwurfs (`drafts/{id}.jpg`)
    DraftPhoto,
    /// Temporäre Datei des CSV-Imports (`tmp/{id}.csv`)
    Tmp,
    /// Dokument als PDF (`documents/{id}.pdf`, SPEC_DOC_UPLOADS)
    DocumentPdf,
    /// Dokument als normalisiertes Foto (`documents/{id}.jpg`)
    DocumentJpeg,
}

impl BlobKind {
    const ALL: [Self; 5] = [
        Self::ReadingPhoto,
        Self::DraftPhoto,
        Self::Tmp,
        Self::DocumentPdf,
        Self::DocumentJpeg,
    ];

    fn dir_and_ext(self) -> (&'static str, &'static str) {
        match self {
            Self::ReadingPhoto => ("readings", "jpg"),
            Self::DraftPhoto => ("drafts", "jpg"),
            Self::Tmp => ("tmp", "csv"),
            Self::DocumentPdf => ("documents", "pdf"),
            Self::DocumentJpeg => ("documents", "jpg"),
        }
    }

    /// Dauerhaft gespeicherte Datei, die in Archive gehört (Fotos und
    /// Dokumente, keine temporären Dateien).
    #[must_use]
    pub fn archived(self) -> bool {
        !matches!(self, Self::Tmp)
    }
}

impl BlobKey {
    fn build(household: HouseholdId, kind: BlobKind, id: Uuid) -> Self {
        let (dir, ext) = kind.dir_and_ext();
        Self {
            key: format!("{}{dir}/{id}.{ext}", Self::household_prefix(household)),
            household,
            kind,
        }
    }

    #[must_use]
    pub fn reading_photo(household: HouseholdId, reading: ReadingId) -> Self {
        Self::build(household, BlobKind::ReadingPhoto, reading.0)
    }

    #[must_use]
    pub fn draft_photo(household: HouseholdId, draft: DraftId) -> Self {
        Self::build(household, BlobKind::DraftPhoto, draft.0)
    }

    /// Temporäre Datei des CSV-Imports.
    #[must_use]
    pub fn tmp(household: HouseholdId, id: Uuid) -> Self {
        Self::build(household, BlobKind::Tmp, id)
    }

    /// Hochgeladenes Dokument; PDF oder normalisiertes Foto (SPEC_DOC_UPLOADS).
    #[must_use]
    pub fn document(household: HouseholdId, document: DocumentId, pdf: bool) -> Self {
        let kind = if pdf {
            BlobKind::DocumentPdf
        } else {
            BlobKind::DocumentJpeg
        };
        Self::build(household, kind, document.0)
    }

    /// Präfix aller Objekte eines Haushalts, mit abschließendem `/`.
    #[must_use]
    pub fn household_prefix(household: HouseholdId) -> String {
        format!("households/{household}/")
    }

    /// Schlüssel aus seiner Textform (Datenbank, Listing des Objektspeichers);
    /// `None` für alles, was kein Konstruktor erzeugen würde.
    #[must_use]
    pub fn parse(key: &str) -> Option<Self> {
        let rest = key.strip_prefix("households/")?;
        let (household, rest) = rest.split_once('/')?;
        let household = HouseholdId(Uuid::try_parse(household).ok()?);
        let (dir, file) = rest.split_once('/')?;
        let (id, ext) = file.split_once('.')?;
        let id = Uuid::try_parse(id).ok()?;
        let kind = BlobKind::ALL
            .into_iter()
            .find(|k| k.dir_and_ext() == (dir, ext))?;
        let parsed = Self::build(household, kind, id);
        // nur die kanonische Schreibweise (UUID in Kleinbuchstaben)
        (parsed.key == key).then_some(parsed)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn household(&self) -> HouseholdId {
        self.household
    }

    #[must_use]
    pub fn kind(&self) -> BlobKind {
        self.kind
    }
}

impl std::fmt::Display for BlobKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // @test SPEC_STORE_BLOBS
    #[test]
    fn blob_keys_are_scoped_to_the_household() {
        let h = HouseholdId(Uuid::now_v7());
        let r = ReadingId(Uuid::now_v7());
        let key = BlobKey::reading_photo(h, r);
        assert_eq!(key.as_str(), format!("households/{h}/readings/{r}.jpg"));
        assert_eq!(key.household(), h);
        assert_eq!(BlobKey::parse(key.as_str()), Some(key.clone()));
        let tmp = BlobKey::tmp(h, Uuid::now_v7());
        assert_eq!(tmp.kind(), BlobKind::Tmp);
        assert_eq!(BlobKey::parse(tmp.as_str()), Some(tmp));
        let d = BlobKey::draft_photo(h, DraftId(Uuid::now_v7()));
        assert!(d.as_str().starts_with(&BlobKey::household_prefix(h)));
        assert_eq!(BlobKey::parse(d.as_str()), Some(d));
    }

    // @test SPEC_STORE_BLOBS, SPEC_SEC_UPLOADS
    #[test]
    fn blob_key_parse_rejects_foreign_shapes() {
        let h = HouseholdId(Uuid::now_v7());
        let id = Uuid::now_v7();
        let upper = id.to_string().to_uppercase();
        for bad in [
            String::new(),
            format!("households/{h}/readings/{id}.png"),
            format!("households/{h}/readings/{id}.jpg/x"),
            format!("households/{h}/other/{id}.jpg"),
            format!("households/{h}/readings/../{id}.jpg"),
            format!("households/../readings/{id}.jpg"),
            format!("/households/{h}/readings/{id}.jpg"),
            format!("households/{h}/readings/{upper}.jpg"),
            format!("households/{h}/tmp/{id}.jpg"),
        ] {
            assert_eq!(BlobKey::parse(&bad), None, "{bad}");
        }
    }
}
