//! Logging (strukturiert) und Prometheus-Metriken.

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::config::{LogConfig, LogFormat};

pub fn init_tracing(cfg: &LogConfig) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cfg.filter));
    let registry = tracing_subscriber::registry().with(filter);
    // TODO(REQ_OPS_004): OpenTelemetry-Layer (OTLP) ergänzen, wenn OTEL_EXPORTER_OTLP_ENDPOINT gesetzt ist.
    match cfg.format {
        LogFormat::Json => registry
            .with(fmt::layer().json().flatten_event(true))
            .init(),
        LogFormat::Pretty => registry.with(fmt::layer()).init(),
    }
}

// @impl SPEC_OPS_METRICS
pub fn init_metrics() -> anyhow::Result<PrometheusHandle> {
    Ok(PrometheusBuilder::new().install_recorder()?)
}
