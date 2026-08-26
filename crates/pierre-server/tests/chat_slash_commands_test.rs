// ABOUTME: Integration tests for slash-command handling on the /api/chat/.../messages endpoint
// ABOUTME: Asserts web and mobile chat use the same dispatcher as the messaging channels
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use axum::http::StatusCode;
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::coaches::{CoachCategory, CreateCoachRequest};
use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
use pierre_core::models::{ConnectionType, TenantId};
use pierre_core::models::{OnboardingState, Tenant, User, UserStatus};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{
    ChatMessageAction, ChatRoutes, ConversationResponse, ReplyBlockResponse, TurnResponse,
};

/// A command turn is marked by its finish reason, the same field every other
/// turn reports its outcome on.
const COMMAND_FINISH_REASON: &str = "command";

/// The controls a turn carries, or an empty slice when it carries none.
fn turn_actions(body: &TurnResponse) -> Vec<&ChatMessageAction> {
    body.assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlockResponse::Actions { actions, .. } => Some(actions.iter().collect()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The label above a turn's controls, when it carried one.
fn turn_actions_title(body: &TurnResponse) -> Option<&str> {
    body.assistant.blocks.iter().find_map(|block| match block {
        ReplyBlockResponse::Actions { title, .. } => title.as_deref(),
        _ => None,
    })
}
use serde_json::json;
use std::sync::Arc;
use tokio::task::spawn_blocking;
use uuid::Uuid;

async fn seed_user_tenant(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId, String) {
    let password_hash = spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
        .await
        .unwrap();

    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Chat Cmd Test".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: "Chat Cmd Tenant".to_owned(),
        slug: format!("chat-cmd-{tenant_id}"),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    // tenants.create() adds the owner to tenant_users automatically.
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    // Legacy tenant_id column mirrors tenant_users; keep them in sync
    // for the /api/chat endpoints that still read it.
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();

    // Register a real (non-synthetic) provider so the onboarding gate
    // (the provider gate, removed in Phase 5) doesn't 403
    // before the slash dispatcher even runs. Slash commands don't consume
    // provider data, but the gate fires upstream of dispatch and counts
    // only non-synthetic connections.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();

    (user_id, tenant_id, format!("Bearer {token}"))
}

async fn seed_coach(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    description: &str,
) -> String {
    let request = CreateCoachRequest {
        title: title.to_owned(),
        description: Some(description.to_owned()),
        system_prompt: "You are a test coach.".to_owned(),
        category: CoachCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    let coach = resources
        .common
        .repos
        .coaches
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();
    coach.id.to_string()
}

/// Put `user_id` in a coaching group with the given role, so `/help` resolves a
/// real membership the way the group handlers do.
async fn seed_group_membership(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    coach_id: &str,
    role: GroupRole,
) -> Uuid {
    let now = chrono::Utc::now();
    let group_id = Uuid::new_v4();
    let group = CoachingGroup {
        id: group_id,
        tenant_id: tenant_id.to_string(),
        name: "Help Filter Group".to_owned(),
        description: None,
        coach_id: coach_id.to_owned(),
        // owner_id must reference a real user (FK). It plays no part in the
        // filter either way: `caller_group_standing` reads the membership row's
        // `role`, which is what `role` below sets.
        owner_id: user_id,
        coach_user_id: None,
        peer_data_sharing: false,
        respond_mode: GroupRespondMode::default(),
        max_members: 20,
        is_active: true,
        channel_type: None,
        channel_chat_id: None,
        created_at: now,
        updated_at: now,
    };
    resources
        .common
        .repos
        .groups
        .create_group(tenant_id, &group)
        .await
        .unwrap();

    resources
        .common
        .repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: tenant_id.to_string(),
            role,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();

    group_id
}

/// Send `/help` and return the rendered listing.
async fn fetch_help(router: axum::Router, auth: &str) -> String {
    let conv_id = create_conversation(router.clone(), auth).await;
    help_in_conversation(router, auth, &conv_id).await
}

/// Send `/help` on an existing conversation and return the rendered listing.
///
/// Separate from [`fetch_help`] so a test can bind the conversation to a
/// specific group first, which pins which group `/help` treats as ambient
/// instead of leaving it to `list_groups_for_user`'s `updated_at DESC`.
async fn help_in_conversation(router: axum::Router, auth: &str, conv_id: &str) -> String {
    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", auth)
        .json(&json!({"content": "/help"}))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    body.assistant.message.content
}

async fn create_conversation(router: axum::Router, auth: &str) -> String {
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", auth)
        .json(&json!({
            "title": "Cmd Test",
            "model": "gemini-1.5-flash"
        }))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let conv: ConversationResponse = resp.json();
    conv.id
}

#[tokio::test]
async fn coach_command_returns_card_with_actions_no_llm_call() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "web-chat@test.com").await;
    let _coach = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Activity Analysis Coach",
        "Analyzes training data. What *not* to overlook.",
    )
    .await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/coach"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();

    // Command response marker is set so clients can skip LLM-specific UI.
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    // The card title labels the controls it belongs to rather than being
    // pre-folded onto the front of the body text.
    assert!(
        turn_actions_title(&body).is_some(),
        "expected a title on /coach's actions block"
    );
    // Actions populated with per-coach select buttons.
    let actions = turn_actions(&body);
    assert!(!actions.is_empty(), "expected at least one coach action");
    assert_eq!(actions[0].action_type, "postback");
    assert!(
        actions[0].value.starts_with("/coach select "),
        "action value should be a postback to /coach select, got: {}",
        actions[0].value
    );
    // Markdown emphasis stripped uniformly (same behaviour as messaging channels).
    assert!(
        !body.assistant.message.content.contains('*'),
        "command response must not contain literal markdown asterisks: {}",
        body.assistant.message.content
    );
    // No LLM tokens billed for a command turn.
    assert_eq!(body.telemetry.execution_time_ms, 0);
    assert_eq!(body.telemetry.model, "command");
}

