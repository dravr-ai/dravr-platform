// ABOUTME: End-to-end HTTP test of a turn that fails — the stream still reports every stage it reached
// ABOUTME: Ported from the AG-UI SSE e2e; the failure now rides the same body as the reply would have
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a client sees when a turn does not finish.
//!
//! The AG-UI e2e this replaces proved a run's event sequence reached an
//! authenticated owner over a second SSE route, and spent most of its
//! assertions on that route's 401/403/404 gates. With the route gone there is
//! no `run_id` to guess and no cross-user read to defend against: a turn's
//! events are readable only on the response body of the request that started
//! it, by the caller `AuthenticatedUser` already resolved.
//!
//! What survives is the part that was about the turn rather than the
//! transport — a real failing run emits a real sequence — so that is what is
//! asserted here, against the pipeline's genuine dispatch-failure path
//! (an LLM provider selected with no API key), which needs no live endpoint.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use futures_util::StreamExt;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::env;
use std::net::SocketAddr;
use std::str;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::timeout;

/// RAII guard that overrides a set of environment variables.
///
/// For the duration of a scope, restores their prior values on drop — both
/// re-setting the original value and unsetting the variable if it was
/// previously absent.
///
/// The `#[serial_test::serial]` attribute on each test is a correctness
/// requirement (concurrent tests must not observe the override); this guard is
/// the other half of the contract, so a panicking or early-returning test
/// still leaves the environment in the shape the rest of the suite expects.
struct EnvGuard {
    restore: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn apply(overrides: &[(&'static str, Option<&str>)]) -> Self {
        let mut restore = Vec::with_capacity(overrides.len());
        for (key, new_value) in overrides {
            restore.push((*key, env::var(key).ok()));
            match new_value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
        Self { restore }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prior) in &self.restore {
            match prior {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
    }
}

/// Force the chat pipeline to fail at the dispatch stage.
///
/// Selects the Gemini provider with no API key, so the turn still walks
/// prompt assembly and enters dispatch — emitting the real progress sequence —
/// before the dispatch error propagates. A complete production-flow proof that
/// does not require a live LLM endpoint.
fn force_dispatch_failure_via_env() -> EnvGuard {
    EnvGuard::apply(&[
        ("PIERRE_LLM_PROVIDER", Some("gemini")),
        ("GEMINI_API_KEY", None),
    ])
}

/// One decoded SSE frame.
#[derive(Debug, Clone)]
struct Frame {
    event: String,
    data: String,
}

async fn spawn_chat_server(resources: &Arc<ServerContext>) -> String {
    let app = ChatRoutes::routes(Arc::clone(resources));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

async fn create_conversation(client: &Client, base_url: &str, token: &str) -> String {
    let resp = client
        .post(format!("{base_url}/api/chat/conversations"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({ "title": "turn-stream-e2e", "model": "gemini-1.5-flash" }))
        .send()
        .await
        .expect("create conv");
    assert!(
        resp.status().is_success(),
        "create conversation: {}",
        resp.status()
    );
    let body: Value = resp.json().await.expect("conv json");
    body["id"].as_str().expect("conv id").to_owned()
}

async fn read_frames(response: reqwest::Response, deadline: Duration) -> Vec<Frame> {
    let mut buffer = String::new();
    let mut out: Vec<Frame> = Vec::new();
    let mut stream = response.bytes_stream();

    let _ = timeout(deadline, async {
        while let Some(chunk) = stream.next().await {
            let Ok(bytes) = chunk else { break };
            let Ok(decoded) = str::from_utf8(&bytes) else {
                continue;
            };
            buffer.push_str(decoded);

            while let Some(end) = buffer.find("\n\n") {
                let raw = buffer[..end].to_owned();
                buffer.drain(..=end + 1);

                let mut event = String::new();
                let mut data_lines: Vec<&str> = Vec::new();
                for line in raw.split('\n') {
                    if let Some(rest) = line.strip_prefix("event:") {
                        rest.trim().clone_into(&mut event);
                    } else if let Some(rest) = line.strip_prefix("data:") {
                        data_lines.push(rest.trim());
                    }
                }
                if !event.is_empty() && !data_lines.is_empty() {
                    let terminal = event == "done" || event == "failed";
                    out.push(Frame {
                        event,
                        data: data_lines.join("\n"),
                    });
                    if terminal {
                        return;
                    }
                }
            }
        }
    })
    .await;

    out
}

/// A turn that dies at dispatch still tells the athlete how far it got, and
/// ends with one `failed` frame carrying a sanitized reason.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn a_failing_turn_reports_the_stages_it_reached_then_one_failed_frame() {
    let _env = force_dispatch_failure_via_env();

    let resources = common::create_test_server_resources()
        .await
        .expect("server resources");
    let base_url = spawn_chat_server(&resources).await;
    let (_user, token) =
        common::create_test_tenant_with_provider(&resources, "turn-stream-fail@example.com")
            .await
            .expect("create user + token + provider");

    let client = Client::builder().no_gzip().build().expect("client");
    let conversation_id = create_conversation(&client, &base_url, &token).await;

    let response = client
        .post(format!(
            "{base_url}/api/chat/conversations/{conversation_id}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "text/event-stream, application/json")
        .json(&json!({ "content": "Tell me about my last run" }))
        .send()
        .await
        .expect("POST send_message");

    // The body opens before the pipeline runs, so a turn that fails mid-flight
    // reports the failure in a frame rather than in the status line.
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the stream opens on the transport's capability, not on the outcome"
    );

    let frames = read_frames(response, Duration::from_secs(30)).await;
    assert!(!frames.is_empty(), "a failing turn is not a silent one");

    let progress: Vec<(String, String)> = frames
        .iter()
        .filter(|f| f.event == "progress")
        .filter_map(|f| serde_json::from_str::<Value>(&f.data).ok())
        .map(|p| {
            (
                p["title"].as_str().unwrap_or_default().to_owned(),
                p["status"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert!(
        progress.contains(&("prompt_assembly".to_owned(), "started".to_owned())),
        "the turn reached prompt assembly: {progress:?}"
    );
    assert!(
        progress.contains(&("dispatch".to_owned(), "started".to_owned())),
        "the turn reached dispatch, which is where it died: {progress:?}"
    );

    let terminal = frames.last().expect("frames is non-empty");
    assert_eq!(
        terminal.event, "failed",
        "the last frame names the outcome: {frames:?}"
    );
    assert!(
        frames.iter().all(|f| f.event != "done"),
        "a failed turn never also reports done: {frames:?}"
    );

    let reason: Value = serde_json::from_str(&terminal.data).expect("failed frame is JSON");
    let message = reason["error"]
        .as_str()
        .expect("failed frame names a reason");
    assert!(
        !message.trim().is_empty(),
        "the reason must be renderable, got {message:?}"
    );
    // The sanitized message is the client-safe one; raw provider internals
    // (API keys, endpoint URLs, stack detail) must never cross the wire.
    assert!(
        !message.contains("GEMINI_API_KEY") && !message.contains("generativelanguage"),
        "the failure reason leaked provider internals: {message}"
    );
}

/// The same failure, asked for as JSON, is still an HTTP error status — the
/// blocking shape did not change when the streaming shape gained a frame.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn the_blocking_shape_still_reports_a_failure_as_a_status() {
    let _env = force_dispatch_failure_via_env();

    let resources = common::create_test_server_resources()
        .await
        .expect("server resources");
    let base_url = spawn_chat_server(&resources).await;
    let (_user, token) =
        common::create_test_tenant_with_provider(&resources, "turn-json-fail@example.com")
            .await
            .expect("create user + token + provider");

    let client = Client::builder().no_gzip().build().expect("client");
    let conversation_id = create_conversation(&client, &base_url, &token).await;

    let response = client
        .post(format!(
            "{base_url}/api/chat/conversations/{conversation_id}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({ "content": "Tell me about my last run" }))
        .send()
        .await
        .expect("POST send_message");

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    assert!(
        status.is_client_error() || status.is_server_error(),
        "expected a failure status on the blocking shape, got {status} body={body}"
    );
}

/// An unauthenticated caller cannot open a turn stream at all.
///
/// The AG-UI route's 401/403/404 gates guarded a *separate* subscription
/// surface. There is only one surface now, and it is the chat endpoint's own
/// `AuthenticatedUser` extractor — so this is the whole of what those gates
/// became.
#[tokio::test(flavor = "multi_thread")]
async fn an_unauthenticated_caller_gets_no_stream() {
    let resources = common::create_test_server_resources()
        .await
        .expect("server resources");
    let base_url = spawn_chat_server(&resources).await;
    let (_user, token) =
        common::create_test_tenant_with_provider(&resources, "turn-stream-auth@example.com")
            .await
            .expect("create user + token + provider");

    let client = Client::builder().no_gzip().build().expect("client");
    let conversation_id = create_conversation(&client, &base_url, &token).await;
    let url = format!("{base_url}/api/chat/conversations/{conversation_id}/messages");

    let anonymous = client
        .post(&url)
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "text/event-stream")
        .json(&json!({ "content": "Hello" }))
        .send()
        .await
        .expect("POST without auth");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let bad_token = client
        .post(&url)
        .header(AUTHORIZATION, "Bearer not-a-real-jwt")
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "text/event-stream")
        .json(&json!({ "content": "Hello" }))
        .send()
        .await
        .expect("POST with a malformed token");
    assert_eq!(bad_token.status(), StatusCode::UNAUTHORIZED);
}
