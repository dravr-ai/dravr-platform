// ABOUTME: Asserts group/coach notify events fire from the chat paths, not just HTTP routes
// ABOUTME: Plus the tier gate and member clamp the messaging auto-bind used to reach past

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression tests for dravr-carnet#27.
//!
//! `group.created`, `group.joined` and `coach.selected` were emitted from the
//! HTTP route handlers, so the messaging paths — which carry effectively all
//! real traffic — emitted nothing. The events had never fired once in
//! `PostHog`, which read as "nobody uses groups" while the operator was
//! creating groups over Telegram daily.
//!
//! These tests drive the *chat* entry points, not the routes, and assert on
//! the emitted event's fields. A regression that moved an emission back onto a
//! transport would leave these silent.
//!
//! The same shortcut skipped `GroupService`'s tier gate, so the auto-bind path
//! is also asserted to honour the tenant plan's member cap.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user_with_plan};
use helpers::notify_capture::{capture_notify, named, only};
use pierre_commands::coach::CoachAddHandler;
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_core::errors::ErrorCode;
use pierre_core::models::coaches::CreateCoachRequest;
use pierre_core::models::groups::CreateGroupRequest;
use pierre_core::models::TenantId;
use pierre_groups::service::ChannelGroupSpec;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::coach_selection::{record_coach_selection, CoachSelectionSource};
use pierre_services::messaging_group_bind::{resolve_or_create_channel_group, ChannelChatBinding};
use serde_json::json;
use uuid::Uuid;

// ============================================================================
// Fixtures
// ============================================================================

/// A tenant on `plan`, a user in it, and a coach that user has selected —
/// everything the chat auto-bind needs to bootstrap a group.
async fn chat_fixture(email: &str, plan: &str) -> (Arc<ServerContext>, Uuid, TenantId, String) {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, _user, tenant_id) = create_test_user_with_plan(&res.coach.database, email, plan)
        .await
        .unwrap();

    let request: CreateCoachRequest = serde_json::from_value(json!({
        "title": "Chat Coach",
        "description": null,
        "system_prompt": "You coach over chat.",
    }))
    .unwrap();
    let coach = res
        .common
        .repos
        .coaches
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();
    let coach_id = coach.id.to_string();
    res.common
        .repos
        .tenants
        .set_selected_coach(tenant_id, user_id, Some(&coach_id))
        .await
        .unwrap();

    (res, user_id, tenant_id, coach_id)
}

/// Another user who can send into the same chat.
///
/// Deliberately not added to the group's tenant: group membership is
/// cross-tenant by design, and the chat binding enrols on user id alone.
async fn second_sender(res: &Arc<ServerContext>, email: &str) -> Uuid {
    let (user_id, _user, _own_tenant) =
        create_test_user_with_plan(&res.coach.database, email, "professional")
            .await
            .unwrap();
    user_id
}

fn binding<'a>(
    tenant_id: TenantId,
    chat_id: &'a str,
    user_id: &'a str,
    title: &'a str,
) -> ChannelChatBinding<'a> {
    ChannelChatBinding {
        tenant_id,
        channel_type: "telegram",
        channel_chat_id: chat_id,
        user_id,
        chat_title_hint: title,
    }
}

// ============================================================================
// group.created — the Telegram auto-bind path
// ============================================================================

#[tokio::test]
async fn telegram_auto_bind_emits_group_created() {
    let (res, user_id, tenant_id, _coach) =
        chat_fixture("autobind-created@test.com", "professional").await;
    let auth = res.common.repos.auth_repos();
    let coach = res.common.repos.coach_repos();
    let user_str = user_id.to_string();

    let (events, _guard) = capture_notify();
    let group_id = resolve_or_create_channel_group(
        &auth,
        &coach,
        res.group_service(),
        &binding(tenant_id, "-100777", &user_str, "Sunday Ride"),
    )
    .await
    .unwrap()
    .expect("auto-bind creates a group");

    let created = only(&events, "group.created");
    assert_eq!(created.field("group_id"), group_id);
    assert_eq!(created.field("user_id"), user_str);
    assert_eq!(created.field("tenant_id"), tenant_id.to_string());

    // The owner's membership is implied by group.created; a second event for
    // the same person would double-count group adoption.
    assert!(
        named(&events, "group.joined").is_empty(),
        "bootstrapping owner must not also emit group.joined"
    );
}

