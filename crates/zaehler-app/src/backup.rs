//! Archivformat für vollständigen Export und Import (SPEC_BACKUP_FORMAT).
//!
//! Ein Archiv ist eine ZIP-Datei mit `data.json` (alle Datensätze, Werte wie in
//! der Datenbank gespeichert), den Fotos unter `blobs/<Objektschlüssel>` und
//! `manifest.json` mit Formatversion, Schemastand und SHA-256 aller Teile. Das
//! Format ist unabhängig vom Datenbankschema versioniert: Neue Felder kommen
//! mit `#[serde(default)]` hinzu, damit ältere Archive lesbar bleiben.

use std::{
    collections::{BTreeSet, HashSet},
    io::{Read, Seek, Write},
};

use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{
    ids::{BlobKey, BlobKind},
    model::DraftStatus,
};

/// Kennung des Formats im Manifest.
pub const FORMAT: &str = "zaehler-backup";
/// Aktuelle Formatversion; Importe akzeptieren diese und ältere Versionen.
pub const FORMAT_VERSION: u32 = 1;
const MANIFEST: &str = "manifest.json";
const DATA: &str = "data.json";
const BLOB_PREFIX: &str = "blobs/";
/// Größte zulässige Datei im Archiv (entpackt), Schutz vor ZIP-Bomben.
pub const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
/// Größte zulässige Anzahl Dateien im Archiv.
pub const MAX_ENTRIES: usize = 2_000_000;

