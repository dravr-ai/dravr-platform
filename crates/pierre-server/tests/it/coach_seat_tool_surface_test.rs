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
//! thread's athlete, stay.
//!
//! carnet#1387. A read of the coach's own data would answer a question about
//! the athlete with the coach's — their sleep, their memory — so the thread
//! withholds every read that declares no data class and takes no `athlete`,
//! the four `groups` tools among them. The training reads, which read the
//! thread's athlete, and the reads of no one's data stay.

use crate::common;
use crate::helpers::coach_roster_fixtures::{coach, coached_group, group_agent};
use pierre_core::permissions::scopes::OAuthScope;

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user, create_test_user_with_plan};
use embacle_tool_host::ToolSurface;
use pierre_chat_pipeline::ToolSessionTurn;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_mcp_server::mcp::resources::tool_surface::{HostedToolBridge, TurnToolSurface};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::ai_scope;
use pierre_tool_runtime::chat_catalogue::SwitchedOff;
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::context::TURN_DIRECT_MESSAGE;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_access::CallerRole;
use serde_json::json;
use uuid::Uuid;

/// The caller's own activities: withheld on the seat.
const OWN_DATA_TOOL: &str = "get_activities";
/// What the agent reads back when it calls a tool its turn did not offer
/// (`function_dispatch::not_offered`, carnet#1511 design call 13): the seat's
/// withholds are left out of the listing, so a call made anyway is refused
/// before the executor's own seat rule is reached.
const NOT_OFFERED: &str = "not found. Available tools";
/// A named athlete's activities, through their consent: kept on the seat.
const ATHLETE_TOOL: &str = "get_group_member_activities";
/// Writes on the caller's own account: withheld in a coach's athlete thread.
const OWN_ACCOUNT_WRITES: [&str; 4] = [
    "remember_fact",
    "commitment_create",
    "set_goal",
    "set_physiology",
];
/// Takes `athlete`, which defaults to the thread's athlete: kept there.
const PLAN_WRITE: &str = "save_training_plan";
/// Reads only the coach's own data: withheld in a coach's athlete thread.
/// The athlete's memory is never read there.
const OWN_READS: [&str; 2] = ["recall_user_memory", "list_coaching_playbooks"];
/// Reads the thread's athlete's wellness under their grant (carnet#1387):
/// kept there.
const WELLNESS_READS: [&str; 2] = ["get_sleep_sessions", "get_recovery_metrics"];
/// The `groups` tools: each would scan every group the coach coaches from a
/// thread, so the thread withholds them; the room keeps them.
const GROUP_TOOLS: [&str; 4] = [
    "get_group_member_activities",
    "get_roster_overview",
    "get_group_member_plan_status",
    "get_provider_roster",
];
/// A training read of the thread's athlete: kept there.
const TRAINING_READ: &str = "get_training_history";
/// Reads no person's data: kept there.
const NO_SUBJECT_READ: &str = "get_weather_forecast";

/// The surface an in-app turn opens: built inside the web app's transport
/// declaration, as the turn service scopes every turn, on a direct-message
/// turn in the user's own conversation, so the seat alone decides what the
/// listing leaves out.
async fn surface(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    seat: TurnSeat,
) -> TurnToolSurface {
    let conversation = common::own_conversation(resources, user_id, tenant).await;
    surface_in(resources, user_id, tenant, seat, &conversation, true).await
}

/// [`surface`] on the turn in `conversation`, a direct message or not.
async fn surface_in(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    seat: TurnSeat,
    conversation: &str,
    direct_message: bool,
) -> TurnToolSurface {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let bridge = HostedToolBridge::new(true, resources.mcp.tool_registry.clone(), tool_runtime);
    // The role the web route resolves for this turn.
    let role = CallerRole::resolve(&resources.common.repos, user_id).await;
    let surface = ai_scope::serve_over(Transport::WebApp, async {
        bridge.turn_surface(ToolSessionTurn {
            user_id: &user_id.to_string(),
            tenant_id: tenant,
            conversation_tenant_id: tenant,
            conversation_id: conversation,
            turn_id: ConversationTurnId(Uuid::new_v4()),
            turn_agent_id: None,
            budget: 64,
            seat,
            switched_off: &SwitchedOff::default(),
            grant: &OAuthScope::self_grant(),
            role,
        })
    });
    TURN_DIRECT_MESSAGE
        .scope(Some(direct_message), surface)
        .await
}

