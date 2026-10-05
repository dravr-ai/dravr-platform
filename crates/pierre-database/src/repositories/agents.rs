// ABOUTME: Repository trait definitions for the agents catalogue and coaching groups domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::agents::{
    Agent, AgentAssignment, AgentCategory, AgentHandle, AgentListItem, AgentVersion,
    CreateAgentRequest, CreateSystemAgentRequest, ListAgentsFilter, UpdateAgentRequest,
};
use pierre_core::models::groups::{
    CoachingGroup, GroupInvite, GroupMember, GroupRole, GroupSummary, GroupTranscriptEntry,
    NewGroupTranscriptEntry, RoomTranscriptEntry, UpdateGroupRequest,
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
    /// Search the user's own agents by text query, within one category when
    /// `category` names one
    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        category: Option<AgentCategory>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Agent>>;
    /// Count coachs
    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32>;

    // --- User methods ---

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

    /// Update whether a member shares their training data with the group's
    /// human coach. Returns `false` when `user_id` is not a live member.
    /// No tenant filter — members update their own consent cross-tenant,
    /// exactly as [`Self::update_peer_sharing_consent`].
    async fn update_coach_sharing_consent(
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
    /// Consent-gated exactly like the group-member fetch: another member's
    /// content is visible to a peer only when the group's `peer_data_sharing`
    /// kill-switch is on AND that member's own `peer_sharing_consent` is on,
    /// and to the group's human coach when that member's
    /// `coach_sharing_consent` is on (and, either way, they have not left).
    /// The viewer's own entries — including the
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

    /// Read one page of the room as the viewer reads it, newest first.
    ///
    /// Unlike [`Self::list_transcript_visible_to`], every entry comes back:
    /// the one consent rule both reads share decides only what an entry
    /// carries, so an entry the viewer may not read arrives as
    /// [`RoomEntryBody::Withheld`](pierre_core::models::groups::RoomEntryBody::Withheld)
    /// — its place in the room without its words or its author. `before` is
    /// the id of the oldest entry the viewer already holds (`None` for the
    /// newest page); the page holds the entries strictly older than it.
    /// No tenant filter — membership is cross-tenant, same as `list_members`;
    /// callers gate access by verifying the viewer's membership first.
    /// `limit` is clamped to `1..=500`.
    async fn list_room_transcript_for(
        &self,
        group_id: &str,
        viewer_user_id: Uuid,
        before: Option<Uuid>,
        limit: i64,
    ) -> AppResult<Vec<RoomTranscriptEntry>>;

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

// ============================================================================
// Agents catalogue, written once
// ============================================================================
//
// One SQL text per operation for both backends; `agents_backend.rs` emits the
// `AgentsRepository` impl over these for each backend type.
//
// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
// Postgres. Booleans bind as `bool` and are spelled `TRUE`/`FALSE`: Postgres
// has the type, `SQLite` stores 1/0 and reads them back as `bool`. Timestamps
// bind as `DateTime<Utc>` on both; sqlx-sqlite writes the RFC 3339 text the
// `SQLite` TEXT columns always held. `agents.user_id` and every other
// `users(id)` reference are `uuid` on Postgres and TEXT on `SQLite`, so the
// shared body binds them through a [`super::uuid_columns`] codec.
// `agents.tenant_id` and `tenant_users.tenant_id` bind as a `TenantId`, which
// encodes as a uuid on Postgres and as its text on `SQLite`.

/// The columns every agent read decodes, each prefixed with `$p` (`""` for the
/// bare table, `"c."` inside a join). One list, so a column added to `Agent`
/// reaches every read at once.
macro_rules! agent_columns {
    ($p:literal) => {
        concat!(
            $p,
            "id, ",
            $p,
            "user_id, ",
            $p,
            "tenant_id, ",
            $p,
            "title, ",
            $p,
            "description, ",
            $p,
            "system_prompt, ",
            $p,
            "category, ",
            $p,
            "tags, ",
            $p,
            "sample_prompts, ",
            $p,
            "token_count, ",
            $p,
            "created_at, ",
            $p,
            "updated_at, ",
            $p,
            "is_system, ",
            $p,
            "visibility, ",
            $p,
            "prerequisites, ",
            $p,
            "forked_from, ",
            $p,
            "slug, ",
            $p,
            "max_tool_iterations, ",
            $p,
            "temperature, ",
            $p,
            "startup_query, ",
            $p,
            "data_requirements, ",
            $p,
            "purpose, ",
            $p,
            "when_to_use, ",
            $p,
            "instructions, ",
            $p,
            "example_inputs, ",
            $p,
            "example_outputs, ",
            $p,
            "success_criteria"
        )
    };
}

/// Give a user an assignment row for an agent unless they already hold one;
/// the user assigns themselves. Favourites, usage and selection all update
/// that row, so each of them makes sure it exists first.
pub(crate) const ENSURE_ASSIGNMENT_SQL: &str = r"
    INSERT INTO agent_assignments (id, agent_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
    VALUES ($1, $2, $3, $3, $4, FALSE, 0, NULL)
    ON CONFLICT (agent_id, user_id) DO NOTHING
";

/// Whether an agent is a system agent.
pub(crate) const AGENT_IS_SYSTEM_SQL: &str =
    "SELECT 1 FROM agents WHERE id = $1 AND is_system = TRUE";

/// Whether an agent of the caller's tenant is assigned to the caller. The
/// tenant join keeps an agent id from another tenant answering the same as a
/// nonexistent one.
pub(crate) const AGENT_ASSIGNED_IN_TENANT_SQL: &str = r"
    SELECT 1 FROM agent_assignments ca
    INNER JOIN agents c ON c.id = ca.agent_id
    WHERE ca.agent_id = $1 AND ca.user_id = $2 AND c.tenant_id = $3
";

/// Create a user's agent.
pub(crate) const INSERT_AGENT_SQL: &str = r"
    INSERT INTO agents (
        id, user_id, tenant_id, title, description, system_prompt,
        category, tags, sample_prompts, token_count,
        created_at, updated_at, is_system, visibility, prerequisites,
        forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
        purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria,
        content_hash
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, FALSE, $12, NULL, NULL, $13, NULL,
              $14, $15, $16, $17, $18, $19, $20, $21, $22)
";

/// An agent the user owns in the tenant, a system agent, or one assigned to them.
pub(crate) const GET_AGENT_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE id = $1 AND (
        (user_id = $2 AND tenant_id = $3)
        OR is_system = TRUE
        OR id IN (SELECT agent_id FROM agent_assignments WHERE user_id = $2)
    )"
);

/// An agent of the tenant, or a system agent.
pub(crate) const GET_AGENT_IN_TENANT_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE id = $1 AND (tenant_id = $2 OR is_system = TRUE)"
);

/// The copy of a handle the user has installed: their own first, then the
/// oldest.
pub(crate) const FIND_INSTALLED_BY_HANDLE_SQL: &str = concat!(
    "SELECT ",
    agent_columns!("c."),
    " FROM agents c
    JOIN agent_assignments ca ON ca.agent_id = c.id AND ca.user_id = $1
    WHERE c.slug = $2 AND (c.tenant_id = $3 OR c.is_system = TRUE)
    ORDER BY CASE WHEN c.user_id = $1 THEN 0 ELSE 1 END, c.created_at ASC
    LIMIT 1"
);

/// The agents a user sees, with their per-user assignment state.
///
/// The filters are parameters, not composed text: `$5` includes system
/// agents, `$6` restricts to one category when not NULL, `$7` keeps
/// favourites only, and `$8` includes the agents the user hid.
pub(crate) const LIST_AGENTS_SQL: &str = concat!(
    "SELECT ",
    agent_columns!("c."),
    ",
        CASE WHEN ca.agent_id IS NOT NULL THEN TRUE ELSE FALSE END AS is_assigned,
        COALESCE(ca.is_favorite, FALSE) AS is_favorite,
        CASE WHEN tu.selected_agent_id = c.id THEN TRUE ELSE FALSE END AS is_active,
        COALESCE(ca.use_count, 0) AS use_count,
        ca.last_used_at
    FROM agents c
    LEFT JOIN agent_assignments ca ON c.id = ca.agent_id AND ca.user_id = $1
    LEFT JOIN tenant_users tu ON tu.user_id = $1 AND tu.tenant_id = $2
    WHERE (
        (c.user_id = $1 AND c.is_system = FALSE AND c.tenant_id = $2)
        OR ($5 AND c.is_system = TRUE)
        OR c.id IN (SELECT agent_id FROM agent_assignments WHERE user_id = $1)
    )
    AND ($6 IS NULL OR c.category = $6)
    AND (NOT $7 OR ca.is_favorite = TRUE)
    AND ($8 OR c.id NOT IN (
        SELECT agent_id FROM user_agent_preferences WHERE user_id = $1 AND is_hidden = TRUE))
    ORDER BY c.updated_at DESC LIMIT $3 OFFSET $4"
);

/// The stored startup query, kept by an update that does not name one.
pub(crate) const AGENT_STARTUP_QUERY_SQL: &str = "SELECT startup_query FROM agents WHERE id = $1";

/// The stored data requirements, kept by an update that does not name them.
pub(crate) const AGENT_DATA_REQUIREMENTS_SQL: &str =
    "SELECT data_requirements FROM agents WHERE id = $1";

/// Update a user's own agent.
pub(crate) const UPDATE_AGENT_SQL: &str = r"
    UPDATE agents SET
        title = $1, description = $2, system_prompt = $3,
        category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8,
        startup_query = $12, data_requirements = $13,
        purpose = $14, when_to_use = $15, instructions = $16,
        example_inputs = $17, example_outputs = $18, success_criteria = $19,
        max_tool_iterations = $20
    WHERE id = $9 AND user_id = $10 AND tenant_id = $11
";

/// Delete a user's own agent.
pub(crate) const DELETE_AGENT_SQL: &str =
    "DELETE FROM agents WHERE id = $1 AND user_id = $2 AND tenant_id = $3";

/// Whether the caller can reach an agent: one of their tenant, or a system
/// agent. System agents are pinned to the seed tenant but exposed to every
/// tenant through the catalogue, so usage, favourites and selection accept
/// them unconditionally.
pub(crate) const AGENT_REACHABLE_SQL: &str =
    "SELECT 1 FROM agents WHERE id = $1 AND (tenant_id = $2 OR is_system = TRUE)";

/// Count one use of an agent.
pub(crate) const RECORD_USAGE_SQL: &str = r"
    UPDATE agent_assignments SET use_count = use_count + 1, last_used_at = $1
    WHERE agent_id = $2 AND user_id = $3
";

/// Whether the user marked an agent a favourite.
pub(crate) const FAVORITE_STATUS_SQL: &str =
    "SELECT is_favorite FROM agent_assignments WHERE agent_id = $1 AND user_id = $2";

/// Set the favourite flag.
pub(crate) const SET_FAVORITE_SQL: &str =
    "UPDATE agent_assignments SET is_favorite = $1 WHERE agent_id = $2 AND user_id = $3";

/// Search a user's own agents by title, description or tags, ignoring case,
/// restricted to one category when `$6` is not NULL.
/// `LOWER` on both sides rather than Postgres's `ILIKE`, which `SQLite` lacks;
/// `SQLite`'s `LOWER` folds ASCII letters only, as its `LIKE` does.
pub(crate) const SEARCH_AGENTS_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE user_id = $1 AND tenant_id = $2 AND (
        LOWER(title) LIKE LOWER($3) OR LOWER(description) LIKE LOWER($3)
        OR LOWER(tags) LIKE LOWER($3))
    AND ($6 IS NULL OR category = $6)
    ORDER BY updated_at DESC LIMIT $4 OFFSET $5"
);

