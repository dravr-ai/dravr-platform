// ABOUTME: Central group coaching service coordinating strategies and repository access
// ABOUTME: Contains inject_group_context — the single function that makes all surfaces group-aware
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;
use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachingGroup, CreateGroupRequest, GroupAggregateStats, GroupContext, GroupHealthFlag,
    GroupInvite, GroupMember, GroupRole, GroupSummary, GroupTrend, GroupWeeklyReport,
    HealthFlagSeverity, MemberFitnessSnapshot, MemberFlag, MemberGroupComparison,
    MemberSummaryCard, OvertrainingRiskLevel, UpdateGroupRequest,
};
use pierre_core::models::TenantId;
use pierre_database::repositories::CoachingGroupRepository;
use tracing::{debug, info};
use uuid::Uuid;

// ============================================================================
// Health Flag Thresholds
// ============================================================================

/// TSB threshold below which a member is flagged as overreaching (warning)
pub const OVERREACHING_TSB_THRESHOLD: f64 = -20.0;

/// TSB threshold below which a member is at high fatigue/injury risk (critical)
pub const HIGH_FATIGUE_TSB_THRESHOLD: f64 = -30.0;

/// Days without activity before a member is flagged as inactive
pub const INACTIVITY_DAYS_THRESHOLD: i32 = 7;

/// Weekly volume drop percentage (0.0–1.0) from baseline to trigger a flag
pub const VOLUME_DROP_FRACTION_THRESHOLD: f64 = 0.30;

/// Weekly trend threshold: volume increase above this fraction = "improving"
pub const TREND_IMPROVING_FRACTION: f64 = 0.05;

/// Weekly trend threshold: volume decrease beyond this fraction = "declining"
pub const TREND_DECLINING_FRACTION: f64 = 0.05;

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
                .find_groups_for_user_and_coach(user_id, coach_id)
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

        // Pull the full member list once: we need both the requester's role
        // (for the admin overview view) and the per-member
        // peer_sharing_consent flag (the single privacy gate).
        let members = self
            .repo
            .list_members(&group.id.to_string())
            .await
            .unwrap_or_default();

        let is_admin = members
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
        debug!(
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
            // `peer_data_sharing` is the admin kill switch — defaults
            // to TRUE so individual members' `/group consent yes` opt-
            // ins immediately surface their data. Owner can flip to
            // FALSE in group settings to disable everyone's sharing in
            // one move.
            peer_data_sharing: true,
            max_members: request.max_members.unwrap_or(20),
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
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

        info!(group_id = %invite.group_id, user_id = %user_id, "Member joined group via invite");
        Ok(created)
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
    /// - **Overreaching** (warning): TSB below [`OVERREACHING_TSB_THRESHOLD`] (-20)
    /// - **`InjuryRisk`** (critical): TSB below [`HIGH_FATIGUE_TSB_THRESHOLD`] (-30)
    /// - **Inactive** (warning): no activity for [`INACTIVITY_DAYS_THRESHOLD`]+ days
    /// - **`VolumeDrop`**: weekly volume dropped more than
    ///   [`VOLUME_DROP_FRACTION_THRESHOLD`] (30%) from prior-week baseline
    #[must_use]
    pub fn compute_health_flags(
        &self,
        snapshots: &[MemberFitnessSnapshot],
    ) -> Vec<GroupHealthFlag> {
        // Compute group average volume as baseline for volume-drop detection
        let baseline_volume = Self::compute_baseline_volume(snapshots);

        snapshots
            .iter()
            .filter_map(|s| {
                let mut flags = Vec::new();

                // TSB-based flags
                if let Some(tsb) = s.tsb {
                    if tsb < HIGH_FATIGUE_TSB_THRESHOLD {
                        flags.push(GroupHealthFlag {
                            user_id: s.user_id,
                            display_name: s.display_name.clone(),
                            flag_type: MemberFlag::InjuryRisk,
                            severity: HealthFlagSeverity::Critical,
                            detail: format!("TSB at {tsb:+.0}, high injury risk"),
                        });
                    } else if tsb < OVERREACHING_TSB_THRESHOLD {
                        flags.push(GroupHealthFlag {
                            user_id: s.user_id,
                            display_name: s.display_name.clone(),
                            flag_type: MemberFlag::Overreaching,
                            severity: HealthFlagSeverity::Warning,
                            detail: format!("TSB at {tsb:+.0}, recommend recovery"),
                        });
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
        let flags = self.compute_health_flags(snapshots);

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

        // Highlights: members with positive TSB (fresh form)
        let highlights: Vec<String> = snapshots
            .iter()
            .filter(|s| s.tsb.is_some_and(|tsb| tsb > 0.0))
            .map(|s| {
                format!(
                    "{} is in fresh form (TSB {:+.0})",
                    s.display_name,
                    s.tsb.unwrap_or(0.0)
                )
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
            channel_type: None,
            channel_chat_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
    }
}
