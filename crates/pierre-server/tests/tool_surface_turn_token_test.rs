// ABOUTME: The loopback tool surface binds the utterance's Guardian turn token
// ABOUTME: Taint recorded by one agent tool call must be visible to the next

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::uninlined_format_args)]

//! Cross-call accumulation on the path production runs.
//!
//! `copilot_headless` never reports `FUNCTION_CALLING`, so the agent runs its own
//! tool loop in its own subprocess and every call arrives back through
//! `HostedToolBridge`'s surface. Guardian taint and the per-turn blast-radius
//! budgets are keyed on `TurnKey`, so a surface whose executor carries no turn
//! token mints a fresh key per call: every loopback call would start from a
//! virgin bucket and nothing an earlier call learned could reach a later one.
//!
//! The proof used here is taint rather than the destructive budget, because
//! budget is refunded when a call fails and taint never is: an
//! `UNTRUSTED_OUTPUT` read taints the turn the instant it is cleared to run.
//! Under `GUARDIAN_TAINTED_DESTRUCTIVE=deny` an irreversible tool that follows
//! it in the same turn must be blocked — and only if the two calls share a key.
//!
//! One test fn, because the Guardian policy is captured from the environment
//! once when the server resources are built.

mod common;

use std::env;
use std::sync::Arc;

use common::{create_test_server_resources, create_test_user};
use embacle_tool_host::ToolSurface;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_mcp_server::mcp::resources::tool_surface::HostedToolBridge;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

/// An `UNTRUSTED_OUTPUT` read with no provider requirement, so it reaches the
/// Guardian gate on a bare test account and taints the turn.
const TAINT_SOURCE: &str = "list_coaches";

/// An `IRREVERSIBLE` tool, the sink the taint policy guards.
const DESTRUCTIVE: &str = "disconnect_provider";

/// High enough that the surface's own call budget never fires here.
const AMPLE_BUDGET: usize = 64;

fn guardian_reason(structured: Option<&Value>) -> Option<String> {
    structured
        .and_then(|s| s.get("reason"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn guardian_error_code(structured: Option<&Value>) -> Option<String> {
    structured
        .and_then(|s| s.get("error_code"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[tokio::test]
async fn loopback_calls_in_one_turn_share_a_guardian_turn_key() {
    // Captured once, when the resources below build the Guardian config
    // registry.
    env::set_var("GUARDIAN_MODE", "enforce");
    env::set_var("GUARDIAN_TAINTED_DESTRUCTIVE", "deny");

    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
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

    let turn = ConversationTurnId(Uuid::new_v4());
    let surface = bridge.turn_surface(
        &user_id.to_string(),
        tenant,
        "conv-under-test",
        turn,
        AMPLE_BUDGET,
    );

    // Call 1: the untrusted read. It must not itself be a Guardian block —
    // taint constrains what comes after, not the source.
    let source = surface.call(TAINT_SOURCE, &json!({})).await;
    assert_ne!(
        guardian_error_code(source.structured.as_ref()).as_deref(),
        Some("guardian_denied"),
        "the untrusted source must run; it is what puts taint on the turn. got {}",
        source.text
    );

    // Call 2: the irreversible tool, in the SAME turn. It is blocked only
    // because the surface's executor carries the turn's token — without it this
    // call lands in a bucket of its own where the turn is untainted and the
    // destructive counter is zero, and the Guardian lets it through.
    let sink = surface
        .call(DESTRUCTIVE, &json!({ "provider": "strava" }))
        .await;
    assert!(
        sink.is_error,
        "an irreversible tool after an untrusted read in the same turn must be \
         refused; got {}",
        sink.text
    );
    assert_eq!(
        guardian_error_code(sink.structured.as_ref()).as_deref(),
        Some("guardian_denied"),
        "the refusal must be the Guardian's, not the tool's own failure; got {:?}",
        sink.structured
    );
    assert_eq!(
        guardian_reason(sink.structured.as_ref()).as_deref(),
        Some("tainted_sink"),
        "the reason must name the taint the previous loopback call recorded"
    );

    // And the accumulation is scoped to the utterance, not to the process: a
    // second turn starts clean, so the same call is no longer a taint block.
    let next_turn = bridge.turn_surface(
        &user_id.to_string(),
        tenant,
        "conv-under-test",
        ConversationTurnId(Uuid::new_v4()),
        AMPLE_BUDGET,
    );
    let fresh = next_turn
        .call(DESTRUCTIVE, &json!({ "provider": "strava" }))
        .await;
    assert_ne!(
        guardian_reason(fresh.structured.as_ref()).as_deref(),
        Some("tainted_sink"),
        "a new turn must not inherit the previous turn's taint"
    );
}
