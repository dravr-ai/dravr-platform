// ABOUTME: Central group coaching service coordinating strategies and repository access
// ABOUTME: Contains inject_group_context — the single function that makes all surfaces group-aware
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;
use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::groups::{
    CoachingGroup, CreateGroupRequest, GroupAggregateStats, GroupContext, GroupHealthFlag,
    GroupInvite, GroupInviteKind, GroupMember, GroupRespondMode, GroupRole, GroupSummary,
    GroupTrend, GroupWeeklyReport, HealthFlagSeverity, MemberFitnessSnapshot, MemberFlag,
    MemberGroupComparison, MemberSummaryCard, OvertrainingRiskLevel, UpdateGroupRequest,
};
use pierre_core::models::FormBand;
use pierre_core::models::TenantId;
use pierre_database::repositories::CoachingGroupRepository;
use tracing::{info, warn};
use uuid::Uuid;

// ============================================================================
// Health Flag Thresholds
// ============================================================================

/// Days without activity before a member is flagged as inactive
pub const INACTIVITY_DAYS_THRESHOLD: i32 = 7;

/// Weekly volume drop percentage (0.0–1.0) from baseline to trigger a flag
pub const VOLUME_DROP_FRACTION_THRESHOLD: f64 = 0.30;

/// Weekly trend threshold: volume increase above this fraction = "improving"
pub const TREND_IMPROVING_FRACTION: f64 = 0.05;

/// Weekly trend threshold: volume decrease beyond this fraction = "declining"
pub const TREND_DECLINING_FRACTION: f64 = 0.05;

// ============================================================================
// Group Size Bounds
// ============================================================================

/// Smallest coachable group — a group of one is a DM, so creation clamps up
/// to this floor regardless of what the caller requested.
pub const MIN_MEMBERS: i32 = 2;

/// Member count requested when the caller names none.
///
/// Applies to a REST create with `max_members` omitted and to every chat
/// auto-bind, which never asks for a size. The tenant tier's cap clamps this
/// down, so it is a ceiling to aim at rather than a grant.
pub const DEFAULT_REQUESTED_MEMBERS: i32 = 20;

/// The chat-side fields that distinguish a channel-bound coaching group from
/// one created through REST.
///
/// Everything else about creation — tier gate, member clamp, owner
/// membership, the `group.created` emission — is shared, so this carries only
/// what actually differs.
pub struct ChannelGroupSpec<'a> {
    /// Human-readable group name, from the inbound chat title when the
    /// channel supplies one (Telegram `chat.title`, Discord `channel.name`).
    pub name: &'a str,
    /// Coach bound to the group at bootstrap — the first sender's selected
    /// coach, falling back to a system coach.
    pub coach_id: &'a str,
    /// Messaging platform: `telegram`, `slack`, `discord`.
    pub channel_type: &'a str,
    /// Platform-native chat identifier the group is bound to.
    pub channel_chat_id: &'a str,
}

use crate::strategies::context::select_context_strategy;
use crate::strategies::tier::{GroupTierStrategy, OwnerGroupLimit};

/// Central service for group coaching operations.
///
/// Coordinates strategy traits with the repository to provide group-aware
/// coaching intelligence. The key function is [`inject_group_context`],
/// called from `chat.rs` and the chat pipeline to augment coach
/// system prompts with group context.
pub struct GroupService {
    repo: Arc<dyn CoachingGroupRepository>,
    tier: Arc<dyn GroupTierStrategy>,
}

impl GroupService {
    /// Create a new group service with the given repository and tier strategy
    #[must_use]
    pub fn new(repo: Arc<dyn CoachingGroupRepository>, tier: Arc<dyn GroupTierStrategy>) -> Self {
        Self { repo, tier }
    }

    // ========================================================================
    // THE KEY FUNCTION — System Prompt Injection
    // ========================================================================

