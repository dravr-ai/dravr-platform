// ABOUTME: OpenTelemetry OTLP pipeline (traces + metrics) over HTTP/protobuf, gated by the `telemetry` feature
// ABOUTME: Inert until OTEL_EXPORTER_OTLP_ENDPOINT is set; builds the tracer/meter providers and W3C propagator
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! OTLP telemetry pipeline.
//!
//! The pipeline is compiled in only with `--features telemetry` and stays
//! dormant until `OTEL_EXPORTER_OTLP_ENDPOINT` is set, at which point traces
//! and metrics are exported over OTLP HTTP/protobuf (reusing reqwest, so no
//! tonic/gRPC dependency). Spans flow through the `tracing-opentelemetry`
//! layer wired in [`crate::LoggingConfig::init`]; metrics flow through the
//! global meter provider that HTTP middleware records against. Batch span and
//! periodic metric exports run on SDK-owned background threads, so no async
//! runtime handle is required.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{MetricExporter, Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use pierre_core::errors::{AppError, AppResult};
use std::env;

use crate::LoggingConfig;

/// Standard OTLP base-endpoint environment variable. Its presence is what
/// activates the pipeline; absence keeps telemetry inert.
const OTLP_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
/// Per-signal endpoint overrides (take precedence over the base endpoint).
const OTLP_TRACES_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT";
const OTLP_METRICS_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT";
/// Overrides the resource `service.name` independent of the logging config.
const OTEL_SERVICE_NAME_ENV: &str = "OTEL_SERVICE_NAME";

/// Instrumentation scope name stamped on every span the subscriber layer
/// emits through [`TelemetryHandle::tracer`].
const INSTRUMENTATION_SCOPE: &str = "pierre-logging";

/// Live OpenTelemetry providers. Held for the process lifetime so batch
/// exporters keep flushing; [`TelemetryHandle::shutdown`] drains them on exit.
pub struct TelemetryHandle {
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
}

impl TelemetryHandle {
    /// A tracer from the installed provider, used to build the
    /// `tracing-opentelemetry` subscriber layer.
    #[must_use]
    pub fn tracer(&self) -> SdkTracer {
        self.tracer_provider.tracer(INSTRUMENTATION_SCOPE)
    }

    /// Flush and shut down both providers. Batch span/metric exporters buffer
    /// in the background, so skipping this on exit drops the final batch.
    pub fn shutdown(&self) {
        if let Err(e) = self.tracer_provider.shutdown() {
            tracing::warn!("OTel tracer provider shutdown error: {e}");
        }
        if let Err(e) = self.meter_provider.shutdown() {
            tracing::warn!("OTel meter provider shutdown error: {e}");
        }
    }
}

/// Build the OTLP traces + metrics pipeline when an endpoint is configured.
///
/// Returns `Ok(None)` when `OTEL_EXPORTER_OTLP_ENDPOINT` is unset (the
/// off-by-default path), so callers treat telemetry as a no-op.
///
/// # Errors
///
/// Returns [`AppError::internal`] when the configured endpoint is set but the
/// OTLP trace or metric exporter fails to build.
pub fn build_telemetry(config: &LoggingConfig) -> AppResult<Option<TelemetryHandle>> {
    let Some(base) = non_empty_env(OTLP_ENDPOINT_ENV) else {
        return Ok(None);
    };

    let traces_endpoint =
        non_empty_env(OTLP_TRACES_ENDPOINT_ENV).unwrap_or_else(|| join_path(&base, "v1/traces"));
    let metrics_endpoint =
        non_empty_env(OTLP_METRICS_ENDPOINT_ENV).unwrap_or_else(|| join_path(&base, "v1/metrics"));

    let resource = build_resource(config);

    // W3C Trace Context so spans chain across service boundaries.
    global::set_text_map_propagator(TraceContextPropagator::new());

    let span_exporter = SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(traces_endpoint)
        .build()
        .map_err(|e| AppError::internal(format!("OTLP trace exporter init failed: {e}")))?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(resource.clone())
        .build();
    global::set_tracer_provider(tracer_provider.clone());

    let metric_exporter = MetricExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(metrics_endpoint)
        .build()
        .map_err(|e| AppError::internal(format!("OTLP metric exporter init failed: {e}")))?;

    let meter_provider = SdkMeterProvider::builder()
        .with_periodic_exporter(metric_exporter)
        .with_resource(resource)
        .build();
    global::set_meter_provider(meter_provider.clone());

    // The tracing subscriber is not installed yet at this point in bootstrap,
    // so a tracing macro here would be dropped — emit to stderr like the rest
    // of the pre-subscriber startup path in the server binary.
    eprintln!(
        "OpenTelemetry OTLP pipeline enabled (traces + metrics): endpoint={base} service={}",
        resolve_service_name(config)
    );

    Ok(Some(TelemetryHandle {
        tracer_provider,
        meter_provider,
    }))
}

/// Resource attributes that tag every span and metric with service identity.
fn build_resource(config: &LoggingConfig) -> Resource {
    Resource::builder()
        .with_service_name(resolve_service_name(config))
        .with_attributes([
            KeyValue::new("service.version", config.service_version.clone()),
            KeyValue::new("deployment.environment", config.environment.clone()),
        ])
        .build()
}

/// `OTEL_SERVICE_NAME` wins over the logging config's service name when set.
fn resolve_service_name(config: &LoggingConfig) -> String {
    non_empty_env(OTEL_SERVICE_NAME_ENV).unwrap_or_else(|| config.service_name.clone())
}

/// Read an env var, treating empty values as unset.
fn non_empty_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
}

/// Join an OTLP base endpoint with a signal path (`v1/traces`, `v1/metrics`),
/// collapsing the boundary slash so both `http://host:4318` and
/// `http://host:4318/` produce a single-slash URL.
fn join_path(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path)
}
