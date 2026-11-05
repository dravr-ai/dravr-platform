// ABOUTME: Confirm HITL end-to-end — decide/gate outcomes, the parked row, and /confirm·/deny resolution
// ABOUTME: Covers park-at-chokepoint, single-use owner-checked claims, expiry, wrong-user probing, replay

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Guardian Confirm human-in-the-loop flow.
//!
//! Every resource-building test builds its server under
//! `GUARDIAN_MODE=enforce` + `GUARDIAN_TAINTED_DESTRUCTIVE=confirm`, as the
//! server's config carries the overrides startup reads.

use crate::common;

use pierre_core::permissions::scopes::OAuthScope;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use common::{
    create_test_server_resources_with_config, create_test_server_resources_with_llm_and_tools,
    create_test_user, test_server_config_with_guardian,
};
use dravr_tronc::mcp::schema::{Tool as McpToolDefinition, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_chat_pipeline::stages::prompt_builder::build_llm_messages;
use pierre_commands::guardian_confirm::{ConfirmHandler, DenyHandler};
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_config::environment::GuardianSettings;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_GUARDIAN_CONFIRM_DENIED, KEY_GUARDIAN_CONFIRM_DONE,
    KEY_GUARDIAN_CONFIRM_EXPIRED, KEY_GUARDIAN_CONFIRM_FAILED, KEY_GUARDIAN_CONFIRM_NOT_FOUND,
};
use pierre_core::constants::oauth_providers::GARMIN_ATTRIBUTION;
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{ClaimOutcome, PendingGuardianAction};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::ai_scope::{self, Provenance};
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::guardian::{
    Decision, ExternalSendAllowlist, Guardian, GuardianMode, GuardianPolicy, PlanMode,
    TaintedDestructive, TurnKey, TurnState,
};
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::{RuntimeTool, SecurityLabels};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::guardian_e2e_test::AnswersPlainly;

