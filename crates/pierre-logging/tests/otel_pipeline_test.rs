// ABOUTME: Pins the OTLP pipeline's endpoint-driven activation contract (inert without env, live with it)
// ABOUTME: Runs only with `--features telemetry` (CI: the dedicated telemetry-feature test step)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "telemetry")]
#![allow(missing_docs)]

use opentelemetry::trace::{Span, Tracer};
use pierre_logging::otel::build_telemetry;
use pierre_logging::LoggingConfig;
use std::env;

/// Both activation paths in one test: env mutation is process-global, so
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
    let mut span = handle.tracer().start("otel-pipeline-test-probe");
    span.end();

    // Shutdown drains both providers. The flush of the probe span hits the
    // closed port and fails with connection-refused (logged as a warning),
    // which is exactly the degrade-gracefully contract.
    handle.shutdown();

    env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
}
