// ABOUTME: The system prompt must state "no connected provider", not fall silent, when there is none
// ABOUTME: Phase 5 prompt half — silence is what the model misread as "no recent activity"
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Telling the model what it cannot infer
//!
//! `build_provider_context` used to return an empty string for a user with no
//! connections, and the caller appends nothing when it is empty. So the prompt
//! for someone who had connected nothing was *identical* to the prompt for
//! someone whose providers were connected but quiet.
//!
//! The model cannot distinguish those, and the failure mode is not silence but
//! invention — the incident recorded in `pierre_services::onboarding_gate` is a
//! cheerful *"nice 12 km ride yesterday!"* to a user who had never connected
//! anything. We diagnosed that correctly and then blocked the door instead of
//! turning on the light.
//!
//! These tests assert the light is on: the providerless prompt names the
//! absence, forbids specifics, and still says what the coach *can* do. They
//! assert content rather than non-emptiness, because a section that merely
//! exists would satisfy the second and still leave the model guessing.

mod common;
mod helpers;

use helpers::axum_test::AxumTestRequest;
use serde_json::json;

use pierre_chat_pipeline::stages::prompt_builder::build_provider_context;

/// A user with nothing connected gets an explicit statement of that.
///
/// The three assertions are the three jobs the block has to do, and they fail
/// independently: name the absence, ban invented specifics, and offer the
/// honest alternative.
#[tokio::test]
async fn a_providerless_user_is_named_as_such_in_the_prompt() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user, _token) = common::create_test_tenant(&resources, "providerless@test.local")
        .await
        .unwrap();

    let context = build_provider_context(&resources.data(), user.id).await;

    assert!(
        !context.is_empty(),
        "an empty section is the silence the model misreads as 'nothing recent'"
    );
    assert!(
        context.contains("has not connected any fitness data source"),
        "the prompt must name the absence outright, got: {context}"
    );
    assert!(
        context.contains("nothing at all"),
        "it must rule out the 'no recent activity' reading specifically, got: {context}"
    );
    assert!(
        context.contains("Never state or imply a specific figure"),
        "it must forbid invented specifics — the actual failure mode, got: {context}"
    );
    assert!(
        context.contains("cannot see their training yet"),
        "it must give the model the honest answer to use, not only a prohibition, got: {context}"
    );
}

/// A connected user's prompt is unchanged, and must not carry the denial.
///
/// The risk of stating absence is stating it to the wrong person: a coach that
/// tells a connected athlete it cannot see their data is a worse bug than the
/// one being fixed, and it would pass a test that only checked the empty case.
#[tokio::test]
async fn a_connected_user_never_sees_the_providerless_wording() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user, _token) =
        common::create_test_tenant_with_provider(&resources, "connected@test.local")
            .await
            .unwrap();

    let context = build_provider_context(&resources.data(), user.id).await;

    assert!(
        context.contains("strava"),
        "the connected provider must still be listed, got: {context}"
    );
    assert!(
        !context.contains("has not connected any fitness data source"),
        "a connected athlete must never be told their data is absent, got: {context}"
    );
    assert!(
        !context.contains("Unknown — the lookup failed"),
        "the lookup succeeded; the uncertainty wording belongs only to failures, got: {context}"
    );
}

/// The two blocks say different things on purpose.
///
/// A failed lookup is not evidence of absence. Reusing the providerless wording
/// there would have the prompt assert something we did not establish, so the
/// only claim shared by both is the ban on invented figures.
#[tokio::test]
async fn absence_and_uncertainty_are_not_the_same_statement() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user, _token) = common::create_test_tenant(&resources, "distinct@test.local")
        .await
        .unwrap();

    let providerless = build_provider_context(&resources.data(), user.id).await;

    // The providerless block asserts absence; it must not hedge it as unknown.
    assert!(
        !providerless.contains("Unknown"),
        "zero connections is a known fact, not an unknown one, got: {providerless}"
    );
    // Both blocks carry the prohibition, which is the invariant worth pinning.
    assert!(
        providerless.contains("specific figure"),
        "the ban on specifics is the part that must hold on every path"
    );
}

/// The door is open: a providerless athlete's message reaches the chat route
/// instead of being refused at it.
///
/// This assertion was impossible to write before Phase 5 — `send_message`
/// called `require_connected_provider` and returned 403 with
/// `details.action = "connect_provider"` before the handler did anything, so
/// no providerless request ever got further.
///
/// Deliberately asserts on the *absence of that specific refusal* rather than
/// on a successful turn: this harness has no LLM, so the request legitimately
/// fails later for want of a provider-backed model. What must never happen
/// again is a refusal on the grounds of having no fitness provider.
#[tokio::test]
async fn a_providerless_user_is_no_longer_refused_at_the_chat_route() {
    use axum::http::StatusCode;
    use pierre_mcp_server::routes::chat::ChatRoutes;

    let resources = common::create_test_server_resources().await.unwrap();
    let (user, token) = common::create_test_tenant(&resources, "open-door@test.local")
        .await
        .unwrap();

    // Sanity-check the premise: this user really has nothing connected, so a
    // pass here cannot be an accident of fixture state.
    let connections = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user.id, None)
        .await
        .unwrap();
    assert!(
        connections.is_empty(),
        "the fixture must be genuinely providerless for this test to mean anything"
    );

    let router = ChatRoutes::routes(resources);
    let created = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({"title": "providerless", "model": "gemini-1.5-flash"}))
        .send(router.clone())
        .await;
    assert_eq!(
        created.status_code(),
        StatusCode::CREATED,
        "creating a conversation was never gated"
    );
    let conversation: serde_json::Value = created.json();
    let conversation_id = conversation["id"].as_str().expect("conversation id");

    let sent = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{conversation_id}/messages"
    ))
    .header("authorization", &format!("Bearer {token}"))
    .json(&json!({"content": "How is my training going?"}))
    .send(router)
    .await;

    let status = sent.status_code();
    let body = sent.text();
    assert!(
        !body.contains("connect_provider"),
        "the provider refusal is gone; a providerless athlete must reach the coach. \
         status={status}, body={}",
        &body[..body.len().min(400)]
    );
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "403 was the gate's signature response, body={}",
        &body[..body.len().min(400)]
    );
}
