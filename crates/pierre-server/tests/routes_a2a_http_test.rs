// ABOUTME: HTTP integration tests for A2A 1.0 protocol routes (JSONRPC + HTTP+JSON bindings)
// ABOUTME: Covers version negotiation, auth gating, SendMessage tool-intent gating, and REST error shapes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::uninlined_format_args)]

//! Comprehensive HTTP integration tests for A2A protocol routes
//!
//! This test suite validates that all A2A endpoints are correctly registered
//! in the router, enforce `A2A-Version` negotiation, and handle HTTP
//! requests appropriately on both bindings.

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
                turn_runner: None,
            },
        )
        .await,
    )
}

/// Build the A2A HTTP router from already-seeded resources.
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

/// Get A2A routes for testing
async fn a2a_routes() -> axum::Router {
    let resources = create_a2a_test_resources().await;
    a2a_router_from(&resources)
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

// ============================================================================
// GET /.well-known/agent-card.json - discovery
// ============================================================================

#[tokio::test]
async fn test_agent_card_discovery_route() {
    let routes = a2a_routes().await;

    // Discovery is public and NOT version-gated (it is how clients learn
    // the supported versions in the first place).
    let response = AxumTestRequest::get("/.well-known/agent-card.json")
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);
    let card: serde_json::Value = response.json();
    assert_eq!(card["name"], "Dravr AI");
    let interfaces = card["supportedInterfaces"].as_array().unwrap();
    assert!(!interfaces.is_empty());
    assert_eq!(interfaces[0]["protocolBinding"], "JSONRPC");
    assert_eq!(interfaces[0]["protocolVersion"], "1.0");
    assert_eq!(card["capabilities"]["streaming"], true);
    assert_eq!(card["capabilities"]["pushNotifications"], true);
}

// ============================================================================
// POST /a2a/jsonrpc - JSONRPC binding
// ============================================================================

#[tokio::test]
async fn test_a2a_jsonrpc_requires_version_header() {
    let routes = a2a_routes().await;

    // Spec §3.6: an absent A2A-Version header means 0.3 semantics, which
    // this server does not carry — VersionNotSupportedError (-32009).
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "GetExtendedAgentCard",
        "id": 1
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200, "/a2a/jsonrpc must be mounted");
    let envelope: serde_json::Value = response.json();
    assert_eq!(envelope["jsonrpc"], "2.0");
    assert_eq!(envelope["id"], 1);
    assert_eq!(envelope["error"]["code"], -32009);
    assert_eq!(
        envelope["error"]["data"][0]["reason"], "VERSION_NOT_SUPPORTED",
        "-32009 must carry google.rpc.ErrorInfo: {envelope:?}"
    );
}

#[tokio::test]
async fn test_a2a_jsonrpc_rejects_unknown_version() {
    let routes = a2a_routes().await;

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "GetExtendedAgentCard",
        "id": 2
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "0.3")
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);
    let envelope: serde_json::Value = response.json();
    assert_eq!(envelope["error"]["code"], -32009);
}

#[tokio::test]
async fn test_a2a_jsonrpc_version_negotiated_dispatch() {
    let routes = a2a_routes().await;

    // With A2A-Version: 1.0 the request reaches method dispatch; the
    // authenticated method's 401 (rather than -32009) proves the gate passed.
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "GetExtendedAgentCard",
        "id": 3
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 401);
    let envelope: serde_json::Value = response.json();
    assert_eq!(envelope["error"]["code"], -32000);
    assert_eq!(
        envelope["error"]["data"][0]["reason"],
        "AUTHENTICATION_REQUIRED"
    );
}

#[tokio::test]
async fn test_extended_agent_card_lists_live_tools() {
    let resources = create_a2a_test_resources().await;
    let (_user, jwt) = common::create_test_tenant(&resources, "a2a-extcard@example.com")
        .await
        .expect("seed user + tenant + JWT with active_tenant_id");
    let routes = a2a_router_from(&resources);

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "GetExtendedAgentCard",
        "id": 8
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("Authorization", &format!("Bearer {jwt}"))
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);
    let envelope: serde_json::Value = response.json();
    assert!(
        envelope["error"].is_null(),
        "authenticated GetExtendedAgentCard must not error: {envelope:?}"
    );
    let card = &envelope["result"];
    assert_eq!(card["capabilities"]["extendedAgentCard"], true);
    let skills = card["skills"].as_array().expect("skills array");
    // The extended card is the public card enriched with the live registry:
    // strictly more skills than the four curated public ones, each tagged.
    assert!(
        skills.len() > 4,
        "extended card must include live registry skills, got {}",
        skills.len()
    );
    assert!(skills.iter().any(|skill| skill["tags"]
        .as_array()
        .is_some_and(|tags| tags.iter().any(|tag| tag == "tool"))));
}

