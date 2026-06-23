// ABOUTME: HTTP integration tests for A2A (Agent-to-Agent) protocol routes
// ABOUTME: Covers unauthenticated endpoints plus the authenticated message/send execution path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::uninlined_format_args)]

//! Comprehensive HTTP integration tests for A2A protocol routes
//!
//! This test suite validates that all A2A endpoints are correctly registered
//! in the router and handle HTTP requests appropriately.

mod common;
mod helpers;

use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_a2a::{A2ARoutes, A2ARoutesState};
use pierre_tool_runtime::runtime::ToolRuntime;
use std::sync::Arc;

/// Create test resources for A2A route testing
async fn create_a2a_test_resources() -> Arc<ServerContext> {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let auth_manager = common::create_test_auth_manager();
    let cache = common::create_test_cache().await.unwrap();
    let temp_dir = tempfile::tempdir().unwrap();

    let config = Arc::new(ServerConfig {
        http_port: 8081,
        database: DatabaseConfig {
            url: DatabaseUrl::Memory,
            backup: BackupConfig {
                directory: temp_dir.path().to_path_buf(),
                ..Default::default()
            },
            ..Default::default()
        },
        app_behavior: AppBehaviorConfig {
            ci_mode: true,
            auto_approve_users: false,
            ..Default::default()
        },
        security: SecurityConfig {
            headers: SecurityHeadersConfig {
                environment: Environment::Testing,
            },
            ..Default::default()
        },
        ..Default::default()
    });

    Arc::new(
        ServerContext::new(
            (*database).clone(),
            (*auth_manager).clone(),
            "test_jwt_secret",
            config,
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
            },
        )
        .await,
    )
}

/// Get A2A routes for testing
async fn a2a_routes() -> axum::Router {
    let resources = create_a2a_test_resources().await;
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let state = A2ARoutesState {
        ctx: resources.clone(),
        client_manager: resources.a2a.a2a_client_manager.clone(),
        auth_middleware: resources.auth.auth_middleware.clone(),
        tool_runtime,
    };
    A2ARoutes::routes(state)
}

// ============================================================================
// GET /a2a/status - A2A Status Tests
// ============================================================================

#[tokio::test]
async fn test_a2a_status_success() {
    let routes = a2a_routes().await;

    let response = AxumTestRequest::get("/a2a/status").send(routes).await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "active");
}

#[tokio::test]
async fn test_a2a_status_no_auth_required() {
    let routes = a2a_routes().await;

    // A2A status endpoint should work without any authentication
    let response = AxumTestRequest::get("/a2a/status").send(routes).await;

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_a2a_status_response_structure() {
    let routes = a2a_routes().await;

    let response = AxumTestRequest::get("/a2a/status").send(routes).await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    assert!(body.is_object());
    assert!(body["status"].is_string());
    assert_eq!(body["status"], "active");
}

#[tokio::test]
async fn test_a2a_status_content_type() {
    let routes = a2a_routes().await;

    let response = AxumTestRequest::get("/a2a/status").send(routes).await;

    assert_eq!(response.status(), 200);

    // Response should be valid JSON
    let body: serde_json::Value = response.json();
    assert!(body.is_object());
}

// ============================================================================
// Additional Integration Tests
// ============================================================================

#[tokio::test]
async fn test_a2a_status_concurrent_requests() {
    // Make multiple A2A status requests concurrently
    let mut handles = vec![];

    for _ in 0..10 {
        let handle = tokio::spawn(async {
            let routes = a2a_routes().await;
            AxumTestRequest::get("/a2a/status").send(routes).await
        });

        handles.push(handle);
    }

    // All requests should succeed
    for handle in handles {
        let response = handle.await.expect("Task panicked");
        assert_eq!(response.status(), 200);

        let body: serde_json::Value = response.json();
        assert_eq!(body["status"], "active");
    }
}

#[tokio::test]
async fn test_a2a_status_idempotency() {
    let routes = a2a_routes().await;

    // Make multiple requests and verify they all return the same result
    let responses = vec![
        AxumTestRequest::get("/a2a/status")
            .send(routes.clone())
            .await,
        AxumTestRequest::get("/a2a/status")
            .send(routes.clone())
            .await,
        AxumTestRequest::get("/a2a/status").send(routes).await,
    ];

    for response in responses {
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = response.json();
        assert_eq!(body["status"], "active");
    }
}

// ============================================================================
// POST /a2a/jsonrpc - A2A JSON-RPC transport endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_a2a_jsonrpc_route_is_mounted() {
    let routes = a2a_routes().await;

    // The agent card advertises {base_url}/a2a/jsonrpc; the route must exist
    // and dispatch into A2AServer (initialize requires no auth).
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "a2a/initialize",
        "id": 1
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .json(&body)
        .send(routes)
        .await;

    // A mounted route returns 200, not 404.
    assert_eq!(response.status(), 200, "/a2a/jsonrpc must be mounted");

    let envelope: serde_json::Value = response.json();
    assert_eq!(envelope["jsonrpc"], "2.0");
    assert_eq!(envelope["id"], 1);
    assert!(
        envelope["error"].is_null(),
        "a2a/initialize must not error: {envelope:?}"
    );
    assert!(envelope["result"].is_object());
}

