// ABOUTME: Pins the OTLP pipeline's endpoint-driven activation contract (inert without env, live with it)
// ABOUTME: Runs only with `--features telemetry` (CI: the dedicated telemetry-feature test step)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! OTLP telemetry pipeline activation-contract test.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "telemetry")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opentelemetry::trace::{Span, Tracer};
use pierre_logging::otel::build_telemetry;
use pierre_logging::LoggingConfig;
use std::env;

/// Every activation path in one test: env mutation is process-global, so
/// sequencing inside a single test avoids cross-test races.
///
/// Without `OTEL_EXPORTER_OTLP_ENDPOINT` the pipeline must stay inert
/// (`Ok(None)`); with it set, the exporters must build, hand out a usable
/// tracer, and shut down cleanly. The endpoint points at a closed local port:
/// exporter construction is offline, and shutdown's flush fails fast with
/// connection-refused instead of hanging on a timeout.
#[test]
fn test_pipeline_inert_without_endpoint_and_builds_with_it() {
    let config = LoggingConfig::default();

    env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    let inert = build_telemetry(&config).expect("no-endpoint path must not error");
    assert!(
        inert.is_none(),
        "pipeline must stay inert without OTEL_EXPORTER_OTLP_ENDPOINT"
    );

    // Empty value counts as unset.
    env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "");
    let empty = build_telemetry(&config).expect("empty-endpoint path must not error");
    assert!(empty.is_none(), "empty endpoint must count as unset");

    env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:1");
    let handle = build_telemetry(&config)
        .expect("exporter build must succeed offline")
        .expect("endpoint set means the pipeline must activate");

    // The tracer must be usable for span emission (the subscriber layer's job).
    let mut span = handle
        .tracer()
        .expect("traces are on when OTEL_TRACES_EXPORTER is unset")
        .start("otel-pipeline-test-probe");
    span.end();

    // Shutdown drains both providers. The flush of the probe span hits the
    // closed port and fails with connection-refused (logged as a warning),
    // which is exactly the degrade-gracefully contract.
    handle.shutdown();

    // OTEL_TRACES_EXPORTER=none keeps metrics but hands out no tracer, so the
    // subscriber layer is never installed and no span is exported.
    env::set_var("OTEL_TRACES_EXPORTER", "none");
    let metrics_only = build_telemetry(&config)
        .expect("metrics-only build must succeed offline")
        .expect("metrics still on means the pipeline must activate");
    assert!(
        metrics_only.tracer().is_none(),
        "OTEL_TRACES_EXPORTER=none must leave no tracer"
    );
    metrics_only.shutdown();

    // Both signals `none`: the endpoint alone must not start anything.
    env::set_var("OTEL_METRICS_EXPORTER", "none");
    let both_off = build_telemetry(&config).expect("both-off path must not error");
    assert!(
        both_off.is_none(),
        "with every signal `none` the pipeline must stay inert"
    );

    // Traces only — the Cloud Trace sidecar setting: a tracer is handed out.
    env::remove_var("OTEL_TRACES_EXPORTER");
    let traces_only = build_telemetry(&config)
        .expect("traces-only build must succeed offline")
        .expect("traces on means the pipeline must activate");
    assert!(
        traces_only.tracer().is_some(),
        "OTEL_METRICS_EXPORTER=none must keep the tracer"
    );
    traces_only.shutdown();

    env::remove_var("OTEL_METRICS_EXPORTER");
    env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
}