#[tokio::test]
async fn test_a2a_jsonrpc_send_message_requires_auth() {
    let routes = a2a_routes().await;

    // SendMessage performs real, tenant-scoped work and is auth-gated; an
    // unauthenticated call surfaces HTTP 401 (auth is transport-level).
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": "m-1",
                "role": "ROLE_USER",
                "parts": [{ "text": "hi" }]
            }
        },
        "id": 4
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(
        response.status(),
        401,
        "unauthenticated SendMessage must surface HTTP 401"
    );

    let envelope: serde_json::Value = response.json();
    assert!(
        envelope["result"].is_null(),
        "unauthenticated SendMessage must not return a result"
    );
    assert_eq!(envelope["error"]["code"], -32000);
    assert_eq!(
        envelope["error"]["data"][0]["reason"], "AUTHENTICATION_REQUIRED",
        "auth errors must carry the AUTHENTICATION_REQUIRED reason: {envelope:?}"
    );
}

#[tokio::test]
async fn test_a2a_jsonrpc_send_message_plain_text_is_refused_not_echoed() {
    // SendMessage executes registered tools addressed by a `data` part. A
    // plain-text message names no tool, so it is refused with the spec's
    // ContentTypeNotSupported error — never answered by handing the caller
    // its own sentence back as the agent's reply.
    let resources = create_a2a_test_resources().await;
    let (_user, jwt) = common::create_test_tenant(&resources, "a2a-auth@example.com")
        .await
        .expect("seed user + tenant + JWT with active_tenant_id");
    let routes = a2a_router_from(&resources);

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": "m-7",
                "role": "ROLE_USER",
                "parts": [{ "text": "hello agent" }]
            }
        },
        "id": 7
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("Authorization", &format!("Bearer {jwt}"))
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);
    let envelope: serde_json::Value = response.json();

    assert!(
        envelope["result"].is_null(),
        "a message naming no tool must not produce a result: {envelope:?}"
    );
    assert_eq!(envelope["error"]["code"], -32005);
    assert_eq!(
        envelope["error"]["data"][0]["reason"], "CONTENT_TYPE_NOT_SUPPORTED",
        "refusal must carry the spec reason: {envelope:?}"
    );
    assert!(
        envelope["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("tool_name"),
        "the refusal must name the accepted shape: {envelope:?}"
    );
    assert!(
        !serde_json::to_string(&envelope)
            .expect("serialize envelope")
            .contains("hello agent"),
        "the caller's own text must never come back as the agent reply: {envelope:?}"
    );
}

// ============================================================================
// HTTP+JSON binding (REST paths)
// ============================================================================

#[tokio::test]
async fn test_rest_binding_requires_version() {
    let routes = a2a_routes().await;

    // REST version failures are google.rpc.Status JSON with HTTP 400.
    let response = AxumTestRequest::get("/a2a/tasks").send(routes).await;

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["code"], 400);
    assert_eq!(body["error"]["status"], "FAILED_PRECONDITION");
    assert_eq!(
        body["error"]["details"][0]["reason"],
        "VERSION_NOT_SUPPORTED"
    );
}

#[tokio::test]
async fn test_rest_binding_auth_maps_to_401() {
    let routes = a2a_routes().await;

    let response = AxumTestRequest::get("/a2a/tasks?A2A-Version=1.0")
        .send(routes)
        .await;

    assert_eq!(response.status(), 401);
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["status"], "UNAUTHENTICATED");
}

#[tokio::test]
async fn test_rest_message_send_plain_text_is_refused_not_echoed() {
    let resources = create_a2a_test_resources().await;
    let (_user, jwt) = common::create_test_tenant(&resources, "a2a-rest@example.com")
        .await
        .expect("seed user + tenant + JWT with active_tenant_id");
    let routes = a2a_router_from(&resources);

    // The REST body is the SendMessageRequest object itself.
    let body = serde_json::json!({
        "message": {
            "messageId": "m-rest-1",
            "role": "ROLE_USER",
            "parts": [{ "text": "ping over rest" }]
        }
    });
    let response = AxumTestRequest::post("/a2a/message:send")
        .header("A2A-Version", "1.0")
        .header("Authorization", &format!("Bearer {jwt}"))
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 400);
    let result: serde_json::Value = response.json();
    assert_eq!(result["error"]["status"], "INVALID_ARGUMENT");
    assert_eq!(
        result["error"]["details"][0]["reason"],
        "CONTENT_TYPE_NOT_SUPPORTED"
    );
    assert!(
        !serde_json::to_string(&result)
            .expect("serialize body")
            .contains("ping over rest"),
        "the caller's own text must never come back as the agent reply: {result:?}"
    );
}

