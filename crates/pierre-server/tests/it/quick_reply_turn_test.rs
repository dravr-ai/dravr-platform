// ABOUTME: A reply ending on the model's dravr-actions fence stores clean prose and offers its items as buttons
// ABOUTME: Drives the real send_message handler against a scripted model, then taps a qr: postback (carnet#1486)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Quick replies, end to end over HTTP.
//!
//! The model closes a reply on a ```` ```dravr-actions ```` fence holding the
//! replies the athlete might send next. Post-processing lifts it out of the
//! stored prose, and `next_steps` offers its items first, as postbacks
//! `qr:<slot>:<text>`; a tap sends the words as the athlete's message and
//! writes no catalogue exposure.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common::{self, spawn_chat_server};

use async_trait::async_trait;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;

/// The prose the model writes before its fence.
const PROSE: &str = "Happy to plan tomorrow with you. Easy spin or a full rest?";

/// The model's reply: the prose, then the fence.
const FENCED_REPLY: &str = "Happy to plan tomorrow with you. Easy spin or a full rest?\n\n```dravr-actions\n[\"Easy spin\", \"Rest day\", \"Show my week\"]\n```";

/// A provider that answers every call with [`FENCED_REPLY`].
struct FencedModel {
    model: String,
}

#[async_trait]
impl LlmProvider for FencedModel {
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
        Ok(ChatResponse {
            replay_blocks: None,
            content: FENCED_REPLY.to_owned(),
            model: self.model.clone(),
            usage: Some(TokenUsage::new(42, 11, 53)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: FENCED_REPLY.to_owned(),
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
        .json(&json!({ "title": "quick-reply-test", "model": "mock-model" }))
        .send()
        .await
        .expect("create conv");
    assert!(
        resp.status().is_success(),
        "bug: create conversation answered {}",
        resp.status()
    );
    let body: Value = resp.json().await.expect("conv json");
    body["id"].as_str().expect("conv id").to_owned()
}

/// Send `content` and read the one JSON envelope the turn answers with.
async fn send(
    client: &Client,
    base_url: &str,
    token: &str,
    conversation: &str,
    content: &str,
) -> Value {
    let response = client
        .post(format!(
            "{base_url}/api/chat/conversations/{conversation}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({ "content": content }))
        .send()
        .await
        .expect("POST send_message");
    assert!(
        response.status().is_success(),
        "bug: the route answered {}",
        response.status()
    );
    response.json().await.expect("envelope JSON")
}

/// The conversation's stored messages, as a reload reads them.
async fn reload(client: &Client, base_url: &str, token: &str, conversation: &str) -> Value {
    let response = client
        .get(format!(
            "{base_url}/api/chat/conversations/{conversation}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .expect("GET messages");
    assert!(
        response.status().is_success(),
        "bug: the route answered {}",
        response.status()
    );
    response.json().await.expect("messages JSON")
}

/// Every postback value in `value`, wherever it sits.
fn postbacks(value: &Value) -> Vec<String> {
    match value {
        Value::Object(map) => {
            let own = map
                .get("value")
                .and_then(Value::as_str)
                .filter(|v| v.starts_with("qr:") || v.starts_with("nx:"))
                .map(str::to_owned);
            own.into_iter()
                .chain(map.values().flat_map(postbacks))
                .collect()
        }
        Value::Array(items) => items.iter().flat_map(postbacks).collect(),
        _ => Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fenced_reply_stores_prose_and_offers_its_items_and_a_tap_sends_the_words() {
    let resources = common::create_test_server_resources_with_llm(Arc::new(FencedModel {
        model: "mock-model".to_owned(),
    }))
    .await
    .expect("server resources");
    let base_url = spawn_chat_server(&resources).await;
    let (user, token) = common::create_test_tenant(&resources, "quick-reply@example.com")
        .await
        .expect("create user + token");
    let client = Client::builder().no_gzip().build().expect("client");
    let conversation = create_conversation(&client, &base_url, &token).await;

    let envelope = send(
        &client,
        &base_url,
        &token,
        &conversation,
        "How was my week?",
    )
    .await;
    let content = envelope["assistant"]["message"]["content"]
        .as_str()
        .expect("the reply's content");
    assert_eq!(content, PROSE, "bug: the stored prose must carry no fence");
    let offered = postbacks(&envelope);
    assert_eq!(
        offered.iter().take(3).cloned().collect::<Vec<_>>(),
        vec![
            "qr:0:Easy spin".to_owned(),
            "qr:1:Rest day".to_owned(),
            "qr:2:Show my week".to_owned()
        ],
        "bug: the model's items must be offered first, in order, as qr: postbacks: {envelope}"
    );

    let stored = reload(&client, &base_url, &token, &conversation).await;
    let stored_text = stored.to_string();
    assert!(
        !stored_text.contains("dravr-actions"),
        "bug: no stored row may carry the fence: {stored}"
    );
    assert!(
        stored_text.contains("qr:1:Rest day"),
        "bug: the stored reply must keep its quick replies as its actions block: {stored}"
    );

    send(&client, &base_url, &token, &conversation, "qr:1:Rest day").await;
    let stored = reload(&client, &base_url, &token, &conversation).await;
    let user_rows: Vec<&str> = stored
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row["role"] == "user")
        .filter_map(|row| row["content"].as_str())
        .collect();
    assert!(
        user_rows.contains(&"Rest day"),
        "bug: the tap must arrive as the athlete's own words: {user_rows:?}"
    );
    assert!(
        !user_rows.iter().any(|row| row.starts_with("qr:")),
        "bug: the postback itself must never be stored as the athlete's words: {user_rows:?}"
    );
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()[0]
        .id;
    let exposures = resources
        .common
        .repos
        .use_case_exposures
        .use_case_exposures(tenant, user.id)
        .await
        .unwrap();
    assert!(
        exposures
            .iter()
            .all(|exposure| exposure.tapped_at.is_none()),
        "bug: a quick reply's tap must write no catalogue exposure: {exposures:?}"
    );
}
