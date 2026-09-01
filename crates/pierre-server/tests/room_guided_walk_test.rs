// ABOUTME: Room guided walks — /calibrate in a shared room binds its subject, and the withhold follows
// ABOUTME: Content-asserting on real rows: state under the channel tenant, subject binding, predicate scope
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A guided interview started in a shared room (carnet#44, narrowed scope):
//! the athlete types `/calibrate` in the room, the walk binds to them alone,
//! and the exchange is room-visible. The fixture mirrors production tenancy
//! exactly — the room's conversation row is the WALKER's own row filed under
//! the CHANNEL tenant, while the walker's data lives under their home tenant.
//!
//! What must hold, each load-bearing:
//!
//! 1. **Activation writes where the row lives.** The handler updates the
//!    conversation under `conversation_tenant_id`; writing under the caller's
//!    own tenant matches zero rows and the interview never starts (the exact
//!    silent failure the pre-room handler would have had in a room).
//! 2. **The state binds subject and audience.** A default-audience state walks
//!    DM-only topics room-visibly; an unbound one advances on anyone's turn.
//! 3. **The withhold follows the walker, and only the walker.** The predicate
//!    must see the walk through the conversation-tenant fallback — the
//!    execute-refusal path runs under the walker's HOME tenant, where the
//!    room's row does not resolve — and must NOT withhold from other members.

use anyhow::Result;
use pierre_commands::calibration::CalibrateHandler;
use pierre_commands::{CommandHandler, PlatformCommandContext};
use pierre_core::models::{GuidedFlow, OnboardingState, TenantId, WalkAudience};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_runtime_context::CoachesCtx;
use pierre_tool_runtime::implementations::guided_flow::guided_flow_is_active;
use std::sync::Arc;
use uuid::Uuid;

mod common;

struct RoomFixture {
    resources: Arc<ServerContext>,
    /// The athlete who types `/calibrate` in the room.
    walker_id: Uuid,
    /// The walker's own tenant — where their data lives and their tools run.
    walker_tenant: TenantId,
    /// A second member (the watching human coach).
    coach_id: Uuid,
    coach_tenant: TenantId,
    /// The channel/bot tenant that owns the room's conversation rows.
    channel_tenant: TenantId,
    /// The walker's own conversation row for the room, under `channel_tenant`.
    conversation_id: String,
}

/// Two linked members with their own tenants, plus a third tenant standing in
/// for the channel owner, and the walker's per-member room conversation filed
/// under it — the shape `resolve_linked_session` produces for a group chat.
async fn setup() -> Result<RoomFixture> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;

    let walker_email = format!("room_walker_{}@example.com", Uuid::new_v4());
    let (walker_id, _) =
        common::create_test_user_with_email(resources.database(), &walker_email).await?;
    let coach_email = format!("room_coach_{}@example.com", Uuid::new_v4());
    let (coach_id, _) =
        common::create_test_user_with_email(resources.database(), &coach_email).await?;
    let bot_email = format!("room_bot_{}@example.com", Uuid::new_v4());
    let (bot_owner_id, _) =
        common::create_test_user_with_email(resources.database(), &bot_email).await?;

    let tenants = resources.common.repos.tenants.get_all().await?;
    let owned = |user: Uuid| {
        tenants
            .iter()
            .find(|t| t.owner_user_id == user)
            .map(|t| t.id)
            .expect("every created user owns a tenant")
    };
    let walker_tenant = owned(walker_id);
    let coach_tenant = owned(coach_id);
    let channel_tenant = owned(bot_owner_id);

    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &walker_id.to_string(),
            channel_tenant,
            "Messaging: telegram",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await?;

    Ok(RoomFixture {
        resources,
        walker_id,
        walker_tenant,
        coach_id,
        coach_tenant,
        channel_tenant,
        conversation_id: conversation.id,
    })
}

/// The command context a messaging room turn hands the handler: the caller's
/// own tenant for their data, the channel tenant for the conversation row.
fn room_ctx(fix: &RoomFixture) -> PlatformCommandContext {
    PlatformCommandContext {
        user_id: fix.walker_id,
        tenant_id: fix.walker_tenant,
        channel_type: "telegram".to_owned(),
        args: vec![],
        raw_text: "/calibrate".to_owned(),
        ctx: Arc::<ServerContext>::clone(&fix.resources)
            as Arc<dyn pierre_runtime_context::CommandCtx>,
        locale: "en".to_owned(),
        is_direct_message: false,
        ambient_group_fallback: true,
        conversation_id: Some(fix.conversation_id.clone()),
        conversation_tenant_id: fix.channel_tenant,
        sender_id: Some("tg-walker".to_owned()),
        tool_runtime: Arc::<ServerContext>::clone(&fix.resources),
    }
}