#[tokio::test]
async fn test_rest_get_task_not_found_is_404() {
    let resources = create_a2a_test_resources().await;
    let (_user, jwt) = common::create_test_tenant(&resources, "a2a-404@example.com")
        .await
        .expect("seed user + tenant + JWT with active_tenant_id");
    let routes = a2a_router_from(&resources);

    let response = AxumTestRequest::get("/a2a/tasks/task_missing?A2A-Version=1.0")
        .header("Authorization", &format!("Bearer {jwt}"))
        .send(routes)
        .await;

    assert_eq!(response.status(), 404);
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["status"], "NOT_FOUND");
    assert_eq!(body["error"]["details"][0]["reason"], "TASK_NOT_FOUND");
}

#[tokio::test]
async fn test_rest_extended_agent_card_requires_auth() {
    let routes = a2a_routes().await;

    let response = AxumTestRequest::get("/a2a/extendedAgentCard?A2A-Version=1.0")
        .send(routes)
        .await;

    assert_eq!(response.status(), 401);
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["status"], "UNAUTHENTICATED");
}

#[tokio::test]
async fn test_rest_unknown_task_action_is_404() {
    let routes = a2a_routes().await;

    let response = AxumTestRequest::post("/a2a/tasks/task_1:explode?A2A-Version=1.0")
        .json(&serde_json::json!({}))
        .send(routes)
        .await;

    assert_eq!(response.status(), 404);
}

// ============================================================================
// Client-credentials principals + credential management
// ============================================================================

/// Register an A2A client for the seeded user and mint a client-credentials
/// JWT for it (the `client:{id}` subject shape `/a2a/auth` issues).
async fn register_client_and_mint_token(
    resources: &Arc<ServerContext>,
    user_id: uuid::Uuid,
) -> (String, String) {
    let registration = pierre_a2a::ClientRegistrationRequest {
        name: format!("cc-client-{user_id}"),
        description: "client-credentials test client".into(),
        capabilities: vec!["fitness-data-analysis".into()],
        redirect_uris: vec![],
        contact_email: "cc@example.com".into(),
    };
    let credentials = resources
        .a2a
        .a2a_client_manager
        .register_client(registration, user_id)
        .await
        .expect("register A2A client");

    let token = resources
        .auth
        .auth_manager
        .generate_client_credentials_token(
            &resources.auth.jwks_manager,
            &credentials.client_id,
            &["read_activities".to_owned()],
            None,
        )
        .expect("mint client-credentials token");

    (credentials.client_id, token)
}

#[tokio::test]
async fn test_client_credentials_token_authenticates_protocol_methods() {
    let resources = create_a2a_test_resources().await;
    let (user, _jwt) = common::create_test_tenant(&resources, "a2a-cc@example.com")
        .await
        .expect("seed user + tenant");
    let (client_id, token) = register_client_and_mint_token(&resources, user.id).await;
    let routes = a2a_router_from(&resources);

    // SendMessage authenticates with the client token: the request reaches
    // the protocol handler, which refuses it on content grounds rather than
    // on authentication grounds.
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": "m-cc-1",
                "role": "ROLE_USER",
                "parts": [{ "text": "hello from a machine" }]
            }
        },
        "id": 21
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("Authorization", &format!("Bearer {token}"))
        .json(&body)
        .send(routes.clone())
        .await;

    assert_eq!(response.status(), 200);
    let envelope: serde_json::Value = response.json();
    assert_eq!(
        envelope["error"]["code"], -32005,
        "client-credentials SendMessage must pass auth and fail on content: {envelope:?}"
    );
    assert_eq!(
        envelope["error"]["data"][0]["reason"],
        "CONTENT_TYPE_NOT_SUPPORTED"
    );

    // ListTasks under the client token is scoped to the acting client.
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "ListTasks",
        "params": {},
        "id": 22
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("Authorization", &format!("Bearer {token}"))
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);
    let envelope: serde_json::Value = response.json();
    assert!(
        envelope["error"].is_null(),
        "client-credentials ListTasks must not error: {envelope:?}"
    );
    assert!(envelope["result"]["tasks"].is_array());
    let _ = client_id;
}