    /// Inject group context into a coach's system prompt.
    ///
    /// Called from two places in pierre-server:
    /// - `chat.rs::get_augmented_system_prompt()` — web + mobile path
    /// - `chat_pipeline::stages::prompt_builder` — messaging path
    ///
    /// If the coach is assigned to a group the user belongs to, augments
    /// the system prompt with member summaries, aggregate stats, and
    /// role-appropriate context (admin overview vs individual focus).
    ///
    /// Returns the original prompt unchanged if no group context applies.
    /// Fast early return (single DB query) for non-group conversations.
    ///
    /// # Arguments
    /// * `base_system_prompt` — The coach's system prompt (possibly already augmented with provider context)
    /// * `coach_id` — The coach persona ID for this conversation
    /// * `user_id` — The user sending the message
    /// * `tenant_id` — Multi-tenant isolation
    /// * `conversation_group_id` — If already selected, skip disambiguation
    /// * `member_snapshots` — Pre-fetched fitness data for group members
    ///
    /// # Errors
    ///
    /// Returns an error if database queries fail or the group data is invalid.
    pub async fn inject_group_context(
        &self,
        base_system_prompt: &str,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_group_id: Option<&str>,
        member_snapshots: &[MemberFitnessSnapshot],
    ) -> AppResult<String> {
        // Fast path: if conversation already has a group, use it
        let group = if let Some(gid) = conversation_group_id {
            self.repo.get_group(gid, tenant_id).await?
        } else {
            // Find groups this user belongs to with this coach
            let groups = self
                .repo
                .find_groups_for_user_and_coach(user_id, coach_id)
                .await?;

            match groups.len() {
                0 => return Ok(base_system_prompt.to_owned()),
                1 => Some(groups.into_iter().next().ok_or_else(|| {
                    AppError::internal("group list reported len()==1 but yielded no element")
                })?),
                _ => {
                    // Multiple groups — inject disambiguation prompt
                    let names: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
                    let disambiguation = format!(
                        "{base_system_prompt}\n\n\
                        Note: This user is part of multiple groups with you: {}. \
                        Ask which group context they'd like to use.",
                        names.join(", ")
                    );
                    return Ok(disambiguation);
                }
            }
        };

        let Some(group) = group else {
            return Ok(base_system_prompt.to_owned());
        };

        // Pull the full member list once: we need both the requester's role
        // (for the admin overview view) and the per-member
        // peer_sharing_consent flag (the single privacy gate).
        let members = self
            .repo
            .list_members(&group.id.to_string())
            .await
            .unwrap_or_default();

        // The group's human coach gets the same whole-group overview an admin
        // sees. The visibility filter below still gates each member's snapshot
        // behind their own `peer_sharing_consent`, so a coach never sees data a
        // member hasn't shared — coach access reuses the existing peer gate
        // rather than bypassing it.
        let is_coach = group.coach_user_id == Some(user_id);
        let is_admin = is_coach
            || members
                .iter()
                .find(|m| m.user_id == user_id)
                .is_some_and(|m| m.role.can_manage_members());

        let member_count = members.len();

        // Build summary cards from snapshots.
        //
        // Per-member `peer_sharing_consent` is the single source of
        // truth: each individual member opts in independently and only
        // their snapshot surfaces. A subset of members can be sharing
        // while the rest stay private.
        //
        // `group.peer_data_sharing` is repurposed as an admin kill
        // switch — when explicitly set to FALSE, every member's
        // snapshot is hidden regardless of their consent (admin nuked
        // sharing for the whole group). Default on auto-bound groups
        // is TRUE so individual consent works without an extra step.
        //
        // The requester's own snapshot is always visible regardless of
        // their own consent flag — they can see their own data even if
        // they haven't opted in to peer sharing.
        let summarizer = self.tier.summarization_strategy();
        let consenting_user_ids: HashSet<Uuid> = members
            .iter()
            .filter(|m| m.peer_sharing_consent)
            .map(|m| m.user_id)
            .collect();
        let visible_snapshots: Vec<&MemberFitnessSnapshot> = if group.peer_data_sharing {
            member_snapshots
                .iter()
                .filter(|s| s.user_id == user_id || consenting_user_ids.contains(&s.user_id))
                .collect()
        } else {
            // Kill switch: only the requester's own snapshot leaks.
            member_snapshots
                .iter()
                .filter(|s| s.user_id == user_id)
                .collect()
        };
        info!(
            group_id = %group.id,
            requester_user_id = %user_id,
            peer_data_sharing = group.peer_data_sharing,
            total_members = members.len(),
            consenting_count = consenting_user_ids.len(),
            input_snapshot_count = member_snapshots.len(),
            visible_count = visible_snapshots.len(),
            "Group context visibility filter applied"
        );
        let cards: Vec<MemberSummaryCard> = visible_snapshots
            .iter()
            .map(|s| summarizer.summarize_member(s))
            .collect();

        let context = GroupContext {
            group: group.clone(),
            member_count,
            active_count: member_snapshots.len(),
            requester_is_admin: is_admin,
            requester_user_id: user_id,
        };

        // Select context strategy based on role
        let ctx_strategy = select_context_strategy(is_admin);

        // Build the context block
        let my_card = cards.iter().find(|c| c.user_id == user_id);
        let context_block = if is_admin {
            ctx_strategy.build_group_context(&context, &cards)
        } else {
            my_card.map_or_else(
                || ctx_strategy.build_group_context(&context, &cards),
                |card| ctx_strategy.build_member_context(card, &context, &cards),
            )
        };

        info!(
            group_id = %group.id,
            member_count = member_count,
            is_admin = is_admin,
            tokens = summarizer.estimated_tokens(cards.len()),
            "Injected group context into system prompt"
        );

        // Connection alerts: name any visible member whose provider connection died so the
        // coach reports the dead provider instead of treating it as merely quiet. Gated
        // identically to the snapshots (only visible/consenting members appear). The
        // member's own reconnect link is delivered out-of-band — never leaked to peers here.
        let mut reauth_lines: Vec<String> = visible_snapshots
            .iter()
            .filter(|s| !s.needs_reauth_providers.is_empty())
            .map(|s| {
                format!(
                    "- {} needs to reconnect: {}",
                    s.display_name,
                    s.needs_reauth_providers.join(", ")
                )
            })
            .collect();
        let reauth_alerts = if reauth_lines.is_empty() {
            String::new()
        } else {
            reauth_lines.sort();
            format!(
                "\n\n## Connection alerts\n\
                These members have a disconnected provider — you cannot pull their fresh data \
                for it. Tell them to reconnect and do not invent data for a disconnected \
                source:\n{}",
                reauth_lines.join("\n")
            )
        };

        Ok(format!(
            "{base_system_prompt}{context_block}{reauth_alerts}"
        ))
    }

