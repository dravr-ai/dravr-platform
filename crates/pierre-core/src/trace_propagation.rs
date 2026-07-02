// ABOUTME: Outbound W3C trace-context propagation middleware for the shared HTTP clients
// ABOUTME: Emits a CLIENT span per request and injects `traceparent` when the OTLP pipeline is active
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Outbound trace propagation.
//!
//! [`TracePropagationMiddleware`] links every request sent through the shared
//! HTTP clients to the caller's active trace: it opens a CLIENT span, injects
//! the span's context as W3C `traceparent`/`tracestate` headers via the global
//! text-map propagator, and records the response status on completion.
//!
//! When the OTLP pipeline is not active the middleware is inert: the span's
//! OpenTelemetry context is invalid, so the propagator writes no headers, and
//! the span is just an ordinary tracing span. This replaces reqwest-tracing's
//! `TracingMiddleware`, whose OpenTelemetry 0.32 support requires reqwest
//! 0.13 — a major the workspace has not adopted.

use async_trait::async_trait;
use http::Extensions;
use opentelemetry::global;
use opentelemetry::propagation::Injector;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Request, Response};
use reqwest_middleware::{Middleware, Next, Result};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Adapts reqwest's `HeaderMap` to the OpenTelemetry propagation [`Injector`]
/// so the global text-map propagator can write `traceparent`/`tracestate`.
struct HeaderInjector<'a>(&'a mut HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        // Propagator-produced keys/values are spec-shaped ASCII; a malformed
        // pair is dropped rather than aborting the request.
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, val);
        }
    }
}

/// Middleware that emits a CLIENT span per outbound request and injects the
/// active trace context into the request headers.
///
/// Span fields follow OpenTelemetry HTTP client semantic conventions. Only the
/// method and host are recorded — never the full URL, which can carry signed
/// query tokens or credentials that must not reach logs or trace exports.
#[derive(Clone, Copy, Debug, Default)]
pub struct TracePropagationMiddleware;

#[async_trait]
impl Middleware for TracePropagationMiddleware {
    async fn handle(
        &self,
        mut req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> Result<Response> {
        let method = req.method().as_str().to_owned();
        let host = req
            .url()
            .host_str()
            .map_or_else(|| "unknown".to_owned(), ToOwned::to_owned);

        let span = tracing::info_span!(
            "http.client.request",
            otel.kind = "client",
            otel.name = format!("{method} {host}"),
            otel.status_code = tracing::field::Empty,
            http.request.method = method,
            server.address = host,
            http.response.status_code = tracing::field::Empty,
        );

        global::get_text_map_propagator(|propagator| {
            propagator.inject_context(&span.context(), &mut HeaderInjector(req.headers_mut()));
        });

        let response = next.run(req, extensions).instrument(span.clone()).await;
        match &response {
            Ok(resp) => {
                span.record("http.response.status_code", resp.status().as_u16());
            }
            Err(_) => {
                span.record("otel.status_code", "ERROR");
            }
        }
        response
    }
}
