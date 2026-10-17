//! Vollständiger Export und Import: ganze Installation (Betrieb, CLI) und
//! einzelner Haushalt (Owner, Oberfläche) (SPEC_BACKUP_INSTALLATION,
//! SPEC_BACKUP_HOUSEHOLD).

use std::{
    collections::HashMap,
    io::{Read, Seek, Write},
};

use uuid::Uuid;

use crate::{
    AppError, PortError, Role, Tenant,
    backup::{
        ArchiveError, ArchiveReader, ArchiveWriter, BackupData, Counts, FORMAT, FORMAT_VERSION,
        Manifest, MembershipRecord, Scope,
    },
    ids::{BlobKey, BlobKind, DocumentId, DraftId, HouseholdId, ReadingId},
    usecase::{Ports, use_case},
};

use_case!(
    /// Archiv der ganzen Installation schreiben.
    ExportInstallation,
    /// Archiv in eine leere Installation einspielen.
    ImportInstallation,
    /// Archiv des aktiven Haushalts schreiben.
    ExportHousehold,
    /// Haushaltsarchiv als neuen Haushalt einspielen.
    ImportHousehold,
);

/// Ergebnis eines Exports oder Imports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupReport {
    pub counts: Counts,
    pub photos: usize,
    /// Referenzierte Fotos, die in der Quelle fehlten
    pub missing_photos: Vec<String>,
}

fn archive_err(e: ArchiveError) -> AppError {
    match e {
        ArchiveError::Io(msg) => AppError::Internal(msg),
        other => {
            tracing::warn!(error = %other, "backup archive rejected");
            AppError::field("file", other.code())
        }
    }
}

/// Schreibt Datensätze und Fotos; fehlende Fotos stehen im Manifest.
// @impl SPEC_BACKUP_FORMAT
async fn write_archive<W: Write + Seek + Send>(
    p: &Ports,
    out: W,
    scope: Scope,
    data: &BackupData,
) -> Result<(W, BackupReport), AppError> {
    let mut writer = ArchiveWriter::new(out);
    let data_sha256 = writer.data(data).map_err(archive_err)?;
    let mut missing = Vec::new();
    let mut photos = 0;
    for key in data.blob_keys() {
        let Some(parsed) = BlobKey::parse(&key) else {
            missing.push(key);
            continue;
        };
        match p.blobs.get(&parsed).await {
            Ok(bytes) => {
                writer.blob(&key, &bytes).map_err(archive_err)?;
                photos += 1;
            }
            Err(PortError::NotFound) => missing.push(key),
            Err(e) => return Err(AppError::from_storage(e)),
        }
    }
    if !missing.is_empty() {
        tracing::warn!(count = missing.len(), "photos missing in object storage");
    }
    let manifest = Manifest {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        scope,
        app_version: env!("CARGO_PKG_VERSION").into(),
        schema: p.backup.schema().await?,
        created_at: p.clock.now(),
        counts: data.counts(),
        data_sha256,
        blobs: Vec::new(),
        missing_blobs: missing.clone(),
    };
    let out = writer.finish(manifest).map_err(archive_err)?;
    Ok((
        out,
        BackupReport {
            counts: data.counts(),
            photos,
            missing_photos: missing,
        },
    ))
}

/// Legt die referenzierten Fotos unter ihrem Zielschlüssel ab; liefert die
/// geschriebenen Schlüssel.
async fn put_photos<R: Read + Seek + Send>(
    p: &Ports,
    reader: &mut ArchiveReader<R>,
    targets: &[(String, BlobKey)],
) -> Result<Vec<BlobKey>, AppError> {
    let mut written = Vec::new();
    for (source, target) in targets {
        let result = match reader.blob(source) {
            Ok(bytes) => p
                .blobs
                .put(target, bytes)
                .await
                .map_err(AppError::from_storage),
            Err(e) => Err(archive_err(e)),
        };
        if let Err(e) = result {
            remove_photos(p, &written).await;
            return Err(e);
        }
        written.push(target.clone());
    }
    Ok(written)
}

async fn remove_photos(p: &Ports, keys: &[BlobKey]) {
    for key in keys {
        if let Err(e) = p.blobs.delete(key).await {
            tracing::warn!(error = %e, %key, "imported photo not removed");
        }
    }
}

/// Fotos des Manifests, auf die Datensätze verweisen.
fn present_photos(reader: &ArchiveReader<impl Read + Seek>, data: &BackupData) -> Vec<String> {
    let keys = data.blob_keys();
    reader
        .manifest
        .blobs
        .iter()
        .map(|b| b.key.clone())
        .filter(|k| keys.binary_search(k).is_ok())
        .collect()
}

impl ExportInstallation {
    /// Schreibt alle Daten der Installation als Archiv (Betrieb, ohne Tenant).
    // @impl SPEC_BACKUP_INSTALLATION
    #[tracing::instrument(name = "ExportInstallation", skip_all)]
    pub async fn execute<W: Write + Seek + Send>(
        &self,
        out: W,
    ) -> Result<(W, BackupReport), AppError> {
        let data = self.p.backup.export().await?;
        write_archive(&self.p, out, Scope::Installation, &data).await
    }
}