/// What a call saw of its derivation: the policy, the attributions, the member.
type Derivation = (TransportPolicy, BTreeSet<&'static str>, Option<Uuid>);

/// A destructive write reporting the derivation it ran under.
struct ReportsProvenance {
    seen: Arc<Mutex<Option<Derivation>>>,
}

#[async_trait]
impl McpTool<dyn ToolRuntime> for ReportsProvenance {
    fn definition(&self) -> McpToolDefinition {
        McpToolDefinition {
            name: "reports_provenance".to_owned(),
            description: "Reports the derivation it ran under".to_owned(),
            input_schema: json!({ "type": "object" }),
            annotations: None,
            output_schema: None,
            execution: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        let attributions = ai_scope::current_provenance()
            .map(|provenance| provenance.attributions())
            .unwrap_or_default();
        *self.seen.lock().expect("lock") = Some((
            ai_scope::derived_policy(),
            attributions,
            ai_scope::derived_member(),
        ));
        ToolResponse::text("done".to_owned())
    }
}

pierre_tool_runtime::declare_security!(ReportsProvenance => IRREVERSIBLE);

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

/// A server under `GUARDIAN_MODE=enforce` + `GUARDIAN_TAINTED_DESTRUCTIVE=confirm`.
async fn confirming_server() -> Arc<ServerContext> {
    create_test_server_resources_with_config(test_server_config_with_guardian(GuardianSettings {
        mode: Some("enforce".to_owned()),
        tainted_destructive: Some("confirm".to_owned()),
        ..GuardianSettings::default()
    }))
    .await
    .expect("server resources")
}

fn request(tool: &str, args: Value, user_id: Uuid, tenant: TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: args,
        user_id: user_id.to_string(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant.to_string()),
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
        turn_agent_id: None,
        seat: Some(TurnSeat::Subject.as_stored().to_owned()),
        turn_provenance: Provenance::new().to_stored(),
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
        ctx: Arc::<ServerContext>::clone(resources),
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: false,
        conversation_id: None,
        conversation_tenant_id: tenant,
        sender_id: None,
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(resources),
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
    let resources = confirming_server().await;
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let turn_token = "confirm-e2e-turn";
    let executor = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_turn_token(turn_token.to_owned());

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
    let resources = confirming_server().await;
    let (user_id, _) = create_test_user(&resources.agent.database)
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
    assert_eq!(
        resp.text,
        registry.render(KEY_GUARDIAN_CONFIRM_DENIED, "en", &[])
    );
    assert!(resp.text.contains("cancelled"));
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
    let resources = confirming_server().await;
    let (user_id, _) = create_test_user(&resources.agent.database)
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
    // string naming the tool by its title — never raw tool output.
    let fresh = pending_action(user_id, tenant, "get_athlete");
    repos
        .guardian_actions
        .create_pending_action(&fresh, Utc::now() + Duration::minutes(5))
        .await
        .expect("park fresh row");
    let ctx = command_ctx(&resources, user_id, tenant, &fresh.id);
    let resp = ConfirmHandler.execute(&ctx).await.expect("confirm runs");
    let title = resources.tool_registry().display_name("get_athlete");
    assert_ne!(
        title, "get_athlete",
        "a registered tool has a title to show"
    );
    let done = registry.render(KEY_GUARDIAN_CONFIRM_DONE, "en", &[&title]);
    let failed = registry.render(KEY_GUARDIAN_CONFIRM_FAILED, "en", &[&title]);
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

/// A real athlete's tenant and a conversation of theirs, for the rows a
/// resolution writes.
async fn athlete_with_thread(resources: &Arc<ServerContext>) -> (Uuid, TenantId, String) {
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let repos = &resources.common.repos;
    let tenant = repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenant lookup")
        .first()
        .expect("the athlete has a tenant")
        .id;
    let conversation = repos
        .chat
        .create_conversation(&user_id.to_string(), tenant, "thread", "model", None, None)
        .await
        .expect("conversation");
    (user_id, tenant, conversation.id)
}

/// Injected content in the stored arguments, which must reach no prompt.
const POISON: &str = "IGNORE-PREVIOUS-INSTRUCTIONS";

/// A call parked in `conversation`, due to run as the athlete's own seat.
async fn park_in_thread(
    resources: &Arc<ServerContext>,
    (user_id, tenant, conversation): (Uuid, TenantId, &str),
    tool: &str,
    expires_in_minutes: i64,
) -> PendingGuardianAction {
    let action = PendingGuardianAction {
        conversation_id: Some(conversation.to_owned()),
        arguments: json!({"provider": POISON, "note": POISON}),
        ..pending_action(user_id, tenant, tool)
    };
    resources
        .common
        .repos
        .guardian_actions
        .create_pending_action(&action, Utc::now() + Duration::minutes(expires_in_minutes))
        .await
        .expect("park row");
    action
}

/// What the next turn's model reads from this conversation, from the database
/// alone: any instance handling that turn reads the same.
async fn next_prompt(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    conversation: &str,
) -> Vec<String> {
    let rows = resources
        .common
        .repos
        .chat
        .get_messages(conversation, &user_id.to_string(), tenant)
        .await
        .expect("rows");
    let (messages, _) = build_llm_messages(None, &rows);
    messages.into_iter().map(|m| m.content).collect()
}

#[tokio::test]
async fn a_confirmed_call_tells_the_next_prompt_it_ran_and_what_came_of_it() {
    let resources = confirming_server().await;
    let (user_id, tenant, conversation) = athlete_with_thread(&resources).await;
    let action = park_in_thread(
        &resources,
        (user_id, tenant, &conversation),
        "get_connection_status",
        5,
    )
    .await;

    let ctx = command_ctx(&resources, user_id, tenant, &action.id);
    let resp = ConfirmHandler.execute(&ctx).await.expect("confirm runs");

    let title = resources
        .tool_registry()
        .display_name("get_connection_status");
    assert_eq!(
        resp.text,
        MessagingStringsRegistry::new().render(KEY_GUARDIAN_CONFIRM_DONE, "en", &[&title])
    );
    assert_eq!(
        next_prompt(&resources, user_id, tenant, &conversation).await,
        ["[Tool Result for get_connection_status]: \
             {\"athlete\":\"confirmed\",\"ran\":true,\"success\":true}"],
        "one line, naming the tool and its outcome, and nothing the tool returned"
    );
}

#[tokio::test]
async fn a_tool_that_fails_on_confirm_tells_the_next_prompt_without_its_error_text() {
    let resources = confirming_server().await;
    let (user_id, tenant, conversation) = athlete_with_thread(&resources).await;
    // The tool's error can echo the stored arguments, which carry the poison.
    let action = park_in_thread(
        &resources,
        (user_id, tenant, &conversation),
        "disconnect_provider",
        5,
    )
    .await;

    let ctx = command_ctx(&resources, user_id, tenant, &action.id);
    let resp = ConfirmHandler.execute(&ctx).await.expect("confirm runs");

    let title = resources
        .tool_registry()
        .display_name("disconnect_provider");
    assert_eq!(
        resp.text,
        MessagingStringsRegistry::new().render(KEY_GUARDIAN_CONFIRM_FAILED, "en", &[&title])
    );
    assert!(
        !resp.text.contains(POISON),
        "the athlete never reads the arguments: {}",
        resp.text
    );
    let prompt = next_prompt(&resources, user_id, tenant, &conversation).await;
    assert_eq!(
        prompt,
        ["[Tool Result for disconnect_provider]: \
             {\"athlete\":\"confirmed\",\"ran\":true,\"success\":false}"],
        "the failure, and neither the arguments nor the tool's own words"
    );
}

#[tokio::test]
async fn a_denied_or_lapsed_call_tells_the_next_prompt_it_never_ran() {
    let resources = confirming_server().await;
    let (user_id, tenant, conversation) = athlete_with_thread(&resources).await;

    let denied = park_in_thread(
        &resources,
        (user_id, tenant, &conversation),
        "disconnect_provider",
        5,
    )
    .await;
    let ctx = command_ctx(&resources, user_id, tenant, &denied.id);
    DenyHandler.execute(&ctx).await.expect("deny runs");

    let lapsed = park_in_thread(
        &resources,
        (user_id, tenant, &conversation),
        "disconnect_provider",
        -1,
    )
    .await;
    let ctx = command_ctx(&resources, user_id, tenant, &lapsed.id);
    let resp = ConfirmHandler.execute(&ctx).await.expect("confirm runs");
    assert_eq!(
        resp.text,
        MessagingStringsRegistry::new().render(KEY_GUARDIAN_CONFIRM_EXPIRED, "en", &[])
    );

    assert_eq!(
        next_prompt(&resources, user_id, tenant, &conversation).await,
        [
            "[Tool Result for disconnect_provider]: {\"athlete\":\"denied\",\"ran\":false}",
            "[Tool Result for disconnect_provider]: \
             {\"athlete\":\"did_not_answer_in_time\",\"ran\":false}",
        ]
    );
}

#[tokio::test]
async fn a_tool_disabled_since_the_park_does_not_run_on_confirm() {
    let resources = confirming_server().await;
    let (user_id, tenant, conversation) = athlete_with_thread(&resources).await;
    let action = park_in_thread(
        &resources,
        (user_id, tenant, &conversation),
        "get_connection_status",
        5,
    )
    .await;

    resources
        .common
        .repos
        .tool_selection
        .upsert_override(
            tenant,
            "get_connection_status",
            false,
            Some(user_id),
            Some("switched off after the park".to_owned()),
        )
        .await
        .expect("disable the tool");

    let ctx = command_ctx(&resources, user_id, tenant, &action.id);
    let resp = ConfirmHandler.execute(&ctx).await.expect("confirm runs");

    let title = resources
        .tool_registry()
        .display_name("get_connection_status");
    assert_eq!(
        resp.text,
        MessagingStringsRegistry::new().render(KEY_GUARDIAN_CONFIRM_FAILED, "en", &[&title]),
        "the athlete's yes does not outrank the super admin's off switch"
    );
    assert_eq!(
        next_prompt(&resources, user_id, tenant, &conversation).await,
        ["[Tool Result for get_connection_status]: \
             {\"athlete\":\"confirmed\",\"ran\":false}"],
        "the chokepoint refused it, so it did not run"
    );
}

#[tokio::test]
async fn a_confirmed_call_answers_to_the_seat_that_parked_it() {
    let resources = confirming_server().await;
    let (user_id, tenant, conversation) = athlete_with_thread(&resources).await;
    // The same call runs on the athlete's own seat
    // (a_confirmed_call_tells_the_next_prompt_it_ran_and_what_came_of_it). On a
    // coach's seat it reads the coach's own data, so it is withheld.
    let action = PendingGuardianAction {
        seat: Some(TurnSeat::Coach.as_stored().to_owned()),
        conversation_id: Some(conversation.clone()),
        ..pending_action(user_id, tenant, "get_connection_status")
    };
    resources
        .common
        .repos
        .guardian_actions
        .create_pending_action(&action, Utc::now() + Duration::minutes(5))
        .await
        .expect("park row");

    let ctx = command_ctx(&resources, user_id, tenant, &action.id);
    let resp = ConfirmHandler.execute(&ctx).await.expect("confirm runs");

    let title = resources
        .tool_registry()
        .display_name("get_connection_status");
    assert_eq!(
        resp.text,
        MessagingStringsRegistry::new().render(KEY_GUARDIAN_CONFIRM_FAILED, "en", &[&title])
    );
    assert_eq!(
        next_prompt(&resources, user_id, tenant, &conversation).await,
        ["[Tool Result for get_connection_status]: \
             {\"athlete\":\"confirmed\",\"ran\":false}"]
    );
}

#[tokio::test]
async fn a_row_whose_seat_cannot_be_read_does_not_run() {
    let resources = confirming_server().await;
    let (user_id, tenant, conversation) = athlete_with_thread(&resources).await;
    for seat in [None, Some("captain".to_owned())] {
        let action = PendingGuardianAction {
            seat,
            conversation_id: Some(conversation.clone()),
            ..pending_action(user_id, tenant, "get_connection_status")
        };
        resources
            .common
            .repos
            .guardian_actions
            .create_pending_action(&action, Utc::now() + Duration::minutes(5))
            .await
            .expect("park row");
        let ctx = command_ctx(&resources, user_id, tenant, &action.id);
        let resp = ConfirmHandler.execute(&ctx).await.expect("confirm runs");
        let title = resources
            .tool_registry()
            .display_name("get_connection_status");
        assert_eq!(
            resp.text,
            MessagingStringsRegistry::new().render(KEY_GUARDIAN_CONFIRM_FAILED, "en", &[&title]),
            "an unreadable seat must not read as the athlete's own"
        );
    }
    assert_eq!(
        next_prompt(&resources, user_id, tenant, &conversation).await,
        [
            "[Tool Result for get_connection_status]: \
             {\"athlete\":\"confirmed\",\"ran\":false}",
            "[Tool Result for get_connection_status]: \
             {\"athlete\":\"confirmed\",\"ran\":false}",
        ]
    );
}

#[tokio::test]
async fn the_park_records_the_turns_agent_and_seat() {
    let resources = confirming_server().await;
    let (user_id, tenant, _) = athlete_with_thread(&resources).await;
    let turn_token = "confirm-context-turn";
    resources.guardian_turns().record_taint(
        &TurnKey::new(Some(tenant.as_uuid()), turn_token.to_owned()),
        "seeded_source",
        SecurityLabels::UNTRUSTED_OUTPUT,
    );
    // A tool that takes `athlete`, so a coach's athlete thread does not withhold it.
    let parked = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_turn_token(turn_token.to_owned())
        .with_turn_agent(Some("taper-agent".to_owned()))
        .with_seat(TurnSeat::AthleteThread)
        .execute_tool(request("publish_training_plan", json!({}), user_id, tenant))
        .await
        .expect("dispatch returns in-band");
    let pending_id = parked
        .result
        .as_ref()
        .and_then(|result| result.get("pending_id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("the call must park: {:?}", parked.error))
        .to_owned();

    let claim = resources
        .common
        .repos
        .guardian_actions
        .claim_pending_action(
            &pending_id,
            &user_id.to_string(),
            &tenant.to_string(),
            "denied",
        )
        .await
        .expect("claim runs");
    let ClaimOutcome::Claimed(action) = claim else {
        panic!("expected Claimed, got {claim:?}");
    };
    assert_eq!(action.turn_agent_id.as_deref(), Some("taper-agent"));
    assert_eq!(
        action.seat.as_deref().and_then(TurnSeat::from_stored),
        Some(TurnSeat::AthleteThread)
    );
}

/// Confirm a parked call of [`ReportsProvenance`] whose row holds `turn_provenance`,
/// and return what the tool saw.
async fn derivation_on_confirm(turn_provenance: Option<String>) -> Derivation {
    let seen = Arc::new(Mutex::new(None));
    let tool: Arc<dyn RuntimeTool> = Arc::new(ReportsProvenance {
        seen: Arc::clone(&seen),
    });
    let provider = Arc::new(AnswersPlainly {
        models: vec!["plain-model".to_owned()],
    });
    let resources = create_test_server_resources_with_llm_and_tools(provider, vec![tool])
        .await
        .expect("server resources");
    let (user_id, tenant, _) = athlete_with_thread(&resources).await;
    let action = PendingGuardianAction {
        turn_provenance,
        ..pending_action(user_id, tenant, "reports_provenance")
    };
    resources
        .common
        .repos
        .guardian_actions
        .create_pending_action(&action, Utc::now() + Duration::minutes(5))
        .await
        .expect("park row");

    ConfirmHandler
        .execute(&command_ctx(&resources, user_id, tenant, &action.id))
        .await
        .expect("confirm answers");

    let observed = seen.lock().expect("lock").take();
    observed.expect("the confirmed call must run")
}

/// carnet#1398: the park records what the turn had served, and the confirm runs the
/// call inside it.
#[tokio::test]
async fn a_call_parked_by_a_first_party_only_turn_is_first_party_only_when_confirmed() {
    let resources = confirming_server().await;
    let (user_id, tenant, _) = athlete_with_thread(&resources).await;
    let turn_token = "first-party-turn";
    resources.guardian_turns().record_taint(
        &TurnKey::new(Some(tenant.as_uuid()), turn_token.to_owned()),
        "seeded_source",
        SecurityLabels::UNTRUSTED_OUTPUT,
    );
    let member = Uuid::new_v4();
    let turn = Provenance::new();
    turn.mark();
    turn.attribute(GARMIN_ATTRIBUTION);
    let parked = ai_scope::tracking(turn, async {
        ai_scope::note_member_read(member);
        UniversalToolExecutor::new(resources.clone())
            .with_scopes(OAuthScope::self_grant())
            .with_turn_token(turn_token.to_owned())
            .execute_tool(request("publish_training_plan", json!({}), user_id, tenant))
            .await
    })
    .await
    .expect("dispatch returns in-band");
    let pending_id = parked
        .result
        .as_ref()
        .and_then(|result| result.get("pending_id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("the call must park: {:?}", parked.error))
        .to_owned();
    let claim = resources
        .common
        .repos
        .guardian_actions
        .claim_pending_action(
            &pending_id,
            &user_id.to_string(),
            &tenant.to_string(),
            "denied",
        )
        .await
        .expect("claim runs");
    let ClaimOutcome::Claimed(action) = claim else {
        panic!("expected Claimed, got {claim:?}");
    };

    let (policy, attributions, about) = derivation_on_confirm(action.turn_provenance.clone()).await;

    assert_eq!(policy, TransportPolicy::FirstPartyOnly);
    assert_eq!(attributions, BTreeSet::from([GARMIN_ATTRIBUTION]));
    assert_eq!(about, Some(member));
}

#[tokio::test]
async fn a_call_parked_by_a_clear_turn_stays_clear_when_confirmed() {
    let (policy, attributions, about) = derivation_on_confirm(Provenance::new().to_stored()).await;
    assert_eq!(policy, TransportPolicy::AnyTransport);
    assert!(attributions.is_empty());
    assert_eq!(about, None);
}

#[tokio::test]
async fn a_null_or_unreadable_provenance_fails_closed_when_confirmed() {
    for stored in [None, Some("{not json".to_owned())] {
        let (policy, _, _) = derivation_on_confirm(stored.clone()).await;
        assert_eq!(
            policy,
            TransportPolicy::FirstPartyOnly,
            "{stored:?} must read as first-party-only served"
        );
    }
}