#[tokio::test]
async fn test_client_credentials_tasks_keyed_to_acting_client() {
    let resources = create_a2a_test_resources().await;
    let (user, _jwt) = common::create_test_tenant(&resources, "a2a-cc-key@example.com")
        .await
        .expect("seed user + tenant");
    let (client_id, token) = register_client_and_mint_token(&resources, user.id).await;
    let routes = a2a_router_from(&resources);

    // A tool-intent message creates a task; the unknown tool fails it, but
    // the task row exists and must be keyed to the acting client.
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": "m-cc-2",
                "role": "ROLE_USER",
                "parts": [{ "data": { "tool_name": "definitely_not_a_tool", "parameters": {} } }]
            }
        },
        "id": 23
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("Authorization", &format!("Bearer {token}"))
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);
    let envelope: serde_json::Value = response.json();
    let task = &envelope["result"]["task"];
    assert_eq!(task["status"]["state"], "TASK_STATE_FAILED");

    // The stored record is keyed to the acting client, not a fallback.
    let task_id = task["id"].as_str().expect("task id");
    let record = pierre_runtime_context::A2ACtx::repos(resources.as_ref())
        .a2a
        .get_task(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(record.client_id, client_id);
}

#[tokio::test]
async fn test_plain_text_continuation_fails_the_task_without_echoing() {
    // A follow-up message on a live task that names no tool has nothing to
    // execute. The task must terminate as FAILED carrying the accepted
    // shape — never as COMPLETED carrying the caller's own words.
    let resources = create_a2a_test_resources().await;
    let (user, _jwt) = common::create_test_tenant(&resources, "a2a-continuation@example.com")
        .await
        .expect("seed user + tenant");
    let (client_id, token) = register_client_and_mint_token(&resources, user.id).await;
    let routes = a2a_router_from(&resources);

    // Seed a live (non-terminal) task directly so the continuation path is
    // reached deterministically.
    let context_id = "ctx-continuation";
    let task_id = pierre_runtime_context::A2ACtx::repos(resources.as_ref())
        .a2a
        .create_task(
            &client_id,
            None,
            "message",
            &serde_json::json!({ "seed": true }),
            Some(context_id),
        )
        .await
        .expect("seed a live task");

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": "m-continuation-1",
                "role": "ROLE_USER",
                "taskId": task_id,
                "parts": [{ "text": "analyze my last week of training" }]
            }
        },
        "id": 31
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("Authorization", &format!("Bearer {token}"))
        .json(&body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);
    let envelope: serde_json::Value = response.json();
    let task = &envelope["result"]["task"];
    assert_eq!(
        task["status"]["state"], "TASK_STATE_FAILED",
        "a task that executed nothing must not report completion: {envelope:?}"
    );

    let status_text = task["status"]["message"]["parts"][0]["text"]
        .as_str()
        .expect("failed status carries an explanatory agent message");
    assert!(
        status_text.contains("tool_name"),
        "the failure must name the accepted shape, got: {status_text}"
    );
    assert_ne!(
        status_text, "analyze my last week of training",
        "the agent reply must never be the caller's own text"
    );

    // The persisted record agrees with the wire snapshot.
    let record = pierre_runtime_context::A2ACtx::repos(resources.as_ref())
        .a2a
        .get_task(&task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert!(
        record.status.is_terminal(),
        "the task must reach a terminal state"
    );
}

#[tokio::test]
async fn test_a2a_credentials_endpoint_stores_oauth_apps() {
    let resources = create_a2a_test_resources().await;
    let (_user, jwt) = common::create_test_tenant(&resources, "a2a-creds@example.com")
        .await
        .expect("seed user + tenant + JWT");
    let routes = a2a_router_from(&resources);

    // Unauthenticated requests are rejected.
    let body = serde_json::json!({
        "credentials": {
            "strava": { "clientId": "app-id-1", "clientSecret": "app-secret-1" }
        }
    });
    let response = AxumTestRequest::post("/a2a/credentials")
        .json(&body)
        .send(routes.clone())
        .await;
    assert_eq!(response.status(), 401);

    // Authenticated requests store the per-user OAuth app credentials.
    let response = AxumTestRequest::post("/a2a/credentials")
        .header("Authorization", &format!("Bearer {jwt}"))
        .json(&body)
        .send(routes.clone())
        .await;
    assert_eq!(response.status(), 200);
    let stored: serde_json::Value = response.json();
    assert_eq!(stored["stored"][0], "strava");

    // An empty map is invalid input.
    let response = AxumTestRequest::post("/a2a/credentials")
        .header("Authorization", &format!("Bearer {jwt}"))
        .json(&serde_json::json!({ "credentials": {} }))
        .send(routes)
        .await;
    assert_eq!(response.status(), 400);
}
