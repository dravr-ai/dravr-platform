// ABOUTME: The tool surface the platform publishes to an ACP agent through embacle
// ABOUTME: Listing is re-asked per call, so a guided walk withholds mid-session
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::uninlined_format_args)]

//! The platform's half of the provider contract.
//!
//! embacle owns the listener, the credential and its revocation; those are
//! tested in embacle against real agents. What is ours is which tools a turn
//! may see and what running one means — and both are exercised here without
//! needing an agent, because the surface is a plain trait.
//!
//! The withholding test is the one that matters. A `/pillars` walk withholds
//! plan-writing for its duration, and that state can change between the
//! agent's `tools/list` and its `tools/call`. Answering from a snapshot taken
//! when the session opened would let a coach mid-interview reach a tool it was
//! meant to be denied — the same defect as an advertisement filter that
//! silently no-ops on the path nobody exercises.

mod common;

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user};
use embacle_tool_host::ToolSurface;
use pierre_core::models::{GuidedFlow, OnboardingState, TenantId};
use pierre_mcp_server::mcp::resources::tool_surface::TurnToolSurface;
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use serde_json::json;
use uuid::Uuid;

/// A tool a guided walk must withhold, and one it must not.
const WITHHELD: &str = "save_training_plan";
const ALWAYS_VISIBLE: &str = "get_activities";

async fn surface_for(
    resources: &Arc<pierre_mcp_server::mcp::resources::ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> TurnToolSurface {
    let executor = Arc::new(
        UniversalToolExecutor::new(resources.clone())
            .with_conversation_id("conv-under-test".into()),
    );
    TurnToolSurface::new(
        resources.mcp.tool_registry.clone(),
        resources.common.repos.clone(),
        executor,
        user_id.to_string(),
        tenant,
    )
}

/// The agent sees the chat-callable surface, by registry name.
#[tokio::test]
async fn the_surface_publishes_the_chat_callable_tools() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let surface = surface_for(&resources, user_id, tenant).await;
    let tools = surface.list_tools().await;

    assert!(
        tools.len() > 5,
        "the coaching surface should be substantial, got {}",
        tools.len()
    );
    assert!(
        tools.iter().any(|t| t.name == ALWAYS_VISIBLE),
        "a read tool must always be visible"
    );
    assert!(
        tools.iter().all(|t| !t.input_schema.is_null()),
        "every tool must carry a schema the model can call against"
    );
}

/// A tool that runs comes back as a success carrying its payload.
#[tokio::test]
async fn a_successful_call_carries_its_payload() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let surface = surface_for(&resources, user_id, tenant).await;
    let outcome = surface.call("list_fitness_configs", &json!({})).await;

    assert!(
        !outcome.is_error,
        "a tool that ran must not be reported as a refusal: {}",
        outcome.text
    );
    assert!(
        outcome.structured.is_some(),
        "the payload must reach the model as structuredContent, not only prose"
    );
}

/// A tool that does not exist is refused, and the refusal is machine-readable.
#[tokio::test]
async fn an_unknown_tool_is_refused_not_faked() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let surface = surface_for(&resources, user_id, tenant).await;
    let outcome = surface.call("analyze_my_vibes", &json!({})).await;

    assert!(
        outcome.is_error,
        "an unregistered tool must be a refusal, not a success with empty data"
    );
}

/// THE ONE THAT MATTERS: a guided walk withholds plan-writing, and the surface
/// is asked again rather than answering from when the session opened.
#[tokio::test]
async fn a_guided_walk_withholds_the_write_tool_on_a_later_listing() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let surface = surface_for(&resources, user_id, tenant).await;

    // No walk yet: the write tool is offered.
    let before = surface.list_tools().await;
    assert!(
        before.iter().any(|t| t.name == WITHHELD),
        "{WITHHELD} should be visible outside a guided walk"
    );

    // A walk starts — mid-session, after the surface was already listed once.
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "walk",
            "test-model",
            None,
            None,
        )
        .await
        .expect("conversation");
    resources
        .common
        .repos
        .chat
        .set_conversation_onboarding_state(
            &conversation.id,
            Some(&OnboardingState::start_now_column(GuidedFlow::Pillars)),
            tenant,
        )
        .await
        .expect("walk starts");

    // Asked again, the surface must reflect it.
    let during = surface.list_tools().await;
    assert!(
        !during.iter().any(|t| t.name == WITHHELD),
        "{WITHHELD} must be withheld while a guided walk owns the turn — \
         listing is re-asked, not snapshotted at session open"
    );
    assert!(
        during.iter().any(|t| t.name == ALWAYS_VISIBLE),
        "read tools stay available so the athlete can still be answered"
    );
}