/// How many agents a user owns in the tenant.
pub(crate) const COUNT_AGENTS_SQL: &str =
    "SELECT COUNT(*) AS count FROM agents WHERE user_id = $1 AND tenant_id = $2";

/// Select the agent a user talks to: one pointer on their membership row, so
/// they can never hold zero or two.
pub(crate) const SELECT_AGENT_SQL: &str =
    "UPDATE tenant_users SET selected_agent_id = $1 WHERE user_id = $2 AND tenant_id = $3";

/// Clear the selection, leaving the roster row intact.
pub(crate) const DESELECT_AGENT_SQL: &str = r"
    UPDATE tenant_users SET selected_agent_id = NULL
    WHERE user_id = $1 AND tenant_id = $2 AND selected_agent_id IS NOT NULL
";

/// The agent a user selected in the tenant.
pub(crate) const GET_ACTIVE_AGENT_SQL: &str = concat!(
    "SELECT ",
    agent_columns!("c."),
    " FROM agents c
    JOIN tenant_users tu ON c.id = tu.selected_agent_id
    WHERE tu.user_id = $1 AND tu.tenant_id = $2"
);

/// A user's agent with a given content hash, for deduplicating a create.
pub(crate) const FIND_BY_CONTENT_HASH_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE content_hash = $1 AND user_id = $2 AND tenant_id = $3 LIMIT 1"
);

