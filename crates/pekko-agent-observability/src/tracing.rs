use opentelemetry::{trace::TracerProvider as _, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    runtime::Tokio,
    trace::{Config, TracerProvider},
    Resource,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialise the global tracing subscriber.
///
/// JSON structured logs are always emitted.
/// If `OTLP_ENDPOINT` is set (e.g. `http://localhost:4317`), distributed
/// traces are also exported via OpenTelemetry OTLP/gRPC (Jaeger, Grafana
/// Tempo, or any OTLP-compatible backend).
///
/// Call once at process startup. Panics if the subscriber is already set.
pub fn init(service_name: &'static str) -> Option<TracerProvider> {
    let otlp_endpoint = std::env::var("OTLP_ENDPOINT").ok();

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer().json();

    if let Some(endpoint) = otlp_endpoint {
        let resource = Resource::new(vec![KeyValue::new("service.name", service_name)]);

        let exporter = opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint(endpoint);

        let tracer_provider = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(exporter)
            .with_trace_config(Config::default().with_resource(resource))
            .install_batch(Tokio)
            .expect("Failed to install OTel OTLP pipeline");

        let otel_layer = tracing_opentelemetry::layer()
            .with_tracer(tracer_provider.tracer(service_name));

        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .with(otel_layer)
            .init();

        tracing::info!(service = service_name, "Tracing initialised with OTLP export");
        Some(tracer_provider)
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .init();

        tracing::info!(service = service_name, "Tracing initialised (JSON only — set OTLP_ENDPOINT for distributed tracing)");
        None
    }
}

/// Flush pending OTel spans before process exit.
/// Call in the graceful-shutdown handler.
pub fn shutdown(provider: Option<TracerProvider>) {
    if let Some(p) = provider {
        // force_flush returns Vec<Result<(),...>>; log any failures
        for r in p.force_flush() {
            if let Err(e) = r {
                tracing::warn!(error = ?e, "OTel span flush error on shutdown");
            }
        }
        p.shutdown().ok();
    }
}
