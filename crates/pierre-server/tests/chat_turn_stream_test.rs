// ABOUTME: Drives the real send_message handler and asserts the one stream carries the whole turn
// ABOUTME: Progress, blocks and the terminal done frame ride one body — there is no second rail
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The unified turn stream, over real HTTP.
//!
//! Ported from the AG-UI handler test, which proved the same fact against a
//! parallel `GET /api/agui/runs/{run_id}/stream` subscription the client had
//! to open itself and correlate by a per-turn run id. That rail is gone: the
//! stages the pipeline works through arrive as `progress` frames on the very
//! body the reply arrives on, so the sequence asserted here is what a client
//! actually reads.
//!
//! The turn runs against a deterministic mock provider, so the whole
//! frame sequence — every stage, every block, the terminal envelope — is
//! pinned without touching the network.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use async_trait::async_trait;
use futures_util::stream::{self, StreamExt};
use pierre_core::errors::AppError;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::str;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::timeout;

/// The coach's reply, verbatim. Every assertion about prose compares against
/// this exact string so a surface that quietly rewrote it fails loudly.
const MOCK_REPLY: &str = "Ta charge grimpe depuis trois semaines. On coupe jeudi.";

/// Deterministic provider that returns [`MOCK_REPLY`] on every `complete()`
/// call, so the turn runs end to end without reaching the network.
struct MockLlmProvider {
    model: String,
    calls: Arc<AtomicUsize>,
}

