// ABOUTME: Pins that the MCP server advertises only what it implements, and lists only what a caller may call
// ABOUTME: Capabilities, the methods it leaves to -32601, resource templates/not-found, and tools/list per caller
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The advertised MCP surface matches the implemented one
//!
//! The server used to declare `logging` and `completions` while `logging/setLevel`
//! answered -32601 and completion suggested arguments no prompt declares; it
//! answered the client-side `sampling/createMessage` and `roots/list`; a
//! missing resource was -32602; the anonymous stdio caller was shown every
//! tool and then refused each one; and an admin was shown tools `tools/call`
//! would refuse them. Each test below fails on that code.

mod common;

use std::sync::Arc;

use dravr_tronc::mcp::protocol::JsonRpcRequest;
use dravr_tronc::mcp::server::McpServer;
use dravr_tronc::mcp::tool::ToolContext;
use pierre_mcp_server::mcp::host_seams::build_mcp_server;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};

async fn server() -> (Arc<ServerContext>, Arc<McpServer<dyn ToolRuntime>>) {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let server = build_mcp_server(resources.clone());
    (resources, server)
}

fn request(method: &str, params: &Value) -> JsonRpcRequest {
    serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    }))
    .unwrap()
}

async fn call(
    server: &McpServer<dyn ToolRuntime>,
    method: &str,
    params: Value,
    ctx: &ToolContext,
) -> Value {
    let response = server
        .handle_request_with_context(request(method, &params), ctx)
        .await
        .expect("a request with an id is answered");
    serde_json::to_value(response).unwrap()
}

fn tool_names(response: &Value) -> Vec<String> {
    response["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools/list answered: {response}"))
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn capabilities_name_only_the_areas_the_server_answers() {
    let (_resources, server) = server().await;
    let response = call(
        &server,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "clientInfo": { "name": "surface-test", "version": "1.0.0" },
            "capabilities": {}
        }),
        &ToolContext::default(),
    )
    .await;

    let capabilities = &response["result"]["capabilities"];
    assert!(capabilities["tools"].is_object(), "{capabilities}");
    assert!(capabilities["resources"].is_object(), "{capabilities}");
    assert!(capabilities["prompts"].is_object(), "{capabilities}");
    let declared = capabilities.as_object().unwrap();
    assert!(
        !declared.contains_key("logging"),
        "no log notifications are emitted: {capabilities}"
    );
    assert!(
        !declared.contains_key("completions"),
        "no argument completion is offered: {capabilities}"
    );
}

#[tokio::test]
async fn methods_the_server_does_not_implement_are_method_not_found() {
    let (_resources, server) = server().await;
    for (method, params) in [
        ("logging/setLevel", json!({ "level": "debug" })),
        (
            "completion/complete",
            json!({
                "ref": { "type": "ref/prompt", "name": "weekly_review" },
                "argument": { "name": "week", "value": "2" }
            }),
        ),
        ("roots/list", json!({})),
        (
            "sampling/createMessage",
            json!({ "messages": [], "maxTokens": 10 }),
        ),
        ("authenticate", json!({ "email": "a@b.c", "password": "x" })),
    ] {
        let response = call(&server, method, params, &ToolContext::default()).await;
        assert_eq!(
            response["error"]["code"], -32601,
            "{method} must be method-not-found: {response}"
        );
    }
}

#[tokio::test]
async fn resource_templates_name_the_coach_uri_and_a_missing_coach_is_32002() {
    let (_resources, server) = server().await;

    let templates = call(
        &server,
        "resources/templates/list",
        json!({}),
        &ToolContext::default(),
    )
    .await;
    let listed = templates["result"]["resourceTemplates"]
        .as_array()
        .unwrap_or_else(|| panic!("resources/templates/list answered: {templates}"));
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["uriTemplate"], "dravr://coaches/{id}");
    assert_eq!(listed[0]["mimeType"], "text/markdown");

    let missing = call(
        &server,
        "resources/read",
        json!({ "uri": "dravr://coaches/00000000-0000-0000-0000-000000000000" }),
        &ToolContext::default(),
    )
    .await;
    assert_eq!(missing["error"]["code"], -32002, "{missing}");
}

#[tokio::test]
async fn an_anonymous_caller_is_listed_nothing_it_could_not_call() {
    let (_resources, server) = server().await;
    let response = call(&server, "tools/list", json!({}), &ToolContext::default()).await;
    assert!(
        tool_names(&response).is_empty(),
        "tools/call refuses a caller with no identity, so none is listed: {response}"
    );
}

#[tokio::test]
async fn an_admin_is_not_listed_a_tool_disabled_for_their_tenant() {
    let (resources, server) = server().await;
    let (user, _token) = common::create_test_tenant(&resources, "surface-admin@example.com")
        .await
        .unwrap();
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .remove(0);
    let admin = ToolContext::new()
        .with_user(user.id.to_string())
        .with_tenant(tenant.id.to_string())
        .as_admin(true);

    let before = tool_names(&call(&server, "tools/list", json!({}), &admin).await);
    assert!(before.iter().any(|name| name == "get_activities"));

    resources
        .mcp
        .tool_selection
        .set_tool_override(tenant.id, "get_activities", false, user.id, None)
        .await
        .unwrap();

    let after = tool_names(&call(&server, "tools/list", json!({}), &admin).await);
    assert!(
        !after.iter().any(|name| name == "get_activities"),
        "a tool tools/call would refuse is not listed, even to an admin"
    );
    assert_eq!(after.len(), before.len() - 1);
}