#[tokio::test]
async fn the_coach_seat_lists_the_athlete_tool_and_not_the_coachs_own() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let names: Vec<String> = surface(&resources, user_id, tenant, TurnSeat::Coach)
        .await
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
async fn a_withheld_tool_called_anyway_is_refused_as_not_offered() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let outcome = surface(&resources, user_id, tenant, TurnSeat::Coach)
        .await
        .call(OWN_DATA_TOOL, &json!({}))
        .await;

    assert!(outcome.is_error, "the coach's own data must be refused");
    assert!(
        outcome.text.contains(NOT_OFFERED),
        "the refusal must be the not-offered one, got: {}",
        outcome.text
    );
}

/// carnet#1511 design call 13 on the loopback: the agent's call to a tool
/// outside the chat categories, or to an admin tool, is refused as not
/// offered on the caller's own seat, where nothing else withholds it.
#[tokio::test]
async fn the_loopback_runs_no_tool_the_turn_did_not_offer() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let surface = surface(&resources, user_id, tenant, TurnSeat::Subject).await;
    let listed: Vec<String> = surface
        .list_tools()
        .await
        .into_iter()
        .map(|t| t.name)
        .collect();

    for tool in [
        "list_agents",
        "calculate_personalized_zones",
        "admin_list_system_agents",
    ] {
        assert!(
            !listed.iter().any(|name| name == tool),
            "{tool} is not offered"
        );
        let refused = surface.call(tool, &json!({})).await;
        assert!(refused.is_error, "{tool}: {}", refused.text);
        assert!(
            refused
                .text
                .contains(&format!("Tool '{tool}' {NOT_OFFERED}")),
            "{tool}: the not-offered refusal, got: {}",
            refused.text
        );
    }
}

#[tokio::test]
async fn without_the_seat_the_callers_own_tools_are_listed() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let names: Vec<String> = surface(&resources, user_id, tenant, TurnSeat::Subject)
        .await
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
        .await
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
    for read in OWN_READS {
        assert!(
            !names.iter().any(|n| n == read),
            "{read} reads the coach's own data, never the thread's athlete"
        );
    }
    if cfg!(feature = "tools-groups") {
        for group_tool in GROUP_TOOLS {
            assert!(
                !names.iter().any(|n| n == group_tool),
                "{group_tool} would scan every group the coach coaches from the thread"
            );
        }
    }
    for kept in [OWN_DATA_TOOL, TRAINING_READ, NO_SUBJECT_READ]
        .into_iter()
        .chain(WELLNESS_READS)
    {
        assert!(
            names.iter().any(|n| n == kept),
            "{kept} reads the thread's athlete or no one's data, and stays"
        );
    }
}

