// ABOUTME: Integration tests for messaging route handlers (Slack webhooks, connections, bindings)
// ABOUTME: Tests webhook verification, connection CRUD, channel binding CRUD, and auth enforcement
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![cfg(feature = "messaging-slack")]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_database::plugins::ChatRepository;
use pierre_mcp_server::mcp::resources::ServerResources;
use pierre_mcp_server::models::TenantId;
use pierre_mcp_server::routes::messaging::messaging_routes;
use serde_json::json;
use uuid::Uuid;

// ============================================================================
// Test Helpers
// ============================================================================

/// Test environment with router, auth token, and access to server resources.
/// The default user is an admin so write operations (create/delete) succeed.
struct TestEnv {
    router: Router,
    auth_token: String,
    resources: Arc<ServerResources>,
    user_id: Uuid,
}

/// Create a test router with messaging routes nested under `/api/messaging`
/// and generate an auth token for an **admin** test user.
async fn setup_test_environment() -> TestEnv {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.database).await.unwrap();

    // Promote the test user to admin so write endpoints pass the admin guard
    let pool = resources.database.sqlite_pool().unwrap();
    sqlx::query("UPDATE users SET role = 'admin', is_admin = 1 WHERE id = $1")
        .bind(user_id.to_string())
        .execute(pool)
        .await
        .unwrap();

    let token = generate_test_token(&resources, &user).await;

    let router = Router::new().nest("/api/messaging", messaging_routes(Arc::clone(&resources)));

    TestEnv {
        router,
        auth_token: format!("Bearer {token}"),
        resources,
        user_id,
    }
}

/// Create a test environment with a **non-admin** (regular user) for negative auth tests.
async fn setup_non_admin_test_environment() -> TestEnv {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.database).await.unwrap();

    // User::new defaults to UserRole::User, no promotion needed
    let token = generate_test_token(&resources, &user).await;

    let router = Router::new().nest("/api/messaging", messaging_routes(Arc::clone(&resources)));

    TestEnv {
        router,
        auth_token: format!("Bearer {token}"),
        resources,
        user_id,
    }
}

/// Create a messaging connection via the API and return its ID.
async fn create_test_connection(router: &Router, auth_token: &str) -> String {
    let response = AxumTestRequest::post("/api/messaging/connections")
        .header("authorization", auth_token)
        .json(&json!({
            "provider": "slack",
            "team_id": "T_TEST_TEAM",
            "team_name": "Test Workspace",
            "bot_token": "xoxb-test-token-value",
            "signing_secret": "test-signing-secret"
        }))
        .send(router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    body["id"].as_str().unwrap().to_owned()
}

/// Create a chat conversation in the database and return its ID.
/// Needed because `channel_bindings` has a FK to `chat_conversations`.
async fn create_test_conversation(env: &TestEnv) -> String {
    let tenant_id = TenantId::from(env.user_id);
    let record = env
        .resources
        .database
        .create_conversation(
            &env.user_id.to_string(),
            tenant_id,
            "Test conversation for binding",
            "test-model",
            None,
        )
        .await
        .unwrap();
    record.id
}

// ============================================================================
// Slack Webhook Tests
// ============================================================================

#[tokio::test]
async fn test_slack_url_verification() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/messaging/webhooks/slack")
        .json(&json!({
            "type": "url_verification",
            "challenge": "abc123challenge",
            "token": "Jhj5dZrVaK7ZwHHjRyZWjbDl"
        }))
        .send(env.router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert_eq!(body["challenge"], "abc123challenge");
}

#[tokio::test]
async fn test_slack_webhook_invalid_payload() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/messaging/webhooks/slack")
        .header("content-type", "application/json")
        .send(env.router)
        .await;

    // Empty body should fail JSON parsing with a 4xx error
    let status = response.status();
    assert!(
        (400..500).contains(&status),
        "Invalid payload should return client error, got {status}"
    );
}

#[tokio::test]
async fn test_slack_webhook_unknown_team() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/messaging/webhooks/slack")
        .json(&json!({
            "type": "event_callback",
            "team_id": "T_NONEXISTENT",
            "event": {
                "type": "message",
                "text": "hello",
                "channel": "C123",
                "ts": "1234567890.123456"
            },
            "event_id": "Ev123",
            "event_time": 1_234_567_890
        }))
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::NOT_FOUND,
        "Unknown team should return 404"
    );
}

#[tokio::test]
async fn test_slack_webhook_unsupported_type() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/messaging/webhooks/slack")
        .json(&json!({
            "type": "app_rate_limited",
            "team_id": "T123"
        }))
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "Unsupported event type should return 400"
    );
}

// ============================================================================
// Connection CRUD Tests
// ============================================================================

#[tokio::test]
async fn test_list_connections_empty() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/messaging/connections")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: Vec<serde_json::Value> = response.json();
    assert!(body.is_empty(), "new tenant should have no connections");
}

