// ABOUTME: Auto-bind a messaging group chat (Telegram/Slack/Discord) to a coaching_groups row
// ABOUTME: First sender becomes Owner and later senders auto-enroll; a chat connected from the web app admits members only

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Channel-group binding for the messaging ingress path.
//!
//! When a linked user sends a message in a non-DM chat (Telegram group,
//! Slack channel, Discord channel), `resolve_or_create_channel_group`
//! returns the `coaching_groups.id` to attach to the chat conversation.
//!
//! This module owns the *messaging-specific* decisions — which agent seeds
//! the group, which tenant plan applies — and delegates every group write to
//! [`GroupService`], which owns the member clamp and the catalogued
//! `group.created` / `group.joined` emissions. Whether the plan includes group
//! coaching at all is asked here, before creation, through the one gate every
//! creation path shares ([`refuses_group_coaching`]). Reaching past the
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
//!   `/group consent yes` for their data to surface to peers. Every
//!   enrolment carries `coach_sharing_consent = true`, as every join does
//!   (ADR-002: membership is consent to share with the group's coach); a
//!   member revokes it with `/group consent coach no`.
//!
//! - A chat connected to an existing group from the web app (carnet#1683)
//!   is different: it enrols nobody. A sender who is a member, or the
//!   group's coach, gets the group; anyone else stays ungrouped. Once such a
//!   chat is disconnected it creates no group of its own either.
//!
//! - DMs (`is_direct_message == true`) skip the helper entirely — the
//!   caller doesn't invoke this path.
//!
//! Agent selection for the bootstrap row is [`group_default`]: the first
//! sender's selected agent unless it is coach-facing, else the tenant's first
//! athlete-facing system agent — the group's athletes talk to it. If neither
//! exists, returns `Ok(None)` — the chat operates without group context until
//! an agent exists.

use std::sync::Arc;

use pierre_core::errors::AppResult;
use pierre_core::models::groups::{CoachingGroup, UpdateGroupRequest};
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::{AgentRepos, AuthRepos};
use pierre_groups::creation_policy::refuses_group_coaching;
use pierre_groups::service::ChannelGroupSpec;
use pierre_groups::strategies::tier::{tier_strategy_for, GroupTierStrategy};
use pierre_groups::GroupService;
use tracing::warn;
use uuid::Uuid;

use crate::default_agent::group_default;

/// The chat a message arrived in, and the sender to bind to it.
///
/// Bundled rather than passed as loose arguments because every field travels
/// together from the ingress layer down to the group row.
#[derive(Debug)]
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
    /// The chat's own title from the inbound payload (Telegram `chat.title`,
    /// Discord `channel.name`), when the platform sends one.
    pub chat_title: Option<&'a str>,
}

impl ChannelChatBinding<'_> {
    /// The chat's title, when it carries a non-blank one.
    fn title(&self) -> Option<&str> {
        self.chat_title.map(str::trim).filter(|t| !t.is_empty())
    }
}

/// The label a channel-bound group is created under when its chat sends no
/// title: `"{channel} group {chat_id}"`.
///
/// It names the chat by its raw platform id, which means nothing to a reader,
/// so [`resolve_or_create_channel_group`] replaces it with the chat's title
/// the first time one arrives.
#[must_use]
pub fn fallback_group_name(channel_type: &str, channel_chat_id: &str) -> String {
    format!("{channel_type} group {channel_chat_id}")
}

