// ABOUTME: End-to-end tests for Slack messaging webhook pipeline with HMAC signature verification
// ABOUTME: Exercises webhook reception → signature verification → event routing → message persistence
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
use pierre_database::plugins::{ChatRepository, TenantRepository};
use pierre_mcp_server::mcp::resources::ServerResources;
use pierre_mcp_server::routes::messaging::messaging_routes;
use ring::hmac;
use serde_json::json;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

/// Test signing secret shared between the "Slack app" (test) and the server (connection record)
const TEST_SIGNING_SECRET: &str = "e2e-test-signing-secret-abc123";

/// Slack signing protocol version prefix
const SIGNING_VERSION: &str = "v0";

// ============================================================================
// Signature Helpers
// ============================================================================

/// Compute a Slack-compatible HMAC-SHA256 signature over a request body.
///
/// Mirrors the algorithm in `pierre_messaging::slack::signature::verify_slack_signature`:
/// `base_string = "v0:{timestamp}:{body}"`, then HMAC-SHA256 with the signing secret.
fn generate_slack_signature(signing_secret: &str, timestamp: &str, body: &str) -> String {
    let base_string = format!("{SIGNING_VERSION}:{timestamp}:{body}");
    let key = hmac::Key::new(hmac::HMAC_SHA256, signing_secret.as_bytes());
    let tag = hmac::sign(&key, base_string.as_bytes());
    format!("{SIGNING_VERSION}={}", hex::encode(tag.as_ref()))
}

/// Get the current Unix timestamp as a string
fn current_timestamp() -> String {
    chrono::Utc::now().timestamp().to_string()
}

// ============================================================================
// Test Environment Setup
// ============================================================================

/// E2E test environment with router, auth token, and pre-created connection
struct E2eEnv {
    router: Router,
    auth_token: String,
    resources: Arc<ServerResources>,
    user_id: Uuid,
    connection_id: String,
    team_id: String,
}

/// Set up a basic test environment with router, admin user, and a Slack connection.
/// Does NOT create a conversation or channel binding.
async fn setup_e2e_environment() -> E2eEnv {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.database).await.unwrap();

    // Promote user to admin
    let pool = resources.database.sqlite_pool().unwrap();
    sqlx::query("UPDATE users SET role = 'admin', is_admin = 1 WHERE id = $1")
        .bind(user_id.to_string())
        .execute(pool)
        .await
        .unwrap();

    let token = generate_test_token(&resources, &user).await;
    let router = Router::new().nest("/api/messaging", messaging_routes(Arc::clone(&resources)));

    // Create a Slack connection with the test signing secret
    let team_id = "T_E2E_TEAM".to_owned();
    let response = AxumTestRequest::post("/api/messaging/connections")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({
            "provider": "slack",
            "team_id": team_id,
            "team_name": "E2E Test Workspace",
            "bot_token": "xoxb-e2e-test-token",
            "signing_secret": TEST_SIGNING_SECRET
        }))
        .send(router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    let connection_id = body["id"].as_str().unwrap().to_owned();

    E2eEnv {
        router,
        auth_token: format!("Bearer {token}"),
        resources,
        user_id,
        connection_id,
        team_id,
    }
}

/// Set up a full pipeline: connection + conversation + channel binding.
/// Returns the environment plus the conversation ID and channel ID.
struct FullPipelineEnv {
    env: E2eEnv,
    conversation_id: String,
    channel_id: String,
}

