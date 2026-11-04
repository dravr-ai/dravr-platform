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

use crate::common::{self, spawn_chat_server};

use async_trait::async_trait;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use pierre_test_support::sse::{read_frames, Frame};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The agent's reply, verbatim. Every assertion about prose compares against
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
            usage: Some(TokenUsage::new(42, 11, 53)),
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
        .map(|frame| frame.json().expect("frame JSON"))
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
        .map(|frame| frame.json().expect("frame JSON"))
        .collect();
    assert!(
        !block_frames.is_empty(),
        "a reply resolves to at least one block: {frames:?}"
    );
    let envelope = terminal.json().expect("done frame JSON");
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

/// A turn that fails ends on a `failed` frame naming the stable error code,
/// so a client words the failure from its own catalogue in the athlete's
/// language instead of printing the server's English description.
///
/// Driven by the fail-fast path: with the LLM probe reporting the provider
/// down, the turn is refused once its question is on file, before it reaches
/// the provider, and the refusal rides the stream the client already opened.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_turn_names_its_error_code_on_the_terminal_frame() {
    let resources = common::create_test_server_resources_with_llm(Arc::new(MockLlmProvider::new()))
        .await
        .expect("server resources");
    resources
        .common
        .llm_health
        .record_unhealthy("mock", "probe failed")
        .await;
    let base_url = spawn_chat_server(&resources).await;
    let (_user, token) =
        common::create_test_tenant_with_provider(&resources, "turn-failed@example.com")
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

    let terminal = frames.last().expect("stream produced no frames");
    assert_eq!(terminal.event, "failed", "frames = {frames:?}");
    let payload = terminal.json().expect("failed frame JSON");
    assert_eq!(
        payload["code"], "ResourceUnavailable",
        "the failed frame carries the code a client maps to its catalogue: {payload}"
    );
    assert_eq!(
        payload["error"], "The resource is temporarily unavailable",
        "the message stays the sanitized description, never the provider's internals: {payload}"
    );
}

/// A chat server whose LLM probe reads the provider down, so the health gate
/// refuses every turn: its base URL and a provider-connected user's token.
async fn refusing_server(email: &str) -> (String, String) {
    let resources = common::create_test_server_resources_with_llm(Arc::new(MockLlmProvider::new()))
        .await
        .expect("server resources");
    resources
        .common
        .llm_health
        .record_unhealthy("mock", "probe failed")
        .await;
    let base_url = spawn_chat_server(&resources).await;
    let (_user, token) = common::create_test_tenant_with_provider(&resources, email)
        .await
        .expect("create user + token + provider");
    (base_url, token)
}

/// What a reload reads: the conversation's newest window of messages.
async fn reload(client: &Client, base_url: &str, token: &str, conversation_id: &str) -> Value {
    let response = client
        .get(format!(
            "{base_url}/api/chat/conversations/{conversation_id}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .expect("GET messages");
    assert!(
        response.status().is_success(),
        "list messages: {}",
        response.status()
    );
    response.json().await.expect("messages JSON")
}

/// A reload after a refused turn shows the coach's words, closed by the
/// platform's "send it again" line, with no reply announced as coming.
fn assert_refused_turn_on_file(body: &Value, sent: &str) {
    let messages = body["messages"].as_array().expect("messages array");
    let questions: Vec<&Value> = messages.iter().filter(|m| m["role"] == "user").collect();
    assert_eq!(
        questions.len(),
        1,
        "the sent message is on file exactly once: {body}"
    );
    assert_eq!(questions[0]["content"], sent, "{body}");
    let position = messages
        .iter()
        .position(|m| m["role"] == "user")
        .expect("question position");
    let closing = messages
        .get(position + 1)
        .expect("a row closes the question");
    assert_eq!(closing["role"], "assistant", "{body}");
    assert_eq!(closing["finish_reason"], "interrupted", "{body}");
    assert!(
        body["awaiting_reply"].is_null(),
        "a refused turn is not a reply on its way: {body}"
    );
}

/// A turn the health gate refuses keeps the message the coach sent: a reload
/// shows it, closed by the interrupted notice, rather than losing it
/// (carnet#1362).
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_streamed_turn_keeps_the_sent_message_on_file() {
    let (base_url, token) = refusing_server("turn-refused-sse@example.com").await;
    let client = Client::builder().no_gzip().build().expect("client");
    let conversation_id = create_conversation(&client, &base_url, &token).await;
    let sent = "Build Marie's plan for the 10K";

    let frames = send_turn_streaming(&client, &base_url, &token, &conversation_id, sent).await;
    let terminal = frames.last().expect("stream produced no frames");
    assert_eq!(terminal.event, "failed", "frames = {frames:?}");
    assert_eq!(
        terminal.json().expect("failed frame JSON")["code"],
        "ResourceUnavailable",
        "the health gate's refusal names its code on the failed frame: {frames:?}"
    );

    let body = reload(&client, &base_url, &token, &conversation_id).await;
    assert_refused_turn_on_file(&body, sent);
}

/// The same refusal over the blocking JSON branch: a 503, and the message
/// still on file.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_blocking_turn_keeps_the_sent_message_on_file() {
    let (base_url, token) = refusing_server("turn-refused-json@example.com").await;
    let client = Client::builder().no_gzip().build().expect("client");
    let conversation_id = create_conversation(&client, &base_url, &token).await;
    let sent = "Build Marie's plan for the 10K";

    let response = client
        .post(format!(
            "{base_url}/api/chat/conversations/{conversation_id}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({ "content": sent }))
        .send()
        .await
        .expect("POST send_message");
    assert_eq!(response.status().as_u16(), 503, "the refusal is a 503");

    let body = reload(&client, &base_url, &token, &conversation_id).await;
    assert_refused_turn_on_file(&body, sent);
}