#[tokio::test]
async fn coach_select_in_chat_sets_users_default_coach() {
    // Mirrors the Telegram-DM regression test: calling /coach select
    // from the web chat endpoint must write the selection pointer, the
    // same behaviour the messaging dispatcher gives in a DM.
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "coach-sel@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Polarized Training Coach",
        "Experts in training intensity distribution.",
    )
    .await;

    let before = resources
        .common
        .repos
        .tenants
        .get_selected_coach(tenant_id, user_id)
        .await
        .unwrap();
    assert!(before.is_none(), "nothing selected before the command");

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": format!("/coach select {coach_id}")}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    // DM wording (KEY_COACH_USER_UPDATED) — never "pour le groupe".
    assert!(
        !body
            .assistant
            .message
            .content
            .to_lowercase()
            .contains("group"),
        "web/mobile chat is DM-like; must not mention 'group': {}",
        body.assistant.message.content
    );

    let after = resources
        .common
        .repos
        .tenants
        .get_selected_coach(tenant_id, user_id)
        .await
        .unwrap();
    assert_eq!(after.as_deref(), Some(coach_id.as_str()));
}

#[tokio::test]
async fn unknown_slash_command_short_circuits_before_llm() {
    let resources = create_test_server_resources().await.unwrap();
    let (_uid, _tid, auth) = seed_user_tenant(&resources, "unknown-cmd@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/notarealcommand"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    // No card, no actions — unknown commands render as plain localized text.
    assert!(turn_actions_title(&body).is_none());
    assert!(turn_actions(&body).is_empty());
    assert_eq!(body.telemetry.model, "command");
}

#[tokio::test]
async fn client_platform_header_shapes_channel_type_without_breaking() {
    // Web and mobile currently share the same dispatch semantics, but
    // the X-Client-Platform header still rides through the pipeline for
    // analytics. Smoke-test that mobile flag is accepted (no 400).
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "mobile-cmd@test.com").await;
    let _coach = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Recovery Coach",
        "Rest-day specialist.",
    )
    .await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .header("x-client-platform", "mobile")
        .json(&json!({"content": "/coach"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
}

#[tokio::test]
async fn slash_command_is_not_persisted_to_history() {
    // Web/mobile parity with the messaging channels: a slash command and its
    // account-level output (connected providers, group count, privacy state)
    // must not be written to the durable conversation transcript, where it
    // would persist and bleed into the LLM context on the next turn. The
    // command still executes and its result is returned for display.
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "ephemeral-cmd@test.com").await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/status"}))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );

    // The command turn is ephemeral: nothing landed in the transcript.
    let history = resources
        .common
        .repos
        .chat
        .get_messages(&conv_id, &user_id.to_string(), tenant_id)
        .await
        .unwrap();
    assert!(
        history.is_empty(),
        "slash command must not persist to chat history, found {} message(s)",
        history.len()
    );
}

#[tokio::test]
async fn pillars_command_activates_onboarding_mode_no_llm_call() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "context-cmd@test.com").await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/pillars"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();

    // Deterministic command dispatch — no LLM turn, marked so clients skip
    // streaming UI. The greeting opens the guided walk on the North Star.
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON),
        "/pillars must dispatch as a command"
    );
    assert!(
        body.assistant.message.content.contains("North Star"),
        "expected the onboarding greeting, got: {}",
        body.assistant.message.content
    );

    // The handler flips the conversation into onboarding mode: subsequent
    // (non-command) turns run the guided pillar walk until coverage completes.
    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(&conv_id, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .expect("conversation exists");
    assert!(
        OnboardingState::from_column(conv.onboarding_state.as_deref()).is_some(),
        "/pillars must activate onboarding_state on the conversation"
    );
}

