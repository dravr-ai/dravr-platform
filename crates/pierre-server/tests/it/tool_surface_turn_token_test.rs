// ABOUTME: The loopback tool surface binds the utterance's Guardian turn token
// ABOUTME: The budget one agent tool call spends must be visible to the next

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::uninlined_format_args)]

//! Cross-call accumulation on the path production runs.
//!
//! `copilot_headless` never reports `FUNCTION_CALLING`, so the agent runs its own
//! tool loop in its own subprocess and every call arrives back through
//! `HostedToolBridge`'s surface. The per-turn blast-radius budgets are keyed on
//! `TurnKey`, so a surface whose executor carries no turn token mints a fresh key
//! per call: every loopback call would start from a virgin bucket and nothing an
//! earlier call spent could reach a later one.
//!
//! The proof used here is a write budget of one, with a write that succeeds,
//! because a failed call refunds its charge: the second write in the turn must be
//! blocked — and only if the two calls share a key.

use crate::common;

use std::sync::Arc;

use common::{
    create_test_server_resources_with_config, create_test_user, test_server_config_with_guardian,
};
use embacle_tool_host::{ToolOutcome, ToolSurface};
use pierre_chat_pipeline::ToolSessionTurn;
use pierre_config::environment::GuardianSettings;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_mcp_server::mcp::resources::tool_surface::HostedToolBridge;
use pierre_tool_runtime::chat_catalogue::SwitchedOff;
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

/// An `IRREVERSIBLE` tool, for the turn that gets denied first.
const DESTRUCTIVE: &str = "disconnect_provider";

/// A plain write that succeeds on a bare test account, so its charge stays spent.
const WRITE: &str = "set_goal";

/// High enough that the surface's own call budget never fires here.
const AMPLE_BUDGET: usize = 64;

/// The machine-readable part of an outcome: a success's structured payload, or
/// the JSON a refusal carries after its reason. An error result has no
/// structured part on any surface, so a refusal that still carries one fails
/// here rather than being read.
fn outcome_data(outcome: &ToolOutcome) -> Option<Value> {
    if !outcome.is_error {
        return outcome.structured.clone();
    }
    assert!(
        outcome.structured.is_none(),
        "an error result carries no structuredContent; got {:?}",
        outcome.structured
    );
    outcome
        .text
        .rsplit_once("\n\n")
        .and_then(|(_, json)| serde_json::from_str(json).ok())
}

