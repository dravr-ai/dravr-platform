// ABOUTME: The loopback surface's refusals are catalogued notify events, not log lines
// ABOUTME: A turn that could not refresh its data must not look like a turn that asked for none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! What the executor cannot report, and this surface therefore must.
//!
//! `messaging.tool_executed` is emitted once per dispatch from
//! `UniversalExecutor::record_dispatch`, which every transport funnels through
//! — the native ACP path included, since `TurnToolSurface::call` dispatches
//! through that same executor. Per-call native observability was never the gap,
//! whatever the closing analysis on registre#103 said.
//!
//! The gap is the calls that never produce a response. `record_dispatch` is
//! `execute_tool`'s last statement before `Ok`, so three exits skip it, and all
//! three live in the surface rather than the executor: the per-turn budget
//! refusing before `execute_tool` is called, the 90s bound abandoning its
//! future, and a `ProtocolError` returning early. Each means the coach answered
//! the athlete from data it could not refresh.
//!
//! Budget is the case tested here because it is the only one reachable without
//! faking a provider or waiting 90 seconds, and it is the one that carries the
//! enforcement registre#103 shipped. The assertion that matters is not that a
//! refusal was logged — it is that the refusal fired its own event while
//! `messaging.tool_executed` did NOT, because the two together are what tell an
//! operator a budget is set too low rather than that nobody asked for anything.

mod common;
mod helpers;

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user};
use embacle_tool_host::ToolSurface;
use helpers::notify_capture::{capture_notify, named, only};
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_mcp_server::mcp::resources::tool_surface::HostedToolBridge;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use uuid::Uuid;

/// Answers from the platform's own rows, so spending the budget does not depend
/// on a provider being reachable. The same tool the turn-token test uses.
const PLATFORM_TOOL: &str = "list_agents";

/// One call served; the next is refused. The smallest budget that still proves
/// a served call and a refused call are reported differently.
const BUDGET_OF_ONE: usize = 1;

#[tokio::test]
async fn a_spent_tool_budget_fires_its_own_catalogued_event() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();

    let bridge = HostedToolBridge::new(
        true,
        resources.mcp.tool_registry.clone(),
        resources.common.repos.clone(),
        tool_runtime,
    );
    let surface = bridge.turn_surface(
        &user_id.to_string(),
        tenant,
        "conv-under-test",
        ConversationTurnId(Uuid::new_v4()),
        BUDGET_OF_ONE,
    );

    // Installed after the fixtures, before the code under test: the capture is
    // this thread's whole subscriber, and the pool's spans belong to the global
    // one that `common` installed.
    let (events, _guard) = capture_notify();

    // Call 1 spends the budget and reaches a dispatch. Its success is not
    // asserted — `record_dispatch` reports a declined tool too, and what this
    // test needs from it is only that it got that far.
    let _served = surface.call(PLATFORM_TOOL, &json!({})).await;

    // Call 2 is refused by the surface, before `execute_tool` is reached.
    let refused = surface.call(PLATFORM_TOOL, &json!({})).await;
    assert!(
        refused.is_error,
        "a second call against a budget of one must be refused; got {}",
        refused.text
    );
    assert!(
        refused.text.contains("budget"),
        "the refusal the model reads must name the budget so it can adapt; got {}",
        refused.text
    );

    // The discriminating pair. One dispatch happened, so the executor's event
    // fired exactly once; the refusal dispatched nothing, so it cannot be
    // counted there — which is precisely why it needs an event of its own.
    assert_eq!(
        named(&events, "messaging.tool_executed").len(),
        1,
        "exactly one call reached a dispatch, so exactly one executed event is \
         owed. Two would mean the budget did not hold; zero would mean the \
         first call never dispatched and this test is measuring the wrong thing"
    );

    let refusal = only(&events, "messaging.tool_call_refused");
    assert_eq!(
        refusal.field("reason"),
        "budget_spent",
        "the reason is the whole point of the event — it separates a bound \
         working as designed from a timeout and from a dispatch failure"
    );
    assert_eq!(
        refusal.field("tool_name"),
        PLATFORM_TOOL,
        "the refused tool must be named; 'some tool was refused' is not actionable"
    );
    assert_eq!(
        refusal.field("tenant_id"),
        tenant.to_string(),
        "the event is routed and deduped per tenant, so the tenant must be its own"
    );
    assert_eq!(
        refusal.field("budget"),
        "1",
        "the bound that was hit, so an operator can tell a budget set too low \
         from a turn that genuinely needed more tools than anyone would grant"
    );

    // The dimension both events share. A refusal rate is refused-over-executed,
    // and that ratio only exists if `channel` means the same thing on each.
    let executed = only(&events, "messaging.tool_executed");
    assert_eq!(
        refusal.field("channel"),
        executed.field("channel"),
        "the refusal must report the same channel the executed event does, or \
         the two cannot be divided by one another"
    );
    assert_eq!(
        refusal.field("channel"),
        "mcp",
        "the agent reached us over MCP, which is what the dispatch is charged under"
    );
}