#[tokio::test]
async fn auto_bound_group_carries_the_chat_binding_and_tier_cap() {
    let (res, user_id, tenant_id, coach_id) =
        chat_fixture("autobind-fields@test.com", "professional").await;
    let auth = res.common.repos.auth_repos();
    let coach = res.common.repos.coach_repos();
    let user_str = user_id.to_string();

    let group_id = resolve_or_create_channel_group(
        &auth,
        &coach,
        res.group_service(),
        &binding(tenant_id, "-100888", &user_str, "Thursday Track"),
    )
    .await
    .unwrap()
    .unwrap();

    let group = res
        .group_service()
        .get_group(&group_id, tenant_id)
        .await
        .unwrap()
        .expect("group persisted");

    assert_eq!(group.name, "Thursday Track");
    assert_eq!(group.channel_type.as_deref(), Some("telegram"));
    assert_eq!(group.channel_chat_id.as_deref(), Some("-100888"));
    assert_eq!(group.coach_id, coach_id);
    assert_eq!(group.owner_id, user_id);
    // Professional caps a group at 10 members. Before the auto-bind path went
    // through GroupService it wrote a hardcoded 20, ignoring the plan.
    assert_eq!(group.max_members, 10);
}

// ============================================================================
// group.joined — the auto-enrol path
// ============================================================================

#[tokio::test]
async fn second_chat_sender_emits_group_joined() {
    let (res, owner_id, tenant_id, _coach) =
        chat_fixture("autobind-joined@test.com", "professional").await;
    let auth = res.common.repos.auth_repos();
    let coach = res.common.repos.coach_repos();
    let owner_str = owner_id.to_string();

    let group_id = resolve_or_create_channel_group(
        &auth,
        &coach,
        res.group_service(),
        &binding(tenant_id, "-100999", &owner_str, "Club Chat"),
    )
    .await
    .unwrap()
    .unwrap();

    let joiner_id = second_sender(&res, "autobind-joiner@test.com").await;
    let joiner_str = joiner_id.to_string();

    // Capture only the second sender's turn, so the bootstrap's events don't
    // pollute the assertion.
    let (events, _guard) = capture_notify();
    let resolved = resolve_or_create_channel_group(
        &auth,
        &coach,
        res.group_service(),
        &binding(tenant_id, "-100999", &joiner_str, "Club Chat"),
    )
    .await
    .unwrap()
    .expect("second sender joins the existing group");

    assert_eq!(resolved, group_id);
    let joined = only(&events, "group.joined");
    assert_eq!(joined.field("group_id"), group_id);
    assert_eq!(joined.field("user_id"), joiner_str);
    assert_eq!(joined.field("tenant_id"), tenant_id.to_string());
    assert!(
        named(&events, "group.created").is_empty(),
        "an existing binding must not re-create the group"
    );
}

