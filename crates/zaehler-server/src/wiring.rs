//! Composition Root: Adapter erzeugen und mit den Anwendungsfällen
//! verdrahten (ADR_022).

use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use zaehler_ai::OpenRouter;
use zaehler_app::{
    port::{
        BlobStore, DisabledNotifier, DocumentExtractor, LeaderLock, MeterRecognizer, Notifier,
        SingleInstance, SystemClock, UuidV7,
    },
    usecase::{
        Ports, Settings, UseCases,
        backup::ExportInstallation,
        housekeeping::Housekeeping,
        jobs::{DispatchOutbox, SendReminders},
    },
};
use zaehler_db::{Db, DbSessionStore, PgLeaderLock, SeaOrmStore};
use zaehler_media::{ImageCrateNormalizer, ImageOptions};
use zaehler_notify::SmtpNotifier;
use zaehler_storage::{ObjectStoreBlobStore, S3Options};
use zaehler_web::{OidcClient, WebConfig, WebState};

use crate::{
    backup,
    config::{Config, StorageConfig},
    scheduler::{self, Scheduler},
};

/// Router und Scheduler der Instanz.
pub struct App {
    pub router: Router,
    pub scheduler: Scheduler,
    /// Anwendungsfälle des Routers (Seed-Daten im Leistungstest)
    #[cfg_attr(not(test), allow(dead_code))]
    pub uc: Arc<UseCases>,
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

/// E-Mail-Versand gemäß `notify.smtp`; ohne Konfiguration deaktiviert
/// (SPEC_NTF_EMAIL).
// @impl SPEC_NTF_EMAIL
fn notifier(config: &Config) -> anyhow::Result<Arc<dyn Notifier>> {
    let Some(smtp) = &config.notify.smtp else {
        tracing::info!("notify.smtp nicht konfiguriert; keine E-Mail-Benachrichtigungen");
        return Ok(Arc::new(DisabledNotifier));
    };
    tracing::info!(host = %smtp.host, port = smtp.port, "E-Mail-Versand aktiv");
    Ok(Arc::new(
        SmtpNotifier::new(smtp).map_err(anyhow::Error::msg)?,
    ))
}

/// Erzeugt die Adapter; `ai` (Zählerstände, Rechnungen) nur für den
/// Webserver.
pub async fn adapters(config: &Config, ai: Option<Arc<OpenRouter>>) -> anyhow::Result<Adapters> {
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
        recognizer: ai.clone().map(|a| a as Arc<dyn MeterRecognizer>),
        extractor: ai.map(|a| a as Arc<dyn DocumentExtractor>),
        backup: store.clone(),
        jobs: store.clone(),
        notifier: notifier(config)?,
        settings: Settings {
            invitation_ttl: chrono::Duration::days(i64::from(config.invitations.ttl_days)),
            public_url: config.server.public_url.trim_end_matches('/').to_owned(),
            max_document_bytes: config.upload.max_document_bytes,
        },
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

/// Leader-Lock: bei PostgreSQL ein Advisory-Lock auf eigener Verbindung
/// (`database.lock_url`), bei SQLite immer Leader (SPEC_OPS_REPLICAS).
// @impl SPEC_OPS_REPLICAS
async fn leader_lock(config: &Config) -> anyhow::Result<Arc<dyn LeaderLock>> {
    let url = config
        .database
        .lock_url
        .as_deref()
        .unwrap_or(&config.database.url);
    if url.starts_with("postgres") {
        Ok(Arc::new(PgLeaderLock::connect(url).await.context(
            "Verbindung für den Leader-Lock fehlgeschlagen",
        )?))
    } else {
        Ok(Arc::new(SingleInstance))
    }
}

/// Scheduler mit den Jobs der Instanz (SPEC_JOB_SCHEDULER).
async fn scheduler(config: &Config, a: &Adapters) -> anyhow::Result<Scheduler> {
    let housekeeping = Housekeeping::new(
        a.store.clone(),
        a.blobs.clone(),
        a.ports.clock.clone(),
        chrono::Duration::hours(config.housekeeping.draft_ttl_hours),
    );
    let every_tick = chrono::Duration::seconds(i64::try_from(config.scheduler.tick_seconds)?);
    let housekeeping_every =
        chrono::Duration::minutes(i64::try_from(config.housekeeping.interval_minutes)?);
    Ok(Scheduler::new(
        leader_lock(config).await?,
        a.store.clone(),
        a.ports.clock.clone(),
    )
    .with(scheduler::dispatch_outbox(
        DispatchOutbox::new(a.ports.clone()),
        every_tick,
    ))
    .with(scheduler::send_reminders(
        SendReminders::new(a.ports.clone()),
        chrono::Duration::hours(1),
    ))
    .with(scheduler::housekeeping(housekeeping, housekeeping_every)))
}

/// Baut den HTTP-Router und den Scheduler mit allen Adaptern; `traces`
/// übernimmt den Trace-Context eingehender Requests (SPEC_OPS_OTEL).
pub async fn build(config: &Config, traces: bool) -> anyhow::Result<App> {
    let ai = if let Some(ai) = &config.ai {
        let client = OpenRouter::new(ai.clone()).context("AI-Client nicht erzeugbar")?;
        tracing::info!(
            model = %ai.model,
            document_model = %client.document_model_name(),
            "AI-Erkennung aktiv"
        );
        Some(Arc::new(client))
    } else {
        tracing::info!(
            "AI-Erkennung nicht konfiguriert; Foto-Erfassung und Rechnungsimport mit manueller Eingabe"
        );
        None
    };
    let a = adapters(config, ai).await?;
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

    let uc = Arc::new(UseCases::new(&a.ports));
    let state = WebState {
        uc: uc.clone(),
        readiness: a.store.clone(),
        cfg: Arc::new({
            let web = WebConfig::new(
                &config.server.public_url,
                config.auth.dev_login,
                config.cookie_secure(),
                config.session.ttl_hours,
                env!("CARGO_PKG_VERSION"),
            )
            .with_max_photo_bytes(config.upload.max_photo_bytes)
            .with_max_csv_bytes(config.upload.max_csv_bytes)
            .with_max_backup_bytes(config.upload.max_backup_bytes)
            .with_max_document_bytes(config.upload.max_document_bytes);
            if traces {
                web.with_trace_parent(crate::telemetry::set_remote_parent)
            } else {
                web
            }
        }),
        oidc,
        analysis: Arc::default(),
    };
    Ok(App {
        router: zaehler_web::router(state, DbSessionStore::new(a.db.clone())),
        scheduler: scheduler(config, &a).await?,
        uc,
    })
}