/// An athlete who belongs to no group is shown no group commands.
///
/// Every one of them resolves a group before doing anything, so for this user
/// they can only answer "you are not a member of a group" — listing them
/// teaches nothing and reads as a broken bot.
#[tokio::test]
async fn help_hides_group_commands_from_an_athlete_with_no_group() {
    let resources = create_test_server_resources().await.unwrap();
    let (_uid, _tid, auth) = seed_user_tenant(&resources, "help-nogroup@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let text = fetch_help(router, &auth).await;

    for hidden in [
        "/group consent",
        "/group invite",
        "/group members",
        "/group respond",
        "/group status",
        "/group leave",
        "/group coach",
        "/coach assign",
    ] {
        assert!(
            !text.contains(hidden),
            "`{hidden}` needs a group this athlete does not have:\n{text}"
        );
    }
    // `/group` itself needs no group (it lists yours, possibly none), so the
    // domain survives — with that single line and nothing else.
    let group_lines: Vec<&str> = text
        .lines()
        .filter(|l| l.trim_start().starts_with("/group"))
        .collect();
    assert_eq!(
        group_lines.len(),
        1,
        "only `/group` should survive for a groupless athlete, got {group_lines:?}"
    );
    // Everything that does not need a group is untouched.
    for shown in ["/status", "/plan", "/coach ", "/pillars", "/timezone"] {
        assert!(
            text.contains(shown),
            "`{shown}` must still be listed:\n{text}"
        );
    }
}

/// A plain member sees the member-level group commands and none of the
/// admin-only ones — the four whose handlers check `can_modify_settings` /
/// `can_manage_members` before acting.
#[tokio::test]
async fn help_hides_admin_only_commands_from_a_plain_group_member() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "help-member@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Group Coach",
        "Coaches a group.",
    )
    .await;
    seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Member).await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let text = fetch_help(router, &auth).await;

    for shown in [
        "/group consent",
        "/group members",
        "/group status",
        "/group leave",
    ] {
        assert!(
            text.contains(shown),
            "a member can run `{shown}`, so it must be listed:\n{text}"
        );
    }
    for hidden in [
        "/group invite",
        "/group coach",
        "/group respond",
        "/coach assign",
    ] {
        assert!(
            !text.contains(hidden),
            "`{hidden}` is owner/admin only and must not be listed for a member:\n{text}"
        );
    }
}

/// The group owner sees the admin-only commands the member does not.
#[tokio::test]
async fn help_shows_admin_only_commands_to_a_group_owner() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "help-owner@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Owned Group Coach",
        "Coaches a group.",
    )
    .await;
    seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Owner).await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let text = fetch_help(router, &auth).await;

    for shown in [
        "/group invite",
        "/group coach coach-name",
        "/group respond mentions|all",
        "/coach assign coach-id group-id",
        "/group consent yes|no",
    ] {
        assert!(
            text.contains(shown),
            "an owner can run `{shown}`, so it must be listed:\n{text}"
        );
    }
}

#[tokio::test]
async fn help_shows_argument_options_localized_headings_and_stable_order() {
    // /help is the only discovery surface for command arguments: a reader who
    // cannot see `yes|no` has no way to learn that /group consent takes one.
    // Pins all three properties of the rendered list — argument signatures,
    // localized domain headings, and deterministic ordering.
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "help-args@test.com").await;
    // Owner standing so every command is listed — the signatures, not the
    // role filter, are what this test pins.
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Args Group Coach",
        "Coaches a group.",
    )
    .await;
    seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Owner).await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let text = fetch_help(router, &auth).await;

    // Every command whose handler reads an argument advertises what it takes.
    for signature in [
        "/group consent yes|no",
        "/group respond mentions|all",
        "/group coach coach-name",
        "/plan [week|today]",
        // Named, not a `pillar` placeholder: the athlete must be able to type
        // one of these without knowing the internal DB slugs.
        "/pillars [full|training|fuelling|sleep|mental|community|substances]",
        "/timezone area/city",
        "/coach select coach-id",
        "/coach assign coach-id group-id",
        "/confirm action-id",
        "/deny action-id",
    ] {
        assert!(
            text.contains(signature),
            "/help must show `{signature}`, got:\n{text}"
        );
    }

    // Argument-free commands stay bare — no invented placeholder.
    assert!(
        text.contains("/status — ") && text.contains("/logout — "),
        "argument-free commands must render without a signature:\n{text}"
    );

    // Domain headings are localized (fr is the default locale). A raw domain
    // slug in the output means the heading has no messaging string.
    assert!(
        text.contains("Entraînement:"),
        "training domain heading must be localized:\n{text}"
    );
    assert!(
        !text.contains("training:"),
        "raw domain slug leaked into /help:\n{text}"
    );

    // Commands are sorted within their domain — the registry stores them in a
    // HashMap, so an unsorted render reshuffles on every process start.
    // Compare the command halves, not whole lines: the em-dash separator sorts
    // after letters, so a whole-line sort is not the order commands are in.
    let group_block: Vec<&str> = text
        .lines()
        .filter(|l| l.trim_start().starts_with("/group"))
        .map(|l| l.trim().split(" — ").next().unwrap_or(l))
        .collect();
    assert!(
        group_block.len() >= 7,
        "expected the full /group block, got {group_block:?}"
    );
    let mut sorted = group_block.clone();
    sorted.sort_unstable();
    assert_eq!(
        group_block, sorted,
        "/help must list commands in a stable sorted order"
    );
}

