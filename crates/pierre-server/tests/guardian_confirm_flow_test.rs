// ABOUTME: Confirm HITL end-to-end — decide/gate outcomes, the parked row, and /confirm·/deny resolution
// ABOUTME: Covers park-at-chokepoint, single-use owner-checked claims, expiry, wrong-user probing, replay

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Guardian Confirm human-in-the-loop flow.
//!
//! Every resource-building test sets the SAME `GUARDIAN_MODE=enforce` +
//! `GUARDIAN_TAINTED_DESTRUCTIVE=confirm` env before construction (the
//! guardian registry captures env once), so parallel execution within this
//! binary cannot observe a half-set environment.

mod common;

use std::env;
use std::sync::Arc;

use chrono::{Duration, Utc};
use common::{create_test_server_resources, create_test_user};
use pierre_commands::guardian_confirm::{ConfirmHandler, DenyHandler};
use pierre_commands::{CommandHandler, PlatformCommandContext};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, EN_GUARDIAN_CONFIRM_DENIED, KEY_GUARDIAN_CONFIRM_DONE,
    KEY_GUARDIAN_CONFIRM_EXPIRED, KEY_GUARDIAN_CONFIRM_FAILED, KEY_GUARDIAN_CONFIRM_NOT_FOUND,
};
use pierre_core::models::TenantId;
use pierre_database::repositories::{ClaimOutcome, PendingGuardianAction};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_runtime_context::CommandCtx;
use pierre_tool_runtime::guardian::{
    Decision, ExternalSendAllowlist, Guardian, GuardianMode, GuardianPolicy, PlanMode,
    TaintedDestructive, TurnKey, TurnState,
};
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::SecurityLabels;
use serde_json::{json, Value};
use uuid::Uuid;

fn confirm_policy() -> GuardianPolicy {
    GuardianPolicy {
        mode: GuardianMode::Enforce,
        max_destructive_per_turn: 5,
        max_writes_per_turn: 50,
        external_send: ExternalSendAllowlist::None,
        tainted_destructive: TaintedDestructive::Confirm,
        plan_mode: PlanMode::Off,
    }
}

fn set_confirm_env() {
    env::set_var("GUARDIAN_MODE", "enforce");
    env::set_var("GUARDIAN_TAINTED_DESTRUCTIVE", "confirm");
}