#[tokio::test]
async fn calibrate_in_a_room_binds_subject_audience_and_conversation_tenant() -> Result<()> {
    let fix = setup().await?;

    let response = CalibrateHandler.execute(&room_ctx(&fix)).await?;
    assert!(
        response.text.contains("visible to everyone"),
        "the room opener must state the exchange is room-visible, got: {}",
        response.text
    );

    // The state landed on the row the channel tenant owns — the walker's own
    // tenant holds no such conversation at all.
    assert!(
        fix.resources
            .common
            .repos
            .chat
            .get_conversation(
                &fix.conversation_id,
                &fix.walker_id.to_string(),
                fix.walker_tenant
            )
            .await?
            .is_none(),
        "fixture: the room row must not resolve under the walker's own tenant"
    );
    let conv = fix
        .resources
        .common
        .repos
        .chat
        .get_conversation(
            &fix.conversation_id,
            &fix.walker_id.to_string(),
            fix.channel_tenant,
        )
        .await?
        .expect("the room conversation resolves under the channel tenant");
    let state = OnboardingState::from_column(conv.onboarding_state.as_deref()).expect(
        "the interview must be active — an activation under the wrong tenant matches no row",
    );
    assert_eq!(state.flow, GuidedFlow::Calibration);
    assert_eq!(
        state.subject_user_id.as_deref(),
        Some(fix.walker_id.to_string().as_str()),
        "the walk must bind the walker as its subject"
    );
    assert_eq!(
        state.audience,
        WalkAudience::Room,
        "the walk must record the room audience the athlete consented to"
    );

    // The opener is durable history on the same row, so the walker's first
    // answer is message #2 and the room transcript shows the question.
    let history = fix
        .resources
        .common
        .repos
        .chat
        .get_messages(
            &fix.conversation_id,
            &fix.walker_id.to_string(),
            fix.channel_tenant,
        )
        .await?;
    assert_eq!(history.len(), 1, "exactly the opener should be persisted");
    assert_eq!(history[0].role, "assistant");
    assert_eq!(history[0].content, response.text);
    Ok(())
}

#[tokio::test]
async fn the_withhold_binds_the_walking_member_alone() -> Result<()> {
    let fix = setup().await?;
    CalibrateHandler.execute(&room_ctx(&fix)).await?;
    let repos = &fix.resources.common.repos;

    let conv = repos
        .chat
        .get_conversation(
            &fix.conversation_id,
            &fix.walker_id.to_string(),
            fix.channel_tenant,
        )
        .await?
        .expect("room conversation");

    // With the conversation in hand: withheld for the walker, free for the
    // watching coach — refusing the coach a plan save because their athlete
    // is calibrating would be the wrong refusal.
    assert!(
        guided_flow_is_active(
            repos,
            Some(&conv),
            None,
            fix.walker_tenant,
            &fix.walker_id.to_string()
        )
        .await?,
        "the walker is mid-interview; save_training_plan must be withheld"
    );
    assert!(
        !guided_flow_is_active(
            repos,
            Some(&conv),
            None,
            fix.coach_tenant,
            &fix.coach_id.to_string()
        )
        .await?,
        "the walk binds its subject alone — other members are not withheld"
    );

    // The execute-refusal path: the tool runs under the walker's HOME tenant,
    // where the room's row does not resolve, and recovers the walk through the
    // conversation-tenant fallback. Before that fallback existed this returned
    // false and the withhold silently never fired on room walks.
    assert!(
        guided_flow_is_active(
            repos,
            None,
            Some((fix.conversation_id.as_str(), fix.channel_tenant)),
            fix.walker_tenant,
            &fix.walker_id.to_string()
        )
        .await?,
        "the conversation-tenant fallback must surface the room walk"
    );
    assert!(
        !guided_flow_is_active(
            repos,
            None,
            Some((fix.conversation_id.as_str(), fix.channel_tenant)),
            fix.coach_tenant,
            &fix.coach_id.to_string()
        )
        .await?,
        "the fallback must not withhold from a member who owns no walk on that row"
    );
    Ok(())
}