/// `/coach assign` names its own group in the arguments, so an owner of *any*
/// group can run it — even from a room where they are only a plain member.
///
/// `/group invite`, `/group coach` and `/group respond` resolve the
/// conversation's group and check the caller's role there, so the ambient role
/// decides them. `CoachAssignHandler` does not: it reads `get_member` on the
/// group id the caller typed. Deciding it on the ambient role hid a command
/// that works.
#[tokio::test]
async fn help_shows_coach_assign_to_an_owner_who_is_a_member_of_the_ambient_group() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "help-two-groups@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Two Group Coach",
        "Coaches two groups.",
    )
    .await;

    // Owner of one group, plain member of another.
    seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Owner).await;
    let member_group =
        seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Member).await;

    // Bind the conversation to the group where the caller is only a member, so
    // the ambient role is Member no matter how the group list happens to sort.
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;
    resources
        .common
        .repos
        .chat
        .set_conversation_group_id(&conv_id, Some(&member_group.to_string()), tenant_id)
        .await
        .unwrap();

    let text = help_in_conversation(router, &auth, &conv_id).await;

    assert!(
        text.contains("/coach assign coach-id group-id"),
        "an owner of another group can run `/coach assign`, so it must be listed:\n{text}"
    );
    // The ambient-group commands are still filtered on the ambient role — this
    // is what proves the conversation resolved to the member group, and that
    // the fix did not simply widen the filter for everything.
    for hidden in ["/group invite", "/group coach", "/group respond"] {
        assert!(
            !text.contains(hidden),
            "`{hidden}` acts on the ambient group, where this caller is only a member:\n{text}"
        );
    }
}

/// `/group status`, `/group members` and `/group leave` read
/// `list_groups_for_user().first()`, never the conversation's group — so
/// belonging to *any* group is enough to run them.
///
/// Deciding them on the conversation's group hid all three from someone
/// sitting in a room bound to a group they are not a member of, even though
/// the commands would have answered about their own group. This is the same
/// defect class as `/coach assign`, in three more commands, and it is why
/// `/help` asks each handler instead of applying one shared rule to all of
/// them.
#[tokio::test]
async fn help_shows_own_group_commands_when_the_room_belongs_to_another_group() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "help-foreign-room@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Foreign Room Coach",
        "Coaches a group.",
    )
    .await;

    // The caller's own group, which `/group status` would answer about.
    seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Member).await;

    // A second group the caller does NOT belong to, bound to the conversation.
    let (other_id, _, _) = seed_user_tenant(&resources, "help-room-owner@test.com").await;
    let foreign_group =
        seed_group_membership(&resources, other_id, tenant_id, &coach_id, GroupRole::Owner).await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;
    resources
        .common
        .repos
        .chat
        .set_conversation_group_id(&conv_id, Some(&foreign_group.to_string()), tenant_id)
        .await
        .unwrap();

    let text = help_in_conversation(router, &auth, &conv_id).await;

    for shown in ["/group status", "/group members", "/group leave"] {
        assert!(
            text.contains(shown),
            "`{shown}` reads the caller's own group, so it must be listed even \
             though this room belongs to a group they are not in:\n{text}"
        );
    }
    // The commands that really do act on this room's group stay hidden — the
    // caller is not a member of it, so they would refuse.
    for hidden in [
        "/group invite",
        "/group coach",
        "/group respond",
        "/group consent",
    ] {
        assert!(
            !text.contains(hidden),
            "`{hidden}` acts on this room's group, which the caller is not in:\n{text}"
        );
    }
}