impl ImportInstallation {
    /// Spielt ein Installationsarchiv in eine leere Installation ein; IDs und
    /// Objektschlüssel bleiben erhalten. Schlägt der Datenbankteil fehl,
    /// werden die bereits abgelegten Fotos wieder entfernt.
    // @impl SPEC_BACKUP_INSTALLATION
    #[tracing::instrument(name = "ImportInstallation", skip_all)]
    pub async fn execute<R: Read + Seek + Send>(
        &self,
        source: R,
    ) -> Result<BackupReport, AppError> {
        let mut reader = ArchiveReader::open(source).map_err(archive_err)?;
        if reader.manifest.scope != Scope::Installation {
            return Err(AppError::field("file", "backup.scope"));
        }
        let data = reader.data().map_err(archive_err)?;
        if !self.p.backup.is_empty().await? {
            return Err(AppError::Conflict("backup.not_empty"));
        }
        let targets: Vec<(String, BlobKey)> = present_photos(&reader, &data)
            .into_iter()
            .filter_map(|k| BlobKey::parse(&k).map(|b| (k, b)))
            .collect();
        let written = put_photos(&self.p, &mut reader, &targets).await?;
        if let Err(e) = self.p.backup.import(&data).await {
            remove_photos(&self.p, &written).await;
            return Err(e.into());
        }
        Ok(BackupReport {
            counts: data.counts(),
            photos: written.len(),
            missing_photos: reader.manifest.missing_blobs.clone(),
        })
    }
}

impl ExportHousehold {
    /// Archiv des Haushalts des Tenants (nur Owner).
    // @impl SPEC_BACKUP_HOUSEHOLD
    #[tracing::instrument(name = "ExportHousehold", skip_all)]
    pub async fn execute<W: Write + Seek + Send>(
        &self,
        t: &Tenant,
        out: W,
    ) -> Result<(W, BackupReport), AppError> {
        t.require(Role::Owner)?;
        let data = self.p.backup.export().await?.household(t.household_id().0);
        write_archive(&self.p, out, Scope::Household, &data).await
    }
}