fn request(tool: &str, args: Value, user_id: Uuid, tenant: TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: args,
        user_id: user_id.to_string(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

fn pending_action(user_id: Uuid, tenant: TenantId, tool: &str) -> PendingGuardianAction {
    PendingGuardianAction {
        id: Uuid::new_v4().simple().to_string(),
        tenant_id: tenant.to_string(),
        user_id: user_id.to_string(),
        conversation_id: None,
        tool_name: tool.to_owned(),
        arguments: json!({"provider": "strava"}),
        deny_reason: "tainted_sink".to_owned(),
    }
}

fn command_ctx(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    pending_id: &str,
) -> PlatformCommandContext {
    PlatformCommandContext {
        user_id,
        tenant_id: tenant,
        channel_type: "web".to_owned(),
        args: vec![pending_id.to_owned()],
        raw_text: format!("/confirm {pending_id}"),
        ctx: Arc::<ServerContext>::clone(resources) as Arc<dyn CommandCtx>,
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: false,
        conversation_id: None,
        conversation_tenant_id: tenant,
        sender_id: None,
        tool_runtime: Arc::<ServerContext>::clone(resources) as Arc<dyn ToolRuntime>,
    }
}

#[test]
fn decide_parks_only_a_tainted_destructive_call_under_confirm() {
    let guardian = Guardian::new(confirm_policy());

    let mut tainted = TurnState::default();
    tainted.add_taint("seeded_source", SecurityLabels::UNTRUSTED_OUTPUT);
    assert_eq!(
        guardian.decide(SecurityLabels::IRREVERSIBLE, false, None, &tainted),
        Decision::ConfirmRequired,
        "tainted + destructive + Confirm => park"
    );

    // Untainted destructive call: ordinary allow, no confirmation ceremony.
    assert_eq!(
        guardian.decide(
            SecurityLabels::IRREVERSIBLE,
            false,
            None,
            &TurnState::default()
        ),
        Decision::Allow
    );

    // Same tainted turn, ordinary write: goes through (the documented ReAct gap).
    assert_eq!(
        guardian.decide(SecurityLabels::empty(), true, None, &tainted),
        Decision::Allow
    );
}

#[tokio::test]
async fn chokepoint_parks_the_call_and_the_claim_is_single_use() {
    set_confirm_env();
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let turn_token = "confirm-e2e-turn";
    let executor =
        UniversalToolExecutor::new(resources.clone()).with_turn_token(turn_token.to_owned());

    // Seed taint for this turn deterministically (no dependency on which read
    // tools happen to carry UNTRUSTED_OUTPUT in the registry).
    resources.guardian_turns().record_taint(
        &TurnKey::new(Some(tenant.as_uuid()), turn_token.to_owned()),
        "seeded_source",
        SecurityLabels::UNTRUSTED_OUTPUT,
    );

    let resp = executor
        .execute_tool(request(
            "disconnect_provider",
            json!({"provider": "strava"}),
            user_id,
            tenant,
        ))
        .await
        .expect("dispatch returns an in-band response");

    assert!(!resp.success, "a parked call must not report success");
    let result = resp.result.as_ref().expect("in-band result");
    assert_eq!(
        result.get("error_code").and_then(Value::as_str),
        Some("guardian_confirm_required"),
        "the park must carry its own machine code"
    );
    let pending_id = result
        .get("pending_id")
        .and_then(Value::as_str)
        .expect("the park must carry the claim token")
        .to_owned();

    // The parked row is claimable exactly once, by its owner, with the stored
    // call intact.
    let repos = &resources.common.repos;
    match repos
        .guardian_actions
        .claim_pending_action(
            &pending_id,
            &user_id.to_string(),
            &tenant.to_string(),
            "denied",
        )
        .await
        .expect("claim runs")
    {
        ClaimOutcome::Claimed(action) => {
            assert_eq!(action.tool_name, "disconnect_provider");
            assert_eq!(action.arguments, json!({"provider": "strava"}));
            assert_eq!(action.deny_reason, "tainted_sink");
        }
        other => panic!("expected Claimed, got {other:?}"),
    }
    // Replay: consumed rows are indistinguishable from unknown ids.
    assert!(matches!(
        repos
            .guardian_actions
            .claim_pending_action(
                &pending_id,
                &user_id.to_string(),
                &tenant.to_string(),
                "denied"
            )
            .await
            .expect("claim runs"),
        ClaimOutcome::NotFound
    ));
}

#[tokio::test]
async fn deny_handler_consumes_the_row_and_wrong_user_probes_see_nothing() {
    set_confirm_env();
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let repos = &resources.common.repos;

    let action = pending_action(user_id, tenant, "disconnect_provider");
    repos
        .guardian_actions
        .create_pending_action(&action, Utc::now() + Duration::minutes(5))
        .await
        .expect("park row");

    // A different user probing the id gets the anti-enumeration not-found.
    let stranger = command_ctx(&resources, Uuid::new_v4(), tenant, &action.id);
    let resp = DenyHandler.execute(&stranger).await.expect("deny runs");
    let registry = MessagingStringsRegistry::new();
    assert_eq!(
        resp.text,
        registry.render(KEY_GUARDIAN_CONFIRM_NOT_FOUND, "en", &[])
    );

    // The owner denies: localized ack, row consumed.
    let owner = command_ctx(&resources, user_id, tenant, &action.id);
    let resp = DenyHandler.execute(&owner).await.expect("deny runs");
    assert_eq!(resp.text, EN_GUARDIAN_CONFIRM_DENIED);
    assert!(matches!(
        repos
            .guardian_actions
            .claim_pending_action(
                &action.id,
                &user_id.to_string(),
                &tenant.to_string(),
                "denied"
            )
            .await
            .expect("claim runs"),
        ClaimOutcome::NotFound
    ));
}

#[tokio::test]
async fn confirm_handler_re_dispatches_and_expiry_is_checked_at_resolution() {
    set_confirm_env();
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let repos = &resources.common.repos;
    let registry = MessagingStringsRegistry::new();

    // Expired row: the claim marks it expired and says so.
    let stale = pending_action(user_id, tenant, "disconnect_provider");
    repos
        .guardian_actions
        .create_pending_action(&stale, Utc::now() - Duration::minutes(1))
        .await
        .expect("park stale row");
    let ctx = command_ctx(&resources, user_id, tenant, &stale.id);
    let resp = ConfirmHandler.execute(&ctx).await.expect("confirm runs");
    assert_eq!(
        resp.text,
        registry.render(KEY_GUARDIAN_CONFIRM_EXPIRED, "en", &[])
    );

    // Fresh row: /confirm claims it and re-dispatches the stored call through
    // the chokepoint. The tool's own outcome decides done-vs-failed; either
    // way the row is consumed and the reply is the localized deterministic
    // string naming the tool — never raw tool output.
    let fresh = pending_action(user_id, tenant, "get_athlete");
    repos
        .guardian_actions
        .create_pending_action(&fresh, Utc::now() + Duration::minutes(5))
        .await
        .expect("park fresh row");
    let ctx = command_ctx(&resources, user_id, tenant, &fresh.id);
    let resp = ConfirmHandler.execute(&ctx).await.expect("confirm runs");
    let done = registry.render(KEY_GUARDIAN_CONFIRM_DONE, "en", &["get_athlete"]);
    let failed = registry.render(KEY_GUARDIAN_CONFIRM_FAILED, "en", &["get_athlete"]);
    assert!(
        resp.text == done || resp.text == failed,
        "confirm must reply with the localized done/failed string, got: {}",
        resp.text
    );
    assert!(matches!(
        repos
            .guardian_actions
            .claim_pending_action(
                &fresh.id,
                &user_id.to_string(),
                &tenant.to_string(),
                "confirmed"
            )
            .await
            .expect("claim runs"),
        ClaimOutcome::NotFound
    ));

    // Missing argument gets the same anti-enumeration reply.
    let ctx = command_ctx(&resources, user_id, tenant, "");
    let resp = ConfirmHandler.execute(&ctx).await.expect("confirm runs");
    assert_eq!(
        resp.text,
        registry.render(KEY_GUARDIAN_CONFIRM_NOT_FOUND, "en", &[])
    );
}