    // ========================================================================
    // Group CRUD (delegated to repository with tier enforcement)
    // ========================================================================

    /// Create a new coaching group with tier limit enforcement.
    ///
    /// `tier_member_cap` is the tenant plan's per-group member cap, resolved
    /// by the caller (which owns tenant-plan access via the tenants repo):
    /// `tier_strategy_for(&plan).max_members_per_group()`. The service owns
    /// the policy *application*: a cap of `0` (Starter) rejects creation with
    /// [`ErrorCode::PermissionDenied`]; otherwise the requested `max_members`
    /// (defaulting to 20) is clamped into `2..=tier_member_cap`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::PermissionDenied`] when the tenant tier disables
    /// group coaching (`tier_member_cap == 0`), an `invalid_input` error when
    /// the owner's group limit is reached, or a database error on failure.
    pub async fn create_group(
        &self,
        request: &CreateGroupRequest,
        owner_id: Uuid,
        tenant_id: TenantId,
        tier_member_cap: i32,
    ) -> AppResult<CoachingGroup> {
        let group = CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: tenant_id.to_string(),
            name: request.name.clone(),
            description: request.description.clone(),
            coach_id: request.coach_id.clone(),
            owner_id,
            // No human coach until one redeems a coach-kind invite.
            coach_user_id: None,
            // `peer_data_sharing` is the admin kill switch — defaults
            // to TRUE so individual members' `/group consent yes` opt-
            // ins immediately surface their data. Owner can flip to
            // FALSE in group settings to disable everyone's sharing in
            // one move.
            peer_data_sharing: true,
            // Coach answers every message until the owner narrows it via
            // `/group respond mentions` or the group-settings UI.
            respond_mode: GroupRespondMode::default(),
            // Clamped to the tenant tier's per-group cap by
            // `create_group_with_owner`.
            max_members: request.max_members.unwrap_or(DEFAULT_REQUESTED_MEMBERS),
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.create_group_with_owner(
            group,
            owner_id,
            tenant_id,
            tier_member_cap,
            OwnerGroupLimit::Enforced,
        )
        .await
    }

    /// Create the coaching group bound to a messaging chat (Telegram group,
    /// Slack channel, Discord channel), with the first sender as Owner.
    ///
    /// Shares [`create_group`](Self::create_group)'s tier gate, member-count
    /// clamp, owner auto-membership and `group.created` emission — chat and
    /// REST group creation differ only in the fields carried on the row
    /// (`channel_type` / `channel_chat_id` and a synthesized name), never in
    /// the policy applied to it. Before this shared path existed the chat
    /// binding called the repository directly, which both skipped the tier
    /// gate and made every chat-created group invisible to analytics.
    ///
    /// The owner's tier group *count* allowance deliberately does not apply
    /// here — see [`OwnerGroupLimit`]. The per-group member cap still does.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::PermissionDenied`] when the tenant tier disables
    /// group coaching (`tier_member_cap == 0`), or a database error on
    /// failure.
    pub async fn create_channel_group(
        &self,
        spec: &ChannelGroupSpec<'_>,
        owner_id: Uuid,
        tenant_id: TenantId,
        tier_member_cap: i32,
    ) -> AppResult<CoachingGroup> {
        let now = chrono::Utc::now();
        let group = CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: tenant_id.to_string(),
            name: spec.name.to_owned(),
            description: Some(format!(
                "Auto-created from {} group chat {}",
                spec.channel_type, spec.channel_chat_id
            )),
            coach_id: spec.coach_id.to_owned(),
            owner_id,
            // No human coach until one redeems a coach-kind invite.
            coach_user_id: None,
            // Group-level kill switch — TRUE means "individual member
            // consent is the gate", FALSE means admin nuked all sharing.
            // Defaults to TRUE so a member who runs `/group consent yes`
            // immediately starts surfacing their data to peers without an
            // owner intervention.
            peer_data_sharing: true,
            // Auto-bound groups start in answer-everything mode — the chat
            // behaves exactly as before binding; the owner narrows it via
            // `/group respond mentions`.
            respond_mode: GroupRespondMode::All,
            // Clamped to the tenant tier's per-group cap by
            // `create_group_with_owner`.
            max_members: DEFAULT_REQUESTED_MEMBERS,
            is_active: true,
            channel_type: Some(spec.channel_type.to_owned()),
            channel_chat_id: Some(spec.channel_chat_id.to_owned()),
            created_at: now,
            updated_at: now,
        };

