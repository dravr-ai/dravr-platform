// ABOUTME: The coach's seats on the tool surface: what the seat withholds is neither listed nor callable
// ABOUTME: Group tools and an athlete thread's plan tools stay; without a seat the caller's own tools are untouched
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
//!
//! carnet#1070. In a coach's thread about one athlete, a write on the coach's
//! own account would file what the coach says of the athlete as the coach's
//! own — "her FTP is 250" as the coach's FTP. The same surface leaves every
//! such write out and refuses it, while the plan tools, which act on the
//! thread's athlete, and the coach's own reads stay.

use crate::common;

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user, create_test_user_with_plan};
use embacle_tool_host::ToolSurface;
use pierre_chat_pipeline::ToolSessionTurn;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::mcp::resources::tool_surface::{HostedToolBridge, TurnToolSurface};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::coach_seat::{TurnSeat, ATHLETE_THREAD_ERROR_CODE, COACH_SEAT_ERROR_CODE};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use uuid::Uuid;

/// The caller's own activities: withheld on the seat.
const OWN_DATA_TOOL: &str = "get_activities";
/// A named athlete's activities, through their consent: kept on the seat.
const ATHLETE_TOOL: &str = "get_group_member_activities";
/// Writes on the caller's own account: withheld in a coach's athlete thread.
const OWN_ACCOUNT_WRITES: [&str; 6] = [
    "remember_fact",
    "agent_note_add",
    "agent_followup_schedule",
    "commitment_create",
    "set_goal",
    "set_physiology",
];
/// Takes `athlete`, which defaults to the thread's athlete: kept there.
const PLAN_WRITE: &str = "save_training_plan";

fn surface(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    seat: TurnSeat,
) -> TurnToolSurface {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let bridge = HostedToolBridge::new(true, resources.mcp.tool_registry.clone(), tool_runtime);
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
    for write in OWN_ACCOUNT_WRITES {
        assert!(
            names.iter().any(|n| n == write),
            "{write} is the subject's own to call"
        );
    }
}

#[tokio::test]
async fn the_athlete_thread_lists_the_plan_tools_and_no_write_on_the_coachs_account() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let names: Vec<String> = surface(&resources, user_id, tenant, TurnSeat::AthleteThread)
        .list_tools()
        .await
        .into_iter()
        .map(|t| t.name)
        .collect();

    for write in OWN_ACCOUNT_WRITES {
        assert!(
            !names.iter().any(|n| n == write),
            "{write} would file on the coach's own account from their athlete thread"
        );
    }
    assert!(
        names.iter().any(|n| n == PLAN_WRITE),
        "the plan tools act on the thread's athlete and stay"
    );
    assert!(
        names.iter().any(|n| n == OWN_DATA_TOOL),
        "the coach's own reads stay, and the thread's prompt says whose they are"
    );
}

#[tokio::test]
async fn a_fact_remembered_in_the_athlete_thread_is_refused_and_filed_nowhere() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _, tenant) = create_test_user_with_plan(
        &resources.agent.database,
        "athlete-thread-surface@test.com",
        "professional",
    )
    .await
    .unwrap();
    let fact = json!({
        "kind": "physiology",
        "predicate_code": "have_baseline",
        "object": "FTP 250 W",
        "confidence": 0.9,
    });

    let refused = surface(&resources, user_id, tenant, TurnSeat::AthleteThread)
        .call("remember_fact", &fact)
        .await;

    assert!(refused.is_error, "{}", refused.text);
    assert!(
        refused.text.contains(ATHLETE_THREAD_ERROR_CODE),
        "the refusal must be the athlete-thread one, got: {}",
        refused.text
    );
    assert!(
        refused
            .text
            .contains("nothing said in it is saved to the coach's own account"),
        "{}",
        refused.text
    );
    assert!(
        filed_facts(&resources, user_id, tenant).await.is_empty(),
        "the refused call filed a fact"
    );

    // The control: the same call from the subject's seat files the fact, so
    // the refusal is the seat's doing and not a malformed call.
    let accepted = surface(&resources, user_id, tenant, TurnSeat::Subject)
        .call("remember_fact", &fact)
        .await;
    assert!(!accepted.is_error, "{}", accepted.text);
    assert_eq!(
        filed_facts(&resources, user_id, tenant).await,
        vec!["FTP 250 W".to_owned()]
    );
}

/// What `user_id` has on file as facts, by object: kept or waiting on them
/// alike, since the question is whose account a fact is filed on.
async fn filed_facts(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<String> {
    let memory = &resources.common.repos.memory;
    let user = user_id.to_string();
    let kept = memory
        .list_user_facts(
            tenant,
            &user,
            None,
            None,
            50,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap();
    let drafts = memory
        .list_draft_user_facts(tenant, &user, 50, TransportPolicy::FirstPartyOnly)
        .await
        .unwrap();
    kept.into_iter()
        .chain(drafts)
        .map(|fact| fact.object)
        .collect()
}