#[tokio::test]
async fn returning_sender_emits_nothing() {
    let (res, owner_id, tenant_id, _coach) =
        chat_fixture("autobind-repeat@test.com", "professional").await;
    let auth = res.common.repos.auth_repos();
    let coach = res.common.repos.coach_repos();
    let owner_str = owner_id.to_string();

    let first = resolve_or_create_channel_group(
        &auth,
        &coach,
        res.group_service(),
        &binding(tenant_id, "-100111", &owner_str, "Repeat Chat"),
    )
    .await
    .unwrap()
    .unwrap();

    let (events, _guard) = capture_notify();
    let second = resolve_or_create_channel_group(
        &auth,
        &coach,
        res.group_service(),
        &binding(tenant_id, "-100111", &owner_str, "Repeat Chat"),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(first, second);
    assert!(
        events.lock().unwrap().is_empty(),
        "an already-enrolled sender must emit nothing on every subsequent message"
    );
}

// ============================================================================
// The tier gate the auto-bind path used to reach past
// ============================================================================

#[tokio::test]
async fn tier_gate_refuses_group_creation_and_emits_nothing() {
    let (res, user_id, tenant_id, coach_id) =
        chat_fixture("autobind-gated@test.com", "professional").await;

    let (events, _guard) = capture_notify();
    // A cap of 0 is how a plan disables group coaching. Driven directly
    // because no shipping tier currently sets it — the gate is the mechanism,
    // and it must stay wired whichever tiers use it.
    let refused = res
        .group_service()
        .create_channel_group(
            &ChannelGroupSpec {
                name: "Gated Chat",
                coach_id: &coach_id,
                channel_type: "telegram",
                channel_chat_id: "-100222",
            },
            user_id,
            tenant_id,
            0,
        )
        .await;

    let err = refused.expect_err("cap 0 must reject group creation");
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert!(
        events.lock().unwrap().is_empty(),
        "a refused creation must not emit group.created"
    );
}

#[tokio::test]
async fn a_full_group_leaves_the_sender_ungrouped() {
    let (res, owner_id, tenant_id, coach_id) =
        chat_fixture("autobind-full@test.com", "professional").await;
    let auth = res.common.repos.auth_repos();
    let coach = res.common.repos.coach_repos();

    // A 2-member group (the floor) that already holds its owner.
    res.group_service()
        .create_channel_group(
            &ChannelGroupSpec {
                name: "Tiny Chat",
                coach_id: &coach_id,
                channel_type: "telegram",
                channel_chat_id: "-100333",
            },
            owner_id,
            tenant_id,
            2,
        )
        .await
        .unwrap();

    let second = second_sender(&res, "autobind-full-2@test.com").await;
    let third = second_sender(&res, "autobind-full-3@test.com").await;
    let second_str = second.to_string();
    let third_str = third.to_string();

    // Second sender fills the group.
    assert!(resolve_or_create_channel_group(
        &auth,
        &coach,
        res.group_service(),
        &binding(tenant_id, "-100333", &second_str, "Tiny Chat"),
    )
    .await
    .unwrap()
    .is_some());

    let (events, _guard) = capture_notify();
    let overflow = resolve_or_create_channel_group(
        &auth,
        &coach,
        res.group_service(),
        &binding(tenant_id, "-100333", &third_str, "Tiny Chat"),
    )
    .await
    .unwrap();

    // Not merely "no membership" — no group id either. `inject_group_context`
    // reads peer data off the conversation's group_id without re-checking
    // membership, so binding an unenrolled sender would leak consenting
    // peers' snapshots to an outsider.
    assert!(
        overflow.is_none(),
        "a sender who could not be enrolled must not be bound to the group"
    );
    assert!(
        events.lock().unwrap().is_empty(),
        "a refused enrolment must not emit group.joined"
    );
}

// ============================================================================
// The per-owner group allowance, which the chat path deliberately skips
// ============================================================================

/// Routing auto-bind through `GroupService` newly exposed it to the tier's
/// per-owner group *count* allowance, which the repository shortcut had never
/// applied. Since the service runs with a hardcoded `professional` strategy
/// for every tenant, that would have silently un-grouped the 4th Telegram
/// chat of anyone who owns three — with only a log line to say why.
///
/// Adding the bot to a chat is not a request for a new group, so the chat
/// path is exempt; the member cap is the gate that applies there.
#[tokio::test]
async fn chat_auto_bind_is_exempt_from_the_owner_group_allowance() {
    let (res, user_id, tenant_id, coach_id) =
        chat_fixture("autobind-allowance@test.com", "professional").await;
    let auth = res.common.repos.auth_repos();
    let coach = res.common.repos.coach_repos();
    let user_str = user_id.to_string();

    // Spend the owner's whole allowance (professional = 3 groups).
    for n in 0..3 {
        let chat_id = format!("-10044{n}");
        res.group_service()
            .create_channel_group(
                &ChannelGroupSpec {
                    name: "Filler Chat",
                    coach_id: &coach_id,
                    channel_type: "telegram",
                    channel_chat_id: &chat_id,
                },
                user_id,
                tenant_id,
                10,
            )
            .await
            .unwrap();
    }

    // REST still refuses past the allowance — the limit is real, it just does
    // not govern the chat path.
    let refused = res
        .group_service()
        .create_group(
            &CreateGroupRequest {
                name: "Fourth By REST".to_owned(),
                description: None,
                coach_id: coach_id.clone(),
                max_members: None,
            },
            user_id,
            tenant_id,
            10,
        )
        .await
        .expect_err("REST creation past the allowance must still be refused");
    assert_eq!(refused.code, ErrorCode::InvalidInput);

    // The 4th chat still binds, and still emits.
    let (events, _guard) = capture_notify();
    let group_id = resolve_or_create_channel_group(
        &auth,
        &coach,
        res.group_service(),
        &binding(tenant_id, "-100555", &user_str, "Fourth Chat"),
    )
    .await
    .unwrap()
    .expect("auto-bind must not spend the owner's group allowance");

    let created = only(&events, "group.created");
    assert_eq!(created.field("group_id"), group_id);
    assert_eq!(created.field("user_id"), user_str);
}

// ============================================================================
// coach.selected — shared by REST, web chat, slash commands, and messaging
// ============================================================================

#[tokio::test]
async fn coach_selection_emits_from_the_shared_recorder() {
    let (res, user_id, tenant_id, coach_id) =
        chat_fixture("coach-selected@test.com", "professional").await;

    let (events, _guard) = capture_notify();
    let recorded = record_coach_selection(
        res.common.repos.coaches.as_ref(),
        &coach_id,
        user_id,
        tenant_id,
        CoachSelectionSource::Rest,
    )
    .await
    .unwrap();

    assert!(recorded, "selecting a visible coach records usage");
    let selected = only(&events, "coach.selected");
    assert_eq!(selected.field("coach_slug"), coach_id);
    assert_eq!(selected.field("user_id"), user_id.to_string());
    assert_eq!(selected.field("tenant_id"), tenant_id.to_string());
    // The surface is on the event so an explicit pick can be told apart from
    // a conversation re-reporting one the athlete already made.
    assert_eq!(selected.field("source"), "rest");
}

#[tokio::test]
async fn an_invisible_coach_records_nothing_and_emits_nothing() {
    let (res, user_id, tenant_id, _coach) =
        chat_fixture("coach-invisible@test.com", "professional").await;

    let (events, _guard) = capture_notify();
    let recorded = record_coach_selection(
        res.common.repos.coaches.as_ref(),
        &Uuid::new_v4().to_string(),
        user_id,
        tenant_id,
        CoachSelectionSource::Rest,
    )
    .await
    .unwrap();

    assert!(
        !recorded,
        "a coach the tenant cannot see is not a selection"
    );
    assert!(
        events.lock().unwrap().is_empty(),
        "nothing was selected, so coach.selected must not fire"
    );
}

/// `/coach add` in a personal thread is the chat equivalent of picking a coach
/// on Discover, and it was the one selection surface still emitting nothing
/// after the events moved off the REST route — the surface most Dravr users
/// have.
#[tokio::test]
async fn slash_coach_add_emits_coach_selected() {
    let (res, user_id, tenant_id, coach_id) =
        chat_fixture("coach-slash-add@test.com", "professional").await;

    let ctx = PlatformCommandContext {
        user_id,
        tenant_id,
        channel_type: "telegram".to_owned(),
        args: vec![coach_id.clone()],
        raw_text: format!("/coach add {coach_id}"),
        ctx: Arc::<ServerContext>::clone(&res),
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: true,
        conversation_id: None,
        conversation_tenant_id: tenant_id,
        sender_id: None,
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(&res),
    };

    let (events, _guard) = capture_notify();
    CoachAddHandler.execute(&ctx).await.unwrap();

    let selected = only(&events, "coach.selected");
    assert_eq!(selected.field("coach_slug"), coach_id);
    assert_eq!(selected.field("user_id"), user_id.to_string());
    assert_eq!(selected.field("tenant_id"), tenant_id.to_string());
    assert_eq!(selected.field("source"), "slash_command");
}