#[tokio::test]
async fn test_create_connection() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/messaging/connections")
        .header("authorization", &env.auth_token)
        .json(&json!({
            "provider": "slack",
            "team_id": "T_MY_TEAM",
            "team_name": "My Workspace",
            "bot_token": "xoxb-bot-token-123",
            "signing_secret": "signing-secret-abc"
        }))
        .send(env.router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert_eq!(body["provider"], "slack");
    assert_eq!(body["team_id"], "T_MY_TEAM");
    assert_eq!(body["team_name"], "My Workspace");
    assert!(body["id"].as_str().is_some(), "response should include id");
    assert!(
        body["created_at"].as_str().is_some(),
        "response should include created_at"
    );
}

#[tokio::test]
async fn test_create_connection_unsupported_provider() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/messaging/connections")
        .header("authorization", &env.auth_token)
        .json(&json!({
            "provider": "irc",
            "team_id": "T_MY_TEAM",
            "bot_token": "token",
            "signing_secret": "secret"
        }))
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "unsupported provider should return 400"
    );
}

#[tokio::test]
async fn test_list_connections_after_create() {
    let env = setup_test_environment().await;

    // Create a connection
    create_test_connection(&env.router, &env.auth_token).await;

    // List connections
    let response = AxumTestRequest::get("/api/messaging/connections")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: Vec<serde_json::Value> = response.json();
    assert_eq!(body.len(), 1, "should have exactly one connection");
    assert_eq!(body[0]["provider"], "slack");
    assert_eq!(body[0]["team_id"], "T_TEST_TEAM");
}

#[tokio::test]
async fn test_delete_connection() {
    let env = setup_test_environment().await;

    let connection_id = create_test_connection(&env.router, &env.auth_token).await;

    let response = AxumTestRequest::delete(&format!("/api/messaging/connections/{connection_id}"))
        .header("authorization", &env.auth_token)
        .send(env.router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert_eq!(body["deleted"], true);

    // Verify it's gone
    let list_response = AxumTestRequest::get("/api/messaging/connections")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    let connections: Vec<serde_json::Value> = list_response.json();
    assert!(connections.is_empty(), "connection should be deleted");
}

#[tokio::test]
async fn test_delete_connection_not_found() {
    let env = setup_test_environment().await;

    let response =
        AxumTestRequest::delete("/api/messaging/connections/nonexistent-connection-id-999")
            .header("authorization", &env.auth_token)
            .send(env.router)
            .await;

    assert_eq!(
        response.status_code(),
        StatusCode::NOT_FOUND,
        "deleting nonexistent connection should return 404"
    );
}

#[tokio::test]
async fn test_connections_require_auth_get() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/messaging/connections")
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::UNAUTHORIZED,
        "GET /connections without auth should return 401"
    );
}

#[tokio::test]
async fn test_connections_require_auth_post() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/messaging/connections")
        .json(&json!({
            "provider": "slack",
            "team_id": "T_MY_TEAM",
            "bot_token": "xoxb-token",
            "signing_secret": "secret"
        }))
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::UNAUTHORIZED,
        "POST /connections without auth should return 401"
    );
}

// ============================================================================
// Channel Binding CRUD Tests
// ============================================================================

#[tokio::test]
async fn test_list_bindings_empty() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/messaging/bindings")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: Vec<serde_json::Value> = response.json();
    assert!(body.is_empty(), "new tenant should have no bindings");
}

#[tokio::test]
async fn test_create_binding() {
    let env = setup_test_environment().await;

    // Create prerequisites: a connection and a conversation (FK target)
    let connection_id = create_test_connection(&env.router, &env.auth_token).await;
    let conversation_id = create_test_conversation(&env).await;

    let response = AxumTestRequest::post("/api/messaging/bindings")
        .header("authorization", &env.auth_token)
        .json(&json!({
            "messaging_connection_id": connection_id,
            "channel_id": "C_TEST_CHANNEL",
            "channel_name": "general",
            "conversation_id": conversation_id
        }))
        .send(env.router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert_eq!(body["messaging_connection_id"], connection_id);
    assert_eq!(body["channel_id"], "C_TEST_CHANNEL");
    assert_eq!(body["channel_name"], "general");
    assert_eq!(body["conversation_id"], conversation_id);
    assert_eq!(body["active"], true);
    assert!(body["id"].as_str().is_some(), "response should include id");
}

#[tokio::test]
async fn test_create_binding_invalid_connection() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/messaging/bindings")
        .header("authorization", &env.auth_token)
        .json(&json!({
            "messaging_connection_id": "nonexistent-connection-id",
            "channel_id": "C_TEST",
            "conversation_id": "conv-123"
        }))
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::NOT_FOUND,
        "binding to nonexistent connection should return 404"
    );
}

