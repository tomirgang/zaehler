//! `zaehler`: Binary für Webserver und Verwaltungsbefehle.

mod config;
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
    let wiring::App {
        router: app,
        housekeeping,
    } = wiring::build(&config).await?;
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