/// Create a system agent.
pub(crate) const INSERT_SYSTEM_AGENT_SQL: &str = r"
    INSERT INTO agents (
        id, user_id, tenant_id, title, description, system_prompt,
        category, tags, sample_prompts, token_count,
        created_at, updated_at, is_system, visibility, prerequisites,
        forked_from, max_tool_iterations, temperature, startup_query, data_requirements
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, TRUE, $12,
              NULL, NULL, NULL, NULL, NULL, NULL)
";

/// A tenant's system agents, newest first.
pub(crate) const LIST_SYSTEM_AGENTS_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE tenant_id = $1 AND is_system = TRUE ORDER BY created_at DESC"
);

/// One system agent of the tenant.
pub(crate) const GET_SYSTEM_AGENT_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE id = $1 AND tenant_id = $2 AND is_system = TRUE"
);

/// Update a system agent's content.
pub(crate) const UPDATE_SYSTEM_AGENT_SQL: &str = r"
    UPDATE agents SET title = $1, description = $2, system_prompt = $3,
        category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
    WHERE id = $9 AND tenant_id = $10 AND is_system = TRUE
";

/// Delete a system agent.
pub(crate) const DELETE_SYSTEM_AGENT_SQL: &str =
    "DELETE FROM agents WHERE id = $1 AND tenant_id = $2 AND is_system = TRUE";