/// Resolve (or auto-create on first sender) the `coaching_group` bound
/// to a messaging chat.
///
/// Returns `Ok(Some(group_id_string))` when a binding exists or was
/// created and the sender is enrolled in it; `Ok(None)` when the chat must
/// stay ungrouped — no agent available to bootstrap, the tenant's plan does
/// not include group coaching, the group is already at its member cap, or the
/// chat was disconnected from its group in the web app.
///
/// A chat connected to a group from the web app is answered for its members
/// (and the group's coach) only: anyone else gets `Ok(None)`, never an
/// enrolment.
///
/// The `Ok(None)` on a failed enrolment is a privacy requirement, not a
/// convenience: `GroupService::inject_group_context` builds peer context from
/// the conversation's `group_id` without re-checking the requester's
/// membership, so attaching a non-member to the conversation would surface
/// consenting peers' snapshots to someone outside the group.
///
/// Takes narrow `AuthRepos` (for the `users`/`tenants` lookups of the
/// bootstrapping sender's selected agent and plan) plus `AgentRepos` (for the
/// fallback system-agent lookup) instead of the full `RepositoryRegistry`.
///
/// # Errors
///
/// Returns database errors from group / member / agent / user lookups.
pub async fn resolve_or_create_channel_group(
    auth: &AuthRepos,
    agent: &AgentRepos,
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
        // A chat connected from the web app enrols nobody: its members came
        // in through invites and their own consent (carnet#1167, #1683).
        if group.chat_connection.is_some() {
            return Ok(groups
                .admits_to_connected_chat(&group, user_uuid)
                .await?
                .then(|| group.id.to_string()));
        }
        adopt_chat_title(groups, &group, binding).await;
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

    // A chat whose connected group was disconnected stays ungrouped: the
    // disconnect would otherwise be undone by the next message.
    if groups
        .repo()
        .is_group_chat_released(
            binding.tenant_id,
            binding.channel_type,
            binding.channel_chat_id,
        )
        .await?
    {
        return Ok(None);
    }

    // 2. No binding — first sender bootstraps. Pick the agent the group's
    //    athletes will talk to; a lookup failure reads as no selection.
    let selected = match auth
        .tenants
        .get_selected_agent(binding.tenant_id, user_uuid)
        .await
    {
        Ok(Some(id)) => agent
            .agents
            .get_by_id(&id, user_uuid, binding.tenant_id)
            .await
            .ok()
            .flatten(),
        _ => None,
    };
    let system = agent
        .agents
        .list_catalogue_system_agents()
        .await
        .unwrap_or_default();
    let Some(agent_id) = group_default(&system, selected.as_ref()) else {
        // No agent available — skip group binding. The conversation runs
        // with the default Pierre prompt; the LLM still answers using
        // only the requesting user's data (no peer leakage risk).
        return Ok(None);
    };

    // First sender = Owner. (We can't reliably detect Telegram/Slack/Discord
    // chat-admin status across all three platforms without per-channel API
    // calls; first-sender = Owner is the channel-agnostic baseline. Operators
    // can transfer ownership via REST PUT /api/groups/{id}/members/{user}/role.)
    let fallback_name = fallback_group_name(binding.channel_type, binding.channel_chat_id);
    let spec = ChannelGroupSpec {
        name: binding.title().unwrap_or(&fallback_name),
        agent_id: &agent_id,
        channel_type: binding.channel_type,
        channel_chat_id: binding.channel_chat_id,
    };
    // The tenant plan decides whether the chat may become a group at all. A
    // plan without group coaching is not an ingress failure: the chat answers
    // normally, just without group context, so it stays ungrouped rather than
    // failing the message.
    let plan_tier = resolve_plan_tier(auth, binding.tenant_id).await;
    let Some(tier_member_cap) = channel_group_member_cap(plan_tier.as_deref(), user_uuid, binding)
    else {
        return Ok(None);
    };

    let created = groups
        .create_channel_group(&spec, user_uuid, binding.tenant_id, tier_member_cap)
        .await?;
    Ok(Some(created.id.to_string()))
}

/// Give an already-bound chat's group its chat title when the group still
/// carries the fallback label.
///
/// Groups created before a platform sent chat titles hold
/// [`fallback_group_name`] for good unless something renames them, and their
/// conversations never reach [`resolve_or_create_channel_group`] again, so the
/// per-message path calls this instead. A chat with no title costs nothing;
/// one with a title costs a single keyed lookup.
///
/// # Errors
///
/// Returns the database error from the group lookup. The rename itself is
/// best-effort and never fails the call.
pub async fn refresh_channel_group_title(
    groups: &GroupService,
    binding: &ChannelChatBinding<'_>,
) -> AppResult<()> {
    if binding.title().is_none() {
        return Ok(());
    }
    if let Some(group) = groups
        .get_group_by_channel(
            binding.tenant_id,
            binding.channel_type,
            binding.channel_chat_id,
        )
        .await?
    {
        adopt_chat_title(groups, &group, binding).await;
    }
    Ok(())
}

/// Replace a group's fallback label with its chat's title, once the chat
/// sends one.
///
/// Only the fallback is replaced: a name that differs from it was chosen by
/// someone, and a chat title never overrides that. Best-effort — a failed
/// rename leaves the old label and never blocks the message.
async fn adopt_chat_title(
    groups: &GroupService,
    group: &CoachingGroup,
    binding: &ChannelChatBinding<'_>,
) {
    let Some(title) = binding.title() else {
        return;
    };
    if group.name != fallback_group_name(binding.channel_type, binding.channel_chat_id) {
        return;
    }
    let request = UpdateGroupRequest {
        name: Some(title.to_owned()),
        description: None,
        agent_id: None,
        peer_data_sharing: None,
        respond_mode: None,
        digest_mode: None,
        is_active: None,
    };
    if let Err(e) = groups
        .update_group(&group.id.to_string(), binding.tenant_id, &request, None)
        .await
    {
        warn!(
            group_id = %group.id,
            error = %e,
            "Could not replace the channel group's fallback name with the chat title"
        );
    }
}

