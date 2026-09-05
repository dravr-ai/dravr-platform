// ABOUTME: Regression tests pinning A2A cross-user denial on every ownership-checked path
// ABOUTME: Covers GetTask, CancelTask, push-config CRUD, ListTasks scoping, client CRUD, extended card
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The A2A surface carries no `tenant_id` at the route or schema layer: its
//! isolation lives entirely in application code — `load_owned_task` calling
//! `verify_client_access` on the protocol side, and `client.user_id !=
//! user_id` on the client-management side. With no schema-level backstop,
//! a handler that forgets that call leaks silently.
//!
//! These tests pin the existing defence. Each ownership-checked path is
//! exercised twice against the same resource: once by the owner (who must
//! get through) and once by a second registered A2A user (who must be told
//! the resource does not exist). Asserting both halves is what makes the
//! denial meaningful — a path broken for everyone would otherwise read as
//! "isolated".

mod common;
mod helpers;

use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_a2a::{A2ARoutes, A2ARoutesState};
use pierre_runtime_context::A2ACtx;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use std::sync::Arc;

/// A2A spec error code for an unknown (or unreachable) task.
const TASK_NOT_FOUND_CODE: i64 = -32001;
/// JSON-RPC invalid-params, returned once ownership has been satisfied and
/// the referenced push-notification config turns out not to exist.
const INVALID_PARAMS_CODE: i64 = -32602;

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

/// One seeded A2A user: a JWT plus the id of the client its tasks key to.
struct A2AUser {
    jwt: String,
    client_id: String,
}

async fn seed_a2a_user(resources: &Arc<ServerContext>, email: &str) -> A2AUser {
    let (user, jwt) = common::create_test_tenant(resources, email)
        .await
        .expect("seed user + tenant + JWT with active_tenant_id");

    let registration = pierre_a2a::ClientRegistrationRequest {
        name: format!("client-for-{email}"),
        description: "cross-user isolation fixture".into(),
        capabilities: vec!["fitness-data-analysis".into()],
        redirect_uris: vec![],
        contact_email: email.to_owned(),
    };
    let credentials = resources
        .a2a
        .a2a_client_manager
        .register_client(registration, user.id)
        .await
        .expect("register A2A client");

    A2AUser {
        jwt,
        client_id: credentials.client_id,
    }
}

/// Seed a live (non-terminal) task keyed to `client_id`.
async fn seed_task(resources: &Arc<ServerContext>, client_id: &str, context_id: &str) -> String {
    A2ACtx::repos(resources.as_ref())
        .a2a
        .create_task(
            client_id,
            None,
            "message",
            &json!({ "seed": true }),
            Some(context_id),
        )
        .await
        .expect("seed a live A2A task")
}

/// Send a JSON-RPC method on the A2A binding as the bearer of `jwt`.
async fn rpc(routes: &axum::Router, jwt: &str, method: &str, params: Value) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("Authorization", &format!("Bearer {jwt}"))
        .json(&body)
        .send(routes.clone())
        .await;
    assert_eq!(
        response.status(),
        200,
        "{method} must answer on the JSON-RPC envelope"
    );
    response.json()
}

/// Assert an envelope is the spec's "task does not exist" denial — the
/// deliberate shape for a task the caller does not own, so ownership never
/// leaks task existence.
fn assert_denied(envelope: &Value, what: &str) {
    assert!(
        envelope["result"].is_null(),
        "{what} must not return a result to a non-owner: {envelope:?}"
    );
    assert_eq!(
        envelope["error"]["code"], TASK_NOT_FOUND_CODE,
        "{what} must deny a non-owner with TaskNotFound: {envelope:?}"
    );
    assert_eq!(
        envelope["error"]["data"][0]["reason"], "TASK_NOT_FOUND",
        "{what} denial must carry the spec reason: {envelope:?}"
    );
}