/// Neue IDs für alle Datensätze eines Haushalts; der importierende Nutzer
/// wird Owner und Urheber. Liefert die umgeschriebenen Daten und die
/// Zuordnung alter zu neuen Objektschlüsseln.
// @impl SPEC_BACKUP_HOUSEHOLD
#[allow(clippy::too_many_lines)] // eine Umschlüsselung je Tabelle
pub fn remap_household(
    mut data: BackupData,
    owner: Uuid,
    mut new_id: impl FnMut() -> Uuid,
) -> Result<(BackupData, HashMap<String, BlobKey>), AppError> {
    let invalid = || AppError::field("file", "backup.invalid");
    let [household] = data.households.as_mut_slice() else {
        return Err(invalid());
    };
    let new_household = new_id();
    household.id = new_household;
    let hh = HouseholdId(new_household);
    let mut ids: HashMap<Uuid, Uuid> = HashMap::new();
    let mut map =
        |old: Uuid, ids: &mut HashMap<Uuid, Uuid>| *ids.entry(old).or_insert_with(&mut new_id);
    for m in &mut data.meters {
        m.id = map(m.id, &mut ids);
        m.household_id = new_household;
    }
    let known = |old: Uuid, ids: &HashMap<Uuid, Uuid>| ids.get(&old).copied().ok_or_else(invalid);
    for d in &mut data.devices {
        d.meter_id = known(d.meter_id, &ids)?;
        d.parent_meter_id = d.parent_meter_id.map(|p| known(p, &ids)).transpose()?;
        d.id = map(d.id, &mut ids);
    }
    for r in &mut data.registers {
        r.meter_id = known(r.meter_id, &ids)?;
        r.id = map(r.id, &mut ids);
    }
    let mut keys = HashMap::new();
    for r in &mut data.readings {
        r.register_id = known(r.register_id, &ids)?;
        r.device_id = known(r.device_id, &ids)?;
        r.id = map(r.id, &mut ids);
        r.created_by = r.created_by.map(|_| owner);
        if let Some(old) = r.photo_key.take() {
            let key = BlobKey::reading_photo(hh, ReadingId(r.id));
            r.photo_key = Some(key.to_string());
            keys.insert(old, key);
        }
    }
    for d in &mut data.drafts {
        d.id = map(d.id, &mut ids);
        d.household_id = new_household;
        d.created_by = owner;
        d.matched_meter_id = d.matched_meter_id.and_then(|m| ids.get(&m).copied());
        d.confirmed_reading_id = d.confirmed_reading_id.and_then(|r| ids.get(&r).copied());
        let key = BlobKey::draft_photo(hh, DraftId(d.id));
        keys.insert(std::mem::replace(&mut d.photo_key, key.to_string()), key);
    }
    for r in &mut data.revisions {
        r.id = map(r.id, &mut ids);
        r.household_id = new_household;
        r.meter_id = known(r.meter_id, &ids)?;
        r.register_id = known(r.register_id, &ids)?;
        // gelöschte Ablesungen haben keine neue ID; sie erhalten eine eigene
        r.reading_id = map(r.reading_id, &mut ids);
        r.changed_by = owner;
    }
    for d in &mut data.documents {
        d.id = map(d.id, &mut ids);
        d.household_id = new_household;
        d.created_by = owner;
        d.meter_id = d.meter_id.and_then(|m| ids.get(&m).copied());
        let old = BlobKey::parse(&d.blob_key).ok_or_else(invalid)?;
        let key = BlobKey::document(hh, DocumentId(d.id), old.kind() == BlobKind::DocumentPdf);
        keys.insert(std::mem::replace(&mut d.blob_key, key.to_string()), key);
    }
    for c in &mut data.contracts {
        c.meter_id = known(c.meter_id, &ids)?;
        c.id = map(c.id, &mut ids);
    }
    for p in &mut data.cost_positions {
        p.contract_id = known(p.contract_id, &ids)?;
        p.register_id = p.register_id.map(|r| known(r, &ids)).transpose()?;
        p.id = map(p.id, &mut ids);
    }
    for c in &mut data.price_components {
        c.position_id = known(c.position_id, &ids)?;
        c.id = map(c.id, &mut ids);
    }
    for x in &mut data.installments {
        x.contract_id = known(x.contract_id, &ids)?;
        x.id = map(x.id, &mut ids);
    }
    for x in &mut data.adjustments {
        x.contract_id = known(x.contract_id, &ids)?;
        x.id = map(x.id, &mut ids);
    }
    for x in &mut data.settlements {
        x.contract_id = known(x.contract_id, &ids)?;
        x.id = map(x.id, &mut ids);
        x.document_id = x.document_id.and_then(|d| ids.get(&d).copied());
    }
    for v in &mut data.vehicles {
        v.meter_id = known(v.meter_id, &ids)?;
        v.home_charging_meter_id = v
            .home_charging_meter_id
            .map(|m| known(m, &ids))
            .transpose()?;
    }
    for p in &mut data.consumption_profiles {
        p.vehicle_id = known(p.vehicle_id, &ids)?;
        p.id = map(p.id, &mut ids);
    }
    for a in &mut data.acquisitions {
        a.vehicle_id = known(a.vehicle_id, &ids)?;
    }
    for c in &mut data.charging_sessions {
        c.vehicle_id = known(c.vehicle_id, &ids)?;
        c.id = map(c.id, &mut ids);
        c.document_id = c.document_id.and_then(|d| ids.get(&d).copied());
    }
    for s in &mut data.pv_systems {
        s.household_id = new_household;
        s.id = map(s.id, &mut ids);
    }
    for m in &mut data.pv_system_meters {
        m.pv_system_id = known(m.pv_system_id, &ids)?;
        m.meter_id = known(m.meter_id, &ids)?;
    }
    for c in &mut data.asset_costs {
        c.household_id = new_household;
        // Fahrzeug: Messstelle, PV-Anlage: bereits umgeschlüsselt
        c.asset_id = known(c.asset_id, &ids)?;
        c.id = map(c.id, &mut ids);
        c.document_id = c.document_id.and_then(|d| ids.get(&d).copied());
    }
    data.users.clear();
    data.invitations.clear();
    data.api_tokens.clear();
    for r in &mut data.readings {
        r.api_token_id = None;
    }
    data.notification_settings.clear();
    data.memberships = vec![MembershipRecord {
        household_id: new_household,
        user_id: owner,
        role: Role::Owner.as_str().into(),
    }];
    Ok((data, keys))
}

impl ImportHousehold {
    /// Spielt ein Haushaltsarchiv als neuen Haushalt des Nutzers ein; alle
    /// IDs werden neu vergeben, damit derselbe Haushalt mehrfach und in jede
    /// Installation importiert werden kann.
    // @impl SPEC_BACKUP_HOUSEHOLD
    #[tracing::instrument(name = "ImportHousehold", skip_all)]
    pub async fn execute<R: Read + Seek + Send>(
        &self,
        t: &Tenant,
        source: R,
    ) -> Result<(HouseholdId, BackupReport), AppError> {
        let mut reader = ArchiveReader::open(source).map_err(archive_err)?;
        if reader.manifest.scope != Scope::Household {
            return Err(AppError::field("file", "backup.scope"));
        }
        let data = reader.data().map_err(archive_err)?;
        let present = present_photos(&reader, &data);
        let ids = self.p.ids.clone();
        let (data, keys) = remap_household(data, t.user_id().0, || ids.new_uuid())?;
        let targets: Vec<(String, BlobKey)> = present
            .into_iter()
            .filter_map(|k| keys.get(&k).cloned().map(|b| (k, b)))
            .collect();
        let written = put_photos(&self.p, &mut reader, &targets).await?;
        if let Err(e) = self.p.backup.import(&data).await {
            remove_photos(&self.p, &written).await;
            return Err(e.into());
        }
        let id = HouseholdId(data.households[0].id);
        Ok((
            id,
            BackupReport {
                counts: data.counts(),
                photos: written.len(),
                missing_photos: reader.manifest.missing_blobs.clone(),
            },
        ))
    }
}