#[tokio::test]
async fn a_read_of_the_coachs_own_data_called_in_the_athlete_thread_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let thread = surface(&resources, user_id, tenant, TurnSeat::AthleteThread).await;

    let mut refused_reads: Vec<&str> = OWN_READS.to_vec();
    if cfg!(feature = "tools-groups") {
        refused_reads.extend(GROUP_TOOLS);
    }
    for read in refused_reads {
        let refused = thread.call(read, &json!({})).await;
        assert!(refused.is_error, "{read}: {}", refused.text);
        assert!(
            refused.text.contains(NOT_OFFERED),
            "{read}: the not-offered refusal, got: {}",
            refused.text
        );
    }

    // The room keeps the group tools: the thread's rule is the thread's. In
    // a real coached group's room, on a turn others read, each coach tool
    // whose gate takes a call there stays; the provider roster, whose gate
    // refuses every room, does not.
    if cfg!(feature = "tools-groups") {
        let casey = coach(&resources, "casey@seat-surface.test", "Casey Coach").await;
        let agent = group_agent(&resources, &casey).await;
        let group = coached_group(&resources, casey.tenant, casey.id, &agent, "Squad").await;
        let room = resources
            .common
            .repos
            .chat
            .create_conversation(
                &casey.id.to_string(),
                casey.tenant,
                "Squad",
                "mock-model",
                Some(&agent),
                Some(&group.to_string()),
            )
            .await
            .unwrap()
            .id;
        let listed: Vec<String> = surface_in(
            &resources,
            casey.id,
            casey.tenant,
            TurnSeat::Coach,
            &room,
            false,
        )
        .await
        .list_tools()
        .await
        .into_iter()
        .map(|t| t.name)
        .collect();
        for group_tool in GROUP_TOOLS {
            assert_eq!(
                listed.iter().any(|n| n == group_tool),
                group_tool != "get_provider_roster",
                "{group_tool} in the coached group's room: {listed:?}"
            );
        }
    }
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
        .await
        .call("remember_fact", &fact)
        .await;

    assert!(refused.is_error, "{}", refused.text);
    assert!(
        refused.text.contains(NOT_OFFERED),
        "the refusal must be the not-offered one, got: {}",
        refused.text
    );
    assert!(
        filed_facts(&resources, user_id, tenant).await.is_empty(),
        "the refused call filed a fact"
    );

    // The control: the same call from the subject's seat files the fact, so
    // the refusal is the seat's doing and not a malformed call.
    let accepted = surface(&resources, user_id, tenant, TurnSeat::Subject)
        .await
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

/// The loopback reads the turn's room under the tenant its conversation row
/// is filed under, as the turn's declarations and names index read it: a
/// coach's own conversation filed under another tenant than the one their
/// tools run under is offered the coach tools, and takes them.
#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn the_loopback_reads_the_room_under_the_conversations_own_tenant() {
    const ROSTER: &str = "get_roster_overview";
    let resources = create_test_server_resources().await.unwrap();
    let casey = coach(&resources, "casey@loopback-tenant.test", "Casey Coach").await;
    let agent = group_agent(&resources, &casey).await;
    coached_group(&resources, casey.tenant, casey.id, &agent, "Squad").await;
    let filed = TenantId::from_uuid(Uuid::new_v4());
    let conversation = common::own_conversation(&resources, casey.id, filed).await;
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let bridge = HostedToolBridge::new(true, resources.mcp.tool_registry.clone(), tool_runtime);
    let role = CallerRole::resolve(&resources.common.repos, casey.id).await;
    let surface = TURN_DIRECT_MESSAGE
        .scope(
            Some(true),
            ai_scope::serve_over(Transport::WebApp, async {
                bridge.turn_surface(ToolSessionTurn {
                    user_id: &casey.id.to_string(),
                    tenant_id: casey.tenant,
                    conversation_tenant_id: filed,
                    conversation_id: &conversation,
                    turn_id: ConversationTurnId(Uuid::new_v4()),
                    turn_agent_id: None,
                    budget: 64,
                    seat: TurnSeat::Subject,
                    switched_off: &SwitchedOff::default(),
                    grant: &OAuthScope::self_grant(),
                    role,
                })
            }),
        )
        .await;

    let names: Vec<String> = surface
        .list_tools()
        .await
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(
        names.iter().any(|n| n == ROSTER),
        "the coach's own conversation offers the roster: {names:?}"
    );
    let outcome = surface.call(ROSTER, &json!({})).await;
    assert!(
        !outcome.is_error,
        "the roster call is taken in the coach's own conversation: {}",
        outcome.text
    );
}