async fn setup_full_pipeline() -> FullPipelineEnv {
    let env = setup_e2e_environment().await;

    // Resolve the actual tenant ID assigned to this user by create_test_user()
    let tenants = env
        .resources
        .database
        .list_for_user(env.user_id)
        .await
        .unwrap();
    let tenant_id = tenants.first().expect("test user must have a tenant").id;

    // Create a conversation using the real tenant ID
    let conv = env
        .resources
        .database
        .create_conversation(
            &env.user_id.to_string(),
            tenant_id,
            "E2E test conversation",
            "test-model",
            None,
        )
        .await
        .unwrap();

    // Create a channel binding linking the Slack channel to the conversation
    let channel_id = "C_E2E_CHANNEL".to_owned();
    let response = AxumTestRequest::post("/api/messaging/bindings")
        .header("authorization", &env.auth_token)
        .json(&json!({
            "messaging_connection_id": env.connection_id,
            "channel_id": channel_id,
            "channel_name": "e2e-test",
            "conversation_id": conv.id
        }))
        .send(env.router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    FullPipelineEnv {
        env,
        conversation_id: conv.id,
        channel_id,
    }
}

/// Build a valid Slack `event_callback` JSON body string for a message event.
fn build_event_callback_body(team_id: &str, channel_id: &str, text: &str) -> String {
    json!({
        "type": "event_callback",
        "team_id": team_id,
        "event": {
            "type": "message",
            "user": "U_E2E_USER",
            "text": text,
            "channel": channel_id,
            "ts": "1234567890.000001",
            "channel_type": "channel"
        },
        "event_id": "Ev_E2E_001",
        "event_time": 1_234_567_890
    })
    .to_string()
}

/// Build a valid Slack `event_callback` JSON body string for an `app_mention` event.
fn build_app_mention_body(team_id: &str, channel_id: &str, text: &str) -> String {
    json!({
        "type": "event_callback",
        "team_id": team_id,
        "event": {
            "type": "app_mention",
            "user": "U_E2E_USER",
            "text": text,
            "channel": channel_id,
            "ts": "1234567890.000002",
            "channel_type": "channel"
        },
        "event_id": "Ev_E2E_002",
        "event_time": 1_234_567_890
    })
    .to_string()
}

/// Build a Slack `event_callback` JSON body for a bot message (has `bot_id`).
fn build_bot_message_body(team_id: &str, channel_id: &str) -> String {
    json!({
        "type": "event_callback",
        "team_id": team_id,
        "event": {
            "type": "message",
            "bot_id": "B_E2E_BOT",
            "text": "I am a bot",
            "channel": channel_id,
            "ts": "1234567890.000003",
            "channel_type": "channel"
        },
        "event_id": "Ev_E2E_003",
        "event_time": 1_234_567_890
    })
    .to_string()
}

/// Send a signed Slack webhook request with the given body and signing secret.
async fn send_signed_webhook(
    router: &Router,
    body: &str,
    signing_secret: &str,
) -> helpers::axum_test::AxumTestResponse {
    let timestamp = current_timestamp();
    let signature = generate_slack_signature(signing_secret, &timestamp, body);

    AxumTestRequest::post("/api/messaging/webhooks/slack")
        .header("x-slack-signature", &signature)
        .header("x-slack-request-timestamp", &timestamp)
        .raw_body(body, "application/json")
        .send(router.clone())
        .await
}

// ============================================================================
// E2E Tests
// ============================================================================

#[tokio::test]
async fn test_e2e_valid_signature_accepted() {
    let pipeline = setup_full_pipeline().await;
    let body = build_event_callback_body(
        &pipeline.env.team_id,
        &pipeline.channel_id,
        "Hello from e2e test",
    );

    let response = send_signed_webhook(&pipeline.env.router, &body, TEST_SIGNING_SECRET).await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "Properly signed event_callback should be accepted"
    );
    let json: serde_json::Value = response.json();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn test_e2e_invalid_signature_rejected() {
    let pipeline = setup_full_pipeline().await;
    let body = build_event_callback_body(
        &pipeline.env.team_id,
        &pipeline.channel_id,
        "This should be rejected",
    );

    // Sign with the wrong secret
    let response =
        send_signed_webhook(&pipeline.env.router, &body, "wrong-signing-secret-xyz").await;

    assert_eq!(
        response.status_code(),
        StatusCode::UNAUTHORIZED,
        "Webhook signed with wrong secret should return 401"
    );
}

#[tokio::test]
async fn test_e2e_expired_timestamp_rejected() {
    let pipeline = setup_full_pipeline().await;
    let body = build_event_callback_body(
        &pipeline.env.team_id,
        &pipeline.channel_id,
        "This timestamp is stale",
    );

    // Use a timestamp 10 minutes in the past (beyond the 5-minute window)
    let old_timestamp = (chrono::Utc::now().timestamp() - 600).to_string();
    let signature = generate_slack_signature(TEST_SIGNING_SECRET, &old_timestamp, &body);

    let response = AxumTestRequest::post("/api/messaging/webhooks/slack")
        .header("x-slack-signature", &signature)
        .header("x-slack-request-timestamp", &old_timestamp)
        .raw_body(&body, "application/json")
        .send(pipeline.env.router.clone())
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "Webhook with expired timestamp should return 400"
    );
}

#[tokio::test]
async fn test_e2e_missing_signature_headers() {
    let pipeline = setup_full_pipeline().await;
    let body = build_event_callback_body(
        &pipeline.env.team_id,
        &pipeline.channel_id,
        "No signature headers",
    );

    // Send without x-slack-signature and x-slack-request-timestamp headers
    let response = AxumTestRequest::post("/api/messaging/webhooks/slack")
        .raw_body(&body, "application/json")
        .send(pipeline.env.router.clone())
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "Webhook without signature headers should return 400"
    );
}

#[tokio::test]
async fn test_e2e_bot_message_ignored() {
    let pipeline = setup_full_pipeline().await;
    let body = build_bot_message_body(&pipeline.env.team_id, &pipeline.channel_id);

    let response = send_signed_webhook(&pipeline.env.router, &body, TEST_SIGNING_SECRET).await;

    // Bot messages are accepted (200) but silently ignored (no bridge processing)
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "Bot messages should return 200 (accepted but ignored)"
    );
    let json: serde_json::Value = response.json();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn test_e2e_app_mention_accepted() {
    let pipeline = setup_full_pipeline().await;
    let body = build_app_mention_body(
        &pipeline.env.team_id,
        &pipeline.channel_id,
        "<@U_BOT> what is my training status?",
    );

    let response = send_signed_webhook(&pipeline.env.router, &body, TEST_SIGNING_SECRET).await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "app_mention events should be accepted"
    );
    let json: serde_json::Value = response.json();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn test_e2e_message_persisted_in_conversation() {
    let pipeline = setup_full_pipeline().await;
    let message_text = "E2E persistence test message";
    let body = build_event_callback_body(&pipeline.env.team_id, &pipeline.channel_id, message_text);

    let response = send_signed_webhook(&pipeline.env.router, &body, TEST_SIGNING_SECRET).await;
    assert_eq!(response.status_code(), StatusCode::OK);

    // The bridge runs in a tokio::spawn task, so poll until the message appears.
    // persist_user_message() is called BEFORE create_chat_provider(), so the user
    // message is saved even when LLM setup fails in the test environment.
    let mut found = false;
    for _ in 0..10 {
        sleep(Duration::from_millis(100)).await;

        let messages = pipeline
            .env
            .resources
            .database
            .get_messages(&pipeline.conversation_id, &pipeline.env.user_id.to_string())
            .await;

        if let Ok(msgs) = messages {
            if msgs.iter().any(|m| m.content.contains(message_text)) {
                found = true;
                break;
            }
        }
    }

    assert!(
        found,
        "User message should be persisted in the conversation within 1 second"
    );
}