#[tokio::test]
async fn test_a2a_jsonrpc_tools_list_returns_registry_tools() {
    let routes = a2a_routes().await;

    // tools/list is dispatched into A2AServer with real resources, so it
    // returns the live tool registry's user-visible schemas.
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "a2a/tools/list",
        "id": 2
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);

    let envelope: serde_json::Value = response.json();
    let tools = &envelope["result"]["tools"];
    assert!(tools.is_array(), "tools/list must return a tools array");
    assert!(
        !tools.as_array().unwrap().is_empty(),
        "tool registry should expose at least one tool"
    );
}

#[tokio::test]
async fn test_a2a_jsonrpc_message_send_requires_auth() {
    let routes = a2a_routes().await;

    // message/send now performs real, tenant-scoped work and is auth-gated;
    // an unauthenticated call must surface an auth error rather than a
    // hardcoded success constant.
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "message/send",
        "params": { "parts": [{ "type": "text", "content": "hi" }] },
        "id": 3
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);

    let envelope: serde_json::Value = response.json();
    assert!(
        envelope["result"].is_null(),
        "unauthenticated message/send must not return a result"
    );
    assert_eq!(
        envelope["error"]["code"], -32001,
        "unauthenticated message/send must return an auth error: {envelope:?}"
    );
}

#[tokio::test]
async fn test_a2a_status_always_active() {
    let routes = a2a_routes().await;

    // Verify that status is always "active"
    for _ in 0..5 {
        let response = AxumTestRequest::get("/a2a/status")
            .send(routes.clone())
            .await;

        assert_eq!(response.status(), 200);

        let body: serde_json::Value = response.json();
        assert_eq!(body["status"], "active");
    }
}

/// Build the A2A HTTP router from already-seeded resources (mirrors
/// `a2a_routes` but lets the caller seed a user/tenant first so the request
/// can be authenticated).
fn a2a_router_from(resources: &Arc<ServerContext>) -> axum::Router {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let state = A2ARoutesState {
        ctx: resources.clone(),
        client_manager: resources.a2a.a2a_client_manager.clone(),
        auth_middleware: resources.auth.auth_middleware.clone(),
        tool_runtime,
    };
    A2ARoutes::routes(state)
}

#[tokio::test]
async fn test_a2a_jsonrpc_message_send_authenticated_executes_and_replies() {
    // `test_a2a_jsonrpc_message_send_requires_auth` proves the unauthenticated
    // call surfaces -32001. This proves the *authenticated happy path*: a
    // valid JWT-bearing caller reaches the real, tenant-scoped message/send
    // execution path and receives an agent reply Message rather than an auth
    // error — the gap the auth-rejection test leaves open. message/send routes
    // a plain-text message (no tool intent) to a real echo reply, so the assert
    // is deterministic and needs no provider or LLM.
    let resources = create_a2a_test_resources().await;
    let (_user, jwt) = common::create_test_tenant(&resources, "a2a-auth@example.com")
        .await
        .expect("seed user + tenant + JWT with active_tenant_id");
    let routes = a2a_router_from(&resources);

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "message/send",
        "params": { "parts": [{ "type": "text", "content": "hello agent" }] },
        "id": 7
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("Authorization", &format!("Bearer {jwt}"))
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);
    let envelope: serde_json::Value = response.json();

    // Authenticated: NOT the -32001 the unauth path returns.
    assert!(
        envelope["error"].is_null(),
        "authenticated message/send must not error, got: {envelope:?}"
    );
    let parts = &envelope["result"]["message"]["parts"];
    assert!(
        parts.is_array() && !parts.as_array().unwrap().is_empty(),
        "authenticated message/send must return a reply message with parts, got: {envelope:?}"
    );
    // The agent's reply echoes the sent text back through the real handler.
    let reply = serde_json::to_string(&envelope["result"]).unwrap_or_default();
    assert!(
        reply.contains("hello agent"),
        "authenticated reply must echo the sent text, got: {envelope:?}"
    );
}