fn guardian_reason(outcome: &ToolOutcome) -> Option<String> {
    outcome_data(outcome)
        .as_ref()
        .and_then(|s| s.get("reason"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn guardian_error_code(outcome: &ToolOutcome) -> Option<String> {
    outcome_data(outcome)
        .as_ref()
        .and_then(|s| s.get("error_code"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[tokio::test]
async fn loopback_calls_in_one_turn_share_a_guardian_turn_key() {
    let resources = create_test_server_resources_with_config(test_server_config_with_guardian(
        GuardianSettings {
            mode: Some("enforce".to_owned()),
            max_writes_per_turn: Some("1".to_owned()),
            ..GuardianSettings::default()
        },
    ))
    .await
    .expect("server resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();

    let bridge = HostedToolBridge::new(true, resources.mcp.tool_registry.clone(), tool_runtime);

    let turn = ConversationTurnId(Uuid::new_v4());
    let surface = bridge.turn_surface(ToolSessionTurn {
        user_id: &user_id.to_string(),
        tenant_id: tenant,
        conversation_id: "conv-under-test",
        turn_id: turn,
        turn_agent_id: None,
        budget: AMPLE_BUDGET,
        seat: TurnSeat::Subject,
        switched_off: &SwitchedOff::default(),
    });
    let goal = json!({ "goal_type": "distance", "target_value": 100 });

    // Call 1: the write the budget allows. It has to succeed, or its charge comes
    // back and the next call proves nothing.
    let first = surface.call(WRITE, &goal).await;
    assert!(!first.is_error, "the first write runs; got {}", first.text);

    // Call 2: another write, in the SAME turn. It is blocked only because the
    // surface's executor carries the turn's token — without it this call lands in
    // a bucket of its own where the write counter is zero, and the Guardian lets
    // it through.
    let second = surface.call(WRITE, &goal).await;
    assert!(
        second.is_error,
        "a write past the turn's budget must be refused; got {}",
        second.text
    );
    assert_eq!(
        guardian_error_code(&second).as_deref(),
        Some("guardian_denied"),
        "the refusal must be the Guardian's, not the tool's own failure; got {}",
        second.text
    );
    assert_eq!(
        guardian_reason(&second).as_deref(),
        Some("budget_exceeded"),
        "the reason must name the budget the previous loopback call spent"
    );
    // The agent reads the refusal as its reason, then the Guardian's code and
    // reason as JSON — in the text, never as structuredContent.
    assert_eq!(
        second.text,
        format!(
            "Tool '{WRITE}' was blocked by security policy\n\n{}",
            json!({ "error_code": "guardian_denied", "reason": "budget_exceeded" })
        )
    );

    // And the accumulation is scoped to the utterance, not to the process: a
    // second turn starts clean, so the same call is no longer a budget block.
    let next_turn = bridge.turn_surface(ToolSessionTurn {
        user_id: &user_id.to_string(),
        tenant_id: tenant,
        conversation_id: "conv-under-test",
        turn_id: ConversationTurnId(Uuid::new_v4()),
        turn_agent_id: None,
        budget: AMPLE_BUDGET,
        seat: TurnSeat::Subject,
        switched_off: &SwitchedOff::default(),
    });
    let fresh = next_turn.call(WRITE, &goal).await;
    assert_ne!(
        guardian_error_code(&fresh).as_deref(),
        Some("guardian_denied"),
        "a new turn must not inherit the previous turn's budget; got {}",
        fresh.text
    );
}

/// carnet#1397, on the path it happened on: the agent's loop runs on in its own
/// subprocess after a refusal, so a write it makes next must not commit behind
/// the blocked reply the athlete is about to get. A read still runs, so the agent
/// can finish its answer.
#[tokio::test]
async fn a_denied_call_stops_the_agents_later_writes_in_the_same_turn() {
    let resources = create_test_server_resources_with_config(test_server_config_with_guardian(
        GuardianSettings {
            mode: Some("enforce".to_owned()),
            max_destructive_per_turn: Some("0".to_owned()),
            ..GuardianSettings::default()
        },
    ))
    .await
    .expect("server resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let bridge = HostedToolBridge::new(true, resources.mcp.tool_registry.clone(), tool_runtime);
    let surface = bridge.turn_surface(ToolSessionTurn {
        user_id: &user_id.to_string(),
        tenant_id: tenant,
        conversation_id: "conv-under-test",
        turn_id: ConversationTurnId(Uuid::new_v4()),
        turn_agent_id: None,
        budget: AMPLE_BUDGET,
        seat: TurnSeat::Subject,
        switched_off: &SwitchedOff::default(),
    });

    let denied = surface
        .call(DESTRUCTIVE, &json!({ "provider": "strava" }))
        .await;
    assert_eq!(guardian_reason(&denied).as_deref(), Some("budget_exceeded"));

    let write = surface
        .call(
            WRITE,
            &json!({ "goal_type": "distance", "target_value": 100 }),
        )
        .await;
    assert_eq!(
        guardian_reason(&write).as_deref(),
        Some("turn_blocked"),
        "a write after the block must be refused; got {}",
        write.text
    );

    let read = surface.call("get_connection_status", &json!({})).await;
    assert_ne!(
        guardian_error_code(&read).as_deref(),
        Some("guardian_denied"),
        "a read still runs; got {}",
        read.text
    );
}
