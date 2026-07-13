// ABOUTME: Single HTTP failure logger — records every 5xx with its originating endpoint
// ABOUTME: and downgrades designed backpressure (a Retry-After 503) from ERROR to WARN
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response failure logging middleware.
//!
//! This is the single place the HTTP stack logs a failed response.
//!
//! The tower-http `TraceLayer` is deliberately configured with a no-op
//! `on_failure`, so a failed response is recorded exactly once — here — carrying
//! the request method and path. `dravr-tronc` forwards `ERROR` events to the
//! `#dev-dravr-errors` Slack channel; before this middleware existed the
//! forwarded event came from tower-http's default `on_failure`, which knows only
//! the latency and status, so the operator alert never named the endpoint that
//! failed.
//!
//! A `503 Service Unavailable` that carries a `Retry-After` header is graceful,
//! client-retryable load-shedding from a backpressure gate (for example the
//! Sciotte Chrome limiter), not a server fault. It logs at `WARN` and so never
//! reaches the ops channel. Every other 5xx logs at `ERROR`, now enriched with
//! the endpoint so the alert is self-explanatory.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use http::{header, StatusCode};
use std::time::Instant;
use tracing::{error, warn};

/// Log any 5xx response with the endpoint that produced it, downgrading a
/// `Retry-After` 503 to `WARN` so intentional backpressure never pages ops.
///
/// Installed once, outside tower-http's `TraceLayer` (whose `on_failure` is a
/// no-op), so it observes the final response for every route. Non-5xx responses
/// pass through untouched — tower-http's `on_response` already logs them at
/// `INFO`.
pub async fn response_failure_log_middleware(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let started = Instant::now();

    let response = next.run(req).await;

    let status = response.status();
    if status.is_server_error() {
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok());

        match retry_after {
            Some(retry_after) if status == StatusCode::SERVICE_UNAVAILABLE => {
                warn!(
                    http_method = %method,
                    http_path = %path,
                    http_status = status.as_u16(),
                    retry_after_secs = retry_after,
                    latency_ms,
                    "request shed by backpressure (retryable, not a server fault)"
                );
            }
            _ => {
                error!(
                    http_method = %method,
                    http_path = %path,
                    http_status = status.as_u16(),
                    latency_ms,
                    "response failed"
                );
            }
        }
    }

    response
}