#[tokio::test]
async fn test_list_bindings_after_create() {
    let env = setup_test_environment().await;

    let connection_id = create_test_connection(&env.router, &env.auth_token).await;
    let conversation_id = create_test_conversation(&env).await;

    // Create a binding
    let create_resp = AxumTestRequest::post("/api/messaging/bindings")
        .header("authorization", &env.auth_token)
        .json(&json!({
            "messaging_connection_id": connection_id,
            "channel_id": "C_BOUND",
            "channel_name": "bound-channel",
            "conversation_id": conversation_id
        }))
        .send(env.router.clone())
        .await;
    assert_eq!(create_resp.status_code(), StatusCode::OK);

    // List bindings
    let response = AxumTestRequest::get("/api/messaging/bindings")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: Vec<serde_json::Value> = response.json();
    assert_eq!(body.len(), 1, "should have exactly one binding");
    assert_eq!(body[0]["channel_id"], "C_BOUND");
}

#[tokio::test]
async fn test_delete_binding() {
    let env = setup_test_environment().await;

    let connection_id = create_test_connection(&env.router, &env.auth_token).await;
    let conversation_id = create_test_conversation(&env).await;

    // Create a binding
    let create_response = AxumTestRequest::post("/api/messaging/bindings")
        .header("authorization", &env.auth_token)
        .json(&json!({
            "messaging_connection_id": connection_id,
            "channel_id": "C_DELETE_ME",
            "conversation_id": conversation_id
        }))
        .send(env.router.clone())
        .await;
    assert_eq!(create_response.status_code(), StatusCode::OK);
    let binding: serde_json::Value = create_response.json();
    let binding_id = binding["id"].as_str().unwrap();

    // Delete the binding
    let response = AxumTestRequest::delete(&format!("/api/messaging/bindings/{binding_id}"))
        .header("authorization", &env.auth_token)
        .send(env.router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert_eq!(body["deleted"], true);

    // Verify it's gone
    let list_response = AxumTestRequest::get("/api/messaging/bindings")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    let bindings: Vec<serde_json::Value> = list_response.json();
    assert!(bindings.is_empty(), "binding should be deleted");
}

#[tokio::test]
async fn test_delete_binding_not_found() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::delete("/api/messaging/bindings/nonexistent-binding-id-999")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::NOT_FOUND,
        "deleting nonexistent binding should return 404"
    );
}

#[tokio::test]
async fn test_bindings_require_auth() {
    let env = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/messaging/bindings")
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::UNAUTHORIZED,
        "GET /bindings without auth should return 401"
    );
}

// ============================================================================
// Admin Role Enforcement Tests
// ============================================================================

#[tokio::test]
async fn test_create_connection_requires_admin() {
    let env = setup_non_admin_test_environment().await;

    let response = AxumTestRequest::post("/api/messaging/connections")
        .header("authorization", &env.auth_token)
        .json(&json!({
            "provider": "slack",
            "team_id": "T_MY_TEAM",
            "bot_token": "xoxb-token",
            "signing_secret": "secret"
        }))
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::FORBIDDEN,
        "non-admin should be rejected with 403 on POST /connections"
    );
}

#[tokio::test]
async fn test_delete_connection_requires_admin() {
    let env = setup_non_admin_test_environment().await;

    let response = AxumTestRequest::delete("/api/messaging/connections/some-connection-id")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::FORBIDDEN,
        "non-admin should be rejected with 403 on DELETE /connections/:id"
    );
}

#[tokio::test]
async fn test_create_binding_requires_admin() {
    let env = setup_non_admin_test_environment().await;

    let response = AxumTestRequest::post("/api/messaging/bindings")
        .header("authorization", &env.auth_token)
        .json(&json!({
            "messaging_connection_id": "conn-id",
            "channel_id": "C_TEST",
            "conversation_id": "conv-123"
        }))
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::FORBIDDEN,
        "non-admin should be rejected with 403 on POST /bindings"
    );
}

#[tokio::test]
async fn test_delete_binding_requires_admin() {
    let env = setup_non_admin_test_environment().await;

    let response = AxumTestRequest::delete("/api/messaging/bindings/some-binding-id")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::FORBIDDEN,
        "non-admin should be rejected with 403 on DELETE /bindings/:id"
    );
}

#[tokio::test]
async fn test_list_connections_allowed_for_non_admin() {
    let env = setup_non_admin_test_environment().await;

    let response = AxumTestRequest::get("/api/messaging/connections")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "non-admin should be able to list connections"
    );
}

#[tokio::test]
async fn test_list_bindings_allowed_for_non_admin() {
    let env = setup_non_admin_test_environment().await;

    let response = AxumTestRequest::get("/api/messaging/bindings")
        .header("authorization", &env.auth_token)
        .send(env.router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "non-admin should be able to list bindings"
    );
}
