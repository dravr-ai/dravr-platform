// ABOUTME: How the athlete produced a message reaches the turn's analytics as the origin of question_asked and answer_delivered
// ABOUTME: Pins origin=next_step for a resolved next step, and that a client cannot claim one

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#830.
//!
//! A next step is what a reply hands the athlete as it ends — a guided walk's
//! wrap-up, a command's answer — and a tap on one is counted apart from a
//! typed question and from a welcome starter. The share of an athlete's turns
//! that were taps is read from `chat.question_asked.origin`, so a next step
//! reporting itself as `typed` would hide the very taps it exists to count.
//! Like a resolved starter, a next step is the server's finding: a client may
//! not claim one.

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::transport::Transport;
use serde_json::json;
use uuid::Uuid;

use common::{
    create_test_server_resources_with_chat_provider, create_test_user_with_plan,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named, only};
use helpers::recording_llm::RecordingProvider;
use pierre_chat_pipeline::{
    CommandPersistence, InputSource, PipelineHooks, ServedTurn, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnOrigin, TurnRequest,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;

const REPLY: &str = "Your next two weeks build on the long run you did on Sunday.";

struct Fixture {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    auth: String,
    conversation_id: String,
}

async fn setup(email: &str) -> Fixture {
    let provider = Arc::new(RecordingProvider::answering(REPLY));
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let (user_id, user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    let auth = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let conversation_id = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant_id,
            "Next steps",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    Fixture {
        resources,
        user_id,
        tenant_id,
        auth,
        conversation_id,
    }
}

/// The turn a resolved next step is: the athlete's own, tapped rather than
/// typed.
fn next_step_turn(fx: &Fixture, content: &str) -> TurnRequest<'static> {
    TurnRequest {
        origin: TurnOrigin::Athlete,
        input_source: InputSource::NextStep,
        conversation_id: fx.conversation_id.clone(),
        user_id: fx.user_id,
        conversation_tenant_id: fx.tenant_id,
        tool_tenant_id: fx.tenant_id,
        content: content.to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        channel_type: "web",
        transport: Transport::WebApp,
        is_direct_message: true,
        ambient_group_fallback: false,
        command_persistence: CommandPersistence::Always,
        sender_id: None,
        hooks: PipelineHooks::none(),
    }
}

fn web_profile() -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "en".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

#[tokio::test]
async fn a_resolved_next_step_is_counted_as_a_next_step_not_as_typing() {
    let fx = setup("next-step-origin@test.com").await;
    let ctx = fx.resources.chat_pipeline_context();
    let (events, _guard) = capture_notify();

    let served = pierre_chat_pipeline::execute(
        &ctx,
        next_step_turn(&fx, "How did my week compare with the one before?"),
        &web_profile(),
    )
    .await
    .expect("the tapped step is answered");
    assert!(
        matches!(served, ServedTurn::Pipeline(_)),
        "a question runs the pipeline"
    );

    assert_eq!(
        only(&events, "chat.question_asked").field("origin"),
        "next_step"
    );
    assert_eq!(
        only(&events, "chat.answer_delivered").field("origin"),
        "next_step",
        "the answer is attributed to the same tap as the question"
    );
}

#[tokio::test]
async fn a_client_cannot_claim_a_next_step() {
    let fx = setup("next-step-claim@test.com").await;
    let (events, _guard) = capture_notify();

    let claimed = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/messages",
        fx.conversation_id
    ))
    .header("authorization", &fx.auth)
    .json(&json!({ "content": "/season", "origin": "next_step" }))
    .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
    .await;

    assert_eq!(
        claimed.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "only the server resolves a next step"
    );
    assert!(
        named(&events, "chat.question_asked").is_empty(),
        "a refused claim asks nothing"
    );
    let rows = fx
        .resources
        .common
        .repos
        .chat
        .get_messages(&fx.conversation_id, &fx.user_id.to_string(), fx.tenant_id)
        .await
        .unwrap();
    assert!(rows.is_empty(), "and writes nothing: {rows:?}");
}