#[tokio::test]
async fn test_task_paths_deny_a_second_user() {
    let resources = create_a2a_test_resources().await;
    let owner = seed_a2a_user(&resources, "a2a-owner@example.com").await;
    let stranger = seed_a2a_user(&resources, "a2a-stranger@example.com").await;
    let routes = a2a_router_from(&resources);

    let task_id = seed_task(&resources, &owner.client_id, "ctx-owner").await;

    // GetTask: the owner reads the task, the stranger is told it is absent.
    let envelope = rpc(&routes, &owner.jwt, "GetTask", json!({ "id": task_id })).await;
    assert_eq!(
        envelope["result"]["id"], task_id,
        "the owner must be able to read their own task: {envelope:?}"
    );
    let envelope = rpc(&routes, &stranger.jwt, "GetTask", json!({ "id": task_id })).await;
    assert_denied(&envelope, "GetTask");

    // ListTasks is scoped to the caller's own clients.
    let envelope = rpc(&routes, &stranger.jwt, "ListTasks", json!({})).await;
    let listed = envelope["result"]["tasks"]
        .as_array()
        .expect("ListTasks returns an array");
    assert!(
        !listed.iter().any(|task| task["id"] == task_id.as_str()),
        "ListTasks leaked another user's task: {envelope:?}"
    );

    // Push-notification config CRUD: all four verbs gate on task ownership.
    let envelope = rpc(
        &routes,
        &stranger.jwt,
        "CreateTaskPushNotificationConfig",
        json!({ "taskId": task_id, "config": { "url": "https://example.com/hook" } }),
    )
    .await;
    assert_denied(&envelope, "CreateTaskPushNotificationConfig");

    let envelope = rpc(
        &routes,
        &stranger.jwt,
        "ListTaskPushNotificationConfigs",
        json!({ "taskId": task_id }),
    )
    .await;
    assert_denied(&envelope, "ListTaskPushNotificationConfigs");

    let envelope = rpc(
        &routes,
        &stranger.jwt,
        "GetTaskPushNotificationConfig",
        json!({ "taskId": task_id, "configId": "cfg-absent" }),
    )
    .await;
    assert_denied(&envelope, "GetTaskPushNotificationConfig");

    let envelope = rpc(
        &routes,
        &stranger.jwt,
        "DeleteTaskPushNotificationConfig",
        json!({ "taskId": task_id, "configId": "cfg-absent" }),
    )
    .await;
    assert_denied(&envelope, "DeleteTaskPushNotificationConfig");

    // The owner clears the same ownership gate on those verbs: listing
    // succeeds, and get/delete of an absent config report the config
    // missing rather than the task missing.
    let envelope = rpc(
        &routes,
        &owner.jwt,
        "ListTaskPushNotificationConfigs",
        json!({ "taskId": task_id }),
    )
    .await;
    assert!(
        envelope["result"]["configs"]
            .as_array()
            .expect("configs array")
            .is_empty(),
        "the owner must reach the config list: {envelope:?}"
    );

    let envelope = rpc(
        &routes,
        &owner.jwt,
        "GetTaskPushNotificationConfig",
        json!({ "taskId": task_id, "configId": "cfg-absent" }),
    )
    .await;
    assert_eq!(
        envelope["error"]["code"], INVALID_PARAMS_CODE,
        "the owner must clear the ownership gate and fail on the config id: {envelope:?}"
    );

    // CancelTask: denied for the stranger, honoured for the owner. Running
    // the stranger first proves the task was still cancelable at the time.
    let envelope = rpc(
        &routes,
        &stranger.jwt,
        "CancelTask",
        json!({ "id": task_id }),
    )
    .await;
    assert_denied(&envelope, "CancelTask");

    let envelope = rpc(&routes, &owner.jwt, "CancelTask", json!({ "id": task_id })).await;
    assert_eq!(
        envelope["result"]["status"]["state"], "TASK_STATE_CANCELED",
        "the owner must be able to cancel their own task: {envelope:?}"
    );

    // The stranger's denials changed nothing about the stored task.
    let record = A2ACtx::repos(resources.as_ref())
        .a2a
        .get_task(&task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(record.client_id, owner.client_id);
}

#[tokio::test]
async fn test_rest_task_paths_deny_a_second_user() {
    // The HTTP+JSON binding reaches the same handlers; its denial surfaces
    // as google.rpc NOT_FOUND rather than a JSON-RPC error object.
    let resources = create_a2a_test_resources().await;
    let owner = seed_a2a_user(&resources, "a2a-rest-owner@example.com").await;
    let stranger = seed_a2a_user(&resources, "a2a-rest-stranger@example.com").await;
    let routes = a2a_router_from(&resources);

    let task_id = seed_task(&resources, &owner.client_id, "ctx-rest").await;

    let response = AxumTestRequest::get(&format!("/a2a/tasks/{task_id}?A2A-Version=1.0"))
        .header("Authorization", &format!("Bearer {}", owner.jwt))
        .send(routes.clone())
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(body["id"], task_id);

    let response = AxumTestRequest::get(&format!("/a2a/tasks/{task_id}?A2A-Version=1.0"))
        .header("Authorization", &format!("Bearer {}", stranger.jwt))
        .send(routes.clone())
        .await;
    assert_eq!(response.status(), 404);
    let body: Value = response.json();
    assert_eq!(body["error"]["status"], "NOT_FOUND");
    assert_eq!(body["error"]["details"][0]["reason"], "TASK_NOT_FOUND");

    let response = AxumTestRequest::get(&format!(
        "/a2a/tasks/{task_id}/pushNotificationConfigs?A2A-Version=1.0"
    ))
    .header("Authorization", &format!("Bearer {}", stranger.jwt))
    .send(routes.clone())
    .await;
    assert_eq!(response.status(), 404);
    let body: Value = response.json();
    assert_eq!(body["error"]["details"][0]["reason"], "TASK_NOT_FOUND");

    let response = AxumTestRequest::delete(&format!(
        "/a2a/tasks/{task_id}/pushNotificationConfigs/cfg-absent?A2A-Version=1.0"
    ))
    .header("Authorization", &format!("Bearer {}", stranger.jwt))
    .send(routes)
    .await;
    assert_eq!(response.status(), 404);
    let body: Value = response.json();
    assert_eq!(body["error"]["details"][0]["reason"], "TASK_NOT_FOUND");
}

#[tokio::test]
async fn test_client_management_paths_deny_a_second_user() {
    // The client-management surface enforces ownership with an explicit
    // `client.user_id != user_id` comparison in every handler that takes a
    // client id from the path. All four must answer a stranger with 404.
    let resources = create_a2a_test_resources().await;
    let owner = seed_a2a_user(&resources, "a2a-client-owner@example.com").await;
    let stranger = seed_a2a_user(&resources, "a2a-client-stranger@example.com").await;
    let routes = a2a_router_from(&resources);

    let client_id = owner.client_id.as_str();

    for path in [
        format!("/a2a/clients/{client_id}"),
        format!("/a2a/clients/{client_id}/usage"),
        format!("/a2a/clients/{client_id}/rate-limit"),
    ] {
        let response = AxumTestRequest::get(&path)
            .header("Authorization", &format!("Bearer {}", owner.jwt))
            .send(routes.clone())
            .await;
        assert_eq!(response.status(), 200, "the owner must reach {path}");

        let response = AxumTestRequest::get(&path)
            .header("Authorization", &format!("Bearer {}", stranger.jwt))
            .send(routes.clone())
            .await;
        assert_eq!(
            response.status(),
            404,
            "{path} must not be readable by another user"
        );
    }

    // Listing is scoped to the caller: the stranger sees only its own client.
    let response = AxumTestRequest::get("/a2a/clients")
        .header("Authorization", &format!("Bearer {}", stranger.jwt))
        .send(routes.clone())
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    let listed = body.as_array().expect("client list is a JSON array");
    assert!(
        !listed.iter().any(|client| client["id"] == client_id),
        "the client list leaked another user's client: {body:?}"
    );
    assert_eq!(
        listed.len(),
        1,
        "the caller must see exactly its own client: {body:?}"
    );

    // Deactivation is the destructive one: a stranger must not reach it,
    // and the client must still be active afterwards.
    let response = AxumTestRequest::delete(&format!("/a2a/clients/{client_id}"))
        .header("Authorization", &format!("Bearer {}", stranger.jwt))
        .send(routes.clone())
        .await;
    assert_eq!(
        response.status(),
        404,
        "a stranger must not be able to deactivate another user's client"
    );

    let client = A2ACtx::repos(resources.as_ref())
        .a2a
        .get_client(client_id)
        .await
        .expect("load client")
        .expect("client exists");
    assert!(
        client.is_active,
        "the refused delete must not have deactivated the client"
    );
}

#[tokio::test]
async fn test_extended_agent_card_requires_authentication() {
    // The extended card is a per-server document, not per-user data, so the
    // gate on it is authentication rather than ownership: anonymous callers
    // are refused, and any authenticated user gets the same card.
    let resources = create_a2a_test_resources().await;
    let user = seed_a2a_user(&resources, "a2a-card@example.com").await;
    let routes = a2a_router_from(&resources);

    let response = AxumTestRequest::get("/a2a/extendedAgentCard?A2A-Version=1.0")
        .send(routes.clone())
        .await;
    assert_eq!(response.status(), 401);
    let body: Value = response.json();
    assert_eq!(body["error"]["status"], "UNAUTHENTICATED");

    let response = AxumTestRequest::get("/a2a/extendedAgentCard?A2A-Version=1.0")
        .header("Authorization", &format!("Bearer {}", user.jwt))
        .send(routes)
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(body["name"], "Dravr AI");
    assert!(
        !body["skills"]
            .as_array()
            .expect("extended card lists skills")
            .is_empty(),
        "the extended card must carry the live tool registry as skills"
    );
}
