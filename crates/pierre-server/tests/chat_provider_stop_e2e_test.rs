// ABOUTME: Drives a real in-app turn whose provider stops early or ignores parameters, over HTTP
// ABOUTME: The athlete is told the reply is incomplete, the row is stamped, and replay drops only the caveat
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A provider that runs out of token budget, or whose content filter trims
//! the output, still answers `Ok` — only `finish_reason` says the reply is a
//! fragment. Shown as-is, the fragment read as a finished answer. The same
//! response's `warnings` (a temperature the provider has no knob for, tools it
//! simulated in text) were dropped at the first conversion and never seen.
//!
//! These tests send a turn through the real `send_message` handler against a
//! scripted provider and assert what reaches the athlete, what the transcript
//! row is stamped with, what the turn's telemetry carries, and what the next
//! turn's prompt replays.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use async_trait::async_trait;
use futures_util::stream;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_REPLY_STOP_FILTERED, KEY_REPLY_STOP_TRUNCATED,
};
use pierre_core::errors::AppError;
use pierre_core::models::{
    FILTERED_REPLY_FINISH_REASON, STOP_CAVEAT_SEPARATOR, TRUNCATED_REPLY_FINISH_REASON,
};
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
    TokenUsage,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// The model's words, cut mid-sentence the way a token budget cuts them.
const PARTIAL_REPLY: &str = "A good taper keeps a little intensity while cutting volume, \
     so in the final week before the race you keep two short sessions at race pace, \
     drop the long ride entirely, and on the day before you replace the planned intervals with";

/// A question answerable from general knowledge, worded to avoid every term
/// capability recovery reads as a data ask: the turn reads none of the
/// athlete's data, so no recovery stage replaces the model's reply.
const GENERAL_QUESTION: &str = "How does caffeine affect endurance?";

/// A question about the athlete's own week. The test athlete's provider link
/// is expired, so the platform answers it with its reconnect message instead
/// of the model's reply — text the provider's stop says nothing about.
const DATA_QUESTION: &str = "How should I adjust this week?";

/// What the scripted provider reports ignoring.
const PROVIDER_WARNING: &str = "mock does not support temperature; requested value will be ignored";