/// The tier of the tenant plan the chat's group would be created under.
///
/// Fails closed: a tenant lookup that errors yields `None`, which keeps the
/// chat ungrouped. An unreadable plan must not grant group coaching by
/// default — that is exactly the bypass this path used to have.
async fn resolve_plan_tier(
    auth: &AuthRepos,
    tenant_id: TenantId,
) -> Option<Arc<dyn GroupTierStrategy>> {
    match auth.tenants.get_by_id(tenant_id).await {
        Ok(tenant) => Some(tier_strategy_for(&tenant.plan)),
        Err(e) => {
            warn!(
                %tenant_id,
                error = %e,
                "Could not read tenant plan for group binding; withholding group coaching"
            );
            None
        }
    }
}

/// The member cap a new channel group is created under, or `None` when the
/// chat stays ungrouped: the plan could not be read, or it includes no group
/// coaching.
///
/// Whether the plan includes group coaching is
/// [`refuses_group_coaching`]'s decision, the one every creation path asks;
/// it logs the refusal against the chat's channel.
fn channel_group_member_cap(
    plan_tier: Option<&dyn GroupTierStrategy>,
    owner_id: Uuid,
    binding: &ChannelChatBinding<'_>,
) -> Option<i32> {
    let plan_tier = plan_tier?;
    if refuses_group_coaching(plan_tier, owner_id, binding.tenant_id, binding.channel_type) {
        warn!(
            channel_type = binding.channel_type,
            channel_chat_id = binding.channel_chat_id,
            "Tenant plan does not include group coaching; conversation stays ungrouped"
        );
        return None;
    }
    Some(i32::try_from(plan_tier.max_members_per_group()).unwrap_or(i32::MAX))
}

#[cfg(test)]
mod tests {
    use pierre_core::usage_log;
    use pierre_groups::creation_policy::GROUP_COACHING_PLAN_LIMIT;
    use pierre_groups::strategies::tier::{ProfessionalTierStrategy, StarterTierStrategy};
    use pierre_test_support::group_tier::NoGroupCoachingTier;
    use pierre_test_support::log_capture::{capture_target, drain};

    use super::*;

    fn chat(tenant_id: TenantId) -> ChannelChatBinding<'static> {
        ChannelChatBinding {
            tenant_id,
            channel_type: "telegram",
            channel_chat_id: "-100900",
            user_id: "unused",
            chat_title: Some("Plan Chat"),
        }
    }

    /// A plan without group coaching leaves the chat ungrouped, and the
    /// refusal is logged against the chat's channel.
    #[test]
    fn a_plan_without_group_coaching_keeps_the_chat_ungrouped() {
        let (events, _guard) = capture_target(usage_log::TARGET);
        let owner_id = Uuid::new_v4();
        let tenant_id = TenantId::generate();

        assert_eq!(
            channel_group_member_cap(Some(&NoGroupCoachingTier), owner_id, &chat(tenant_id)),
            None,
            "no group is created for a plan without group coaching"
        );

        let lines = drain(&events);
        assert_eq!(lines.len(), 1, "one usage line per refusal: {lines:?}");
        let field = |name: &str| lines[0].get(name).map(String::as_str);
        assert_eq!(
            field("user_id"),
            Some(owner_id.to_string().as_str()),
            "the line names the sender who would have owned the group"
        );
        assert_eq!(
            field("tenant_id"),
            Some(tenant_id.to_string().as_str()),
            "the line names the chat's tenant"
        );
        assert_eq!(
            field("limit_type"),
            Some(GROUP_COACHING_PLAN_LIMIT),
            "the line names the plan as the limit"
        );
        assert_eq!(
            field("surface"),
            Some("telegram"),
            "the line names the chat's channel"
        );
    }

    /// A plan with group coaching creates the group under its member cap and
    /// logs no limit.
    #[test]
    fn a_plan_with_group_coaching_creates_under_its_member_cap() {
        let (events, _guard) = capture_target(usage_log::TARGET);
        let tenant_id = TenantId::generate();

        assert_eq!(
            channel_group_member_cap(
                Some(&ProfessionalTierStrategy),
                Uuid::new_v4(),
                &chat(tenant_id)
            ),
            Some(2),
            "a Professional group is a coach and one athlete"
        );
        assert_eq!(
            channel_group_member_cap(Some(&StarterTierStrategy), Uuid::new_v4(), &chat(tenant_id)),
            Some(2),
            "a Starter group is a coach and one athlete"
        );
        let lines = drain(&events);
        assert!(
            lines.is_empty(),
            "an admitted plan logs no limit: {lines:?}"
        );
    }

    /// An unreadable plan keeps the chat ungrouped without blaming the plan.
    #[test]
    fn an_unreadable_plan_keeps_the_chat_ungrouped() {
        let (events, _guard) = capture_target(usage_log::TARGET);

        assert_eq!(
            channel_group_member_cap(None, Uuid::new_v4(), &chat(TenantId::generate())),
            None,
            "an unreadable plan must not grant group coaching"
        );
        let lines = drain(&events);
        assert!(
            lines.is_empty(),
            "an unreadable plan is not a plan limit, so no usage line: {lines:?}"
        );
    }
}
