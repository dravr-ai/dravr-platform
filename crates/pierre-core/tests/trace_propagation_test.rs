// ABOUTME: Pins outbound W3C traceparent injection by the shared HTTP clients' telemetry middleware
// ABOUTME: Runs only with `--features telemetry` (CI: the dedicated telemetry-feature test step)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "telemetry")]
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use pierre_core::http_client::api_client;
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing_subscriber::layer::SubscriberExt;

/// Minimal one-shot HTTP server: accepts a single connection, captures the
/// request headers, and answers `200 OK`. Returns the bound address and a
/// receiver that yields the captured header map (lowercased names).
async fn spawn_header_capture_server() -> (SocketAddr, oneshot::Receiver<HashMap<String, String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local test listener");
    let addr = listener.local_addr().expect("local addr");
    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept test connection");
        let mut buf = Vec::new();
        let mut chunk = [0_u8; 1024];
        // Read until the end of the header block; the test request has no body.
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut chunk).await.expect("read request");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let text = String::from_utf8_lossy(&buf);
        let headers: HashMap<String, String> = text
            .split("\r\n")
            .skip(1)
            .take_while(|line| !line.is_empty())
            .filter_map(|line| {
                line.split_once(':')
                    .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_owned()))
            })
            .collect();
        let _ = tx.send(headers);
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await;
    });

    (addr, rx)
}

/// With the OTLP-style pipeline active (tracer provider + otel subscriber
/// layer + W3C propagator), a request through the shared client must carry a
/// `traceparent` header whose trace id matches the exported CLIENT span.
#[tokio::test]
async fn test_traceparent_injected_when_pipeline_active() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    global::set_text_map_propagator(TraceContextPropagator::new());

    let subscriber = tracing_subscriber::registry().with(
        tracing_opentelemetry::layer().with_tracer(provider.tracer("trace-propagation-test")),
    );
    let _guard = tracing::subscriber::set_default(subscriber);

    let (addr, rx) = spawn_header_capture_server().await;
    let response = api_client()
        .get(format!("http://{addr}/probe"))
        .send()
        .await
        .expect("request through shared client");
    assert!(response.status().is_success());

    let headers = rx.await.expect("captured request headers");
    let traceparent = headers
        .get("traceparent")
        .expect("traceparent header must be injected when the pipeline is active");

    // W3C format: 00-<32 hex trace id>-<16 hex parent id>-<2 hex flags>
    let parts: Vec<&str> = traceparent.split('-').collect();
    assert_eq!(parts.len(), 4, "malformed traceparent: {traceparent}");
    assert_eq!(parts[0], "00");
    assert_eq!(parts[1].len(), 32);
    assert_eq!(parts[2].len(), 16);

    provider.force_flush().expect("flush simple exporter");
    let spans = exporter.get_finished_spans().expect("exported spans");
    let client_span = spans
        .iter()
        .find(|s| format!("{:x}", s.span_context.trace_id()) == parts[1])
        .expect("the traceparent trace id must belong to an exported CLIENT span");
    // The otel layer exports under the `otel.name` field ("{method} {host}"),
    // per OTel HTTP client span naming, not the tracing span name.
    assert_eq!(client_span.name, format!("GET {}", addr.ip()));
}

/// Without the OpenTelemetry subscriber layer the middleware is inert: the
/// span context is invalid, so no `traceparent` is written even when a global
/// propagator is installed (the W3C propagator skips invalid contexts).
#[tokio::test]
async fn test_no_traceparent_when_pipeline_inactive() {
    global::set_text_map_propagator(TraceContextPropagator::new());
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry());

    let (addr, rx) = spawn_header_capture_server().await;
    let response = api_client()
        .get(format!("http://{addr}/probe"))
        .send()
        .await
        .expect("request through shared client");
    assert!(response.status().is_success());

    let headers = rx.await.expect("captured request headers");
    assert!(
        !headers.contains_key("traceparent"),
        "no traceparent may be injected without an active pipeline, got: {:?}",
        headers.get("traceparent")
    );
}