/// Answers every call with [`PARTIAL_REPLY`], the scripted stop and warnings,
/// and keeps every request so a test can read what a later turn replayed.
struct ScriptedStopProvider {
    finish_reason: &'static str,
    warnings: Option<Vec<String>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl ScriptedStopProvider {
    fn new(finish_reason: &'static str, warnings: Option<Vec<String>>) -> Self {
        Self {
            finish_reason,
            warnings,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedStopProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn display_name(&self) -> &'static str {
        "Scripted stop LLM (tests)"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &'static str {
        "mock-model"
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(ChatResponse {
            content: PARTIAL_REPLY.to_owned(),
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(42, 11, 53)),
            finish_reason: Some(self.finish_reason.to_owned()),
            warnings: self.warnings.clone(),
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: PARTIAL_REPLY.to_owned(),
            is_final: true,
            finish_reason: Some(self.finish_reason.to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// A chat server over `provider`, an athlete, and a conversation to talk in.
struct Harness {
    client: Client,
    base_url: String,
    token: String,
    conversation_id: String,
}

impl Harness {
    async fn start(provider: Arc<ScriptedStopProvider>, email: &str) -> Self {
        let resources: Arc<ServerContext> = common::create_test_server_resources_with_llm(provider)
            .await
            .expect("server resources");
        let app = ChatRoutes::routes(Arc::clone(&resources));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        let base_url = format!("http://{addr}");
        let (_user, token) = common::create_test_tenant_with_provider(&resources, email)
            .await
            .expect("create user + token + provider");
        let client = Client::builder().no_gzip().build().expect("client");

        let resp = client
            .post(format!("{base_url}/api/chat/conversations"))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({ "title": "provider-stop-test", "model": "mock-model" }))
            .send()
            .await
            .expect("create conv");
        assert!(
            resp.status().is_success(),
            "create conversation: {}",
            resp.status()
        );
        let body: Value = resp.json().await.expect("conv json");
        let conversation_id = body["id"].as_str().expect("conv id").to_owned();

        Self {
            client,
            base_url,
            token,
            conversation_id,
        }
    }

    /// Send one turn and return the JSON turn document.
    async fn send(&self, content: &str) -> Value {
        let response = self
            .client
            .post(format!(
                "{}/api/chat/conversations/{}/messages",
                self.base_url, self.conversation_id
            ))
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({ "content": content }))
            .send()
            .await
            .expect("POST send_message");
        assert!(response.status().is_success(), "{}", response.status());
        response.json().await.expect("turn json")
    }
}

fn caveat(key: &str) -> String {
    MessagingStringsRegistry::new().get(key, "en")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_reply_is_flagged_stamped_and_replayed_without_its_caveat() {
    let provider = Arc::new(ScriptedStopProvider::new(
        "length",
        Some(vec![PROVIDER_WARNING.to_owned()]),
    ));
    let requests = Arc::clone(&provider.requests);
    let harness = Harness::start(provider, "provider-stop-truncated@example.com").await;

    let turn = harness.send(GENERAL_QUESTION).await;

    // The athlete reads the fragment AND is told it is one.
    let expected = format!(
        "{PARTIAL_REPLY}{STOP_CAVEAT_SEPARATOR}{}",
        caveat(KEY_REPLY_STOP_TRUNCATED)
    );
    assert_eq!(turn["assistant"]["message"]["content"], expected.as_str());
    // The transcript row carries the stamp replay cuts by; the turn itself
    // still reports the provider's own reason.
    assert_eq!(
        turn["assistant"]["message"]["finish_reason"],
        TRUNCATED_REPLY_FINISH_REASON
    );
    assert_eq!(turn["assistant"]["finish_reason"], "length");
    // What the provider ignored is no longer dropped.
    assert_eq!(
        turn["telemetry"]["provider_warnings"],
        json!([PROVIDER_WARNING]),
        "{turn}"
    );

    // The next turn replays the model's words as its own reply...
    requests.lock().unwrap().clear();
    harness.send("Continue please").await;
    let requests = requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .flat_map(|request| request.messages.iter())
            .any(|message| message.role == MessageRole::Assistant
                && message.content.starts_with(PARTIAL_REPLY)),
        "the partial reply is replayed as the assistant's turn"
    );
    // ...and no call the platform makes — the turn itself, or the extraction
    // that learns from a reply — ever reads the platform's caveat as the
    // model's words.
    let caveat_text = caveat(KEY_REPLY_STOP_TRUNCATED);
    for message in requests.iter().flat_map(|request| request.messages.iter()) {
        assert!(
            !message.content.contains(&caveat_text),
            "the caveat must not re-enter a prompt: {}",
            message.content
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_filtered_reply_is_flagged_and_stamped_as_filtered() {
    let provider = Arc::new(ScriptedStopProvider::new("SAFETY", None));
    let harness = Harness::start(provider, "provider-stop-filtered@example.com").await;

    let turn = harness.send(GENERAL_QUESTION).await;

    let expected = format!(
        "{PARTIAL_REPLY}{STOP_CAVEAT_SEPARATOR}{}",
        caveat(KEY_REPLY_STOP_FILTERED)
    );
    assert_eq!(turn["assistant"]["message"]["content"], expected.as_str());
    assert_eq!(
        turn["assistant"]["message"]["finish_reason"],
        FILTERED_REPLY_FINISH_REASON
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_that_finished_is_left_alone() {
    let provider = Arc::new(ScriptedStopProvider::new("stop", None));
    let harness = Harness::start(provider, "provider-stop-complete@example.com").await;

    let turn = harness.send(GENERAL_QUESTION).await;

    assert_eq!(turn["assistant"]["message"]["content"], PARTIAL_REPLY);
    assert_eq!(turn["assistant"]["message"]["finish_reason"], "stop");
    assert!(
        turn["telemetry"].get("provider_warnings").is_none(),
        "no warnings, no field: {turn}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_the_platform_replaced_takes_no_caveat() {
    let provider = Arc::new(ScriptedStopProvider::new("length", None));
    let harness = Harness::start(provider, "provider-stop-replaced@example.com").await;

    let turn = harness.send(DATA_QUESTION).await;

    let content = turn["assistant"]["message"]["content"]
        .as_str()
        .expect("content");
    assert!(
        !content.contains("A good taper"),
        "the premise: recovery replaced the model's reply: {content}"
    );
    assert!(
        !content.contains(STOP_CAVEAT_SEPARATOR),
        "a caveat about the model's truncation must not ride platform text: {content}"
    );
    assert_ne!(
        turn["assistant"]["message"]["finish_reason"],
        TRUNCATED_REPLY_FINISH_REASON
    );
}
