// ABOUTME: An in-app SSE turn is tracked by the shutdown drain and, when the drain gives up on it, closes the conversation honestly
// ABOUTME: Drives the chat route with a hung provider, drains, and asserts the localized interrupted notice landed as the reply
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The web/mobile turn ran in a bare spawn: once the athlete hung up the
//! stream, a deploy could cut it and the conversation kept a question with
//! no answer and no marker (carnet#463). Now the turn is spawned under
//! `InFlightTurns`, so the drain sees it, and a turn the drain still has
//! to give up on writes the same interrupted notice the messaging path
//! closes a placeholder with.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod common;
mod helpers;

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_TURN_INTERRUPTED};
use pierre_core::llm::LlmProvider;
use pierre_core::models::{TenantId, User};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use serde_json::json;

use common::{create_test_server_resources_with_chat_provider, create_test_tenant};
use helpers::axum_test::AxumTestRequest;
use helpers::drained_turn::{wait_for_a_tracked_turn, HangingProvider};

#[tokio::test]
async fn a_drained_sse_turn_is_tracked_and_leaves_the_interrupted_notice() {
    let provider: Arc<dyn LlmProvider> = Arc::new(HangingProvider);
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .expect("resources");
    let (user, token) = create_test_tenant(&resources, "drained-sse@test.local")
        .await
        .expect("seed user + tenant");
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let created = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({"title": "drained", "model": "gemini-1.5-flash"}))
        .send(router.clone())
        .await;
    assert_eq!(created.status_code(), StatusCode::CREATED);
    let conversation: serde_json::Value = created.json();
    let conversation_id = conversation["id"].as_str().expect("id").to_owned();

    // The athlete asks over SSE and hangs up: only the headers are read, the
    // stream is dropped, and the turn keeps running on a hung provider.
    let response = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{conversation_id}/messages"
    ))
    .header("authorization", &format!("Bearer {token}"))
    .header("accept", "text/event-stream")
    .json(&json!({"content": "combien de watts sur ma dernière sortie ?"}))
    .send_sse(router)
    .await;
    assert_eq!(response.status_code(), StatusCode::OK, "the stream opened");

    assert!(
        wait_for_a_tracked_turn(&resources).await,
        "the SSE turn must be spawned under InFlightTurns so the drain can see it"
    );

    // SIGTERM, as the tracker sees it: a short grace, then the signal.
    let report = resources
        .common
        .turns
        .drain(Duration::from_millis(200), Duration::from_secs(10))
        .await;
    assert_eq!(
        report.signalled, 1,
        "the hung turn outlives the grace and is signalled"
    );
    assert_eq!(
        report.abandoned, 0,
        "the signalled turn closes inside its window; report: {report:?}"
    );

    let messages = resources
        .common
        .repos
        .chat
        .get_messages(
            &conversation_id,
            &user.id.to_string(),
            user_tenant(&resources, &user).await,
        )
        .await
        .unwrap();
    let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant"],
        "the question is on file and the drain closed it with one reply"
    );
    let notice = resources
        .mcp
        .messaging_strings_registry
        .get(KEY_TURN_INTERRUPTED, DEFAULT_LOCALE);
    let reply = &messages[1];
    assert_eq!(
        reply.content, notice,
        "the reply is the localized interrupted notice, not silence"
    );
    assert_eq!(reply.finish_reason.as_deref(), Some("interrupted"));
}

async fn user_tenant(resources: &ServerContext, user: &User) -> TenantId {
    resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()[0]
        .id
}
