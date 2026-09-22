// ABOUTME: Pins the MCP initialize instructions onto the hot-reload prompt registry
// ABOUTME: A synced edit must reach the next client handshake without a restart
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The sibling of `system_prompt_hot_reload_test.rs`, for the one prompt whose
//! consumer is not an LLM call.
//!
//! `mcp_server_instructions` moved into the contremaitre catalogue so it could
//! be edited without a deploy, and then did not get the benefit: the engine
//! took the text once at server construction, so every client that connected
//! for the life of the process read the revision loaded at startup. It was the
//! only one of the catalogue's system prompts that still needed a redeploy.
//!
//! The sync's only effect on a prompt is an `update_system_prompt` write into
//! the registry, so writing into it here is what a landing webhook does.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_schema::{McpRequest, McpResponse};
use pierre_mcp_server::mcp::host_seams::build_mcp_server;
use serde_json::{json, Value};

mod common;

const SYNCED_INSTRUCTIONS: &str =
    "This server exposes an athlete's training data. Start with get_connection_status.";
const SYNC_SHA: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn initialize_request(id: i64) -> McpRequest {
    serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "clientInfo": { "name": "test-client", "version": "1.0.0" },
            "capabilities": {}
        }
    }))
    .expect("initialize request should parse")
}

fn advertised(response: &McpResponse) -> Value {
    response.result.as_ref().expect("initialize should succeed")["instructions"].clone()
}

#[tokio::test]
async fn test_synced_instructions_reach_the_next_handshake() {
    common::init_server_config();
    let resources = common::create_test_server_resources()
        .await
        .expect("Failed to create test resources");

    // One server, as a running process has: the registry is mutated under it
    // between the two handshakes, exactly as a contremaitre sync does.
    let server = build_mcp_server(resources.clone());

    let before = server
        .handle_request(initialize_request(1))
        .await
        .expect("engine returns a response for a request with an id");
    let before_text = advertised(&before);
    assert!(
        before_text.as_str().is_some_and(|s| !s.is_empty()),
        "an MCP client must never be handed empty instructions, got: {before_text}"
    );

    resources.mcp.prompt_registry.update_system_prompt(
        "mcp_server_instructions",
        SYNCED_INSTRUCTIONS.to_owned(),
        SYNC_SHA.to_owned(),
    );

    let after = server
        .handle_request(initialize_request(2))
        .await
        .expect("engine returns a response for a request with an id");

    assert_eq!(
        advertised(&after),
        SYNCED_INSTRUCTIONS,
        "a synced mcp_server_instructions must reach the next handshake on the \
         same server instance, not wait for a restart"
    );
    assert_ne!(
        before_text,
        advertised(&after),
        "the fixture must differ from the pre-sync text"
    );
}

#[tokio::test]
async fn test_instructions_are_trimmed_for_the_client() {
    common::init_server_config();
    let resources = common::create_test_server_resources()
        .await
        .expect("Failed to create test resources");
    let server = build_mcp_server(resources.clone());

    resources.mcp.prompt_registry.update_system_prompt(
        "mcp_server_instructions",
        format!("\n\n{SYNCED_INSTRUCTIONS}\n\n"),
        SYNC_SHA.to_owned(),
    );

    let response = server
        .handle_request(initialize_request(3))
        .await
        .expect("engine returns a response for a request with an id");

    assert_eq!(
        advertised(&response),
        SYNCED_INSTRUCTIONS,
        "the catalogue document's surrounding whitespace must not reach the client"
    );
}
