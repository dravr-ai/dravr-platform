// ABOUTME: Pins that the HTTP stack compresses JSON replies and never compresses an SSE stream
// ABOUTME: tools/list ships 528 KB of derived schema; gzip takes it to a fifth, and buffering the chat stream would break it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The compression layer the HTTP server mounts.
//!
//! Worth having because of one reply: `tools/list` carries a derived
//! `outputSchema` for every tool, and JSON Schema is the most repetitive
//! payload this server produces. Uncompressed it is over half a megabyte.
//!
//! Dangerous for exactly one reply: the chat stream. `send_message` answers
//! `Accept: text/event-stream` with tokens as they arrive, and a compressor
//! that buffered them would turn a live stream into one late blob. These
//! tests drive real responses through the same layer the server mounts and
//! assert both halves — the second is the one that would go unnoticed,
//! because a buffered stream still *arrives*.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
    body::{to_bytes, Body},
    http::{
        header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE},
        Request as HttpRequest, StatusCode,
    },
    response::Response,
    routing::get,
    Router,
};
use tower::ServiceExt;
use tower_http::compression::CompressionLayer;

/// Big enough to be worth compressing, and repetitive like a schema is.
fn schema_shaped_json() -> String {
    let one = r#"{"description":"The athlete's heart rate, in beats per minute.","type":["integer","null"],"format":"uint32"},"#;
    format!("{{\"tools\":[{}]}}", one.repeat(400))
}

async fn drive(accept_encoding: &str, content_type: &'static str, body: String) -> Response {
    let app = Router::new()
        .route(
            "/x",
            get(move || {
                let b = body.clone();
                let ct = content_type;
                async move {
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, ct)
                        .body(Body::from(b))
                        .expect("builds")
                }
            }),
        )
        .layer(CompressionLayer::new());

    app.oneshot(
        HttpRequest::builder()
            .uri("/x")
            .header(ACCEPT_ENCODING, accept_encoding)
            .body(Body::empty())
            .expect("builds"),
    )
    .await
    .expect("responds")
}

/// A JSON reply is compressed when the client says it can take it.
#[tokio::test]
async fn a_json_reply_is_compressed_for_a_client_that_accepts_gzip() {
    let raw = schema_shaped_json();
    let raw_len = raw.len();
    let response = drive("gzip", "application/json", raw).await;

    assert_eq!(
        response
            .headers()
            .get(CONTENT_ENCODING)
            .map(|v| v.to_str().unwrap_or("")),
        Some("gzip"),
        "the layer must announce the encoding it applied"
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("reads");
    assert!(
        body.len() < raw_len / 2,
        "schema-shaped JSON should compress hard: {} B from {} B",
        body.len(),
        raw_len
    );
}

/// A client that does not offer an encoding gets the bytes unchanged.
#[tokio::test]
async fn a_client_that_offers_no_encoding_still_gets_plain_json() {
    let raw = schema_shaped_json();
    let raw_len = raw.len();
    let response = drive("identity", "application/json", raw).await;

    assert!(
        response.headers().get(CONTENT_ENCODING).is_none(),
        "nothing was negotiated, so nothing may be encoded"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("reads");
    assert_eq!(body.len(), raw_len, "the body must be untouched");
}

/// The chat stream is never compressed, however the client asks.
///
/// This is the half that would go unnoticed: a compressed SSE response still
/// arrives and still parses. What breaks is *when* — the compressor holds
/// tokens until it has enough to emit, so an athlete watching a reply appear
/// word by word would instead wait and receive it whole. tower-http's
/// `DefaultPredicate` declines `text/event-stream` by name, and this asserts
/// that we are relying on it rather than assuming it.
#[tokio::test]
async fn the_chat_stream_is_never_compressed() {
    let sse = "data: {\"token\":\"you\"}\n\ndata: {\"token\":\" ran\"}\n\n".repeat(200);
    let sse_len = sse.len();
    let response = drive("gzip, br", "text/event-stream", sse).await;

    assert!(
        response.headers().get(CONTENT_ENCODING).is_none(),
        "an SSE stream must reach the athlete token by token, not buffered \
         into a compressor and delivered whole"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("reads");
    assert_eq!(body.len(), sse_len, "and byte for byte");
}