/// Umfang eines Archivs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Alle Nutzer, Haushalte und Daten der Installation
    Installation,
    /// Ein Haushalt ohne Nutzerkonten
    Household,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: Uuid,
    pub oidc_issuer: String,
    pub oidc_subject: String,
    pub email: Option<String>,
    pub display_name: String,
    pub locale: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HouseholdRecord {
    pub id: Uuid,
    pub name: String,
    pub timezone: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipRecord {
    pub household_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeterRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    pub name: String,
    pub kind: String,
    pub unit: String,
    pub reading_interval_days: Option<i32>,
    pub max_daily_rate_milli: Option<i64>,
    pub billing_start_month: Option<i16>,
    pub billing_start_day: Option<i16>,
    pub archived: bool,
    pub created_at: DateTime<Utc>,
    /// Ab `m20261016_000008_meter_v1`
    #[serde(default = "general_purpose")]
    pub purpose: String,
    #[serde(default)]
    pub schedule_kind: Option<String>,
    #[serde(default)]
    pub schedule_value: Option<i16>,
    #[serde(default)]
    pub archived_at: Option<DateTime<Utc>>,
    /// Ab `m20261016_000010_expectation`: angepasstes Profil als JSON
    #[serde(default)]
    pub season_weights: Option<String>,
    #[serde(default)]
    pub annual_target_milli: Option<i64>,
    #[serde(default)]
    pub target_tolerance_pct: Option<i16>,
}

fn general_purpose() -> String {
    "general".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub id: Uuid,
    pub meter_id: Uuid,
    pub serial_number: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub removed_at: Option<DateTime<Utc>>,
    pub rollover_digits: Option<i16>,
    pub decimal_places: i16,
    /// Übergeordnete Messstelle (ab `m20261016_000008_meter_v1`)
    #[serde(default)]
    pub parent_meter_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterRecord {
    pub id: Uuid,
    pub meter_id: Uuid,
    pub code: String,
    pub direction: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingRecord {
    pub id: Uuid,
    pub register_id: Uuid,
    pub device_id: Uuid,
    pub read_at: DateTime<Utc>,
    /// Stand in Tausendsteln (Festkomma wie in der Datenbank)
    pub value_milli: i64,
    pub source: String,
    pub photo_key: Option<String>,
    pub ai_confidence: Option<f64>,
    pub note: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    /// Letzte Korrektur (ab `m20261016_000007_reading_revisions`)
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

/// Revision einer Ablesung (ab `m20261016_000007_reading_revisions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    /// Ablesung; nach dem Löschen nicht mehr vorhanden
    pub reading_id: Uuid,
    pub meter_id: Uuid,
    pub register_id: Uuid,
    /// `update` oder `delete`
    pub action: String,
    pub changed_by: Uuid,
    pub changed_at: DateTime<Utc>,
    pub old_read_at: DateTime<Utc>,
    pub old_value_milli: i64,
    pub old_note: Option<String>,
    pub new_read_at: Option<DateTime<Utc>>,
    pub new_value_milli: Option<i64>,
    pub new_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    pub created_by: Uuid,
    pub photo_key: String,
    pub model: Option<String>,
    pub duration_ms: Option<i64>,
    /// Vorschlag als JSON wie gespeichert
    pub suggestion: Option<String>,
    pub matched_meter_id: Option<Uuid>,
    pub status: String,
    pub confirmed_reading_id: Option<Uuid>,
    pub corrected: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub taken_at: Option<DateTime<Utc>>,
    /// Ab `m20261016_000005`; fehlt in älteren Archiven
    #[serde(default)]
    pub analysis_started_at: Option<DateTime<Utc>>,
}

/// Einladung (ab `m20261016_000006_members`); nur im Installationsarchiv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvitationRecord {
    pub id: Uuid,
    pub household_id: Uuid,
    pub role: String,
    /// SHA-256 des Tokens, hexadezimal
    pub token_hash: String,
    pub email: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub accepted_by: Option<Uuid>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Benachrichtigungseinstellungen eines Nutzers (ab `m20261016_000009_jobs`);
/// nur im Installationsarchiv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSettingRecord {
    pub user_id: Uuid,
    pub email: Option<String>,
    pub reminder: bool,
    pub alarm: bool,
    pub updated_at: DateTime<Utc>,
}

/// Alle Datensätze eines Archivs. Sitzungen und temporäre Dateien sind nicht
/// enthalten.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupData {
    #[serde(default)]
    pub users: Vec<UserRecord>,
    #[serde(default)]
    pub households: Vec<HouseholdRecord>,
    #[serde(default)]
    pub memberships: Vec<MembershipRecord>,
    #[serde(default)]
    pub meters: Vec<MeterRecord>,
    #[serde(default)]
    pub devices: Vec<DeviceRecord>,
    #[serde(default)]
    pub registers: Vec<RegisterRecord>,
    #[serde(default)]
    pub readings: Vec<ReadingRecord>,
    #[serde(default)]
    pub drafts: Vec<DraftRecord>,
    #[serde(default)]
    pub invitations: Vec<InvitationRecord>,
    #[serde(default)]
    pub revisions: Vec<RevisionRecord>,
    #[serde(default)]
    pub notification_settings: Vec<NotificationSettingRecord>,
}

impl BackupData {
    /// Objektschlüssel aller vorhandenen Fotos, sortiert: Belegfotos und
    /// Fotos offener Entwürfe. Abgeschlossene Entwürfe verweisen auf ein
    /// planmäßig gelöschtes Foto (die Ablesung hat eine Kopie).
    #[must_use]
    pub fn blob_keys(&self) -> Vec<String> {
        let keys: BTreeSet<String> = self
            .readings
            .iter()
            .filter_map(|r| r.photo_key.clone())
            .chain(
                self.drafts
                    .iter()
                    .filter(|d| {
                        d.status
                            .parse::<DraftStatus>()
                            .is_ok_and(DraftStatus::is_open)
                    })
                    .map(|d| d.photo_key.clone()),
            )
            .collect();
        keys.into_iter().collect()
    }

    /// Nur die Daten eines Haushalts; Nutzerkonten, Einladungen und
    /// Benachrichtigungseinstellungen entfallen.
    #[must_use]
    pub fn household(self, id: Uuid) -> Self {
        let meters: HashSet<Uuid> = self
            .meters
            .iter()
            .filter(|m| m.household_id == id)
            .map(|m| m.id)
            .collect();
        let registers: HashSet<Uuid> = self
            .registers
            .iter()
            .filter(|r| meters.contains(&r.meter_id))
            .map(|r| r.id)
            .collect();
        Self {
            users: Vec::new(),
            households: self.households.into_iter().filter(|h| h.id == id).collect(),
            memberships: self
                .memberships
                .into_iter()
                .filter(|m| m.household_id == id)
                .collect(),
            meters: self
                .meters
                .into_iter()
                .filter(|m| m.household_id == id)
                .collect(),
            devices: self
                .devices
                .into_iter()
                .filter(|d| meters.contains(&d.meter_id))
                .collect(),
            registers: self
                .registers
                .into_iter()
                .filter(|r| meters.contains(&r.meter_id))
                .collect(),
            readings: self
                .readings
                .into_iter()
                .filter(|r| registers.contains(&r.register_id))
                .collect(),
            drafts: self
                .drafts
                .into_iter()
                .filter(|d| d.household_id == id)
                .collect(),
            invitations: Vec::new(),
            revisions: self
                .revisions
                .into_iter()
                .filter(|r| r.household_id == id)
                .collect(),
            notification_settings: Vec::new(),
        }
    }

    /// Anzahl der Datensätze je Art.
    #[must_use]
    pub fn counts(&self) -> Counts {
        Counts {
            users: self.users.len(),
            households: self.households.len(),
            meters: self.meters.len(),
            readings: self.readings.len(),
            drafts: self.drafts.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub users: usize,
    pub households: usize,
    pub meters: usize,
    pub readings: usize,
    pub drafts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobEntry {
    pub key: String,
    pub size: u64,
    pub sha256: String,
}

/// Beschreibung des Archivs; wird zuletzt geschrieben und zuerst gelesen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub scope: Scope,
    /// Programmversion der Quelle
    pub app_version: String,
    /// Angewendete Migrationen der Quelle
    pub schema: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub counts: Counts,
    pub data_sha256: String,
    pub blobs: Vec<BlobEntry>,
    /// Referenzierte Fotos, die in der Quelle fehlten
    #[serde(default)]
    pub missing_blobs: Vec<String>,
}

/// Fehler beim Schreiben oder Lesen eines Archivs; `code` ist ein
/// Übersetzungsschlüssel.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("ungültiges Archiv: {0}")]
    Invalid(String),
    #[error("Archiv einer neueren Programmversion (Format {0})")]
    NewerFormat(u32),
    #[error("Prüfsumme stimmt nicht: {0}")]
    Checksum(String),
    #[error("Ein-/Ausgabefehler: {0}")]
    Io(String),
}

impl ArchiveError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "backup.invalid",
            Self::NewerFormat(_) => "backup.newer_format",
            Self::Checksum(_) => "backup.checksum",
            Self::Io(_) => "backup.io",
        }
    }
}

