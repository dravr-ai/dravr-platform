// ABOUTME: Request construction for the text-simulation CLI tool loop
// ABOUTME: Keeps the catalog gate and the servers that stand in for it together
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The one place a text-simulation CLI turn's request is built.
//!
//! `run_cli_tool_loop` withholds the prose tool catalog whenever MCP servers
//! are present, because handing a model both lets it pick the prose — which
//! never opens an MCP session, so the server's `initialize` instructions never
//! reach the model's system prompt. That gate is only correct while the servers
//! themselves travel on the request; a request built without them leaves the
//! runner with neither surface, and the turn still returns `Ok` with prose.

use pierre_llm::{ChatMessage, ChatRequest, McpServerConfig};

/// Builds one iteration's request for the text-simulation CLI loop.
///
/// The MCP servers ride on every request. [`run_cli_tool_loop`] withholds the
/// prose tool catalog exactly when they are present, because the runner reaches
/// the same tools through them — `copilot` turns them into
/// `--additional-mcp-config` — so a request that dropped them would leave that
/// runner with no tool surface at all: no catalog and no servers.
#[must_use]
pub fn cli_loop_request(
    messages: Vec<ChatMessage>,
    model: &str,
    temperature: Option<f32>,
    mcp_servers: Vec<McpServerConfig>,
) -> ChatRequest {
    let req = ChatRequest::new(messages)
        .with_model(model)
        .with_mcp_servers(mcp_servers);
    match temperature {
        Some(t) => req.with_temperature(t),
        None => req,
    }
}