        self.create_group_with_owner(
            group,
            owner_id,
            tenant_id,
            tier_member_cap,
            OwnerGroupLimit::Exempt,
        )
        .await
    }

    /// Apply tier policy to `group`, persist it, enrol `owner_id` as Owner and
    /// emit the catalogued `group.created` event.
    ///
    /// The single creation chokepoint behind both
    /// [`create_group`](Self::create_group) (REST + `/coach` slash command)
    /// and [`create_channel_group`](Self::create_channel_group) (messaging
    /// auto-bind). Emitting here rather than at each transport is what makes
    /// `group.created` fire for chat-created groups: `user_id` and
    /// `tenant_id` are supplied inline because the messaging ingress span
    /// carries neither, so the `NotifyLayer` has nothing to inherit there.
    /// `limit` selects whether this creation spends the owner's tier group
    /// allowance; the tier gate and member clamp apply either way.
    async fn create_group_with_owner(
        &self,
        mut group: CoachingGroup,
        owner_id: Uuid,
        tenant_id: TenantId,
        tier_member_cap: i32,
        limit: OwnerGroupLimit,
    ) -> AppResult<CoachingGroup> {
        // Tier gate: a cap of 0 disables group coaching for the plan.
        if tier_member_cap == 0 {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Group coaching requires a Professional or Enterprise plan",
            ));
        }

        // Per-owner group allowance, on the paths that spend it.
        if matches!(limit, OwnerGroupLimit::Enforced) {
            if let Some(max) = self.tier.max_groups() {
                let current = self
                    .repo
                    .count_groups_for_owner(owner_id, tenant_id)
                    .await?;
                if current >= i64::try_from(max).unwrap_or_default() {
                    return Err(AppError::invalid_input(format!(
                        "Group limit reached ({max}). Upgrade your plan for more groups."
                    )));
                }
            }
        }

        // Clamp the requested member count into the tier's per-group range.
        // `max(MIN_MEMBERS)` on the upper bound keeps `clamp` from panicking
        // on a hypothetical tier cap of 1, where min would exceed max.
        group.max_members = group
            .max_members
            .clamp(MIN_MEMBERS, tier_member_cap.max(MIN_MEMBERS));

        let created = self.repo.create_group(tenant_id, &group).await?;

        // Auto-add owner as member with Owner role. This membership is
        // implied by `group.created` (which carries the owner's `user_id`),
        // so it deliberately emits no `group.joined` — that event means "a
        // second person came in", on every surface.
        let owner_member = GroupMember {
            id: Uuid::new_v4(),
            group_id: created.id,
            user_id: owner_id,
            tenant_id: tenant_id.to_string(),
            role: GroupRole::Owner,
            peer_sharing_consent: false,
            consent_given_at: chrono::Utc::now(),
            joined_at: chrono::Utc::now(),
            left_at: None,
            display_name: None,
        };
        self.repo.add_member(&owner_member).await?;

        info!(
            target: "notify",
            event = "group.created",
            user_id = %owner_id,
            tenant_id = %tenant_id,
            group_id = %created.id,
            "coaching group created"
        );
        Ok(created)
    }

    /// Look up the coaching group bound to a messaging chat, if one exists.
    ///
    /// Paired with [`create_channel_group`](Self::create_channel_group) and
    /// [`enroll_channel_member`](Self::enroll_channel_member) so the messaging
    /// ingress reaches the group domain only through this service.
    ///
    /// # Errors
    ///
    /// Returns a database error if the lookup fails.
    pub async fn get_group_by_channel(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_chat_id: &str,
    ) -> AppResult<Option<CoachingGroup>> {
        self.repo
            .get_group_by_channel(tenant_id, channel_type, channel_chat_id)
            .await
    }

    /// Enrol a messaging-chat sender as a Member of the channel-bound group,
    /// emitting the catalogued `group.joined` event on a real enrolment.
    ///
    /// Returns whether the user is a member of `group` once this returns —
    /// `true` when they already were or were just added, `false` when the
    /// group is at its member cap. Callers must not attach a conversation to
    /// a group for which this returned `false`: `inject_group_context` builds
    /// peer context from the conversation's `group_id` without re-checking
    /// the requester's membership, so binding a non-member would surface
    /// consenting peers' data to someone outside the group.
    ///
    /// # Errors
    ///
    /// Returns database errors from the member lookup or insert.
    pub async fn enroll_channel_member(
        &self,
        group: &CoachingGroup,
        user_id: Uuid,
    ) -> AppResult<bool> {
        let group_id = group.id.to_string();
        if self.repo.get_member(&group_id, user_id).await?.is_some() {
            return Ok(true);
        }

        let current_count = self.repo.count_members(&group_id).await?;
        if current_count >= i64::from(group.max_members) {
            warn!(
                group_id = %group.id,
                max_members = group.max_members,
                "channel-bound group is at its member cap; sender stays ungrouped"
            );
            return Ok(false);
        }

        let now = chrono::Utc::now();
        let member = GroupMember {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id,
            tenant_id: group.tenant_id.clone(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        };
        self.repo.add_member(&member).await?;

        // The group's tenant, not the sender's home tenant — a chat runs
        // under the channel's tenant (same distinction ADR-020 drew for
        // peer-data resolution), and the group-adoption metric follows the
        // group.
        info!(
            target: "notify",
            event = "group.joined",
            user_id = %user_id,
            tenant_id = %group.tenant_id,
            group_id = %group.id,
            "user joined coaching group"
        );
        Ok(true)
    }

    /// Get a group by ID
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn get_group(
        &self,
        group_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachingGroup>> {
        self.repo.get_group(group_id, tenant_id).await
    }

    /// List groups for a user
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn list_groups(&self, user_id: Uuid) -> AppResult<Vec<GroupSummary>> {
        self.repo.list_groups_for_user(user_id).await
    }

    /// Update a group
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail or the group is not found.
    pub async fn update_group(
        &self,
        group_id: &str,
        tenant_id: TenantId,
        request: &UpdateGroupRequest,
    ) -> AppResult<Option<CoachingGroup>> {
        self.repo.update_group(group_id, tenant_id, request).await
    }

    /// Soft-delete a group
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn delete_group(&self, group_id: &str, tenant_id: TenantId) -> AppResult<bool> {
        self.repo.delete_group(group_id, tenant_id).await
    }

    // ========================================================================
    // Membership operations
    // ========================================================================

    /// Join a group via invite code
    ///
    /// # Errors
    ///
    /// Returns an error if the invite is invalid, expired, full, or the user is already a member.
    pub async fn join_group(
        &self,
        invite_code: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<GroupMember> {
        let invite = self
            .repo
            .get_invite_by_code(invite_code)
            .await?
            .ok_or_else(|| AppError::not_found("Invalid or expired invite code"))?;

        Self::check_invite_usable(&invite)?;

        // Member invites never attach a coach — coach-kind invites are
        // redeemed through `redeem_coach_invite`, which the route dispatches
        // to based on `invite.kind`.
        if invite.kind == GroupInviteKind::Coach {
            return Err(AppError::invalid_input(
                "This is a coach invite — redeem it from the coach flow, not group join",
            ));
        }

        // Check member limit
        let group = self
            .repo
            .get_group(&invite.group_id.to_string(), tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("Group not found"))?;

        let current_count = self
            .repo
            .count_members(&invite.group_id.to_string())
            .await?;
        if current_count >= i64::from(group.max_members) {
            return Err(AppError::invalid_input("This group is full"));
        }

        // Check not already a member
        let existing = self
            .repo
            .get_member(&invite.group_id.to_string(), user_id)
            .await?;
        if existing.is_some() {
            return Err(AppError::invalid_input(
                "You are already a member of this group",
            ));
        }

        let member = GroupMember {
            id: Uuid::new_v4(),
            group_id: invite.group_id,
            user_id,
            tenant_id: tenant_id.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            consent_given_at: chrono::Utc::now(),
            joined_at: chrono::Utc::now(),
            left_at: None,
            display_name: None,
        };

        let created = self.repo.add_member(&member).await?;
        self.repo
            .increment_invite_use_count(&invite.id.to_string())
            .await?;

        // The invite's tenant is the *group's* tenant, which is what the
        // caller passes in — athlete membership is cross-tenant by design, so
        // the adoption metric must follow the group rather than the
        // redeemer's home tenant.
        info!(
            target: "notify",
            event = "group.joined",
            user_id = %user_id,
            tenant_id = %tenant_id,
            group_id = %invite.group_id,
            "user joined coaching group"
        );
        Ok(created)
    }

    /// Shared invite-validity gate: active, not expired, under its use limit.
    fn check_invite_usable(invite: &GroupInvite) -> AppResult<()> {
        if !invite.is_active {
            return Err(AppError::invalid_input("This invite has been deactivated"));
        }
        if let Some(expires) = invite.expires_at {
            if expires < chrono::Utc::now() {
                return Err(AppError::invalid_input("This invite has expired"));
            }
        }
        if let Some(max) = invite.max_uses {
            if invite.use_count >= max {
                return Err(AppError::invalid_input(
                    "This invite has reached its use limit",
                ));
            }
        }
        Ok(())
    }

    /// Redeem a coach-kind invite, attaching the caller as the group's human
    /// coach (`coach_user_id`).
    ///
    /// Eligibility (the caller is a roster-managing coach and belongs to the
    /// group's tenant) is enforced by the route layer, which owns user-repo
    /// access. This method owns the group-side business logic: invite
    /// validity, the single-coach guard, the attachment write, and the
    /// invite-use increment.
    ///
    /// # Errors
    ///
    /// Returns an error if the invite is invalid/expired/exhausted, is not a
    /// coach invite, the group is missing, or a different coach is already
    /// attached.
    pub async fn redeem_coach_invite(
        &self,
        invite_code: &str,
        coach_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<CoachingGroup> {
        let invite = self
            .repo
            .get_invite_by_code(invite_code)
            .await?
            .ok_or_else(|| AppError::not_found("Invalid or expired invite code"))?;

        Self::check_invite_usable(&invite)?;

        if invite.kind != GroupInviteKind::Coach {
            return Err(AppError::invalid_input(
                "This invite does not grant coach access",
            ));
        }

        let group = self
            .repo
            .get_group(&invite.group_id.to_string(), tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("Group not found"))?;

        // Single human coach per group (v1). Re-redeeming as the same coach is
        // idempotent; a different coach is rejected so an owner explicitly
        // detaches the current coach first.
        match group.coach_user_id {
            Some(existing) if existing == coach_user_id => return Ok(group),
            Some(_) => {
                return Err(AppError::invalid_input(
                    "This group already has a coach. Remove the current coach first.",
                ));
            }
            None => {}
        }

        let attached = self
            .repo
            .set_group_coach_user(&invite.group_id.to_string(), Some(coach_user_id), tenant_id)
            .await?;
        if !attached {
            return Err(AppError::internal("Failed to attach coach to group"));
        }
        self.repo
            .increment_invite_use_count(&invite.id.to_string())
            .await?;

        // Reuses the catalogued `group.joined` event (a coach redeeming a
        // coach-kind invite is still a join); the message distinguishes the
        // coach case for operators. Emitted after the attach succeeds, so
        // re-redeeming the same invite — which returns early above — no
        // longer double-counts the way the route-level emission did.
        info!(
            target: "notify",
            event = "group.joined",
            user_id = %coach_user_id,
            tenant_id = %tenant_id,
            group_id = %invite.group_id,
            "coach joined coaching group"
        );

        self.repo
            .get_group(&invite.group_id.to_string(), tenant_id)
            .await?
            .ok_or_else(|| AppError::internal("Group not found after coach attach"))
    }

    /// Attach or clear the group's human coach directly (admin/owner action).
    /// Pass `None` to detach.
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn set_group_coach(
        &self,
        group_id: &str,
        coach_user_id: Option<Uuid>,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        self.repo
            .set_group_coach_user(group_id, coach_user_id, tenant_id)
            .await
    }

    /// List active groups the user is the attached human coach of.
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn list_coached_groups(&self, user_id: Uuid) -> AppResult<Vec<CoachingGroup>> {
        self.repo.list_groups_coached_by(user_id).await
    }

    /// Leave a group
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn leave_group(&self, group_id: &str, user_id: Uuid) -> AppResult<bool> {
        self.repo.remove_member(group_id, user_id).await
    }

    /// List members
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn list_members(&self, group_id: &str) -> AppResult<Vec<GroupMember>> {
        self.repo.list_members(group_id).await
    }

    /// Remove a member (admin action)
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn remove_member(&self, group_id: &str, user_id: Uuid) -> AppResult<bool> {
        self.repo.remove_member(group_id, user_id).await
    }

    /// Update member role
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn update_member_role(
        &self,
        group_id: &str,
        user_id: Uuid,
        role: GroupRole,
    ) -> AppResult<bool> {
        self.repo.update_member_role(group_id, user_id, role).await
    }

    // ========================================================================
    // Invites
    // ========================================================================

    /// Generate a new invite code for a group
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn create_invite(
        &self,
        group_id: Uuid,
        created_by: Uuid,
        tenant_id: TenantId,
        expires_in_days: Option<i64>,
        max_uses: Option<i32>,
        kind: GroupInviteKind,
    ) -> AppResult<GroupInvite> {
        let code = generate_invite_code();
        let expires_at =
            expires_in_days.map(|days| chrono::Utc::now() + chrono::Duration::days(days));

        let invite = GroupInvite {
            id: Uuid::new_v4(),
            group_id,
            tenant_id: tenant_id.to_string(),
            code,
            kind,
            created_by,
            expires_at,
            max_uses,
            use_count: 0,
            is_active: true,
            created_at: chrono::Utc::now(),
        };

        self.repo.create_invite(&invite).await
    }

    /// List invites for a group
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn list_invites(&self, group_id: &str) -> AppResult<Vec<GroupInvite>> {
        self.repo.list_invites(group_id).await
    }

    // ========================================================================
    // Analytics (used by MCP tools)
    // ========================================================================

    /// Get aggregate stats for a group
    #[must_use]
    pub fn compute_aggregate_stats(
        &self,
        snapshots: &[MemberFitnessSnapshot],
    ) -> GroupAggregateStats {
        let aggregator = self.tier.aggregation_strategy();
        aggregator.aggregate_stats(snapshots)
    }

    /// Compare a member against group norms
    ///
    /// # Errors
    ///
    /// Returns an error if the member is not found in the snapshots.
    pub fn compare_member(
        &self,
        user_id: Uuid,
        snapshots: &[MemberFitnessSnapshot],
    ) -> AppResult<MemberGroupComparison> {
        let aggregator = self.tier.aggregation_strategy();
        aggregator.compare_member(user_id, snapshots)
    }

    /// Get health flags for all members based on physiological thresholds.
    ///
    /// Flags produced:
    /// - **`DeepFatigue`** (critical): [`FormBand::DeepFatigue`], form below -30 % of CTL
    /// - **Overreaching** (warning): [`FormBand::HeavyBlock`], form -30 % to -20 % of CTL
    /// - **Inactive** (warning): no activity for [`INACTIVITY_DAYS_THRESHOLD`]+ days
    /// - **`VolumeDrop`**: weekly volume dropped more than
    ///   [`VOLUME_DROP_FRACTION_THRESHOLD`] (30%) from prior-week baseline
    #[must_use]
    pub fn compute_health_flags(snapshots: &[MemberFitnessSnapshot]) -> Vec<GroupHealthFlag> {
        // Compute group average volume as baseline for volume-drop detection
        let baseline_volume = Self::compute_baseline_volume(snapshots);

        snapshots
            .iter()
            .filter_map(|s| {
                let mut flags = Vec::new();

                // Form-based flags come off the shared band, never raw TSB.
                // Without a chronic base the band is InsufficientHistory and
                // no form flag is raised, so the risk level is the only signal.
                let form_pct = s.tsb.zip(s.ctl).and_then(|(t, c)| FormBand::form_pct(t, c));
                let band = FormBand::from_form_pct(form_pct);
                if let (Some(pct), Some(tsb)) = (form_pct, s.tsb) {
                    match band {
                        FormBand::DeepFatigue => flags.push(GroupHealthFlag {
                            user_id: s.user_id,
                            display_name: s.display_name.clone(),
                            flag_type: MemberFlag::DeepFatigue,
                            severity: HealthFlagSeverity::Critical,
                            detail: format!(
                                "Form at {pct:.0}% of fitness (TSB {tsb:+.0}), deepest fatigue band"
                            ),
                        }),
                        FormBand::HeavyBlock => flags.push(GroupHealthFlag {
                            user_id: s.user_id,
                            display_name: s.display_name.clone(),
                            flag_type: MemberFlag::Overreaching,
                            severity: HealthFlagSeverity::Warning,
                            detail: format!(
                                "Form at {pct:.0}% of fitness (TSB {tsb:+.0}), deep end of the productive zone"
                            ),
                        }),
                        _ => {}
                    }
                } else if matches!(s.overtraining_risk, OvertrainingRiskLevel::High) {
                    // Fall back to the risk level when TSB is absent
                    flags.push(GroupHealthFlag {
                        user_id: s.user_id,
                        display_name: s.display_name.clone(),
                        flag_type: MemberFlag::Overreaching,
                        severity: HealthFlagSeverity::Warning,
                        detail: "High overtraining risk detected, recommend recovery".to_owned(),
                    });
                }

                // Inactivity flag
                if let Some(days) = s.days_since_last_activity {
                    if days >= INACTIVITY_DAYS_THRESHOLD {
                        flags.push(GroupHealthFlag {
                            user_id: s.user_id,
                            display_name: s.display_name.clone(),
                            flag_type: MemberFlag::Inactive,
                            severity: HealthFlagSeverity::Warning,
                            detail: format!("No activity for {days} days"),
                        });
                    }
                }

                // Volume drop flag (compare against group baseline)
                if baseline_volume > 0.0 && s.weekly_volume_km > 0.0 {
                    let drop_fraction = (baseline_volume - s.weekly_volume_km) / baseline_volume;
                    if drop_fraction >= VOLUME_DROP_FRACTION_THRESHOLD {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let pct = (drop_fraction * 100.0).round() as u32;
                        flags.push(GroupHealthFlag {
                            user_id: s.user_id,
                            display_name: s.display_name.clone(),
                            flag_type: MemberFlag::VolumeDrop,
                            severity: HealthFlagSeverity::Info,
                            detail: format!(
                                "Weekly volume {pct}% below group average, possible detraining"
                            ),
                        });
                    }
                }

                if flags.is_empty() {
                    None
                } else {
                    Some(flags)
                }
            })
            .flatten()
            .collect()
    }

    /// Compute group average weekly volume as a baseline for drop detection
    fn compute_baseline_volume(snapshots: &[MemberFitnessSnapshot]) -> f64 {
        let active: Vec<f64> = snapshots
            .iter()
            .filter(|s| s.weekly_volume_km > 0.0)
            .map(|s| s.weekly_volume_km)
            .collect();
        if active.is_empty() {
            return 0.0;
        }
        active.iter().sum::<f64>() / active.len() as f64
    }

    /// Generate a deterministic (non-AI) weekly report from member snapshots.
    ///
    /// Includes summary, highlights (members in fresh form), concerns from
    /// health flags, and recommendations derived from flag count and trend.
    #[must_use]
    pub fn compute_weekly_report(
        &self,
        snapshots: &[MemberFitnessSnapshot],
        group_name: &str,
    ) -> GroupWeeklyReport {
        let stats = self.compute_aggregate_stats(snapshots);
        let flags = Self::compute_health_flags(snapshots);

        let trend_label = match stats.weekly_trend {
            GroupTrend::Improving => "improving",
            GroupTrend::Declining => "declining",
            GroupTrend::Stable => "stable",
        };

        let summary = format!(
            "{group_name} had {}/{} active members this week with average volume of {:.1}km. \
             Overall trend: {trend_label}.",
            stats.active_members, stats.total_members, stats.avg_weekly_volume_km
        );

        // Highlights: members whose form reads Fresh against their own chronic
        // base. A merely positive TSB is not freshness — 0 to +5% of CTL is
        // balanced, and the same +8 is fresh at CTL 40 but balanced at CTL 150.
        let highlights: Vec<String> = snapshots
            .iter()
            .filter_map(|s| {
                let pct = FormBand::form_pct(s.tsb?, s.ctl?)?;
                (FormBand::from_form_pct(Some(pct)) == FormBand::Fresh).then(|| {
                    format!(
                        "{} is in fresh form (TSB {:+.0}, {pct:.0}% of CTL)",
                        s.display_name,
                        s.tsb.unwrap_or_default()
                    )
                })
            })
            .collect();

        let concerns: Vec<String> = flags
            .iter()
            .map(|f| format!("{}: {}", f.display_name, f.detail))
            .collect();

        let mut recommendations = Vec::new();
        if stats.flagged_members > 0 {
            recommendations.push(format!(
                "Review {} flagged member(s) and consider recovery adjustments.",
                stats.flagged_members
            ));
        }
        match stats.weekly_trend {
            GroupTrend::Declining => {
                recommendations.push(
                    "Group volume is declining — check in with less active members.".to_owned(),
                );
            }
            GroupTrend::Improving => {
                recommendations.push(
                    "Good momentum — ensure recovery keeps pace with rising volume.".to_owned(),
                );
            }
            GroupTrend::Stable => {}
        }

        GroupWeeklyReport {
            summary,
            highlights,
            concerns,
            recommendations,
            stats,
        }
    }

    /// Access the tier strategy (for route handlers to check features)
    #[must_use]
    pub fn tier(&self) -> &dyn GroupTierStrategy {
        self.tier.as_ref()
    }

    /// Access the repository (for route handlers)
    #[must_use]
    pub fn repo(&self) -> &dyn CoachingGroupRepository {
        self.repo.as_ref()
    }
}

/// Generate an 8-character alphanumeric invite code
fn generate_invite_code() -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::rng();
    (0..8)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
