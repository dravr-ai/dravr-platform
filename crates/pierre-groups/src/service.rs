// ABOUTME: Central group coaching service coordinating strategies and repository access
// ABOUTME: Contains inject_group_context — the single function that makes all surfaces group-aware
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachingGroup, CreateGroupRequest, GroupAggregateStats, GroupContext, GroupHealthFlag,
    GroupInvite, GroupMember, GroupRole, GroupSummary, GroupWeeklyReport, HealthFlagSeverity,
    MemberFitnessSnapshot, MemberFlag, MemberGroupComparison, MemberSummaryCard,
    OvertrainingRiskLevel, UpdateGroupRequest,
};
use pierre_core::models::TenantId;
use pierre_database::repositories::CoachingGroupRepository;
use tracing::info;
use uuid::Uuid;

use crate::strategies::context::select_context_strategy;
use crate::strategies::tier::GroupTierStrategy;

/// Central service for group coaching operations.
///
/// Coordinates strategy traits with the repository to provide group-aware
/// coaching intelligence. The key function is [`inject_group_context`],
/// called from `chat.rs` and `chat_orchestration.rs` to augment coach
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
    /// - `chat_orchestration.rs` — messaging path
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
                .find_groups_for_user_and_coach(user_id, coach_id, tenant_id)
                .await?;

            match groups.len() {
                0 => return Ok(base_system_prompt.to_owned()),
                1 => Some(groups.into_iter().next().unwrap_or_default_unreachable()),
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

        // Get member info
        let member = self
            .repo
            .get_member(&group.id.to_string(), user_id, tenant_id)
            .await?;

        let is_admin = member.as_ref().is_some_and(|m| m.role.can_manage_members());

        let member_count = self
            .repo
            .count_members(&group.id.to_string(), tenant_id)
            .await
            .unwrap_or(0) as usize;

        // Build summary cards from snapshots
        let summarizer = self.tier.summarization_strategy();
        let cards: Vec<MemberSummaryCard> = member_snapshots
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

        Ok(format!("{base_system_prompt}{context_block}"))
    }

    // ========================================================================
    // Group CRUD (delegated to repository with tier enforcement)
    // ========================================================================

    /// Create a new coaching group with tier limit enforcement
    ///
    /// # Errors
    ///
    /// Returns an error if the group limit is reached or database operations fail.
    pub async fn create_group(
        &self,
        request: &CreateGroupRequest,
        owner_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<CoachingGroup> {
        // Check tier limits
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

        let group = CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: tenant_id.to_string(),
            name: request.name.clone(),
            description: request.description.clone(),
            coach_id: request.coach_id.clone(),
            owner_id,
            peer_data_sharing: false,
            max_members: request.max_members.unwrap_or(20),
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let created = self.repo.create_group(tenant_id, &group).await?;

        // Auto-add owner as member with Owner role
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

        info!(group_id = %created.id, owner = %owner_id, "Created coaching group");
        Ok(created)
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

        // Check invite validity
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

        // Check member limit
        let group = self
            .repo
            .get_group(&invite.group_id.to_string(), tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("Group not found"))?;

        let current_count = self
            .repo
            .count_members(&invite.group_id.to_string(), tenant_id)
            .await?;
        if current_count >= i64::from(group.max_members) {
            return Err(AppError::invalid_input("This group is full"));
        }

        // Check not already a member
        let existing = self
            .repo
            .get_member(&invite.group_id.to_string(), user_id, tenant_id)
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

        info!(group_id = %invite.group_id, user_id = %user_id, "Member joined group via invite");
        Ok(created)
    }

    /// Leave a group
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn leave_group(
        &self,
        group_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        self.repo.remove_member(group_id, user_id, tenant_id).await
    }

    /// List members
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn list_members(
        &self,
        group_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<GroupMember>> {
        self.repo.list_members(group_id, tenant_id).await
    }

    /// Remove a member (admin action)
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    pub async fn remove_member(
        &self,
        group_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        self.repo.remove_member(group_id, user_id, tenant_id).await
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
        tenant_id: TenantId,
        role: GroupRole,
    ) -> AppResult<bool> {
        self.repo
            .update_member_role(group_id, user_id, tenant_id, role)
            .await
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
    ) -> AppResult<GroupInvite> {
        let code = generate_invite_code();
        let expires_at =
            expires_in_days.map(|days| chrono::Utc::now() + chrono::Duration::days(days));

        let invite = GroupInvite {
            id: Uuid::new_v4(),
            group_id,
            tenant_id: tenant_id.to_string(),
            code,
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
    pub async fn list_invites(
        &self,
        group_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<GroupInvite>> {
        self.repo.list_invites(group_id, tenant_id).await
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

    /// Get health flags for all members
    #[must_use]
    pub fn compute_health_flags(
        &self,
        snapshots: &[MemberFitnessSnapshot],
    ) -> Vec<GroupHealthFlag> {
        snapshots
            .iter()
            .filter_map(|s| {
                let mut flags = Vec::new();

                if matches!(s.overtraining_risk, OvertrainingRiskLevel::High) {
                    flags.push(GroupHealthFlag {
                        user_id: s.user_id,
                        display_name: s.display_name.clone(),
                        flag_type: MemberFlag::Overreaching,
                        severity: HealthFlagSeverity::Warning,
                        detail: format!("TSB at {:+.0}, recommend recovery", s.tsb.unwrap_or(0.0)),
                    });
                }

                if let Some(days) = s.days_since_last_activity {
                    if days > 14 {
                        flags.push(GroupHealthFlag {
                            user_id: s.user_id,
                            display_name: s.display_name.clone(),
                            flag_type: MemberFlag::Inactive,
                            severity: HealthFlagSeverity::Warning,
                            detail: format!("No activity for {days} days"),
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

    /// Generate a weekly report
    #[must_use]
    pub fn compute_weekly_report(
        &self,
        snapshots: &[MemberFitnessSnapshot],
        group_name: &str,
    ) -> GroupWeeklyReport {
        let stats = self.compute_aggregate_stats(snapshots);
        let flags = self.compute_health_flags(snapshots);

        let summary = format!(
            "{} had {}/{} active members this week with average volume of {:.1}km.",
            group_name, stats.active_members, stats.total_members, stats.avg_weekly_volume_km
        );

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

        GroupWeeklyReport {
            summary,
            highlights: Vec::new(),
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
    let mut rng = rand::thread_rng();
    (0..8)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Helper trait to work around the fact that `into_iter().next()` on a Vec
/// after checking `.len()` should always produce a value.
trait UnwrapSingle {
    fn unwrap_or_default_unreachable(self) -> CoachingGroup;
}

impl UnwrapSingle for Option<CoachingGroup> {
    fn unwrap_or_default_unreachable(self) -> CoachingGroup {
        // This branch is unreachable because we only call this after checking len() == 1
        self.unwrap_or_else(|| CoachingGroup {
            id: Uuid::nil(),
            tenant_id: String::new(),
            name: String::new(),
            description: None,
            coach_id: String::new(),
            owner_id: Uuid::nil(),
            peer_data_sharing: false,
            max_members: 0,
            is_active: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
    }
}
