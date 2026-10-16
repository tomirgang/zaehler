//! `zaehler`: Binary für Webserver und Verwaltungsbefehle.

mod backup;
mod config;
#[cfg(test)]
mod perf_tests;
mod scheduler;
mod telemetry;
mod wiring;

use anyhow::Context;
use axum::{Router, routing::get};
use clap::{Parser, Subcommand};
use tokio::net::TcpListener;

use crate::config::Config;

#[derive(Parser)]
#[command(
    version,
    about = "Zählerstände erfassen, auswerten und prognostizieren"
)]
struct Cli {
    /// Pfad zur Konfigurationsdatei (TOML). Umgebungsvariablen `ZAEHLER_*` überschreiben.
    #[arg(long, short, env = "ZAEHLER_CONFIG")]
    config: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Webserver starten (Standard).
    Serve,
    /// Datenbankmigrationen ausführen und beenden (z.B. als Helm-Hook/Init-Container);
    /// vorher wird ein Archiv nach `backup.dir` geschrieben.
    Migrate,
    /// Wirksame Konfiguration (ohne Geheimnisse) ausgeben.
    Config,
    /// Alle Daten der Installation inklusive Fotos als Archiv exportieren.
    Export {
        /// Zieldatei (`.zip`); eine vorhandene Datei wird nicht überschrieben.
        file: std::path::PathBuf,
    },
    /// Archiv in eine leere Installation importieren; migriert vorher die Datenbank.
    Import {
        /// Archiv aus `zaehler export`.
        file: std::path::PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Optionale `.env` im Arbeitsverzeichnis; bereits gesetzte Variablen haben Vorrang.
    if let Err(e) = dotenvy::dotenv()
        && !e.not_found()
    {
        return Err(e).context(".env nicht lesbar");
    }
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref()).context("Konfiguration ungültig")?;
    let telemetry = telemetry::init_tracing(&config.log, &config.otel)?;
    let result = run(cli.command.unwrap_or(Command::Serve), config, &telemetry).await;
    telemetry.shutdown().await;
    result
}

async fn run(
    command: Command,
    config: Config,
    telemetry: &telemetry::Telemetry,
) -> anyhow::Result<()> {
    match command {
        Command::Serve => serve(config, telemetry.exports_traces()).await,
        Command::Migrate => {
            let a = wiring::adapters(&config, None).await?;
            wiring::prepare_database(&config, &a).await
        }
        Command::Config => {
            println!("{}", config.redacted());
            Ok(())
        }
        Command::Export { file } => {
            let a = wiring::adapters(&config, None).await?;
            let export = zaehler_app::usecase::backup::ExportInstallation::new(a.ports);
            let report = backup::export_to(&export, &file).await?;
            println!(
                "{}: {} Haushalte, {} Messstellen, {} Ablesungen, {} Fotos",
                file.display(),
                report.counts.households,
                report.counts.meters,
                report.counts.readings,
                report.photos
            );
            if !report.missing_photos.is_empty() {
                eprintln!(
                    "Hinweis: {} referenzierte Fotos fehlten im Objektspeicher und sind nicht enthalten",
                    report.missing_photos.len()
                );
            }
            Ok(())
        }
        Command::Import { file } => {
            let a = wiring::adapters(&config, None).await?;
            wiring::prepare_database(&config, &a).await?;
            let import = zaehler_app::usecase::backup::ImportInstallation::new(a.ports);
            let report = import
                .execute(backup::open(&file)?)
                .await
                .map_err(|e| anyhow::anyhow!("Import fehlgeschlagen: {}", describe(&e)))?;
            println!(
                "importiert: {} Nutzer, {} Haushalte, {} Messstellen, {} Ablesungen, {} Fotos",
                report.counts.users,
                report.counts.households,
                report.counts.meters,
                report.counts.readings,
                report.photos
            );
            Ok(())
        }
    }
}

/// Fehler eines Anwendungsfalls für die Konsole; Feldfehler mit ihrem Code.
fn describe(e: &zaehler_app::AppError) -> String {
    match e {
        zaehler_app::AppError::Validation(errors) => errors
            .0
            .iter()
            .map(|f| f.code)
            .collect::<Vec<_>>()
            .join(", "),
        zaehler_app::AppError::Conflict("backup.not_empty") => {
            "die Installation enthält bereits Nutzer oder Haushalte; \
             importiert wird nur in eine leere Installation"
                .into()
        }
        other => other.to_string(),
    }
}

async fn serve(config: Config, traces: bool) -> anyhow::Result<()> {
    let metrics = telemetry::init_metrics()?;
    let wiring::App {
        router: app,
        housekeeping,
        ..
    } = wiring::build(&config, traces).await?;
    let housekeeping_task = scheduler::spawn_housekeeping(
        housekeeping,
        std::time::Duration::from_secs(config.housekeeping.interval_minutes * 60),
    );
    // Separater Listener, damit /metrics nicht über den Ingress erreichbar ist.
    let metrics_app = Router::new().route(
        "/metrics",
        get(move || std::future::ready(metrics.render())),
    );

    let listener = TcpListener::bind(config.server.listen).await?;
    let metrics_listener = TcpListener::bind(config.server.metrics_listen).await?;
    tracing::info!(listen = %config.server.listen, metrics = %config.server.metrics_listen, "zaehler gestartet");

    let metrics_task =
        tokio::spawn(async move { axum::serve(metrics_listener, metrics_app).await });
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    metrics_task.abort();
    housekeeping_task.abort();
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
