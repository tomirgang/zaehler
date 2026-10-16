//! Logging (strukturiert), Prometheus-Metriken und Trace-Export per OTLP.

use axum::http::HeaderMap;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use opentelemetry::{KeyValue, global, propagation::Extractor, trace::TracerProvider as _};
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{Resource, propagation::TraceContextPropagator, trace::SdkTracerProvider};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::config::{LogConfig, LogFormat, OtelConfig};

/// Hält den Trace-Export bis zum Ende des Prozesses.
pub struct Telemetry {
    provider: Option<SdkTracerProvider>,
}

impl Telemetry {
    /// `true`, wenn Traces exportiert werden.
    pub fn exports_traces(&self) -> bool {
        self.provider.is_some()
    }

    /// Exportiert ausstehende Spans und beendet den Export.
    pub async fn shutdown(self) {
        if let Some(provider) = self.provider {
            // `shutdown` wartet blockierend auf den Export-Thread
            let done = tokio::task::spawn_blocking(move || provider.shutdown()).await;
            if let Ok(Err(e)) = done {
                eprintln!("trace export not flushed: {e}");
            }
        }
    }
}

/// Logging und, bei gesetztem Endpunkt, Trace-Export (SPEC_OPS_OTEL).
// @impl SPEC_OPS_OTEL
pub fn init_tracing(log: &LogConfig, otel: &OtelConfig) -> anyhow::Result<Telemetry> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&log.filter));
    let provider = tracer_provider(otel)?;
    let otel_layer = provider
        .as_ref()
        .map(|p| tracing_opentelemetry::layer().with_tracer(p.tracer("zaehler")));
    if provider.is_some() {
        global::set_text_map_propagator(TraceContextPropagator::new());
    }
    let registry = tracing_subscriber::registry().with(filter).with(otel_layer);
    match log.format {
        LogFormat::Json => registry
            .with(fmt::layer().json().flatten_event(true))
            .init(),
        LogFormat::Pretty => registry.with(fmt::layer()).init(),
    }
    Ok(Telemetry { provider })
}

/// Endpunkt aus `otel.endpoint`, sonst aus den OTel-Umgebungsvariablen.
fn tracer_provider(cfg: &OtelConfig) -> anyhow::Result<Option<SdkTracerProvider>> {
    let env_set = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty());
    let mut exporter = SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary);
    if let Some(endpoint) = cfg.endpoint.as_deref().filter(|e| !e.is_empty()) {
        exporter = exporter.with_endpoint(traces_url(endpoint));
    } else if !env_set("OTEL_EXPORTER_OTLP_ENDPOINT")
        && !env_set("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
    {
        return Ok(None);
    }
    let resource = Resource::builder()
        .with_service_name(cfg.service_name.clone())
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .build();
    Ok(Some(
        SdkTracerProvider::builder()
            .with_batch_exporter(exporter.build()?)
            .with_resource(resource)
            .build(),
    ))
}

/// OTLP/HTTP-Pfad für Traces an die Basis-URL anhängen, wie bei
/// `OTEL_EXPORTER_OTLP_ENDPOINT`.
fn traces_url(base: &str) -> String {
    format!("{}/v1/traces", base.trim_end_matches('/'))
}

struct Headers<'a>(&'a HeaderMap);

impl Extractor for Headers<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(axum::http::HeaderName::as_str).collect()
    }
}

/// Übernimmt den W3C-Trace-Context (`traceparent`) eingehender Requests als
/// Eltern-Span des Request-Spans.
// @impl SPEC_OPS_OTEL
pub fn set_remote_parent(headers: &HeaderMap, span: &tracing::Span) {
    let cx = global::get_text_map_propagator(|p| p.extract(&Headers(headers)));
    // ohne OTel-Layer gibt es keinen Eltern-Span zu setzen
    let _ = span.set_parent(cx);
}

// @impl SPEC_OPS_METRICS
pub fn init_metrics() -> anyhow::Result<PrometheusHandle> {
    Ok(PrometheusBuilder::new().install_recorder()?)
}

#[cfg(test)]
mod tests {
    use opentelemetry::trace::TraceContextExt;
    use opentelemetry_sdk::trace::InMemorySpanExporter;

    use super::*;

    // @test SPEC_OPS_OTEL
    #[test]
    fn endpoint_gets_the_traces_path() {
        assert_eq!(
            traces_url("http://collector:4318/"),
            "http://collector:4318/v1/traces"
        );
        let cfg = OtelConfig {
            endpoint: Some("http://127.0.0.1:4318".into()),
            ..OtelConfig::default()
        };
        let provider = tracer_provider(&cfg).unwrap().expect("Export aktiv");
        provider.shutdown().unwrap();
    }

    /// Route-Template statt ID im Label `path` (SPEC_OPS_METRICS).
    // @test SPEC_OPS_METRICS
    #[tokio::test]
    async fn http_metrics_use_the_route_template() {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;

        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let dir = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::default();
        config.database.url = format!("sqlite://{}?mode=rwc", dir.path().join("m.db").display());
        config.storage = crate::config::StorageConfig::Fs {
            path: dir.path().join("files").to_string_lossy().into_owned(),
        };
        config.auth.dev_login = true;
        config.backup.before_migrate = false;
        let app = crate::wiring::build(&config, false).await.unwrap();
        for uri in ["/healthz", "/meters/01a14414-0026-7579-b067-65ccae93a26c"] {
            app.router
                .clone()
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
        }
        let text = handle.render();
        assert!(
            text.contains(r#"http_requests_total{method="GET",path="/healthz",status="200"} 1"#),
            "{text}"
        );
        assert!(
            text.contains(r#"path="/meters/{id}",status="303""#),
            "{text}"
        );
        assert!(!text.contains("01a14414"), "keine IDs in Labels");
        assert!(text.contains("http_request_duration_seconds"));
    }

    // @test SPEC_OPS_OTEL
    #[test]
    fn incoming_traceparent_becomes_the_parent() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        global::set_text_map_propagator(TraceContextPropagator::new());
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                .parse()
                .unwrap(),
        );
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("http_request");
            set_remote_parent(&headers, &span);
            let trace_id = span.context().span().span_context().trace_id();
            assert_eq!(trace_id.to_string(), "4bf92f3577b34da6a3ce929d0e0e4736");
        });
        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].parent_span_id.to_string(), "00f067aa0ba902b7");
    }
}
