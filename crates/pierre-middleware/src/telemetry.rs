// ABOUTME: HTTP telemetry middleware — inbound W3C trace-context extraction + request metrics
// ABOUTME: Inert unless the OTLP pipeline is active (global meter is no-op, extracted context empty)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! HTTP request telemetry.
//!
//! A single Axum middleware that links each request's tracing span to an
//! upstream `traceparent` (W3C Trace Context) and records request-count and
//! duration metrics against the global meter provider installed by
//! `pierre_logging::otel`. When telemetry is not active the global propagator
//! yields an empty context and the global meter is a no-op, so this adds
//! negligible overhead.

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use http::HeaderName;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::propagation::Extractor;
use opentelemetry::{global, KeyValue};
use std::sync::OnceLock;
use std::time::Instant;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Cached HTTP server instruments, initialized on first request against the
/// global meter provider. Caching avoids rebuilding instruments per request.
struct HttpMetrics {
    requests: Counter<u64>,
    duration: Histogram<f64>,
}

static METRICS: OnceLock<HttpMetrics> = OnceLock::new();

fn metrics() -> &'static HttpMetrics {
    METRICS.get_or_init(|| {
        let meter = global::meter("pierre-middleware");
        HttpMetrics {
            requests: meter
                .u64_counter("http.server.request.count")
                .with_description("Total HTTP server requests, labeled by method/route/status")
                .build(),
            duration: meter
                .f64_histogram("http.server.request.duration")
                .with_description("HTTP server request duration in seconds")
                .build(),
        }
    })
}

/// Adapts an `http::HeaderMap` to the `OpenTelemetry` propagation `Extractor`
/// so the global text-map propagator can read an inbound W3C `traceparent`.
struct HeaderExtractor<'a>(&'a http::HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(HeaderName::as_str).collect()
    }
}

/// Middleware that chains the request span to an upstream `traceparent` and
/// records request-count + duration metrics.
///
/// The route label is the matched template (e.g. `/api/users/{id}`), never the
/// raw path, to keep metric cardinality bounded.
pub async fn telemetry_middleware(req: Request, next: Next) -> Response {
    // Link this request's span to the caller's trace context, if present.
    let parent_cx = global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(req.headers()))
    });
    // Best-effort: `set_parent` returns `Err(SetParentError::LayerNotFound)` when the
    // OTel subscriber layer is absent — the normal state with telemetry off — so the
    // failure is expected and dropped rather than logged on every request.
    let _ = Span::current().set_parent(parent_cx);

    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_owned(), |p| p.as_str().to_owned());
    let method = req.method().as_str().to_owned();

    let start = Instant::now();
    let response = next.run(req).await;
    let elapsed = start.elapsed().as_secs_f64();

    let attributes = [
        KeyValue::new("http.method", method),
        KeyValue::new("http.route", route),
        KeyValue::new("http.status_code", i64::from(response.status().as_u16())),
    ];
    let m = metrics();
    m.requests.add(1, &attributes);
    m.duration.record(elapsed, &attributes);

    response
}