/// A user's favourite flag, use count and last use of an agent.
pub(crate) const USER_PREFERENCES_SQL: &str = r"
    SELECT is_favorite, use_count, last_used_at
    FROM agent_assignments WHERE agent_id = $1 AND user_id = $2
";

/// Assign an agent to a user on someone's behalf; a second assignment is a no-op.
pub(crate) const ASSIGN_AGENT_SQL: &str = r"
    INSERT INTO agent_assignments (id, agent_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
    VALUES ($1, $2, $3, $4, $5, FALSE, 0, NULL)
    ON CONFLICT (agent_id, user_id) DO NOTHING
";

/// Remove a user's assignment.
pub(crate) const UNASSIGN_AGENT_SQL: &str =
    "DELETE FROM agent_assignments WHERE agent_id = $1 AND user_id = $2";

/// Every assignment of an agent, newest first.
pub(crate) const LIST_ASSIGNMENTS_SQL: &str = r"
    SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
    FROM agent_assignments ca LEFT JOIN users u ON ca.user_id = u.id
    WHERE ca.agent_id = $1 ORDER BY ca.created_at DESC
";

/// The assignments of an agent held by members of one tenant, newest first.
pub(crate) const LIST_ASSIGNMENTS_FOR_TENANT_SQL: &str = r"
    SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
    FROM agent_assignments ca LEFT JOIN users u ON ca.user_id = u.id
    INNER JOIN tenant_users tu ON ca.user_id = tu.user_id AND tu.tenant_id = $2
    WHERE ca.agent_id = $1 ORDER BY ca.created_at DESC
";

/// Hide an agent from a user's list.
pub(crate) const HIDE_AGENT_SQL: &str = r"
    INSERT INTO user_agent_preferences (id, user_id, agent_id, is_hidden, created_at)
    VALUES ($1, $2, $3, TRUE, $4)
    ON CONFLICT (user_id, agent_id) DO UPDATE SET is_hidden = TRUE
";

/// Show a hidden agent again. `user_agent_preferences` has no tenant column:
/// the hidden set is a per-user preference and the handler gates the tenant.
pub(crate) const SHOW_AGENT_SQL: &str =
    "DELETE FROM user_agent_preferences WHERE agent_id = $1 AND user_id = $2 AND is_hidden = TRUE";

/// The agents of the tenant a user hid, by title.
pub(crate) const LIST_HIDDEN_AGENTS_SQL: &str = concat!(
    "SELECT ",
    agent_columns!("c."),
    " FROM agents c
    INNER JOIN user_agent_preferences ucp ON c.id = ucp.agent_id
    WHERE ucp.user_id = $1 AND ucp.is_hidden = TRUE AND c.tenant_id = $2
    ORDER BY c.title"
);

/// An agent by id alone, for the version snapshot.
pub(crate) const AGENT_BY_ID_SQL: &str =
    concat!("SELECT ", agent_columns!(""), " FROM agents WHERE id = $1");

/// An agent of the tenant by id, read back after a revert.
pub(crate) const AGENT_OF_TENANT_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE id = $1 AND tenant_id = $2"
);

