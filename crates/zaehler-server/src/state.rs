use std::sync::Arc;

use zaehler_db::Db;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<Config>,
    /// `None`, wenn keine OpenRouter-Konfiguration vorhanden ist.
    #[allow(dead_code)]
    pub ai: Option<zaehler_ai::OpenRouter>,
}