fn io<E: std::fmt::Display>(e: E) -> ArchiveError {
    ArchiveError::Io(e.to_string())
}

fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Zulässiger Objektschlüssel im Archiv: Foto einer Ablesung oder eines Entwurfs.
fn valid_blob_key(key: &str) -> bool {
    BlobKey::parse(key)
        .is_some_and(|k| matches!(k.kind(), BlobKind::ReadingPhoto | BlobKind::DraftPhoto))
}

/// Schreibt ein Archiv: zuerst `data.json`, dann die Fotos, zuletzt das Manifest.
pub struct ArchiveWriter<W: Write + Seek> {
    zip: ZipWriter<W>,
    blobs: Vec<BlobEntry>,
}

impl<W: Write + Seek> ArchiveWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            zip: ZipWriter::new(out),
            blobs: Vec::new(),
        }
    }

    fn put(
        &mut self,
        name: &str,
        bytes: &[u8],
        method: CompressionMethod,
    ) -> Result<(), ArchiveError> {
        let options = SimpleFileOptions::default()
            .compression_method(method)
            .large_file(bytes.len() as u64 >= u64::from(u32::MAX));
        self.zip.start_file(name, options).map_err(io)?;
        self.zip.write_all(bytes).map_err(io)
    }

    /// Schreibt die Datensätze und liefert ihre Prüfsumme.
    ///
    /// # Errors
    /// Bei Schreibfehlern.
    pub fn data(&mut self, data: &BackupData) -> Result<String, ArchiveError> {
        let json = serde_json::to_vec(data).map_err(io)?;
        self.put(DATA, &json, CompressionMethod::Deflated)?;
        Ok(sha256(&json))
    }

    /// Schreibt ein Foto (JPEG, ohne erneute Kompression).
    ///
    /// # Errors
    /// Bei ungültigem Schlüssel oder Schreibfehlern.
    pub fn blob(&mut self, key: &str, bytes: &[u8]) -> Result<(), ArchiveError> {
        if !valid_blob_key(key) {
            return Err(ArchiveError::Invalid(format!("Objektschlüssel {key}")));
        }
        self.put(
            &format!("{BLOB_PREFIX}{key}"),
            bytes,
            CompressionMethod::Stored,
        )?;
        self.blobs.push(BlobEntry {
            key: key.to_owned(),
            size: bytes.len() as u64,
            sha256: sha256(bytes),
        });
        Ok(())
    }

    /// Schreibt das Manifest; `blobs` wird aus den geschriebenen Fotos gefüllt.
    ///
    /// # Errors
    /// Bei Schreibfehlern.
    pub fn finish(mut self, mut manifest: Manifest) -> Result<W, ArchiveError> {
        manifest.blobs = std::mem::take(&mut self.blobs);
        let json = serde_json::to_vec_pretty(&manifest).map_err(io)?;
        self.put(MANIFEST, &json, CompressionMethod::Deflated)?;
        self.zip.finish().map_err(io)
    }
}

