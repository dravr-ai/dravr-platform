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
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use serde_json::json;
use uuid::Uuid;

/// A tool a guided walk must withhold, and one it must not.
const WITHHELD: &str = "save_training_plan";
const ALWAYS_VISIBLE: &str = "get_activities";

/// A budget high enough that the existing cases never reach it — they are
/// about which tools are visible, not about spending the turn.
const AMPLE_BUDGET: usize = 64;

fn surface_for(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) -> TurnToolSurface {
    surface_with_budget(resources, user_id, tenant, AMPLE_BUDGET)
}

fn surface_with_budget(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    budget: usize,
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
        budget,
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

    let surface = surface_for(&resources, user_id, tenant);
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

    let surface = surface_for(&resources, user_id, tenant);
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

    let surface = surface_for(&resources, user_id, tenant);
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
    let surface = surface_for(&resources, user_id, tenant);

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

// ============================================================================
// Per-turn tool budget (registre#103)
// ============================================================================

/// The agent's loop runs in its own subprocess, so the only place the platform
/// can stop it is here. Past the budget a call must be refused rather than
/// executed — and refused with `is_error`, because that is what the agent reads
/// as "adapt" rather than as data.
#[tokio::test]
async fn a_turn_stops_serving_tools_once_its_budget_is_spent() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let budget = 2;
    let surface = surface_with_budget(&resources, user_id, tenant, budget);

    // The budgeted calls are served: whatever the tool answers, the refusal is
    // not the budget one.
    for i in 0..budget {
        let outcome = surface.call(ALWAYS_VISIBLE, &json!({})).await;
        assert!(
            !outcome.text.contains("tool budget"),
            "call {i} is within budget and must not be refused for budget; got {}",
            outcome.text
        );
    }

    // The next one is refused, names the budget, and tells the model what to do
    // instead of leaving it to guess.
    let over = surface.call(ALWAYS_VISIBLE, &json!({})).await;
    assert!(over.is_error, "an over-budget call must read as an error");
    assert!(
        over.text.contains("tool budget of 2 calls is spent"),
        "the refusal must name the spent budget; got {}",
        over.text
    );
    assert!(
        over.text.contains("Answer from the data you already have"),
        "the refusal must tell the model how to proceed; got {}",
        over.text
    );

    // And it stays refused — the budget is a ceiling, not a one-shot warning.
    let further = surface.call(ALWAYS_VISIBLE, &json!({})).await;
    assert!(further.is_error, "the budget must keep holding");
}

/// A budget of zero admits nothing. Worth pinning separately: an off-by-one
/// that served the first call anyway would leave the tightest budget the
/// admin config can express silently ineffective.
#[tokio::test]
async fn a_zero_budget_serves_no_tool_call_at_all() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let surface = surface_with_budget(&resources, user_id, tenant, 0);
    let outcome = surface.call(ALWAYS_VISIBLE, &json!({})).await;

    assert!(outcome.is_error, "a zero budget must refuse the first call");
    assert!(
        outcome.text.contains("tool budget of 0 calls is spent"),
        "got {}",
        outcome.text
    );
}
