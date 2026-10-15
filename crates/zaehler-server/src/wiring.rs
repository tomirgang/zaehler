//! Composition Root: Adapter erzeugen und mit den Anwendungsfällen
//! verdrahten (ADR_022).

use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use zaehler_app::{
    port::{SystemClock, UuidV7},
    usecase::{Ports, UseCases},
};
use zaehler_db::{DbSessionStore, SeaOrmStore};
use zaehler_web::{OidcClient, WebConfig, WebState};

use crate::config::Config;

/// Baut den HTTP-Router mit allen Adaptern.
pub async fn build(config: &Config) -> anyhow::Result<Router> {
    config.validate().map_err(anyhow::Error::msg)?;

    let db = zaehler_db::connect(&config.database.url, config.database.max_connections)
        .await
        .context("Datenbankverbindung fehlgeschlagen")?;
    if config.database.auto_migrate {
        zaehler_db::migrate(&db).await?;
    }

    let store = Arc::new(SeaOrmStore::new(db.clone()));
    let ports = Ports {
        uow: store.clone(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(UuidV7),
    };

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
        uc: Arc::new(UseCases::new(&ports)),
        readiness: store,
        cfg: Arc::new(WebConfig::new(
            &config.server.public_url,
            config.auth.dev_login,
            config.cookie_secure(),
            config.session.ttl_hours,
            env!("CARGO_PKG_VERSION"),
        )),
        oidc,
    };
    Ok(zaehler_web::router(state, DbSessionStore::new(db)))
}