/// Liest ein Archiv; prüft Aufbau und Prüfsummen.
pub struct ArchiveReader<R: Read + Seek> {
    zip: ZipArchive<R>,
    pub manifest: Manifest,
}

impl<R: Read + Seek> ArchiveReader<R> {
    fn read_entry(zip: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>, ArchiveError> {
        let file = zip
            .by_name(name)
            .map_err(|_| ArchiveError::Invalid(format!("{name} fehlt")))?;
        if file.size() > MAX_ENTRY_BYTES {
            return Err(ArchiveError::Invalid(format!("{name} zu groß")));
        }
        let mut out = Vec::new();
        file.take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut out)
            .map_err(|e| ArchiveError::Invalid(format!("{name}: {e}")))?;
        if out.len() as u64 > MAX_ENTRY_BYTES {
            return Err(ArchiveError::Invalid(format!("{name} zu groß")));
        }
        Ok(out)
    }

    /// Öffnet das Archiv und prüft Manifest, Formatversion und Dateinamen.
    ///
    /// # Errors
    /// Bei ungültigem Aufbau oder neuerer Formatversion.
    pub fn open(source: R) -> Result<Self, ArchiveError> {
        let mut zip = ZipArchive::new(source).map_err(|e| ArchiveError::Invalid(e.to_string()))?;
        if zip.len() > MAX_ENTRIES {
            return Err(ArchiveError::Invalid("zu viele Dateien".into()));
        }
        let raw = Self::read_entry(&mut zip, MANIFEST)?;
        let manifest: Manifest = serde_json::from_slice(&raw)
            .map_err(|e| ArchiveError::Invalid(format!("manifest.json: {e}")))?;
        if manifest.format != FORMAT {
            return Err(ArchiveError::Invalid(format!("Format {}", manifest.format)));
        }
        if manifest.format_version > FORMAT_VERSION {
            return Err(ArchiveError::NewerFormat(manifest.format_version));
        }
        let mut expected: HashSet<String> = manifest
            .blobs
            .iter()
            .map(|b| format!("{BLOB_PREFIX}{}", b.key))
            .collect();
        if expected.len() != manifest.blobs.len()
            || manifest.blobs.iter().any(|b| !valid_blob_key(&b.key))
        {
            return Err(ArchiveError::Invalid("Fotoliste".into()));
        }
        expected.insert(MANIFEST.into());
        expected.insert(DATA.into());
        let names: Vec<String> = zip
            .file_names()
            .map(|n| n.map(std::borrow::Cow::into_owned))
            .collect::<Result<_, _>>()
            .map_err(|e| ArchiveError::Invalid(e.to_string()))?;
        let unique: HashSet<&String> = names.iter().collect();
        if unique.len() != names.len()
            || names.len() != expected.len()
            || names.iter().any(|n| !expected.contains(n))
        {
            return Err(ArchiveError::Invalid(
                "Dateien passen nicht zum Manifest".into(),
            ));
        }
        Ok(Self { zip, manifest })
    }

    /// Datensätze; die Prüfsumme muss zum Manifest passen.
    ///
    /// # Errors
    /// Bei abweichender Prüfsumme oder ungültigem JSON.
    pub fn data(&mut self) -> Result<BackupData, ArchiveError> {
        let raw = Self::read_entry(&mut self.zip, DATA)?;
        if sha256(&raw) != self.manifest.data_sha256 {
            return Err(ArchiveError::Checksum(DATA.into()));
        }
        serde_json::from_slice(&raw).map_err(|e| ArchiveError::Invalid(format!("data.json: {e}")))
    }

