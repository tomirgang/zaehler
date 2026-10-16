//! Archive als Dateien: Export, Import und Sicherung vor Migrationen
//! (SPEC_BACKUP_INSTALLATION, SPEC_OPS_UPGRADE).

use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

use anyhow::Context;
use zaehler_app::usecase::backup::{BackupReport, ExportInstallation};

use crate::config::BackupConfig;

/// Präfix der Archive vor Migrationen; der Zeitstempel im Namen sortiert sie.
const BEFORE_MIGRATION: &str = "zaehler-before-migration-";

/// Schreibt ein Archiv der Installation nach `path`. Eine vorhandene Datei
/// wird nicht überschrieben; bis zum Abschluss heißt die Datei `*.part`.
// @impl SPEC_BACKUP_INSTALLATION
pub async fn export_to(export: &ExportInstallation, path: &Path) -> anyhow::Result<BackupReport> {
    anyhow::ensure!(!path.exists(), "{} existiert bereits", path.display());
    let part = path.with_extension("zip.part");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .with_context(|| format!("{} nicht anlegbar", part.display()))?;
    let result = export.execute(file).await;
    let (file, report) = match result {
        Ok(r) => r,
        Err(e) => {
            fs::remove_file(&part).ok();
            return Err(anyhow::anyhow!("Export fehlgeschlagen: {e}"));
        }
    };
    file.sync_all()?;
    drop(file);
    fs::rename(&part, path)?;
    Ok(report)
}

/// Öffnet ein Archiv zum Lesen.
pub fn open(path: &Path) -> anyhow::Result<File> {
    File::open(path).with_context(|| format!("{} nicht lesbar", path.display()))
}

/// Archiv vor ausstehenden Migrationen; behält die neuesten `keep` Archive.
// @impl SPEC_OPS_UPGRADE
pub async fn before_migration(
    config: &BackupConfig,
    export: &ExportInstallation,
    pending: &[String],
) -> anyhow::Result<PathBuf> {
    let dir = Path::new(&config.dir);
    fs::create_dir_all(dir).with_context(|| format!("{} nicht anlegbar", dir.display()))?;
    let target = pending.last().map_or("unbekannt", String::as_str);
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = dir.join(format!("{BEFORE_MIGRATION}{stamp}-{target}.zip"));
    let report = export_to(export, &path).await?;
    tracing::info!(
        households = report.counts.households,
        readings = report.counts.readings,
        photos = report.photos,
        "archive before migration"
    );
    prune(dir, config.keep)?;
    Ok(path)
}

/// Entfernt ältere Archive vor Migrationen über `keep` hinaus.
fn prune(dir: &Path, keep: usize) -> anyhow::Result<()> {
    let mut archives: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(BEFORE_MIGRATION))
                && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))
        })
        .collect();
    archives.sort();
    let excess = archives.len().saturating_sub(keep);
    for old in archives.into_iter().take(excess) {
        fs::remove_file(&old).with_context(|| format!("{} nicht löschbar", old.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // @test SPEC_OPS_UPGRADE
    #[test]
    fn prune_keeps_the_newest_archives() {
        let dir = std::env::temp_dir().join(format!("zaehler-prune-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&dir).unwrap();
        for stamp in ["20260101T000000Z", "20260201T000000Z", "20260301T000000Z"] {
            fs::write(dir.join(format!("{BEFORE_MIGRATION}{stamp}-m.zip")), b"x").unwrap();
        }
        fs::write(dir.join("eigenes-archiv.zip"), b"x").unwrap();
        prune(&dir, 2).unwrap();
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "eigenes-archiv.zip",
                "zaehler-before-migration-20260201T000000Z-m.zip",
                "zaehler-before-migration-20260301T000000Z-m.zip"
            ]
        );
        fs::remove_dir_all(dir).ok();
    }
}
