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
//!
//! Each signal can be switched off on its own with the standard
//! `OTEL_TRACES_EXPORTER=none` / `OTEL_METRICS_EXPORTER=none`, so traces can
//! ship without paying for metrics ingestion (or the reverse). Unset, or
//! `otlp`, keeps the signal on. Sampling is read by the SDK itself:
//! `SdkTracerProvider::builder()` starts from `opentelemetry_sdk::trace::Config::default()`,
//! which honours `OTEL_TRACES_SAMPLER` / `OTEL_TRACES_SAMPLER_ARG`
//! (`parentbased_traceidratio` with a ratio is the production setting). An
//! `_ARG` that does not parse as a float makes the SDK fall back to ratio 1.0,
//! i.e. every trace is kept; the startup line below echoes both values so
//! that fallback is visible.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{MetricExporter, Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use pierre_core::errors::{AppError, AppResult};
use rustls::crypto::ring;
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
/// Per-signal exporter selection; `none` switches that signal off.
const OTEL_TRACES_EXPORTER_ENV: &str = "OTEL_TRACES_EXPORTER";
const OTEL_METRICS_EXPORTER_ENV: &str = "OTEL_METRICS_EXPORTER";
/// Sampler selection, read by the SDK; echoed at startup only.
const OTEL_TRACES_SAMPLER_ENV: &str = "OTEL_TRACES_SAMPLER";
const OTEL_TRACES_SAMPLER_ARG_ENV: &str = "OTEL_TRACES_SAMPLER_ARG";

/// Instrumentation scope name stamped on every span the subscriber layer
/// emits through [`TelemetryHandle::tracer`].
const INSTRUMENTATION_SCOPE: &str = "pierre-logging";

/// What an `OTEL_<SIGNAL>_EXPORTER` value asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignalExporter {
    /// Unset, empty or `otlp`: export the signal over OTLP.
    Otlp,
    /// `none`: the signal is not exported at all.
    Disabled,
    /// Any other value (`console`, `prometheus`, a typo): this pipeline
    /// speaks OTLP only, so the signal stays on OTLP and startup says so.
    Unsupported,
}