    /// Foto zum Objektschlüssel; Größe und Prüfsumme müssen zum Manifest passen.
    ///
    /// # Errors
    /// Bei unbekanntem Schlüssel oder abweichender Prüfsumme.
    pub fn blob(&mut self, key: &str) -> Result<Bytes, ArchiveError> {
        let entry = self
            .manifest
            .blobs
            .iter()
            .find(|b| b.key == key)
            .cloned()
            .ok_or_else(|| ArchiveError::Invalid(format!("{key} nicht im Manifest")))?;
        let raw = Self::read_entry(&mut self.zip, &format!("{BLOB_PREFIX}{key}"))?;
        if raw.len() as u64 != entry.size || sha256(&raw) != entry.sha256 {
            return Err(ArchiveError::Checksum(key.into()));
        }
        Ok(Bytes::from(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{HouseholdId, ReadingId};
    use std::io::Cursor;

    fn manifest() -> Manifest {
        Manifest {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION,
            scope: Scope::Installation,
            app_version: "test".into(),
            schema: vec!["m1".into()],
            created_at: Utc::now(),
            counts: Counts::default(),
            data_sha256: String::new(),
            blobs: Vec::new(),
            missing_blobs: Vec::new(),
        }
    }

    fn archive(
        data: &BackupData,
        blobs: &[(&str, &[u8])],
        edit: impl FnOnce(&mut Manifest),
    ) -> Vec<u8> {
        let mut w = ArchiveWriter::new(Cursor::new(Vec::new()));
        let sha = w.data(data).unwrap();
        for (k, b) in blobs {
            w.blob(k, b).unwrap();
        }
        let mut m = manifest();
        m.data_sha256 = sha;
        edit(&mut m);
        w.finish(m).unwrap().into_inner()
    }

    // @test SPEC_BACKUP_FORMAT
    #[test]
    fn archive_roundtrip_checks_manifest_and_checksums() {
        let key = BlobKey::reading_photo(HouseholdId(Uuid::now_v7()), ReadingId(Uuid::now_v7()))
            .to_string();
        let data = BackupData {
            households: vec![HouseholdRecord {
                id: Uuid::now_v7(),
                name: "Haus".into(),
                timezone: "Europe/Berlin".into(),
                created_at: Utc::now(),
            }],
            ..BackupData::default()
        };
        let bytes = archive(&data, &[(&key, b"\xFF\xD8\xFFjpeg")], |_| {});
        let mut r = ArchiveReader::open(Cursor::new(bytes.clone())).unwrap();
        assert_eq!(r.manifest.blobs.len(), 1);
        assert_eq!(r.data().unwrap(), data);
        assert_eq!(&r.blob(&key).unwrap()[..], b"\xFF\xD8\xFFjpeg");

        // neuere Formatversion wird abgelehnt
        let newer = archive(&data, &[], |m| m.format_version = FORMAT_VERSION + 1);
        assert!(matches!(
            ArchiveReader::open(Cursor::new(newer)),
            Err(ArchiveError::NewerFormat(_))
        ));
        // falsche Prüfsumme der Daten
        let bad = archive(&data, &[], |m| m.data_sha256 = "0".repeat(64));
        assert!(matches!(
            ArchiveReader::open(Cursor::new(bad)).unwrap().data(),
            Err(ArchiveError::Checksum(_))
        ));
        // Datei im Archiv, die das Manifest nicht nennt
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let json = serde_json::to_vec(&data).unwrap();
        let mut m = manifest();
        m.data_sha256 = sha256(&json);
        for (name, bytes) in [
            (DATA.to_owned(), json),
            (format!("{BLOB_PREFIX}{key}"), b"x".to_vec()),
            (MANIFEST.to_owned(), serde_json::to_vec(&m).unwrap()),
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        let foreign = zip.finish().unwrap().into_inner();
        assert!(matches!(
            ArchiveReader::open(Cursor::new(foreign)),
            Err(ArchiveError::Invalid(_))
        ));
        // Pfad außerhalb der erlaubten Schlüssel
        let mut w = ArchiveWriter::new(Cursor::new(Vec::new()));
        assert!(w.blob("../../etc/passwd", b"x").is_err());
        assert!(ArchiveReader::open(Cursor::new(b"kein zip".to_vec())).is_err());
    }
}