/// Whether an agent belongs to the tenant.
pub(crate) const AGENT_IN_TENANT_EXISTS_SQL: &str =
    "SELECT 1 FROM agents WHERE id = $1 AND tenant_id = $2";

/// An agent's latest version number, 0 before its first snapshot.
pub(crate) const MAX_VERSION_SQL: &str =
    "SELECT COALESCE(MAX(version), 0) AS max_version FROM agent_versions WHERE agent_id = $1";

/// Record a version snapshot.
pub(crate) const INSERT_VERSION_SQL: &str = r"
    INSERT INTO agent_versions (id, agent_id, version, content_hash, content_snapshot, change_summary, created_at, created_by)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
";

/// An agent's versions, newest first.
pub(crate) const LIST_VERSIONS_SQL: &str = r"
    SELECT id, agent_id, version, content_hash, content_snapshot, change_summary, created_at, created_by
    FROM agent_versions WHERE agent_id = $1 ORDER BY version DESC LIMIT $2
";

/// One version of an agent.
pub(crate) const GET_VERSION_SQL: &str = r"
    SELECT id, agent_id, version, content_hash, content_snapshot, change_summary, created_at, created_by
    FROM agent_versions WHERE agent_id = $1 AND version = $2
";

/// Restore a snapshot onto the agent. Owner-gated like `UPDATE_AGENT_SQL`:
/// a non-owner in the same tenant matches no row, which closes the
/// version-revert IDOR.
pub(crate) const REVERT_AGENT_SQL: &str = r"
    UPDATE agents SET title = $1, description = $2, system_prompt = $3,
        category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
    WHERE id = $9 AND user_id = $10 AND tenant_id = $11
";

/// What a chat turn needs of an agent of the tenant, or a system agent.
pub(crate) const RUNTIME_CONTEXT_SQL: &str = r"
    SELECT slug, title, source, system_prompt, startup_query, data_requirements, visuals,
           max_tool_iterations, temperature, category
    FROM agents WHERE id = $1 AND (tenant_id = $2 OR is_system = TRUE) LIMIT 1
";

/// The translation overlays of `count` agents in one locale: `$1` is the
/// locale and `$2`… the agent ids.
pub(crate) fn translation_overlays_sql(count: usize) -> String {
    let placeholders: Vec<String> = (0..count).map(|i| format!("${}", i + 2)).collect();
    format!(
        "SELECT agent_id, title, description, purpose, instructions, tags, sample_prompts \
         FROM agent_translations WHERE locale = $1 AND agent_id IN ({})",
        placeholders.join(", ")
    )
}
