// ABOUTME: Unit tests for the A2A message/send forwarding logic and helpers
// ABOUTME: Verifies tool-intent extraction, text collection, and auth gating of message/send
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests for `A2AServer::handle_message_send` and its supporting helpers.
//!
//! `message/send` is now authenticated and forwards real work: either a tool
//! invocation through the shared registry path, or a real reply `Message`
//! echoing the received text. These tests cover the pure helpers and confirm
//! that an unconfigured server rejects `message/send` rather than returning a
//! hardcoded success constant.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_a2a::protocol::A2AServer;
use pierre_a2a::A2ARequest;
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_extract_tool_intent_top_level() {
    let params = json!({
        "tool_name": "get_activities",
        "parameters": { "limit": 5 }
    });

    let (name, tool_params) = A2AServer::extract_tool_intent(&params).expect("intent");
    assert_eq!(name, "get_activities");
    assert_eq!(tool_params["limit"], 5);
}

#[test]
fn test_extract_tool_intent_from_data_part() {
    let params = json!({
        "parts": [
            { "type": "text", "content": "please run this" },
            { "type": "data", "content": { "tool_name": "get_athlete", "parameters": {} } }
        ]
    });

    let (name, _params) = A2AServer::extract_tool_intent(&params).expect("intent");
    assert_eq!(name, "get_athlete");
}

#[test]
fn test_extract_tool_intent_absent_for_plain_text() {
    let params = json!({
        "parts": [{ "type": "text", "content": "hello agent" }]
    });

    assert!(A2AServer::extract_tool_intent(&params).is_none());
}

#[test]
fn test_collect_message_text_joins_text_parts() {
    let params = json!({
        "parts": [
            { "type": "text", "content": "line one" },
            { "type": "data", "content": { "k": "v" } },
            { "type": "text", "content": "line two" }
        ]
    });

    assert_eq!(
        A2AServer::collect_message_text(&params),
        "line one\nline two"
    );
}

#[test]
fn test_collect_message_text_empty_when_no_text_parts() {
    let params = json!({ "parts": [] });
    assert_eq!(A2AServer::collect_message_text(&params), "");
}

#[tokio::test]
async fn test_message_send_requires_auth() {
    let server = A2AServer::new();

    // Without resources/auth the server must reject message/send instead of
    // returning the previous hardcoded {"status":"received"} constant.
    let request = A2ARequest {
        jsonrpc: "2.0".to_owned(),
        method: "message/send".to_owned(),
        params: Some(json!({ "parts": [{ "type": "text", "content": "hi" }] })),
        id: Some(json!(7)),
        auth_token: None,
        headers: None,
        metadata: HashMap::new(),
    };

    let response = server.handle_request(request).await;
    assert!(response.result.is_none());
    let error = response
        .error
        .expect("message/send must error without auth");
    // -32001 (auth required) or -32000 (server not configured) are both valid:
    // the dispatcher requires auth before reaching the (absent) resources.
    assert!(
        error.code == -32001 || error.code == -32000,
        "unexpected error code {}",
        error.code
    );
}
