// ABOUTME: Repository trait definitions for the coaches catalogue, coaching groups, store listings domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::coaches::{
    Coach, CoachAssignment, CoachCategory, CoachHandle, CoachListItem, CoachVersion,
    CreateCoachRequest, CreateSystemCoachRequest, ListCoachesFilter, StoreAdminStats,
    UpdateCoachRequest,
};
use pierre_core::models::groups::{
    CoachingGroup, GroupInvite, GroupMember, GroupRole, GroupSummary, GroupTranscriptEntry,
    NewGroupTranscriptEntry, UpdateGroupRequest,
};

use pierre_core::models::CoachRuntimeContext;
use pierre_core::models::TenantId;
use pierre_core::pagination::{CursorPage, StoreSortOrder};
use std::collections::HashMap;
use uuid::Uuid;

use crate::database::store_listings::{CoachWithListing, StoreListing};

/// Coaches (custom AI personas) storage and management repository (tenant-scoped)
#[async_trait]
pub trait CoachesRepository: Send + Sync {
    /// Create a new coach
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateCoachRequest,
    ) -> AppResult<Coach>;
    /// Get coach by ID
    async fn get_by_id(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;
    /// Resolve an installed coach by its catalogue handle for one user.
    ///
    /// "Installed" means the coach sits on the user's coach list through a
    /// `coach_assignments` row — a Store install, a fork, or an admin
    /// assignment — and belongs to the user's tenant or is a system coach.
    /// A coach the user merely *could* browse in the catalogue does not
    /// resolve. When both the user's own copy and the origin answer to the
    /// handle, the user's copy wins; among several copies the oldest wins.
    async fn find_installed_by_handle(
        &self,
        handle: &CoachHandle,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;
    /// List coaches with optional filtering
    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        filter: &ListCoachesFilter,
    ) -> AppResult<Vec<CoachListItem>>;
    /// Apply per-locale translation overlays to a list of coaches.
    ///
    /// Reads `coach_translations` for `locale` and overlays
    /// `title`/`description`/`purpose`/`instructions` on each matching coach
    /// in-place. Coaches without a matching translation row keep their
    /// canonical English copy. Called after `list` when a channel locale has
    /// been resolved.
    ///
    /// Fast-path: `locale == "en"` returns immediately without touching the
    /// database. English lives on the canonical `coaches` row itself; a
    /// `coach_translations` row with `locale = "en"` is only interesting when
    /// an operator wants to override the canonical, which is out of scope for
    /// Phase 1.
    async fn apply_translations(
        &self,
        coaches: &mut [CoachListItem],
        locale: &str,
    ) -> AppResult<()>;
    /// [`Self::apply_translations`] for bare coach rows — the store listing
    /// and the catalogue detail carry a [`Coach`] without the list-item
    /// wrapper, and a French athlete browsing the store reads the same
    /// `coach_translations` overlay the chat's `/coach list` already shows.
    async fn translate_coaches(&self, coaches: &mut [Coach], locale: &str) -> AppResult<()>;
    /// Update an existing coach.
    ///
    /// Snapshots the pre-update state as a new version before applying the
    /// changes; `change_summary` is recorded on that snapshot so the history
    /// says what the edit was for. `None` records an unsummarized edit.
    async fn update(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
        change_summary: Option<&str>,
    ) -> AppResult<Option<Coach>>;
    /// Delete a coach
    async fn delete(&self, coach_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool>;
    /// Record a usage event for a coach interaction
    async fn record_usage(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
    /// Toggle favorite status for a coach
    async fn toggle_favorite(
        &self,
        coach_id: &str,
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
    ) -> AppResult<Vec<Coach>>;
    /// Count coachs
    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32>;

    // --- User methods ---

    /// Fork a coach into a user-owned copy
    async fn fork_coach(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach>;
    /// Activate a coach for the user
    async fn activate_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;
    /// Deactivate the user's currently active coach
    async fn deactivate_coach(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool>;
    /// Get the user's currently active coach
    async fn get_active_coach(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;

    /// Find a coach by content hash for import deduplication
    async fn find_by_content_hash(
        &self,
        content_hash: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;

    // --- Admin methods ---

    /// Create a system-level coach
    async fn create_system_coach(
        &self,
        admin_user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateSystemCoachRequest,
    ) -> AppResult<Coach>;
    /// List all system coaches for a tenant
    async fn list_system_coaches(&self, tenant_id: TenantId) -> AppResult<Vec<Coach>>;
    /// Get a system coach by ID within a tenant
    async fn get_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>>;
    /// Get a system coach by ID regardless of tenant
    async fn get_system_coach_any_tenant(&self, coach_id: &str) -> AppResult<Option<Coach>>;
    /// Update a system coach
    async fn update_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>>;
    /// Delete a system coach
    async fn delete_system_coach(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<bool>;

    // --- Assignment methods ---

    /// Get user preferences for a coach (`is_favorite`, `is_hidden`, `usage_count`, `last_used_at`)
    /// Per-user coach preferences: `(is_favorite, use_count, last_used_at)`.
    ///
    /// No longer reports an "active" flag. Selection moved to
    /// `tenant_users.selected_coach_id` and `coach_assignments.is_active` was
    /// dropped with it; this returned the column long enough for the only
    /// caller to bind it to `_is_active` and ignore it.
    async fn get_user_preferences(
        &self,
        coach_id: &str,
        user_id: Uuid,
    ) -> AppResult<(bool, u32, Option<DateTime<Utc>>)>;
    /// Assign a coach to a user
    async fn assign_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        assigned_by: Uuid,
    ) -> AppResult<bool>;
    /// Unassign a coach from a user
    async fn unassign_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool>;
    /// List all assignments for a coach
    async fn list_assignments(&self, coach_id: &str) -> AppResult<Vec<CoachAssignment>>;
    /// List assignments for a coach within a tenant
    async fn list_assignments_for_tenant(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAssignment>>;
    /// Hide a coach from the user's view. Tenant-scoped: an assigned coach
    /// is hideable only inside the tenant that owns it, so a foreign
    /// tenant's coach id answers exactly like a nonexistent one.
    async fn hide_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
    /// Show a previously hidden coach. User-scoped, not tenant-scoped, by
    /// design: `user_coach_preferences` carries no `tenant_id` column because
    /// hiding is a personal preference on a coach the user can already see
    /// (a system coach, or one assigned to them), so the delete is keyed on
    /// `(user_id, coach_id)` alone. Each handler still refuses a caller with
    /// no resolved tenant, like every sibling coach handler.
    async fn show_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool>;
    /// List coaches hidden by a user
    async fn list_hidden_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>>;

    // --- Version methods ---

    /// Create a new version snapshot for a coach
    async fn create_version(
        &self,
        coach_id: &str,
        user_id: Uuid,
        change_summary: Option<&str>,
    ) -> AppResult<i32>;
    /// Get version history for a coach
    async fn get_versions(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        limit: u32,
    ) -> AppResult<Vec<CoachVersion>>;
    /// Get a specific version of a coach
    async fn get_version(
        &self,
        coach_id: &str,
        version: i32,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachVersion>>;
    /// Revert a coach to a previous version
    async fn revert_to_version(
        &self,
        coach_id: &str,
        version: i32,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach>;
    /// Get the current version number for a coach
    async fn get_current_version(&self, coach_id: &str) -> AppResult<i32>;

    /// Resolve the full runtime context (system prompt, startup query, data
    /// requirements, tool-iteration override) for a coach attached to a
    /// conversation.
    ///
    /// Tenant-scoped: returns the coach if it belongs to the caller's tenant
    /// or is a system coach. Returns `None` if no matching coach is found.
    async fn get_coach_runtime_context(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachRuntimeContext>>;
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

    /// List groups that use a specific coach persona
    async fn list_groups_for_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachingGroup>>;

    /// List active groups where the given user is the attached human coach
    /// (`coach_user_id`). No tenant filter — groups span tenants and the
    /// coach attachment is the access key.
    async fn list_groups_coached_by(&self, coach_user_id: Uuid) -> AppResult<Vec<CoachingGroup>>;

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
    /// Pass `None` to detach. Tenant-scoped write — the coach must belong to
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

    /// Find groups a user belongs to that use a specific coach.
    /// No tenant filter — groups span tenants via cross-tenant membership.
    async fn find_groups_for_user_and_coach(
        &self,
        user_id: Uuid,
        coach_id: &str,
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
    /// coach replies attributed to them — are always visible to them.
    /// No tenant filter — membership is cross-tenant, same as `list_members`;
    /// callers gate access by verifying the viewer's membership first.
    /// `limit` is clamped to `1..=500`.
    async fn list_transcript_visible_to(
        &self,
        group_id: &str,
        viewer_user_id: Uuid,
        limit: i64,
    ) -> AppResult<Vec<GroupTranscriptEntry>>;
}

/// Store listings for the coach marketplace (cross-tenant browsing, install/uninstall)
#[async_trait]
pub trait StoreListingsRepository: Send + Sync {
    /// Submit a coach for Store review (creates listing if needed)
    async fn submit_for_review(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<StoreListing>;
    /// Get a store listing by coach ID
    async fn get_listing(&self, coach_id: &str) -> AppResult<Option<StoreListing>>;
    /// Approve a coach and publish to the Store
    async fn approve_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
    ) -> AppResult<CoachWithListing>;
    /// Reject a coach with a reason
    async fn reject_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
        reason: &str,
    ) -> AppResult<CoachWithListing>;
    /// Unpublish a coach (revert from published to draft)
    async fn unpublish_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<CoachWithListing>;
    /// Get coaches pending admin review
    async fn get_pending_review_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>>;
    /// Get coaches that have been rejected
    async fn get_rejected_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>>;
    /// Get store admin statistics
    async fn get_store_admin_stats(&self, tenant_id: TenantId) -> AppResult<StoreAdminStats>;
    /// Get author email for a coach
    async fn get_author_email(&self, user_id: Uuid) -> AppResult<Option<String>>;
    /// Get published coaches for the Store (cross-tenant)
    async fn get_published_coaches(
        &self,
        category: Option<CoachCategory>,
        sort_by: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>>;
    /// Get published coaches with cursor-based pagination
    async fn get_published_coaches_cursor(
        &self,
        category: Option<CoachCategory>,
        sort_by: StoreSortOrder,
        limit: u32,
        cursor: Option<&str>,
    ) -> AppResult<CursorPage<CoachWithListing>>;
    /// Search published coaches by title/description/tags, in `locale`.
    ///
    /// The canonical English row and the `coach_translations` overlay for
    /// `locale` are both matched, so an athlete searching the words the Store
    /// showed her — a chip reading `methode-norvegienne`, say — reaches the
    /// coach whose canonical tag is `norwegian-method`. Matching only the
    /// canonical row made every localized label unsearchable; matching only
    /// the overlay would lose the coaches that have no translation.
    async fn search_published_coaches(
        &self,
        query: &str,
        limit: Option<u32>,
        locale: &str,
    ) -> AppResult<Vec<CoachWithListing>>;
    /// Get a single published coach by ID (cross-tenant)
    async fn get_published_coach(&self, coach_id: &str) -> AppResult<Option<CoachWithListing>>;
    /// Resolve a published catalogue coach by its `@handle` (cross-tenant).
    ///
    /// The origin coach only — the row that owns the handle, never an
    /// athlete's installed copy (which carries the handle as a reference) —
    /// and only while its listing is published, so a coach that left the
    /// Store is no longer installable by name.
    async fn find_published_by_handle(
        &self,
        handle: &CoachHandle,
    ) -> AppResult<Option<CoachWithListing>>;
    /// Get category counts for published coaches
    async fn get_category_counts(&self) -> AppResult<HashMap<CoachCategory, i64>>;
    /// Increment install count for a coach's store listing
    async fn increment_install_count(&self, coach_id: &str) -> AppResult<()>;
    /// Decrement install count for a coach's store listing
    async fn decrement_install_count(&self, coach_id: &str) -> AppResult<()>;
    /// Install a coach from the Store (creates user's copy)
    async fn install_from_store(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach>;
    /// Uninstall a coach (deletes user's copy, returns source coach ID)
    async fn uninstall_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<String>;
    /// Get user's installed coaches from the Store
    async fn get_installed_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>>;
    /// Create or ensure a store listing exists for a coach
    async fn ensure_listing(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<StoreListing>;
    /// Give a coach its catalogue `@handle` if it owns none yet, and return it.
    ///
    /// The same assignment Store approval performs, exposed for a coach
    /// created outside the Store (`/coach create`) so `@handle` and
    /// `/coach add @handle` reach it from the moment it exists. An origin
    /// coach already carrying a handle keeps it; otherwise the first free
    /// candidate derived from the title is taken at catalogue scope.
    async fn assign_catalogue_handle(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<String>;
}