impl SignalExporter {
    /// Whether the signal is exported over OTLP.
    const fn exports(self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

/// Parse an `OTEL_<SIGNAL>_EXPORTER` value. Case and surrounding whitespace
/// are ignored, as the OpenTelemetry environment spec requires.
fn parse_signal_exporter(value: Option<&str>) -> SignalExporter {
    match value.map(str::trim) {
        None | Some("") => SignalExporter::Otlp,
        Some(v) if v.eq_ignore_ascii_case("otlp") => SignalExporter::Otlp,
        Some(v) if v.eq_ignore_ascii_case("none") => SignalExporter::Disabled,
        Some(_) => SignalExporter::Unsupported,
    }
}

/// Read one signal's exporter selection, warning on stderr (the subscriber is
/// not installed yet) when the value is one this pipeline cannot honour.
fn signal_exporter_from_env(key: &str) -> SignalExporter {
    let value = non_empty_env(key);
    let exporter = parse_signal_exporter(value.as_deref());
    if exporter == SignalExporter::Unsupported {
        eprintln!(
            "OpenTelemetry: {key}={} is not supported (only `otlp` or `none`); exporting over OTLP",
            value.unwrap_or_default()
        );
    }
    exporter
}

/// Human-readable sampler description for the startup line: the SDK's
/// default when unset, otherwise `sampler` or `sampler:arg`.
fn describe_sampler(sampler: Option<&str>, arg: Option<&str>) -> String {
    match (sampler, arg) {
        (None, _) => "parentbased_always_on (SDK default)".to_owned(),
        (Some(s), None) => s.to_owned(),
        (Some(s), Some(a)) => format!("{s}:{a}"),
    }
}

/// Live OpenTelemetry providers, one per signal that is switched on. Held for
/// the process lifetime so batch exporters keep flushing;
/// [`TelemetryHandle::shutdown`] drains them on exit.
#[derive(Debug)]
pub struct TelemetryHandle {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
}

impl TelemetryHandle {
    /// A tracer from the installed provider, used to build the
    /// `tracing-opentelemetry` subscriber layer. `None` when
    /// `OTEL_TRACES_EXPORTER=none` switched traces off.
    #[must_use]
    pub fn tracer(&self) -> Option<SdkTracer> {
        self.tracer_provider
            .as_ref()
            .map(|provider| provider.tracer(INSTRUMENTATION_SCOPE))
    }

    /// Flush and shut down the live providers. Batch span/metric exporters
    /// buffer in the background, so skipping this on exit drops the final batch.
    pub fn shutdown(&self) {
        if let Some(provider) = &self.tracer_provider {
            if let Err(e) = provider.shutdown() {
                tracing::warn!("OTel tracer provider shutdown error: {e}");
            }
        }
        if let Some(provider) = &self.meter_provider {
            if let Err(e) = provider.shutdown() {
                tracing::warn!("OTel meter provider shutdown error: {e}");
            }
        }
    }
}

/// Build the OTLP traces + metrics pipeline when an endpoint is configured.
///
/// Returns `Ok(None)` when `OTEL_EXPORTER_OTLP_ENDPOINT` is unset (the
/// off-by-default path), or when both `OTEL_TRACES_EXPORTER` and
/// `OTEL_METRICS_EXPORTER` are `none`, so callers treat telemetry as a no-op.
///
/// # Errors
///
/// Returns [`AppError::internal`] when the configured endpoint is set but the
/// OTLP trace or metric exporter fails to build.
pub fn build_telemetry(config: &LoggingConfig) -> AppResult<Option<TelemetryHandle>> {
    let Some(base) = non_empty_env(OTLP_ENDPOINT_ENV) else {
        return Ok(None);
    };

    let traces = signal_exporter_from_env(OTEL_TRACES_EXPORTER_ENV).exports();
    let metrics = signal_exporter_from_env(OTEL_METRICS_EXPORTER_ENV).exports();
    if !traces && !metrics {
        eprintln!(
            "OpenTelemetry OTLP pipeline not started: {OTLP_ENDPOINT_ENV} is set but \
             {OTEL_TRACES_EXPORTER_ENV} and {OTEL_METRICS_EXPORTER_ENV} are both `none`"
        );
        return Ok(None);
    }

    let resource = build_resource(config);

    // W3C Trace Context so spans chain across service boundaries.
    global::set_text_map_propagator(TraceContextPropagator::new());

    // The exporters' reqwest is built with `rustls-no-provider` (Cargo.toml
    // says why), and that build takes its TLS provider from the process
    // default alone — reqwest 0.13 panics with "No provider set" instead of
    // falling back to a compiled-in one. Ring is the provider every other TLS
    // client in the binary links, so it is installed here, before the first
    // exporter builds its client. `Err` means a peer installed the default
    // first (rustls does so from crate features on its first `ClientConfig`),
    // and with ring the only provider compiled in, that default is ring too.
    if ring::default_provider().install_default().is_err() {
        eprintln!("OpenTelemetry OTLP pipeline: rustls CryptoProvider already installed by a peer; keeping it");
    }

    let tracer_provider = if traces {
        let endpoint = non_empty_env(OTLP_TRACES_ENDPOINT_ENV)
            .unwrap_or_else(|| join_path(&base, "v1/traces"));
        let span_exporter = SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(endpoint)
            .build()
            .map_err(|e| AppError::internal(format!("OTLP trace exporter init failed: {e}")))?;
        // `builder()` reads OTEL_TRACES_SAMPLER / OTEL_TRACES_SAMPLER_ARG.
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(span_exporter)
            .with_resource(resource.clone())
            .build();
        global::set_tracer_provider(provider.clone());
        Some(provider)
    } else {
        None
    };

    let meter_provider = if metrics {
        let endpoint = non_empty_env(OTLP_METRICS_ENDPOINT_ENV)
            .unwrap_or_else(|| join_path(&base, "v1/metrics"));
        let metric_exporter = MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(endpoint)
            .build()
            .map_err(|e| AppError::internal(format!("OTLP metric exporter init failed: {e}")))?;
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(metric_exporter)
            .with_resource(resource)
            .build();
        global::set_meter_provider(provider.clone());
        Some(provider)
    } else {
        None
    };

    // The tracing subscriber is not installed yet at this point in bootstrap,
    // so a tracing macro here would be dropped — emit to stderr like the rest
    // of the pre-subscriber startup path in the server binary.
    let sampler = describe_sampler(
        non_empty_env(OTEL_TRACES_SAMPLER_ENV).as_deref(),
        non_empty_env(OTEL_TRACES_SAMPLER_ARG_ENV).as_deref(),
    );
    eprintln!(
        "OpenTelemetry OTLP pipeline enabled: endpoint={base} service={} traces={} sampler={sampler} metrics={}",
        resolve_service_name(config),
        on_off(traces),
        on_off(metrics),
    );

    Ok(Some(TelemetryHandle {
        tracer_provider,
        meter_provider,
    }))
}

/// `on` / `off` for the startup line.
const fn on_off(enabled: bool) -> &'static str {
    if enabled {
        "on"
    } else {
        "off"
    }
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

#[cfg(test)]
mod tests {
    use super::{describe_sampler, parse_signal_exporter, SignalExporter};

    #[test]
    fn unset_or_empty_exporter_keeps_otlp() {
        assert_eq!(
            parse_signal_exporter(None),
            SignalExporter::Otlp,
            "unset must keep today's behaviour (export)"
        );
        assert_eq!(
            parse_signal_exporter(Some("  ")),
            SignalExporter::Otlp,
            "a blank value counts as unset"
        );
    }

    #[test]
    fn otlp_exporter_is_case_and_space_insensitive() {
        for value in ["otlp", "OTLP", " Otlp "] {
            assert_eq!(
                parse_signal_exporter(Some(value)),
                SignalExporter::Otlp,
                "{value:?} must select OTLP"
            );
        }
    }

    #[test]
    fn none_exporter_disables_the_signal() {
        for value in ["none", "NONE", " None\t"] {
            let exporter = parse_signal_exporter(Some(value));
            assert_eq!(exporter, SignalExporter::Disabled, "{value:?} must disable");
            assert!(!exporter.exports(), "a disabled signal must not export");
        }
    }

    #[test]
    fn unsupported_exporter_falls_back_to_otlp() {
        for value in ["console", "prometheus", "nnone", "otlp,console"] {
            let exporter = parse_signal_exporter(Some(value));
            assert_eq!(
                exporter,
                SignalExporter::Unsupported,
                "{value:?} is not an exporter this pipeline has"
            );
            assert!(
                exporter.exports(),
                "an unsupported value must not silently drop the signal"
            );
        }
    }

    #[test]
    fn sampler_description_names_the_sdk_default_and_the_ratio() {
        assert_eq!(
            describe_sampler(None, Some("0.1")),
            "parentbased_always_on (SDK default)",
            "an ARG without a sampler is ignored by the SDK"
        );
        assert_eq!(
            describe_sampler(Some("parentbased_traceidratio"), Some("0.1")),
            "parentbased_traceidratio:0.1",
            "a ratio sampler must be echoed with its ratio"
        );
        assert_eq!(
            describe_sampler(Some("always_off"), None),
            "always_off",
            "a sampler without an ARG is echoed alone"
        );
    }
}
