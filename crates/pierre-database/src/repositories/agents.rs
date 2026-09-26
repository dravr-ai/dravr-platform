// ABOUTME: Repository trait definitions for the agents catalogue and coaching groups domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::agents::{
    Agent, AgentAssignment, AgentHandle, AgentListItem, AgentVersion, CreateAgentRequest,
    CreateSystemAgentRequest, ListAgentsFilter, UpdateAgentRequest,
};
use pierre_core::models::groups::{
    CoachingGroup, GroupInvite, GroupMember, GroupRole, GroupSummary, GroupTranscriptEntry,
    NewGroupTranscriptEntry, UpdateGroupRequest,
};

use pierre_core::models::AgentRuntimeContext;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// Coaches (custom AI personas) storage and management repository (tenant-scoped)
#[async_trait]
pub trait AgentsRepository: Send + Sync {
    /// Create a new agent
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateAgentRequest,
    ) -> AppResult<Agent>;
    /// Get agent by ID
    async fn get_by_id(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Agent>>;
    /// The agent `tenant_id` can run: one of that tenant's own agents, or a
    /// system agent — the scope [`Self::get_agent_runtime_context`] resolves
    /// an agent in, with no user in it.
    ///
    /// For naming an agent a tenant's resource points at to someone who
    /// need not own, install or share a tenant with it: a coaching group's
    /// AI agent, read by a member who joined from another tenant.
    async fn get_in_tenant(&self, agent_id: &str, tenant_id: TenantId) -> AppResult<Option<Agent>>;
    /// Resolve an installed agent by its catalogue handle for one user.
    ///
    /// "Installed" means the agent sits on the user's agent list through a
    /// `agent_assignments` row — a Store install, a fork, or an admin
    /// assignment — and belongs to the user's tenant or is a system agent.
    /// An agent the user merely *could* browse in the catalogue does not
    /// resolve. When both the user's own copy and the origin answer to the
    /// handle, the user's copy wins; among several copies the oldest wins.
    async fn find_installed_by_handle(
        &self,
        handle: &AgentHandle,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Agent>>;
    /// List agents with optional filtering
    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        filter: &ListAgentsFilter,
    ) -> AppResult<Vec<AgentListItem>>;
    /// Apply per-locale translation overlays to a list of agents.
    ///
    /// Reads `agent_translations` for `locale` and overlays
    /// `title`/`description`/`purpose`/`instructions` on each matching agent
    /// in-place. Coaches without a matching translation row keep their
    /// canonical English copy. Called after `list` when a channel locale has
    /// been resolved.
    ///
    /// Fast-path: `locale == "en"` returns immediately without touching the
    /// database. English lives on the canonical `agents` row itself; a
    /// `agent_translations` row with `locale = "en"` is only interesting when
    /// an operator wants to override the canonical, which is out of scope for
    /// Phase 1.
    async fn apply_translations(&self, agents: &mut [AgentListItem], locale: &str)
        -> AppResult<()>;
    /// [`Self::apply_translations`] for bare agent rows — the store listing
    /// and the catalogue detail carry a [`Agent`] without the list-item
    /// wrapper, and a French athlete browsing the store reads the same
    /// `agent_translations` overlay the chat's `/agent list` already shows.
    async fn translate_agents(&self, agents: &mut [Agent], locale: &str) -> AppResult<()>;
    /// Update an existing agent.
    ///
    /// Snapshots the pre-update state as a new version before applying the
    /// changes; `change_summary` is recorded on that snapshot so the history
    /// says what the edit was for. `None` records an unsummarized edit.
    async fn update(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateAgentRequest,
        change_summary: Option<&str>,
    ) -> AppResult<Option<Agent>>;
    /// Delete an agent
    async fn delete(&self, agent_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool>;
    /// Record a usage event for an agent interaction
    async fn record_usage(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
    /// Toggle favorite status for an agent
    async fn toggle_favorite(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<bool>>;
    /// Search coachs by text query
    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Agent>>;
    /// Count coachs
    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32>;

    // --- User methods ---

    /// Fork an agent into a user-owned copy
    async fn fork_agent(
        &self,
        source_agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Agent>;
    /// Activate an agent for the user
    async fn activate_agent(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Agent>>;
    /// Deactivate the user's currently active agent
    async fn deactivate_agent(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool>;
    /// Get the user's currently active agent
    async fn get_active_agent(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Agent>>;

    /// Find an agent by content hash for import deduplication
    async fn find_by_content_hash(
        &self,
        content_hash: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Agent>>;

    // --- Admin methods ---

    /// Create a system-level agent
    async fn create_system_agent(
        &self,
        admin_user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateSystemAgentRequest,
    ) -> AppResult<Agent>;
    /// List all system agents for a tenant
    async fn list_system_agents(&self, tenant_id: TenantId) -> AppResult<Vec<Agent>>;
    /// Get a system agent by ID within a tenant
    async fn get_system_agent(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<Agent>>;
    /// Get a system agent by ID regardless of tenant
    async fn get_system_agent_any_tenant(&self, agent_id: &str) -> AppResult<Option<Agent>>;
    /// Update a system agent
    async fn update_system_agent(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
        request: &UpdateAgentRequest,
    ) -> AppResult<Option<Agent>>;
    /// Delete a system agent
    async fn delete_system_agent(&self, agent_id: &str, tenant_id: TenantId) -> AppResult<bool>;

    // --- Assignment methods ---

    /// Get user preferences for an agent (`is_favorite`, `is_hidden`, `usage_count`, `last_used_at`)
    /// Per-user agent preferences: `(is_favorite, use_count, last_used_at)`.
    ///
    /// No longer reports an "active" flag. Selection moved to
    /// `tenant_users.selected_agent_id` and `agent_assignments.is_active` was
    /// dropped with it; this returned the column long enough for the only
    /// caller to bind it to `_is_active` and ignore it.
    async fn get_user_preferences(
        &self,
        agent_id: &str,
        user_id: Uuid,
    ) -> AppResult<(bool, u32, Option<DateTime<Utc>>)>;
    /// Assign an agent to a user
    async fn assign_agent(
        &self,
        agent_id: &str,
        user_id: Uuid,
        assigned_by: Uuid,
    ) -> AppResult<bool>;
    /// Unassign an agent from a user
    async fn unassign_agent(&self, agent_id: &str, user_id: Uuid) -> AppResult<bool>;
    /// List all assignments for an agent
    async fn list_assignments(&self, agent_id: &str) -> AppResult<Vec<AgentAssignment>>;
    /// List assignments for an agent within a tenant
    async fn list_assignments_for_tenant(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<AgentAssignment>>;
    /// Hide an agent from the user's view. Tenant-scoped: an assigned agent
    /// is hideable only inside the tenant that owns it, so a foreign
    /// tenant's agent id answers exactly like a nonexistent one.
    async fn hide_agent(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
    /// Show a previously hidden agent. User-scoped, not tenant-scoped, by
    /// design: `user_agent_preferences` carries no `tenant_id` column because
    /// hiding is a personal preference on an agent the user can already see
    /// (a system agent, or one assigned to them), so the delete is keyed on
    /// `(user_id, agent_id)` alone. Each handler still refuses a caller with
    /// no resolved tenant, like every sibling agent handler.
    async fn show_agent(&self, agent_id: &str, user_id: Uuid) -> AppResult<bool>;
    /// List agents hidden by a user
    async fn list_hidden_agents(&self, user_id: Uuid, tenant_id: TenantId)
        -> AppResult<Vec<Agent>>;

    // --- Version methods ---

    /// Create a new version snapshot for an agent
    async fn create_version(
        &self,
        agent_id: &str,
        user_id: Uuid,
        change_summary: Option<&str>,
    ) -> AppResult<i32>;
    /// Get version history for an agent
    async fn get_versions(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
        limit: u32,
    ) -> AppResult<Vec<AgentVersion>>;
    /// Get a specific version of an agent
    async fn get_version(
        &self,
        agent_id: &str,
        version: i32,
        tenant_id: TenantId,
    ) -> AppResult<Option<AgentVersion>>;
    /// Revert an agent to a previous version
    async fn revert_to_version(
        &self,
        agent_id: &str,
        version: i32,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Agent>;
    /// Get the current version number for an agent
    async fn get_current_version(&self, agent_id: &str) -> AppResult<i32>;

    /// Resolve the full runtime context (system prompt, startup query, data
    /// requirements, tool-iteration override) for an agent attached to a
    /// conversation.
    ///
    /// Tenant-scoped: returns the agent if it belongs to the caller's tenant
    /// or is a system agent. Returns `None` if no matching agent is found.
    async fn get_agent_runtime_context(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<AgentRuntimeContext>>;
}

/// Coaching group storage, membership management, and invite tracking
#[async_trait]
pub trait CoachingGroupRepository: Send + Sync {
    // -- Group CRUD --

    /// Create a new coaching group
    async fn create_group(
        &self,
        tenant_id: TenantId,
        group: &CoachingGroup,
    ) -> AppResult<CoachingGroup>;

    /// Get a group by ID with tenant isolation
    async fn get_group(
        &self,
        group_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachingGroup>>;

    /// Look up the active group bound to a specific messaging chat.
    ///
    /// Returns `Some(group)` if a `coaching_groups` row was created from
    /// this chat (Telegram group, Slack channel, Discord channel) and is
    /// still active. Returns `None` for unknown chats or REST-created
    /// (web/mobile) groups that have no channel binding.
    async fn get_group_by_channel(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_chat_id: &str,
    ) -> AppResult<Option<CoachingGroup>>;

    /// List groups the user belongs to (as member, admin, or owner).
    /// Membership-based lookup — no tenant filter since members join cross-tenant.
    async fn list_groups_for_user(&self, user_id: Uuid) -> AppResult<Vec<GroupSummary>>;

    /// List groups that use a specific agent persona
    async fn list_groups_for_agent(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachingGroup>>;

    /// List active groups where the given user is the attached human coach
    /// (`coach_user_id`). No tenant filter — groups span tenants and the
    /// agent attachment is the access key.
    async fn list_groups_coached_by(&self, coach_user_id: Uuid) -> AppResult<Vec<CoachingGroup>>;

    /// The athletes a human coach coaches: every live member of every active
    /// group whose `coach_user_id` is `coach_user_id`, each user once. No
    /// tenant filter, for the same reason as [`Self::list_groups_coached_by`].
    /// Empty for a user who coaches no group.
    async fn list_athletes_coached_by(&self, coach_user_id: Uuid) -> AppResult<Vec<Uuid>>;

    /// List every active group owned by a tenant.
    ///
    /// Used by the weekly-digest scheduler to enumerate the groups eligible
    /// for a periodic report. Tenant-scoped so the cross-tenant sweep stays a
    /// loop of per-tenant queries rather than an unscoped table scan.
    async fn list_active_groups_for_tenant(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachingGroup>>;

    /// Update a coaching group
    async fn update_group(
        &self,
        group_id: &str,
        tenant_id: TenantId,
        request: &UpdateGroupRequest,
    ) -> AppResult<Option<CoachingGroup>>;

    /// Soft-delete a coaching group (sets `is_active` = false)
    async fn delete_group(&self, group_id: &str, tenant_id: TenantId) -> AppResult<bool>;

    /// Attach or clear the human coach (`coach_user_id`) for a group.
    /// Pass `None` to detach. Tenant-scoped write — the agent must belong to
    /// the group's tenant (enforced by the caller before attaching).
    async fn set_group_coach_user(
        &self,
        group_id: &str,
        coach_user_id: Option<Uuid>,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    // -- Membership --

    /// Add a member to a group
    async fn add_member(&self, member: &GroupMember) -> AppResult<GroupMember>;

    /// Remove a member from a group (soft removal via `left_at` timestamp).
    /// No tenant filter — members join cross-tenant via invite codes.
    async fn remove_member(&self, group_id: &str, user_id: Uuid) -> AppResult<bool>;

    /// Get member by `group_id` + `user_id` (unique constraint, no tenant filter needed)
    async fn get_member(&self, group_id: &str, user_id: Uuid) -> AppResult<Option<GroupMember>>;

    /// Whether `user_id` may read `group_id`: its info, its members and its
    /// room.
    ///
    /// Only an active group admits anyone, as only an active group is
    /// listed; then a live member is admitted, and so is the group's human
    /// coach, who holds no membership row. Every surface that admits the coach
    /// alongside the members asks here, so none can admit a different set.
    async fn admits_member_or_coach(
        &self,
        group_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let Some(group) = self.get_group(group_id, tenant_id).await? else {
            return Ok(false);
        };
        if !group.is_active {
            return Ok(false);
        }
        if group.coach_user_id == Some(user_id) {
            return Ok(true);
        }
        Ok(self.get_member(group_id, user_id).await?.is_some())
    }

    /// List active members of a group.
    /// No tenant filter — members join cross-tenant via invite codes.
    async fn list_members(&self, group_id: &str) -> AppResult<Vec<GroupMember>>;

    /// Update a member's role.
    /// No tenant filter — admins manage cross-tenant members.
    async fn update_member_role(
        &self,
        group_id: &str,
        user_id: Uuid,
        role: GroupRole,
    ) -> AppResult<bool>;

    /// Update a member's peer sharing consent.
    /// No tenant filter — members update their own consent cross-tenant.
    async fn update_peer_sharing_consent(
        &self,
        group_id: &str,
        user_id: Uuid,
        consent: bool,
    ) -> AppResult<bool>;

    /// Count active members in a group.
    /// No tenant filter — members join cross-tenant via invite codes.
    async fn count_members(&self, group_id: &str) -> AppResult<i64>;

    // -- Invites --

    /// Create a group invite
    async fn create_invite(&self, invite: &GroupInvite) -> AppResult<GroupInvite>;

    /// Look up an invite by its code (cross-tenant for join flow)
    async fn get_invite_by_code(&self, code: &str) -> AppResult<Option<GroupInvite>>;

    /// Increment the use count of an invite
    async fn increment_invite_use_count(&self, invite_id: &str) -> AppResult<bool>;

    /// Deactivate an invite, scoped to its owning group.
    /// The `group_id` filter prevents a group admin from deactivating an invite
    /// that belongs to a different group (IDOR); returns `false` (not found) when
    /// the invite does not exist or belongs to another group.
    async fn deactivate_invite(&self, group_id: &str, invite_id: &str) -> AppResult<bool>;

    /// List invites for a group.
    /// No tenant filter — cross-tenant admins view invites by `group_id`.
    async fn list_invites(&self, group_id: &str) -> AppResult<Vec<GroupInvite>>;

    // -- Context queries --

    /// Find groups a user belongs to that use a specific agent.
    /// No tenant filter — groups span tenants via cross-tenant membership.
    async fn find_groups_for_user_and_agent(
        &self,
        user_id: Uuid,
        agent_id: &str,
    ) -> AppResult<Vec<CoachingGroup>>;

    /// Count groups owned by a user (for tier limit enforcement)
    async fn count_groups_for_owner(&self, owner_id: Uuid, tenant_id: TenantId) -> AppResult<i64>;

    // -- Room transcript (surface-neutral read model) --

    /// Append one utterance to the group's shared room transcript.
    ///
    /// Called by chat-pipeline persistence for every user/assistant row of a
    /// group-bound conversation (whatever surface the turn arrived on), and
    /// by the messaging ingress for ambient room chatter. The entry id and
    /// timestamp are minted by the implementation.
    async fn append_transcript_entry(&self, entry: &NewGroupTranscriptEntry<'_>) -> AppResult<()>;

    /// Read the newest transcript entries the viewer may see, newest first.
    ///
    /// Consent-gated exactly like the peer-grounding fetch: another member's
    /// content is visible only when the group's `peer_data_sharing`
    /// kill-switch is on AND that member's own `peer_sharing_consent` is on
    /// (and they have not left). The viewer's own entries — including the
    /// agent replies attributed to them — are always visible to them.
    /// No tenant filter — membership is cross-tenant, same as `list_members`;
    /// callers gate access by verifying the viewer's membership first.
    /// `limit` is clamped to `1..=500`.
    async fn list_transcript_visible_to(
        &self,
        group_id: &str,
        viewer_user_id: Uuid,
        limit: i64,
    ) -> AppResult<Vec<GroupTranscriptEntry>>;

    // -- Weekly digest deliveries --

    /// Take one group's weekly digest for one week, if nobody has.
    ///
    /// `week_key` names the ISO week in the group's own calendar. The first
    /// claim for a week inserts its row and wins; a later one wins only when
    /// that week was never finished and the earlier claim's lease has run out
    /// — an instance that died mid-send. On success the lease runs to
    /// `now_ms + lease_ms` and `true` comes back. The check and the write are
    /// one statement, so two instances asking at once get one `true` between
    /// them.
    async fn claim_group_digest(
        &self,
        tenant_id: TenantId,
        group_id: Uuid,
        week_key: &str,
        now_ms: i64,
        lease_ms: i64,
    ) -> AppResult<bool>;

    /// Record that the week's digest attempt is over, whatever it delivered:
    /// no later claim for that group and week wins.
    async fn finish_group_digest(
        &self,
        tenant_id: TenantId,
        group_id: Uuid,
        week_key: &str,
        now_ms: i64,
    ) -> AppResult<()>;
}
