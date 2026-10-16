//! Composition Root: Adapter erzeugen und mit den Anwendungsfällen
//! verdrahten (ADR_022).

use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use zaehler_ai::OpenRouter;
use zaehler_app::{
    port::{BlobStore, MeterRecognizer, SystemClock, UuidV7},
    usecase::{Ports, UseCases, backup::ExportInstallation, housekeeping::Housekeeping},
};
use zaehler_db::{Db, DbSessionStore, SeaOrmStore};
use zaehler_media::{ImageCrateNormalizer, ImageOptions};
use zaehler_storage::{ObjectStoreBlobStore, S3Options};
use zaehler_web::{OidcClient, WebConfig, WebState};

use crate::{
    backup,
    config::{Config, StorageConfig},
};

/// Router und Housekeeping der Instanz.
pub struct App {
    pub router: Router,
    pub housekeeping: Housekeeping,
}

/// Datenbank, Speicher und Ports; unabhängig vom Schemastand erzeugbar.
pub struct Adapters {
    pub db: Db,
    pub store: Arc<SeaOrmStore>,
    pub blobs: Arc<dyn BlobStore>,
    pub ports: Ports,
}

/// Objektspeicher gemäß `storage.kind` (SPEC_STORE_BLOBS).
// @impl SPEC_STORE_BLOBS
fn blob_store(config: &StorageConfig) -> anyhow::Result<ObjectStoreBlobStore> {
    let store = match config {
        StorageConfig::Fs { path } => ObjectStoreBlobStore::local(std::path::Path::new(path)),
        StorageConfig::S3 {
            bucket,
            endpoint,
            region,
            access_key_id,
            secret_access_key,
            allow_http,
        } => ObjectStoreBlobStore::s3(&S3Options {
            bucket: bucket.clone(),
            endpoint: endpoint.clone(),
            region: region.clone(),
            access_key_id: access_key_id.clone(),
            secret_access_key: secret_access_key.clone(),
            allow_http: *allow_http,
        }),
    };
    store.map_err(|e| anyhow::anyhow!("Objektspeicher nicht verfügbar: {e}"))
}

/// Erzeugt die Adapter; `recognizer` nur für den Webserver.
pub async fn adapters(
    config: &Config,
    recognizer: Option<Arc<dyn MeterRecognizer>>,
) -> anyhow::Result<Adapters> {
    config.validate().map_err(anyhow::Error::msg)?;
    let db = zaehler_db::connect(&config.database.url, config.database.max_connections)
        .await
        .context("Datenbankverbindung fehlgeschlagen")?;
    let store = Arc::new(SeaOrmStore::new(db.clone()));
    let blobs: Arc<dyn BlobStore> = Arc::new(blob_store(&config.storage)?);
    let ports = Ports {
        uow: store.clone(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(UuidV7),
        blobs: blobs.clone(),
        images: Arc::new(ImageCrateNormalizer::new(ImageOptions {
            max_pixels: config.image.max_pixels,
            jpeg_quality: config.image.jpeg_quality,
        })),
        recognizer,
        backup: store.clone(),
    };
    Ok(Adapters {
        db,
        store,
        blobs,
        ports,
    })
}

/// Bringt die Datenbank auf den Stand dieser Version: verweigert eine
/// Datenbank einer neueren Version, schreibt vor ausstehenden Migrationen
/// ein vollständiges Archiv und migriert danach (SPEC_OPS_UPGRADE).
// @impl SPEC_OPS_UPGRADE
pub async fn prepare_database(config: &Config, a: &Adapters) -> anyhow::Result<()> {
    let status = zaehler_db::schema_status(&a.db).await?;
    if status.pending.is_empty() {
        return Ok(());
    }
    if !status.applied.is_empty() && config.backup.before_migrate {
        let export = ExportInstallation::new(a.ports.clone());
        let path = backup::before_migration(&config.backup, &export, &status.pending)
            .await
            .context("Sicherung vor der Migration fehlgeschlagen; es wurde nichts migriert")?;
        tracing::info!(path = %path.display(), "backup before migration written");
    }
    zaehler_db::migrate(&a.db).await?;
    Ok(())
}

/// Baut den HTTP-Router und das Housekeeping mit allen Adaptern.
pub async fn build(config: &Config) -> anyhow::Result<App> {
    let recognizer: Option<Arc<dyn MeterRecognizer>> = if let Some(ai) = &config.ai {
        tracing::info!(model = %ai.model, "AI-Erkennung aktiv");
        Some(Arc::new(
            OpenRouter::new(ai.clone()).context("AI-Client nicht erzeugbar")?,
        ))
    } else {
        tracing::info!("AI-Erkennung nicht konfiguriert; Foto-Erfassung mit manueller Eingabe");
        None
    };
    let a = adapters(config, recognizer).await?;
    if config.database.auto_migrate {
        prepare_database(config, &a).await?;
    } else {
        let status = zaehler_db::schema_status(&a.db).await?;
        anyhow::ensure!(
            status.pending.is_empty(),
            "ausstehende Migrationen {:?}; zuerst `zaehler migrate` ausführen",
            status.pending
        );
    }

    let oidc = match &config.oidc {
        Some(o) => Some(Arc::new(
            OidcClient::discover(
                &o.issuer_url,
                &o.client_id,
                &o.client_secret,
                &config.server.public_url,
            )
            .await
            .map_err(anyhow::Error::msg)?,
        )),
        None => None,
    };
    if config.auth.dev_login {
        tracing::warn!("Entwicklungs-Login aktiv (auth.dev_login); nicht für den Produktivbetrieb");
    }

    let state = WebState {
        uc: Arc::new(UseCases::new(&a.ports)),
        readiness: a.store.clone(),
        cfg: Arc::new(
            WebConfig::new(
                &config.server.public_url,
                config.auth.dev_login,
                config.cookie_secure(),
                config.session.ttl_hours,
                env!("CARGO_PKG_VERSION"),
            )
            .with_max_photo_bytes(config.upload.max_photo_bytes)
            .with_max_csv_bytes(config.upload.max_csv_bytes)
            .with_max_backup_bytes(config.upload.max_backup_bytes),
        ),
        oidc,
        analysis: Arc::default(),
    };
    Ok(App {
        router: zaehler_web::router(state, DbSessionStore::new(a.db.clone())),
        housekeeping: Housekeeping::new(
            a.store,
            a.blobs,
            a.ports.clock.clone(),
            chrono::Duration::hours(config.housekeeping.draft_ttl_hours),
        ),
    })
}
