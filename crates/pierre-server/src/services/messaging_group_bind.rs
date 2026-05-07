// ABOUTME: Auto-bind a messaging group chat (Telegram/Slack/Discord) to a coaching_groups row
// ABOUTME: First sender becomes Owner; subsequent senders auto-enroll as Member with no peer-sharing consent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Channel-group binding for the messaging ingress path.
//!
//! When a linked user sends a message in a non-DM chat (Telegram group,
//! Slack channel, Discord channel), `resolve_or_create_channel_group`
//! returns the `coaching_groups.id` to attach to the chat conversation.
//!
//! Behavior:
//!
//! - If a `coaching_groups` row already exists for `(tenant, channel_type,
//!   channel_chat_id)`, returns its id and ensures the sender is enrolled
//!   as an active Member (idempotent — no-op if already enrolled).
//!
//! - If no row exists, the *first* sender bootstraps the group as Owner
//!   with `peer_data_sharing = true` (group-level kill switch on, so
//!   individual member consent is the gate) and is auto-enrolled as
//!   Owner with `peer_sharing_consent = false`. Subsequent senders join
//!   as Member with `peer_sharing_consent = false` and must opt in via
//!   `/group consent yes` for their data to surface to peers.
//!
//! - DMs (`is_direct_message == true`) skip the helper entirely — the
//!   caller doesn't invoke this path.
//!
//! Coach selection for the bootstrap row: prefers the user's
//! `default_coach_id`; falls back to the first system coach in the
//! tenant. If neither exists, returns `Ok(None)` — the chat operates
//! without group context until a coach exists.

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRole};
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
use tracing::info;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// Resolve (or auto-create on first sender) the `coaching_group` bound
/// to a messaging chat.
///
/// Returns `Ok(Some(group_id_string))` when a binding exists or was
/// created; `Ok(None)` when no coach is available to bootstrap the group.
///
/// `chat_title_hint` is used as the auto-created group's name (e.g.
/// `"Telegram group -100123456"`); operators can rename via REST.
///
/// # Errors
///
/// Returns database errors from group / member / coach / user lookups.
pub async fn resolve_or_create_channel_group(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel_type: &str,
    channel_chat_id: &str,
    user_id: &str,
    chat_title_hint: &str,
) -> AppResult<Option<String>> {
    let user_uuid = parse_uuid(user_id)?;
    let groups = resources.repos.groups.as_ref();

    // 1. Existing channel binding — enroll caller if missing, return id.
    if let Some(group) = groups
        .get_group_by_channel(tenant_id, channel_type, channel_chat_id)
        .await?
    {
        let group_id_str = group.id.to_string();
        let member = groups.get_member(&group_id_str, user_uuid).await?;
        if member.is_none() {
            let new_member = GroupMember {
                id: Uuid::new_v4(),
                group_id: group.id,
                user_id: user_uuid,
                tenant_id: group.tenant_id.clone(),
                role: GroupRole::Member,
                peer_sharing_consent: false,
                consent_given_at: Utc::now(),
                joined_at: Utc::now(),
                left_at: None,
                display_name: None,
            };
            groups.add_member(&new_member).await?;
            info!(
                group_id = %group.id,
                channel_type,
                channel_chat_id,
                new_user_id = %user_id,
                "Auto-enrolled new sender as Member of existing channel-bound group"
            );
        }
        return Ok(Some(group_id_str));
    }

    // 2. No binding — first sender bootstraps. Pick a coach.
    let user_record = resources.repos.users.get_global(user_uuid).await?;
    let mut coach_id_choice = user_record.and_then(|u| u.default_coach_id);
    if coach_id_choice.is_none() {
        let system_coaches = resources
            .repos
            .coaches
            .list_system_coaches(tenant_id)
            .await
            .unwrap_or_default();
        coach_id_choice = system_coaches.first().map(|c| c.id.to_string());
    }
    let Some(coach_id) = coach_id_choice else {
        // No coach available — skip group binding. The conversation runs
        // with the default Pierre prompt; the LLM still answers using
        // only the requesting user's data (no peer leakage risk).
        return Ok(None);
    };

    let group_uuid = Uuid::new_v4();
    let now = Utc::now();
    let group = CoachingGroup {
        id: group_uuid,
        tenant_id: tenant_id.to_string(),
        name: chat_title_hint.to_owned(),
        description: Some(format!(
            "Auto-created from {channel_type} group chat {channel_chat_id}"
        )),
        coach_id,
        owner_id: user_uuid,
        // Group-level kill switch — TRUE means "individual member
        // consent is the gate", FALSE means admin nuked all sharing.
        // Defaults to TRUE so a member who runs `/group consent yes`
        // immediately starts surfacing their data to peers without an
        // owner intervention. Owner can flip to FALSE in the group
        // settings to disable everyone's sharing in one move.
        peer_data_sharing: true,
        max_members: 20,
        is_active: true,
        channel_type: Some(channel_type.to_owned()),
        channel_chat_id: Some(channel_chat_id.to_owned()),
        created_at: now,
        updated_at: now,
    };
    let created = groups.create_group(tenant_id, &group).await?;

    // First sender = Owner. (We can't reliably detect Telegram/Slack/Discord
    // chat-admin status across all three platforms without per-channel API
    // calls; first-sender = Owner is the channel-agnostic baseline. Operators
    // can transfer ownership via REST PUT /api/groups/{id}/members/{user}/role.)
    let owner_member = GroupMember {
        id: Uuid::new_v4(),
        group_id: created.id,
        user_id: user_uuid,
        tenant_id: created.tenant_id.clone(),
        role: GroupRole::Owner,
        peer_sharing_consent: false,
        consent_given_at: now,
        joined_at: now,
        left_at: None,
        display_name: None,
    };
    groups.add_member(&owner_member).await?;

    info!(
        group_id = %created.id,
        channel_type,
        channel_chat_id,
        owner_user_id = %user_id,
        "Auto-created coaching_group from channel chat (first-sender becomes Owner)"
    );

    Ok(Some(created.id.to_string()))
}
