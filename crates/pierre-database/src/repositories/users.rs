// ABOUTME: Repository trait definitions for the user/profile/impersonation/physiology domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;

use pierre_core::models::TenantId;
use pierre_core::models::{CoachingPersona, User, UserStatus, UserTier};
use pierre_core::models::{Dossier, UserPhysiologicalProfile};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::impersonation::ImpersonationSession;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// User account management repository
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Create a new user account
    async fn create(&self, user: &User) -> AppResult<Uuid>;
    /// Get user by ID, scoped to a specific tenant for multi-tenant isolation
    async fn get(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<User>>;
    /// Get user by ID without tenant scoping (for system-level operations)
    async fn get_global(&self, user_id: Uuid) -> AppResult<Option<User>>;
    /// Batch-fetch users by ID without tenant scoping. Returns a map keyed by
    /// user id; ids with no matching row are omitted. Replaces per-id
    /// `get_global` loops with a single `WHERE id IN (...)` query.
    async fn get_global_many(&self, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, User>>;
    /// Get user by email address
    async fn get_by_email(&self, email: &str) -> AppResult<Option<User>>;
    /// Get user by email (required - fails if not found)
    async fn get_by_email_required(&self, email: &str) -> AppResult<User>;
    /// Get user by Firebase UID
    async fn get_by_firebase_uid(&self, firebase_uid: &str) -> AppResult<Option<User>>;
    /// Update user's last active timestamp
    async fn update_last_active(&self, user_id: Uuid) -> AppResult<()>;
    /// Get total number of users
    async fn count(&self) -> AppResult<i64>;
    /// Get users by status (pending, active, suspended), optionally scoped to a tenant
    async fn get_by_status(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<User>>;
    /// Get users by status with cursor-based pagination
    async fn get_by_status_cursor(
        &self,
        status: &str,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<User>>;
    /// Update user status and approval information
    async fn update_status(
        &self,
        user_id: Uuid,
        new_status: UserStatus,
        approved_by: Option<Uuid>,
    ) -> AppResult<User>;
    /// Set admin status on a user, updating both `is_admin` flag and role column.
    /// When granting admin, users keep `SuperAdmin` role if they already have it; otherwise set to `Admin`.
    /// When revoking admin, role is reset to `User`. Super-admins cannot be demoted via this method.
    async fn set_admin_status(&self, user_id: Uuid, is_admin: bool) -> AppResult<User>;
    /// List all users with `is_admin = true`, ordered by email
    async fn list_admins(&self) -> AppResult<Vec<User>>;
    /// Update user's `tenant_id` to link them to a tenant (`tenant_id` should be UUID string)
    async fn update_tenant_id(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()>;
    /// Update user's password hash
    async fn update_password(&self, user_id: Uuid, password_hash: &str) -> AppResult<()>;
    /// Update user's display name
    async fn update_display_name(&self, user_id: Uuid, display_name: &str) -> AppResult<User>;
    /// Delete a user and all associated data
    async fn delete(&self, user_id: Uuid) -> AppResult<()>;
    /// Get the first admin user by creation date
    async fn get_first_admin_user(&self) -> AppResult<Option<User>>;
    /// Update user's analytics consent preference
    async fn update_analytics_consent(&self, user_id: Uuid, enabled: bool) -> AppResult<()>;
    /// Update the user's preferred locale (BCP-47 short code, e.g. `"fr"`, `"en"`).
    ///
    /// Called by the user-profile PATCH endpoint. The column has `NOT NULL
    /// DEFAULT 'fr'` so an unset user always resolves to French; this method
    /// overrides that default with an explicit choice.
    async fn update_locale(&self, user_id: Uuid, locale: &str) -> AppResult<()>;
    /// Set the user's personal default coach (nullable — pass `None` to clear).
    ///
    /// Called by `/coach select` in DM conversations. The column has a FK on
    /// `coaches(id)` with `ON DELETE SET NULL`, so a deleted coach cleanly
    /// detaches instead of orphaning the user row.
    async fn set_default_coach(&self, user_id: Uuid, coach_id: Option<&str>) -> AppResult<()>;
    /// Set the user's coaching persona (output-format / cadence preference).
    ///
    /// Called by the post-auth onboarding screen and the Settings UI. The
    /// `coaching_persona` column has `NOT NULL DEFAULT 'casual'` so an
    /// unmigrated user always resolves to the least-restrictive style;
    /// this method overrides that default with an explicit choice.
    async fn set_coaching_persona(&self, user_id: Uuid, persona: CoachingPersona) -> AppResult<()>;
    /// Toggle the user's `manages_roster` permission flag.
    ///
    /// Called by admin tooling to grant or revoke the Coach-tier roster
    /// UI / API surface. Independent from `coaching_persona` — see
    /// `Coaching Persona Architecture.md` §8 for the rationale.
    async fn set_manages_roster(&self, user_id: Uuid, manages_roster: bool) -> AppResult<()>;
    /// Persist the user's IANA timezone (e.g. `"America/Toronto"`).
    ///
    /// Captured client-side via `Intl.DateTimeFormat().resolvedOptions().timeZone`
    /// on each authenticated request via the `X-User-Timezone` header. The
    /// auth middleware calls this only when the header differs from the
    /// stored value, so steady-state cost is one write per genuine
    /// timezone change (travel, DST tooling glitches). Reading code
    /// treats `None` as UTC at prompt-assembly time.
    async fn set_timezone(&self, user_id: Uuid, timezone: &str) -> AppResult<()>;
    /// Set the user's billing tier (Starter / Professional / Enterprise).
    ///
    /// Called by Stripe webhook handlers on `customer.subscription.updated`
    /// and by the admin `POST /api/admin/users/{id}/tier` route. The CHECK
    /// constraint on `users.tier` is enforced at write time.
    async fn set_tier(&self, user_id: Uuid, tier: UserTier) -> AppResult<User>;
}

/// User profiles, goals, and configuration repository
#[async_trait]
pub trait ProfileRepository: Send + Sync {
    /// Upsert user profile data
    async fn upsert_profile(&self, user_id: Uuid, profile_data: Value) -> AppResult<()>;
    /// Get user profile data
    async fn get_profile(&self, user_id: Uuid) -> AppResult<Option<Value>>;
    /// Create a new goal for a user
    async fn create_goal(&self, user_id: Uuid, goal_data: Value) -> AppResult<String>;
    /// Get all goals for a user
    async fn get_goals(&self, user_id: Uuid) -> AppResult<Vec<Value>>;
    /// Update progress on a goal, scoped to the owning user
    async fn update_goal_progress(
        &self,
        goal_id: &str,
        user_id: Uuid,
        current_value: f64,
    ) -> AppResult<()>;
    /// Get user configuration data
    async fn get_configuration(&self, user_id: &str) -> AppResult<Option<String>>;
    /// Save user configuration data
    async fn save_configuration(&self, user_id: &str, config_json: &str) -> AppResult<()>;
}

/// Password reset token management repository
#[async_trait]
pub trait PasswordResetRepository: Send + Sync {
    /// Store a password reset token (hashed) for a user
    async fn store_token(
        &self,
        user_id: Uuid,
        token_hash: &str,
        created_by: &str,
    ) -> AppResult<Uuid>;
    /// Store a password reset token with a custom TTL (in minutes)
    ///
    /// Used for self-service password reset codes that expire faster (15 min)
    /// than admin-issued tokens (1 hour).
    async fn store_token_with_ttl(
        &self,
        user_id: Uuid,
        token_hash: &str,
        created_by: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid>;
    /// Consume a password reset token by its hash
    async fn consume_token(&self, token_hash: &str) -> AppResult<Uuid>;
    /// Invalidate all unused reset tokens for a user
    async fn invalidate_tokens(&self, user_id: Uuid) -> AppResult<()>;
    /// Count recent reset tokens for a user (for rate limiting)
    ///
    /// Returns the number of tokens created for the user since the given timestamp,
    /// regardless of whether they have been used or expired.
    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64>;
}

/// Impersonation session management repository
#[async_trait]
pub trait ImpersonationRepository: Send + Sync {
    /// Create a new impersonation session for audit trail
    async fn create_session(&self, session: &ImpersonationSession) -> AppResult<()>;
    /// Get impersonation session by ID
    async fn get_session(&self, session_id: &str) -> AppResult<Option<ImpersonationSession>>;
    /// Get active impersonation session where user is impersonator or target
    async fn get_active_session(&self, user_id: Uuid) -> AppResult<Option<ImpersonationSession>>;
    /// End an impersonation session
    async fn end_session(&self, session_id: &str) -> AppResult<()>;
    /// End all active impersonation sessions for an impersonator
    async fn end_all_sessions(&self, impersonator_id: Uuid) -> AppResult<u64>;
    /// List impersonation sessions with optional filters
    async fn list_sessions(
        &self,
        impersonator_id: Option<Uuid>,
        target_user_id: Option<Uuid>,
        active_only: bool,
        limit: u32,
    ) -> AppResult<Vec<ImpersonationSession>>;
}

/// Typed CRUD for [`UserPhysiologicalProfile`] backed by the
/// `user_physiological_profiles` table.
///
/// Row layout: see `migrations/20260430000003_user_profile_endurance_fields.sql`
/// (`SQLite`) and `migrations_pg/20260430000003_user_profile_endurance_fields.sql`
/// (`PostgreSQL`).
///
/// Every method scopes by `tenant_id` to satisfy the multi-tenant isolation
/// invariant in CLAUDE.md.
#[async_trait]
pub trait UserPhysiologicalProfileRepository: Send + Sync {
    /// Insert or update the profile row for `(tenant_id, user_id)`.
    ///
    /// `profile.user_id` must match `user_id`; the implementation rejects
    /// mismatches with [`pierre_core::errors::AppError`] to prevent
    /// cross-user writes from a confused caller.
    async fn upsert_user_physiological_profile(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        profile: &UserPhysiologicalProfile,
    ) -> AppResult<()>;

    /// Fetch the profile for `(tenant_id, user_id)`. Returns `None` when
    /// the user has no row yet.
    async fn get_user_physiological_profile(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<UserPhysiologicalProfile>>;
}

/// Read-time composer for the Endurance [`Dossier`] aggregate.
///
/// Per the locked architectural decision the dossier is **not** persisted as
/// its own row — the implementation pulls from the existing tables
/// (`user_physiological_profiles` for physiology + zones, `user_profiles`
/// JSON column for goals / nutrition / equipment) and assembles the
/// aggregate per request. Cache invalidation is therefore unnecessary on
/// the dossier itself; only the underlying tables need cache hooks.
#[async_trait]
pub trait DossierRepository: Send + Sync {
    /// Compose the dossier for `(tenant_id, user_id)`.
    ///
    /// Returns an empty dossier shell (all slots `None` / empty) when the
    /// user has no underlying rows so the API endpoint can return a 200
    /// rather than a 404 for fresh accounts.
    async fn compose_dossier(&self, tenant_id: TenantId, user_id: Uuid) -> AppResult<Dossier>;
}