impl MockLlmProvider {
    fn new() -> Self {
        Self {
            model: "mock-model".to_owned(),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn display_name(&self) -> &'static str {
        "Mock LLM (tests)"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ChatResponse {
            content: MOCK_REPLY.to_owned(),
            model: self.model.clone(),
            usage: Some(TokenUsage {
                prompt_tokens: 42,
                completion_tokens: 11,
                total_tokens: 53,
            }),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: MOCK_REPLY.to_owned(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// One decoded SSE frame: its `event:` name and its joined `data:` payload.
#[derive(Debug, Clone)]
struct Frame {
    event: String,
    data: String,
}

impl Frame {
    fn json(&self) -> Value {
        serde_json::from_str(&self.data)
            .unwrap_or_else(|e| panic!("frame {} carried unreadable JSON: {e}", self.event))
    }
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
        .json(&json!({ "title": "turn-stream-test", "model": "mock-model" }))
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

/// Read the whole SSE body, decoding every frame until the stream ends or
/// the deadline elapses.
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

async fn send_turn_streaming(
    client: &Client,
    base_url: &str,
    token: &str,
    conversation_id: &str,
    content: &str,
) -> Vec<Frame> {
    let response = client
        .post(format!(
            "{base_url}/api/chat/conversations/{conversation_id}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "text/event-stream, application/json")
        .json(&json!({ "content": content }))
        .send()
        .await
        .expect("POST send_message");
    assert!(
        response.status().is_success(),
        "streaming turn should open a 200 body, got {}",
        response.status()
    );
    read_frames(response, Duration::from_secs(20)).await
}

/// The stage sequence a turn walks, as `(title, status)` pairs.
fn stage_progress(frames: &[Frame]) -> Vec<(String, String)> {
    frames
        .iter()
        .filter(|f| f.event == "progress")
        .map(Frame::json)
        .filter(|p| p["kind"] == "stage")
        .map(|p| {
            (
                p["title"].as_str().unwrap_or_default().to_owned(),
                p["status"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// A whole turn on one body: the stages it worked through, the blocks it
/// resolved to, and the envelope that ends it — in that order, on one
/// connection, with no run id anywhere.
#[tokio::test(flavor = "multi_thread")]
async fn one_stream_carries_progress_blocks_and_the_finished_turn() {
    let resources = common::create_test_server_resources_with_llm(Arc::new(MockLlmProvider::new()))
        .await
        .expect("server resources");
    let base_url = spawn_chat_server(&resources).await;
    let (_user, token) =
        common::create_test_tenant_with_provider(&resources, "turn-stream@example.com")
            .await
            .expect("create user + token + provider");

    let client = Client::builder().no_gzip().build().expect("client");
    let conversation_id = create_conversation(&client, &base_url, &token).await;
    let frames = send_turn_streaming(
        &client,
        &base_url,
        &token,
        &conversation_id,
        "Tell me about my last run",
    )
    .await;

    // The pipeline's own stage sequence, in order. This is the fact the
    // deleted AG-UI stream existed to deliver.
    assert_eq!(
        stage_progress(&frames),
        vec![
            ("prompt_assembly".to_owned(), "started".to_owned()),
            ("prompt_assembly".to_owned(), "finished".to_owned()),
            ("dispatch".to_owned(), "started".to_owned()),
            ("dispatch".to_owned(), "finished".to_owned()),
        ],
        "stage progress must ride the turn stream, in pipeline order: {frames:?}"
    );

    // Exactly one terminal frame, and it is the last one.
    let terminal = frames.last().expect("stream produced no frames");
    assert_eq!(terminal.event, "done", "frames = {frames:?}");
    assert_eq!(
        frames
            .iter()
            .filter(|f| f.event == "done" || f.event == "failed")
            .count(),
        1,
        "a turn ends exactly once"
    );

    // Every block the server decided arrived as its own frame, ahead of the
    // envelope, and the two agree.
    let block_frames: Vec<Value> = frames
        .iter()
        .filter(|f| f.event == "block")
        .map(Frame::json)
        .collect();
    assert!(
        !block_frames.is_empty(),
        "a reply resolves to at least one block: {frames:?}"
    );
    let envelope = terminal.json();
    let envelope_blocks = envelope["assistant"]["blocks"]
        .as_array()
        .expect("envelope carries an ordered block list");
    assert_eq!(
        &block_frames, envelope_blocks,
        "the block frames and the envelope's list are the same list, in the same order"
    );

    let prose: Vec<&str> = block_frames
        .iter()
        .filter(|b| b["type"] == "prose")
        .filter_map(|b| b["text"].as_str())
        .collect();
    assert_eq!(
        prose,
        vec![MOCK_REPLY],
        "the coach's own words reach the client verbatim, as a prose block"
    );

    assert_eq!(
        envelope["assistant"]["message"]["content"], MOCK_REPLY,
        "the persisted assistant row carries the same reply"
    );
    assert_eq!(envelope["telemetry"]["provider_name"], "mock");
    assert_eq!(envelope["telemetry"]["model"], "mock-model");

    // The deleted rail leaves nothing behind: no correlation id on the
    // envelope, and no frame name from the old vocabulary.
    assert!(
        envelope.get("agui_run_id").is_none(),
        "the turn envelope must not carry a run id: {envelope}"
    );
    assert!(
        frames
            .iter()
            .all(|f| f.event != "agui" && f.event != "tool_call" && f.event != "error"),
        "only the unified frame names may appear: {frames:?}"
    );
}

/// The same endpoint, asked for JSON, answers with one document — the same
/// envelope the `done` frame carries. Two shapes, one egress.
#[tokio::test(flavor = "multi_thread")]
async fn a_caller_that_does_not_ask_for_frames_gets_one_json_document() {
    let resources = common::create_test_server_resources_with_llm(Arc::new(MockLlmProvider::new()))
        .await
        .expect("server resources");
    let base_url = spawn_chat_server(&resources).await;
    let (_user, token) =
        common::create_test_tenant_with_provider(&resources, "turn-json@example.com")
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
    assert!(response.status().is_success(), "{}", response.status());

    let body = response.text().await.expect("body");
    assert!(
        !body.starts_with("event:") && !body.starts_with("data:"),
        "a caller that did not ask for frames must not receive any: {body}"
    );
    let envelope: Value = serde_json::from_str(&body).expect("single JSON document");
    assert_eq!(envelope["assistant"]["message"]["content"], MOCK_REPLY);
    assert!(
        envelope.get("agui_run_id").is_none(),
        "no run id on the JSON shape either: {envelope}"
    );
}

/// An unknown field on the request body is ignored rather than rejected — the
/// deleted `agui_run_id` cannot resurrect as a 400 for a stale client.
#[tokio::test(flavor = "multi_thread")]
async fn a_stale_client_sending_the_deleted_run_id_field_is_not_refused() {
    let resources = common::create_test_server_resources_with_llm(Arc::new(MockLlmProvider::new()))
        .await
        .expect("server resources");
    let base_url = spawn_chat_server(&resources).await;
    let (_user, token) =
        common::create_test_tenant_with_provider(&resources, "turn-stale@example.com")
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
        .json(&json!({ "content": "Hello", "agui_run_id": "not-a-uuid" }))
        .send()
        .await
        .expect("POST send_message");

    assert!(
        response.status().is_success(),
        "the field is simply unknown now, not invalid: {}",
        response.status()
    );
    let envelope: Value = response.json().await.expect("envelope");
    assert!(envelope.get("agui_run_id").is_none());
}
