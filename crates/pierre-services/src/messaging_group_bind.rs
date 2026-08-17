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
//! This module owns the *messaging-specific* decisions — which coach seeds
//! the group, which tenant plan applies — and delegates every group write to
//! [`GroupService`], which owns the tier gate, the member clamp and the
//! catalogued `group.created` / `group.joined` emissions. Reaching past the
//! service to the repository (as this module once did) silently skipped both:
//! chat-created groups ignored the tenant's plan and never reached Slack or
//! `PostHog`.
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
//! selected coach; falls back to the first system coach in the
//! tenant. If neither exists, returns `Ok(None)` — the chat operates
//! without group context until a coach exists.

use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::{AuthRepos, CoachRepos};
use pierre_groups::service::ChannelGroupSpec;
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_groups::GroupService;
use tracing::warn;

/// The chat a message arrived in, and the sender to bind to it.
///
/// Bundled rather than passed as loose arguments because every field travels
/// together from the ingress layer down to the group row.
pub struct ChannelChatBinding<'a> {
    /// Tenant that owns the channel — *not* necessarily the sender's home
    /// tenant. Group membership is cross-tenant by design.
    pub tenant_id: TenantId,
    /// Messaging platform: `telegram`, `slack`, `discord`.
    pub channel_type: &'a str,
    /// Platform-native chat identifier.
    pub channel_chat_id: &'a str,
    /// Sender's Dravr user id, as a string.
    pub user_id: &'a str,
    /// Name for the auto-created group (e.g. `"Telegram group -100123456"`);
    /// operators can rename via REST.
    pub chat_title_hint: &'a str,
}

/// Resolve (or auto-create on first sender) the `coaching_group` bound
/// to a messaging chat.
///
/// Returns `Ok(Some(group_id_string))` when a binding exists or was
/// created and the sender is enrolled in it; `Ok(None)` when the chat must
/// stay ungrouped — no coach available to bootstrap, the tenant's plan does
/// not include group coaching, or the group is already at its member cap.
///
/// The `Ok(None)` on a failed enrolment is a privacy requirement, not a
/// convenience: `GroupService::inject_group_context` builds peer context from
/// the conversation's `group_id` without re-checking the requester's
/// membership, so attaching a non-member to the conversation would surface
/// consenting peers' snapshots to someone outside the group.
///
/// Takes narrow `AuthRepos` (for the `users`/`tenants` lookups of the
/// bootstrapping sender's selected coach and plan) plus `CoachRepos` (for the
/// fallback system-coach lookup) instead of the full `RepositoryRegistry`.
///
/// # Errors
///
/// Returns database errors from group / member / coach / user lookups.
pub async fn resolve_or_create_channel_group(
    auth: &AuthRepos,
    coach: &CoachRepos,
    groups: &GroupService,
    binding: &ChannelChatBinding<'_>,
) -> AppResult<Option<String>> {
    let user_uuid = parse_uuid(binding.user_id)?;

    // 1. Existing channel binding — enroll caller if missing, return id.
    if let Some(group) = groups
        .get_group_by_channel(
            binding.tenant_id,
            binding.channel_type,
            binding.channel_chat_id,
        )
        .await?
    {
        if groups.enroll_channel_member(&group, user_uuid).await? {
            return Ok(Some(group.id.to_string()));
        }
        warn!(
            group_id = %group.id,
            channel_type = binding.channel_type,
            channel_chat_id = binding.channel_chat_id,
            "Channel-bound group is full; sender's conversation stays ungrouped"
        );
        return Ok(None);
    }

    // 2. No binding — first sender bootstraps. Pick a coach.
    let mut coach_id_choice = auth
        .tenants
        .get_selected_coach(binding.tenant_id, user_uuid)
        .await?;
    if coach_id_choice.is_none() {
        let system_coaches = coach
            .coaches
            .list_system_coaches(binding.tenant_id)
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

    // First sender = Owner. (We can't reliably detect Telegram/Slack/Discord
    // chat-admin status across all three platforms without per-channel API
    // calls; first-sender = Owner is the channel-agnostic baseline. Operators
    // can transfer ownership via REST PUT /api/groups/{id}/members/{user}/role.)
    let spec = ChannelGroupSpec {
        name: binding.chat_title_hint,
        coach_id: &coach_id,
        channel_type: binding.channel_type,
        channel_chat_id: binding.channel_chat_id,
    };
    let tier_member_cap = resolve_tier_member_cap(auth, binding.tenant_id).await;

    match groups
        .create_channel_group(&spec, user_uuid, binding.tenant_id, tier_member_cap)
        .await
    {
        Ok(created) => Ok(Some(created.id.to_string())),
        // The tenant's plan does not include group coaching at all. Not an
        // ingress failure: the chat answers normally, just without group
        // context, so this is a warn and an ungrouped conversation rather
        // than an error the caller logs on every message.
        //
        // Only `PermissionDenied` — the tier gate — is swallowed. The
        // per-owner group allowance deliberately does not apply to this path
        // (`OwnerGroupLimit::Exempt`), so an `InvalidInput` here would mean
        // something genuinely wrong with the request, and silently
        // un-grouping the chat would hide it.
        Err(e) if e.code == ErrorCode::PermissionDenied => {
            warn!(
                channel_type = binding.channel_type,
                channel_chat_id = binding.channel_chat_id,
                reason = %e.message,
                "Tenant plan does not include group coaching; conversation stays ungrouped"
            );
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// The tenant plan's per-group member cap, as `GroupService` expects it.
///
/// Fails closed: a tenant lookup that errors yields `0`, which
/// `create_channel_group` rejects. An unreadable plan must not grant group
/// coaching by default — that is exactly the bypass this path used to have.
async fn resolve_tier_member_cap(auth: &AuthRepos, tenant_id: TenantId) -> i32 {
    match auth.tenants.get_by_id(tenant_id).await {
        Ok(tenant) => i32::try_from(tier_strategy_for(&tenant.plan).max_members_per_group())
            .unwrap_or(i32::MAX),
        Err(e) => {
            warn!(
                %tenant_id,
                error = %e,
                "Could not read tenant plan for group binding; withholding group coaching"
            );
            0
        }
    }
}
