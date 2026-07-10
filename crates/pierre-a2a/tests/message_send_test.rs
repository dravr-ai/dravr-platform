// ABOUTME: Unit tests for the A2A 1.0 SendMessage helpers and auth gating
// ABOUTME: Verifies tool-intent extraction from data parts, text collection, and unauthenticated rejection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests for `A2AServer` `SendMessage` supporting helpers.
//!
//! `SendMessage` is authenticated and forwards real work: either a tool
//! invocation through the shared registry path (a `data` part carrying
//! `tool_name`/`parameters`), or a message-only exchange answered with a
//! real reply `Message` echoing the received text.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_a2a::protocol::A2AServer;
use pierre_a2a::protocol_types::{Message, Part, Role};
use pierre_a2a::A2ARequest;
use serde_json::json;
use std::collections::HashMap;

fn user_message(parts: Vec<Part>) -> Message {
    Message {
        message_id: "m-test".to_owned(),
        role: Role::User,
        parts,
        context_id: None,
        task_id: None,
        reference_task_ids: Vec::new(),
        extensions: Vec::new(),
        metadata: None,
    }
}

#[test]
fn test_extract_tool_intent_from_data_part() {
    let message = user_message(vec![
        Part::text("please run this".to_owned()),
        Part::data(json!({ "tool_name": "get_athlete", "parameters": { "limit": 5 } })),
    ]);

    let (name, params) = A2AServer::extract_tool_intent(&message).expect("intent");
    assert_eq!(name, "get_athlete");
    assert_eq!(params["limit"], 5);
}

#[test]
fn test_extract_tool_intent_defaults_parameters() {
    let message = user_message(vec![Part::data(json!({ "tool_name": "get_activities" }))]);

    let (name, params) = A2AServer::extract_tool_intent(&message).expect("intent");
    assert_eq!(name, "get_activities");
    assert!(params.as_object().unwrap().is_empty());
}

#[test]
fn test_extract_tool_intent_absent_for_plain_text() {
    let message = user_message(vec![Part::text("hello agent".to_owned())]);

    assert!(A2AServer::extract_tool_intent(&message).is_none());
}

#[test]
fn test_collect_message_text_joins_text_parts() {
    let message = user_message(vec![
        Part::text("line one".to_owned()),
        Part::data(json!({ "k": "v" })),
        Part::text("line two".to_owned()),
    ]);

    assert_eq!(
        A2AServer::collect_message_text(&message),
        "line one\nline two"
    );
}

#[test]
fn test_collect_message_text_empty_when_no_text_parts() {
    let message = user_message(vec![Part::data(json!({}))]);
    assert_eq!(A2AServer::collect_message_text(&message), "");
}

#[tokio::test]
async fn test_send_message_requires_auth() {
    let server = A2AServer::new();

    // Without resources/auth the server must reject SendMessage.
    let request = A2ARequest {
        jsonrpc: "2.0".to_owned(),
        method: "SendMessage".to_owned(),
        params: Some(json!({
            "message": {
                "messageId": "m-1",
                "role": "ROLE_USER",
                "parts": [{ "text": "hi" }]
            }
        })),
        id: Some(json!(7)),
        auth_token: None,
        headers: None,
        metadata: HashMap::new(),
    };

    let response = server.handle_request(request).await;
    assert!(response.result.is_none());
    let error = response.error.expect("SendMessage must error without auth");
    assert_eq!(
        error.code, -32000,
        "auth/config failures stay in the -32000 space, got {}",
        error.code
    );
}
