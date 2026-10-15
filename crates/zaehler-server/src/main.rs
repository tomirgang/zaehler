//! `zaehler`: Binary für Webserver und Verwaltungsbefehle.

mod assets;
mod config;
mod routes;
mod state;
mod telemetry;

use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokio::net::TcpListener;

use crate::{config::Config, state::AppState};

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
    /// Datenbankmigrationen ausführen und beenden (z.B. als Helm-Hook/Init-Container).
    Migrate,
    /// Wirksame Konfiguration (ohne Geheimnisse) ausgeben.
    Config,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref()).context("Konfiguration ungültig")?;
    telemetry::init_tracing(&config.log);

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(config).await,
        Command::Migrate => {
            let db = zaehler_db::connect(&config.database.url, 1).await?;
            zaehler_db::migrate(&db).await?;
            Ok(())
        }
        Command::Config => {
            println!("{}", config.redacted());
            Ok(())
        }
    }
}

async fn serve(config: Config) -> anyhow::Result<()> {
    let metrics = telemetry::init_metrics()?;
    let db = zaehler_db::connect(&config.database.url, config.database.max_connections)
        .await
        .context("Datenbankverbindung fehlgeschlagen")?;
    if config.database.auto_migrate {
        zaehler_db::migrate(&db).await?;
    }

    let ai = config
        .ai
        .clone()
        .map(zaehler_ai::OpenRouter::new)
        .transpose()?;
    if ai.is_none() {
        tracing::warn!("keine AI-Konfiguration, Foto-Erkennung deaktiviert");
    }

    let state = AppState {
        db,
        config: Arc::new(config.clone()),
        ai,
    };

    let app = routes::router(state);
    let metrics_app = routes::metrics_router(metrics);

    let listener = TcpListener::bind(config.server.listen).await?;
    let metrics_listener = TcpListener::bind(config.server.metrics_listen).await?;
    tracing::info!(listen = %config.server.listen, metrics = %config.server.metrics_listen, "zaehler gestartet");

    let metrics_task =
        tokio::spawn(async move { axum::serve(metrics_listener, metrics_app).await });
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    metrics_task.abort();
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
