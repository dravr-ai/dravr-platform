// ABOUTME: A text-simulation CLI turn must reach its runner with a tool surface, never with none
// ABOUTME: Pins that the CLI loop's request carries the MCP servers the catalog gate stands down for
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `run_cli_tool_loop` withholds the prose tool catalog whenever
//! `mcp_servers` is non-empty, and that is correct: handing a model both a
//! prose catalog and a real toolset lets it pick the prose, which is the form
//! `copilot` declines outright ("that tool isn't part of my real toolset") and
//! which never opens an MCP session, so the server's `initialize` instructions
//! — the caller's persona — never reach its system prompt.
//!
//! The gate only works if the other half holds: the servers themselves have to
//! travel on the request. `copilot` reads `request.mcp_servers` and turns a
//! non-empty slice into `--additional-mcp-config`; an empty one means no flag,
//! no tools, and an ungrounded answer. A request built without them therefore
//! leaves that runner with NEITHER surface — the catalog suppressed and the
//! servers absent — which is silent: the turn still returns `Ok` with prose.
//!
//! `cli_loop_request` is the loop's only request construction site, so pinning
//! it here pins the loop.

// `tool_execution` is gated behind client-chat; without it there is nothing to test.
#![cfg(feature = "client-chat")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use pierre_llm::{ChatMessage, McpHeader, McpServerConfig, McpTransport};
use pierre_tool_runtime::cli_loop::cli_loop_request;

fn dravr_server() -> McpServerConfig {
    McpServerConfig {
        name: "dravr".to_owned(),
        transport: McpTransport::Http {
            url: "http://localhost:8081/mcp".to_owned(),
            headers: vec![McpHeader {
                name: "Authorization".to_owned(),
                value: "Bearer session-token".to_owned(),
            }],
        },
    }
}

fn turn() -> Vec<ChatMessage> {
    vec![
        ChatMessage::system("coach persona"),
        ChatMessage::user("how did my week go?"),
    ]
}

#[test]
fn the_servers_the_catalog_gate_stands_down_for_travel_on_the_request() {
    let request = cli_loop_request(turn(), "claude-sonnet-4", None, vec![dravr_server()]);

    assert_eq!(
        request.mcp_servers.len(),
        1,
        "the CLI loop suppresses the prose catalog when servers are present, so a request \
         without them hands the runner no tool surface at all"
    );
    assert_eq!(request.mcp_servers[0].name, "dravr");
    assert_eq!(
        request.mcp_servers[0].transport,
        McpTransport::Http {
            url: "http://localhost:8081/mcp".to_owned(),
            headers: vec![McpHeader {
                name: "Authorization".to_owned(),
                value: "Bearer session-token".to_owned(),
            }],
        },
        "the transport must cross the request boundary intact — copilot serializes it \
         verbatim into --additional-mcp-config"
    );
}

#[test]
fn every_server_handed_in_reaches_the_runner() {
    let second = McpServerConfig {
        name: "dravr-admin".to_owned(),
        transport: McpTransport::Stdio {
            command: "/usr/local/bin/pierre-mcp".to_owned(),
            args: vec!["--stdio".to_owned()],
            env: Vec::new(),
        },
    };
    let request = cli_loop_request(
        turn(),
        "claude-sonnet-4",
        None,
        vec![dravr_server(), second],
    );

    let names: Vec<&str> = request
        .mcp_servers
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["dravr", "dravr-admin"],
        "the loop must forward the whole server list, in order — a dropped entry is a \
         silently missing tool group"
    );
}

#[test]
fn no_servers_means_the_prose_catalog_is_the_surface() {
    let request = cli_loop_request(turn(), "claude-sonnet-4", None, Vec::new());

    assert!(
        request.mcp_servers.is_empty(),
        "with no servers the loop injects the text catalog instead; fabricating a server \
         here would give the model both forms and it picks the prose"
    );
    assert_eq!(request.model.as_deref(), Some("claude-sonnet-4"));
}

#[test]
fn the_coach_temperature_survives_alongside_the_servers() {
    let request = cli_loop_request(turn(), "claude-sonnet-4", Some(0.35), vec![dravr_server()]);

    let temperature = request
        .temperature
        .expect("the per-coach temperature reaches the request");
    assert!(
        (temperature - 0.35).abs() < f32::EPSILON,
        "the coach's sampling temperature must cross intact; got {temperature}"
    );
    assert_eq!(
        request.mcp_servers.len(),
        1,
        "per-coach temperature and the tool surface are independent — setting one must \
         not drop the other"
    );
    assert_eq!(request.messages.len(), 2, "messages cross unmodified");
}
