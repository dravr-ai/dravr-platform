// ABOUTME: The coach's seat on the tool surface: the coach's own data tools are neither listed nor callable
// ABOUTME: Group tools that reach a named athlete stay; without the seat the caller's own tools are untouched
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#742. In a group the sender coaches, every chat-callable tool would
//! run as the coach, so a model calling `get_activities` with no athlete named
//! would read the coach's own rides. The surface built through
//! `HostedToolBridge::turn_surface` — the loopback path production runs —
//! must leave those tools out of its listing, and its executor must refuse
//! them if the model calls one anyway.

mod common;

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user};
use embacle_tool_host::ToolSurface;
use pierre_chat_pipeline::ToolSessionTurn;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_mcp_server::mcp::resources::tool_surface::{HostedToolBridge, TurnToolSurface};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::coach_seat::{TurnSeat, COACH_SEAT_ERROR_CODE};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use uuid::Uuid;

/// The caller's own activities: withheld on the seat.
const OWN_DATA_TOOL: &str = "get_activities";
/// A named athlete's activities, through their consent: kept on the seat.
const ATHLETE_TOOL: &str = "get_group_member_activities";

fn surface(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    seat: TurnSeat,
) -> TurnToolSurface {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let bridge = HostedToolBridge::new(
        true,
        resources.mcp.tool_registry.clone(),
        resources.common.repos.clone(),
        tool_runtime,
    );
    bridge.turn_surface(ToolSessionTurn {
        user_id: &user_id.to_string(),
        tenant_id: tenant,
        conversation_id: "conv-under-test",
        turn_id: ConversationTurnId(Uuid::new_v4()),
        turn_agent_id: None,
        budget: 64,
        seat,
    })
}

#[tokio::test]
async fn the_coach_seat_lists_the_athlete_tool_and_not_the_coachs_own() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let names: Vec<String> = surface(&resources, user_id, tenant, TurnSeat::Coach)
        .list_tools()
        .await
        .into_iter()
        .map(|t| t.name)
        .collect();

    assert!(
        !names.iter().any(|n| n == OWN_DATA_TOOL),
        "the coach's own activities must not be listed on the seat"
    );
    assert!(
        names.iter().any(|n| n == ATHLETE_TOOL),
        "the coach must keep the tool that reads a named athlete"
    );
}

#[tokio::test]
async fn a_withheld_tool_called_anyway_is_refused_with_the_redirect() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let outcome = surface(&resources, user_id, tenant, TurnSeat::Coach)
        .call(OWN_DATA_TOOL, &json!({}))
        .await;

    assert!(outcome.is_error, "the coach's own data must be refused");
    assert!(
        outcome.text.contains(COACH_SEAT_ERROR_CODE),
        "the refusal must be the coach-seat one, got: {}",
        outcome.text
    );
    assert!(
        outcome.text.contains(ATHLETE_TOOL) && outcome.text.contains("own conversation"),
        "the refusal must point at the athlete tool and the coach's own thread, got: {}",
        outcome.text
    );
}

#[tokio::test]
async fn without_the_seat_the_callers_own_tools_are_listed() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let names: Vec<String> = surface(&resources, user_id, tenant, TurnSeat::Subject)
        .list_tools()
        .await
        .into_iter()
        .map(|t| t.name)
        .collect();

    assert!(names.iter().any(|n| n == OWN_DATA_TOOL));
}
