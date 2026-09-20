// ABOUTME: Repository traits for the user domain, with the statements and shared body of UserRepository
// ABOUTME: One SQL text per operation; each backend shell supplies its uuid codec and its duplicate-key detector
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The user account store, written once.
//!
//! `users.id` and the id-typed columns beside it (`approved_by`,
//! `tenant_users.user_id`) are `uuid` on Postgres and `TEXT` on `SQLite`, so
//! the shell hands the body its [`uuid_columns`](super::uuid_columns) codec.
//! The one other thing the drivers spell differently is how a violated
//! unique index surfaces (`SQLite`'s result codes, Postgres's SQLSTATE and
//! constraint name), so the shell also hands the body the function that turns
//! that driver's error into the structured duplicate error.
//!
//! Every read lists the same columns through [`user_columns!`] and decodes
//! through [`user_from_row`], so no listing can silently return a user with
//! the persona, roster flag or timezone it never selected — two Postgres
//! listings and the `SQLite` cursor page did exactly that. `$n` placeholders
//! throughout; timestamps bind as [`DateTime<Utc>`]; booleans as `bool`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::permissions::UserRole;

use crate::backends::shared::enums::str_to_user_status;
use pierre_core::models::default_locale;
use pierre_core::models::TenantId;
use pierre_core::models::{
    CoachingPersona, PreApprovedEmail, SessionRefreshToken, User, UserStatus, UserTier,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// User account management repository
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Insert a new user account.
    ///
    /// Insert-only on both engines: a duplicate email is `invalid_input`, never a
    /// silent overwrite. Callers that mean "write this User over the existing row"
    /// call [`UserRepository::update`].
    async fn create(&self, user: &User) -> AppResult<Uuid>;
    /// Write a whole `User` onto its existing row, matched by id.
    ///
    /// Every mutable column is written, so the caller must hand a `User` it loaded
    /// and then modified. `email` and `created_at` are not writable. This is the
    /// escape hatch for callers changing several fields at once (Firebase account
    /// linking, `pierre-cli user create --force`); single-field changes keep their
    /// dedicated setters below.
    async fn update(&self, user: &User) -> AppResult<()>;
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
    /// Set the user's coaching persona (output-format / cadence preference).
    ///
    /// Called by the post-auth onboarding screen and the Settings UI. The
    /// `coaching_persona` column has `NOT NULL DEFAULT 'casual'` so an
    /// unmigrated user always resolves to the least-restrictive style;
    /// this method overrides that default with an explicit choice.
    async fn set_coaching_persona(&self, user_id: Uuid, persona: CoachingPersona) -> AppResult<()>;
    /// Toggle the user's `manages_roster` permission flag.
    ///
    /// Called by admin tooling to grant or revoke the Agent-tier roster
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
    /// Persist the user's pinned colour scheme (`"light"` / `"dark"`), or
    /// clear the pin with `None` so clients follow the operating system.
    ///
    /// Written by `PUT /api/user/theme`. Server-side chart renders
    /// (messaging PNG minting) read the stored value and treat `None` as
    /// dark — the scheme messaging clients draw media bubbles on.
    async fn set_theme(&self, user_id: Uuid, theme: Option<&str>) -> AppResult<()>;
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
    /// Create a new goal for a user, returning the generated goal id.
    ///
    /// The id is also embedded into the stored `goal_data` under `goal_id`,
    /// so every goal a [`Self::get_goals`] read returns identifies itself —
    /// progress tracking looks goals up by that key.
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
    /// Store a reset token's `selector` (plaintext lookup half) and `verifier_hash`
    /// (SHA-256 of the secret half). The delivered token is `<selector>.<verifier>`.
    async fn store_token(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        created_by: &str,
    ) -> AppResult<Uuid>;
    /// Store a password reset token with a custom TTL (in minutes)
    ///
    /// Used for self-service password reset codes that expire faster (15 min)
    /// than admin-issued tokens (1 hour).
    async fn store_token_with_ttl(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        created_by: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid>;
    /// Consume a reset token: look it up by `selector`, verify `verifier_hash`, and on
    /// success mark it used and return the user id. A wrong verifier increments a
    /// per-token attempt counter; past the attempt cap the token self-invalidates
    /// (brute-force lockout).
    async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid>;
    /// Invalidate all unused reset tokens for a user
    async fn invalidate_tokens(&self, user_id: Uuid) -> AppResult<()>;
    /// Count recent reset tokens for a user (for rate limiting)
    ///
    /// Returns the number of tokens created for the user since the given timestamp,
    /// regardless of whether they have been used or expired.
    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64>;
}

/// Email-verification token lifecycle — proving an address belongs to whoever typed it.
///
/// Shares the `<selector>.<verifier>` mechanism with [`PasswordResetRepository`]
/// but deliberately not its token space: a token that can reset a password and a
/// token that can verify an address are different capabilities, and invalidating
/// one set must never clear the other.
#[async_trait]
pub trait EmailVerificationRepository: Send + Sync {
    /// Store one half of a verification token, with a TTL in minutes.
    ///
    /// `selector` is the plaintext lookup half and `verifier_hash` the SHA-256
    /// of the secret half. The delivered token is `<selector>.<verifier>` and is
    /// never stored whole.
    async fn store_token(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid>;
    /// Claim a verification token single-use, returning the user it proves.
    ///
    /// Looks the token up by `selector` and checks `verifier_hash`. A wrong
    /// verifier costs one attempt without consuming the token; past the attempt
    /// cap the token self-invalidates (brute-force lockout).
    async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid>;
    /// Count tokens issued for a user since `since`, for rate limiting.
    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64>;
    /// Stamp `users.email_verified_at`. Idempotent — a second call leaves the
    /// original timestamp in place, so re-verifying never rewrites history.
    async fn mark_verified(&self, user_id: Uuid) -> AppResult<()>;
    /// Whether this user's address has been proven.
    async fn is_verified(&self, user_id: Uuid) -> AppResult<bool>;
}

/// First-party refresh tokens — the credential a device holds between JWTs.
///
/// A login opens a family; each exchange stores the successor in that family
/// and revokes the token it replaced, so at most one member is live. Tokens
/// are passed in plaintext and stored as their HMAC, the same blind index the
/// `OAuth2` server's refresh tokens use, so a database read cannot replay one.
///
/// Not [`OAuth2ServerRepository`](super::OAuth2ServerRepository)'s refresh
/// tokens: those are keyed to a registered OAuth client and cascade with it,
/// which a password login has no counterpart for.
#[async_trait]
pub trait SessionRefreshTokenRepository: Send + Sync {
    /// Store a freshly issued token under its family.
    async fn store_token(&self, token: &str, record: &SessionRefreshToken) -> AppResult<()>;
    /// Exchange a token: mark it revoked and return its record, in one
    /// statement so two concurrent exchanges cannot both succeed. `None` when
    /// the token is unknown, already revoked, or expired at `now`.
    async fn consume_token(
        &self,
        token: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<SessionRefreshToken>>;
    /// Revoke every live member of the family this token belongs to, and
    /// return how many were revoked. Zero means the token was unknown or its
    /// family was already dead; more than zero after a failed exchange means a
    /// rotated-out token was replayed and its successor is now dead too.
    async fn revoke_token_family(&self, token: &str, now: DateTime<Utc>) -> AppResult<u64>;
    /// Revoke every live token the user holds, on any device — the password
    /// changed, so every session minted under the old one ends.
    async fn revoke_user_tokens(&self, user_id: Uuid, now: DateTime<Utc>) -> AppResult<u64>;
}

/// Standing per-email pre-approvals — an operator "allow" recorded before the
/// person has an account.
///
/// The registration approval decision consults this list so an allowed address
/// lands `Active` without the pending queue; `pierre-cli user allow / disallow /
/// list-allowed` manages it. Implementations store and compare emails
/// lowercase, so lookups are case-insensitive.
#[async_trait]
pub trait PreApprovedEmailRepository: Send + Sync {
    /// Record an allow for `email`. Idempotent: returns `false` when the
    /// address was already on the list (the original row is kept).
    async fn allow(
        &self,
        email: &str,
        allowed_by: Option<Uuid>,
        note: Option<&str>,
    ) -> AppResult<bool>;
    /// Remove the allow for `email`. Returns `false` when none existed.
    async fn remove(&self, email: &str) -> AppResult<bool>;
    /// Fetch the allow for `email`, if present.
    async fn get(&self, email: &str) -> AppResult<Option<PreApprovedEmail>>;
    /// Every standing allow, oldest first.
    async fn list(&self) -> AppResult<Vec<PreApprovedEmail>>;
}

/// The columns every [`User`] read lists, in the order [`CREATE_USER_SQL`]
/// binds them; `$prefix` is the table alias where a join needs one.
macro_rules! user_columns {
    ($prefix:literal) => {
        concat!(
            $prefix,
            "id, ",
            $prefix,
            "email, ",
            $prefix,
            "display_name, ",
            $prefix,
            "password_hash, ",
            $prefix,
            "tier, ",
            $prefix,
            "is_active, ",
            $prefix,
            "user_status, ",
            $prefix,
            "is_admin, ",
            $prefix,
            "role, ",
            $prefix,
            "approved_by, ",
            $prefix,
            "approved_at, ",
            $prefix,
            "created_at, ",
            $prefix,
            "last_active, ",
            $prefix,
            "firebase_uid, ",
            $prefix,
            "auth_provider, ",
            $prefix,
            "analytics_consent, ",
            $prefix,
            "analytics_consent_at, ",
            $prefix,
            "locale, ",
            $prefix,
            "coaching_persona, ",
            $prefix,
            "manages_roster, ",
            $prefix,
            "timezone, ",
            $prefix,
            "theme"
        )
    };
}

/// Insert a new account. Tenant membership is not on this row; it lives in
/// `tenant_users`.
pub(crate) const CREATE_USER_SQL: &str = concat!(
    "INSERT INTO users (",
    user_columns!(""),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22)"
);

/// Write a whole account onto its row. `email` and `created_at` are not
/// writable here: an email change would race the unique index, and a
/// creation date is not a mutable fact.
pub(crate) const UPDATE_USER_SQL: &str = r"
            UPDATE users SET
                display_name = $2,
                password_hash = $3,
                tier = $4,
                is_active = $5,
                user_status = $6,
                is_admin = $7,
                role = $8,
                approved_by = $9,
                approved_at = $10,
                firebase_uid = $11,
                auth_provider = $12,
                analytics_consent = $13,
                analytics_consent_at = $14,
                locale = $15,
                coaching_persona = $16,
                manages_roster = $17,
                timezone = $18,
                theme = $19,
                last_active = CURRENT_TIMESTAMP
            WHERE id = $1
            ";

/// One user, only when they are a member of the tenant.
pub(crate) const GET_USER_IN_TENANT_SQL: &str = concat!(
    "SELECT ",
    user_columns!("u."),
    " FROM users u
            INNER JOIN tenant_users tu ON u.id = tu.user_id AND tu.tenant_id = $2
            WHERE u.id = $1"
);

/// One user by id, without tenant scoping: for system-level callers only.
pub(crate) const GET_USER_BY_ID_SQL: &str =
    concat!("SELECT ", user_columns!(""), " FROM users WHERE id = $1");

/// One user by email.
pub(crate) const GET_USER_BY_EMAIL_SQL: &str =
    concat!("SELECT ", user_columns!(""), " FROM users WHERE email = $1");

/// One user by the Firebase UID they signed in with.
pub(crate) const GET_USER_BY_FIREBASE_UID_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users WHERE firebase_uid = $1"
);

/// The oldest admin account, for system agent seeding.
pub(crate) const FIRST_ADMIN_USER_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users WHERE is_admin = true ORDER BY created_at ASC LIMIT 1"
);

/// Every admin account, by email.
pub(crate) const LIST_ADMIN_USERS_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users WHERE is_admin = true ORDER BY email ASC"
);

/// The batch read behind `get_global_many`: one placeholder per id, so the
/// ids bind through the codec on both drivers rather than as an array on one.
pub(crate) fn users_by_ids_sql(count: usize) -> String {
    let placeholders = (1..=count)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT {} FROM users WHERE id IN ({placeholders})",
        user_columns!("")
    )
}

/// Stamp the moment the user was last seen.
pub(crate) const UPDATE_LAST_ACTIVE_SQL: &str =
    "UPDATE users SET last_active = CURRENT_TIMESTAMP WHERE id = $1";

/// How many accounts exist.
pub(crate) const COUNT_USERS_SQL: &str = "SELECT COUNT(*) AS count FROM users";

/// Every account in a status, newest first. A row whose status was never set
/// counts as active.
pub(crate) const USERS_BY_STATUS_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users
            WHERE COALESCE(user_status, 'active') = $1
            ORDER BY created_at DESC"
);

/// Every account in a status that is a member of the tenant, newest first.
pub(crate) const USERS_BY_STATUS_IN_TENANT_SQL: &str = concat!(
    "SELECT ",
    user_columns!("u."),
    " FROM users u
            INNER JOIN tenant_users tu ON u.id = tu.user_id AND tu.tenant_id = $2
            WHERE COALESCE(u.user_status, 'active') = $1
            ORDER BY u.created_at DESC"
);

/// The first page of a status listing under keyset pagination.
pub(crate) const USERS_BY_STATUS_PAGE_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users
            WHERE COALESCE(user_status, 'active') = $1
            ORDER BY created_at DESC, id DESC
            LIMIT $2"
);

/// A later page: every row strictly after the `(created_at, id)` cursor in
/// the listing's order, so an account created meanwhile neither repeats nor
/// hides another. The id compares natively on both drivers: a uuid orders
/// bytewise on Postgres, which is the order its hyphenated text has on `SQLite`.
pub(crate) const USERS_BY_STATUS_AFTER_CURSOR_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users
            WHERE COALESCE(user_status, 'active') = $1
              AND (created_at < $2 OR (created_at = $2 AND id < $3))
            ORDER BY created_at DESC, id DESC
            LIMIT $4"
);

/// Approve, suspend or reset an account.
pub(crate) const UPDATE_USER_STATUS_SQL: &str = r"
            UPDATE users
            SET user_status = $1, approved_by = $2, approved_at = $3
            WHERE id = $4
            ";

/// Flip the admin flag and keep the role column in step with it.
pub(crate) const SET_USER_ADMIN_STATUS_SQL: &str =
    "UPDATE users SET is_admin = $1, role = $2 WHERE id = $3";

/// Link the account to a tenant on its own row; membership follows in
/// [`ADD_TENANT_MEMBER_SQL`].
pub(crate) const SET_USER_TENANT_SQL: &str = "UPDATE users SET tenant_id = $1 WHERE id = $2";

/// Add the account as a member of the tenant, once.
pub(crate) const ADD_TENANT_MEMBER_SQL: &str = r"
            INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at)
            VALUES ($1, $2, $3, 'member', $4, $4)
            ON CONFLICT (tenant_id, user_id) DO NOTHING
            ";

/// Replace the password hash.
pub(crate) const UPDATE_USER_PASSWORD_SQL: &str =
    "UPDATE users SET password_hash = $1, last_active = CURRENT_TIMESTAMP WHERE id = $2";

/// Replace the display name.
pub(crate) const UPDATE_USER_DISPLAY_NAME_SQL: &str =
    "UPDATE users SET display_name = $1, last_active = CURRENT_TIMESTAMP WHERE id = $2";

/// Remove the account; every dependent row cascades.
pub(crate) const DELETE_USER_SQL: &str = "DELETE FROM users WHERE id = $1";

/// Move the account to a billing tier.
pub(crate) const SET_USER_TIER_SQL: &str =
    "UPDATE users SET tier = $1, last_active = CURRENT_TIMESTAMP WHERE id = $2";

/// The status strings a listing may ask for.
///
/// # Errors
/// Returns an invalid-input error for any other string.
pub(crate) fn user_status_filter(status: &str) -> AppResult<&str> {
    match status {
        "active" | "pending" | "suspended" => Ok(status),
        _ => Err(AppError::invalid_input(format!(
            "Invalid user status: {status}"
        ))),
    }
}

/// Decode one `users` row. The two uuid columns are read by the caller
/// through its backend's codec; every other column decodes the same way on
/// both drivers. The columns a migration may not yet have added read with
/// their defaults, as both backends always did; `tier` is parsed strictly,
/// because both schemas constrain it to the three values the model knows.
///
/// # Errors
/// Returns a database error naming the first required column that cannot be
/// decoded, or an internal error when the stored tier is not one the model
/// knows.
pub(crate) fn user_from_row<R>(row: &R, id: Uuid, approved_by: Option<Uuid>) -> AppResult<User>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column =
        |col: &str, e: sqlx::Error| AppError::database(format!("Failed to get users.{col}: {e}"));
    let tier: String = row.try_get("tier").map_err(|e| column("tier", e))?;
    let user_status: String = row
        .try_get("user_status")
        .map_err(|e| column("user_status", e))?;
    let is_admin: bool = row.try_get("is_admin").map_err(|e| column("is_admin", e))?;
    // The role column may predate the account (a seeder that omitted it left
    // the DEFAULT 'user'); an admin flag on such a row means Admin.
    let mut role = row
        .try_get::<Option<String>, _>("role")
        .ok()
        .flatten()
        .map_or_else(
            || {
                if is_admin {
                    UserRole::Admin
                } else {
                    UserRole::User
                }
            },
            |r| UserRole::from_str_lossy(&r),
        );
    if is_admin && role == UserRole::User {
        role = UserRole::Admin;
    }

    Ok(User {
        id,
        email: row.try_get("email").map_err(|e| column("email", e))?,
        display_name: row
            .try_get("display_name")
            .map_err(|e| column("display_name", e))?,
        password_hash: row
            .try_get("password_hash")
            .map_err(|e| column("password_hash", e))?,
        tier: tier
            .parse()
            .map_err(|e| AppError::internal(format!("Failed to parse tier: {e}")))?,
        strava_token: None,
        fitbit_token: None,
        is_active: row
            .try_get("is_active")
            .map_err(|e| column("is_active", e))?,
        user_status: str_to_user_status(&user_status),
        is_admin,
        role,
        approved_by,
        approved_at: row
            .try_get("approved_at")
            .map_err(|e| column("approved_at", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
        last_active: row
            .try_get("last_active")
            .map_err(|e| column("last_active", e))?,
        firebase_uid: row.try_get("firebase_uid").ok().flatten(),
        auth_provider: row
            .try_get("auth_provider")
            .unwrap_or_else(|_| "email".to_owned()),
        analytics_consent: row.try_get("analytics_consent").unwrap_or(false),
        analytics_consent_at: row.try_get("analytics_consent_at").ok().flatten(),
        locale: row.try_get("locale").ok().unwrap_or_else(default_locale),
        coaching_persona: row
            .try_get::<String, _>("coaching_persona")
            .ok()
            .and_then(|s| s.parse::<CoachingPersona>().ok())
            .unwrap_or_default(),
        manages_roster: row.try_get("manages_roster").ok().unwrap_or(false),
        timezone: row.try_get("timezone").ok().flatten(),
        theme: row.try_get("theme").ok().flatten(),
    })
}

/// Emit the whole [`UserRepository`] implementation for one backend type.
///
/// `$row` is the driver's row type, `$ids` is that backend's
/// [`uuid_columns`](super::uuid_columns) codec, and
/// `$duplicate` is its `fn(&sqlx::Error) -> Option<AppError>`, which reads a
/// violated unique index off the driver's error and names the column it was
/// on. The body is written once here; each backend's shell invokes it with
/// its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion. The single-column preference writes come from
/// `preferences`, the backend's expansion of the shared
/// [`user_preferences`](super::user_preferences) body.
macro_rules! impl_user_repository {
    ($ty:ty, $row:ty, $ids:ident, $duplicate:path) => {
        impl $ty {
            /// Decode a row through the backend's codec for its two uuid columns.
            fn user_row(row: &$row) -> AppResult<User> {
                user_from_row(
                    row,
                    $ids::read(row, "id")?,
                    $ids::read_opt(row, "approved_by")?,
                )
            }
        }

        #[async_trait::async_trait]
        impl UserRepository for $ty {
            async fn create(&self, user: &User) -> AppResult<Uuid> {
                sqlx::query(CREATE_USER_SQL)
                    .bind($ids::bind(user.id))
                    .bind(&user.email)
                    .bind(&user.display_name)
                    .bind(&user.password_hash)
                    .bind(user.tier.as_str())
                    .bind(user.is_active)
                    .bind(user_status_to_str(&user.user_status))
                    .bind(user.is_admin)
                    .bind(user.role.as_str())
                    .bind($ids::bind_opt(user.approved_by))
                    .bind(user.approved_at)
                    .bind(user.created_at)
                    .bind(user.last_active)
                    .bind(&user.firebase_uid)
                    .bind(&user.auth_provider)
                    .bind(user.analytics_consent)
                    .bind(user.analytics_consent_at)
                    .bind(&user.locale)
                    .bind(user.coaching_persona.as_str())
                    .bind(user.manages_roster)
                    .bind(&user.timezone)
                    .bind(&user.theme)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        $duplicate(&e).unwrap_or_else(|| {
                            AppError::database(format!("Failed to create user: {e}"))
                        })
                    })?;

                Ok(user.id)
            }

            async fn update(&self, user: &User) -> AppResult<()> {
                let result = sqlx::query(UPDATE_USER_SQL)
                    .bind($ids::bind(user.id))
                    .bind(&user.display_name)
                    .bind(&user.password_hash)
                    .bind(user.tier.as_str())
                    .bind(user.is_active)
                    .bind(user_status_to_str(&user.user_status))
                    .bind(user.is_admin)
                    .bind(user.role.as_str())
                    .bind($ids::bind_opt(user.approved_by))
                    .bind(user.approved_at)
                    .bind(&user.firebase_uid)
                    .bind(&user.auth_provider)
                    .bind(user.analytics_consent)
                    .bind(user.analytics_consent_at)
                    .bind(&user.locale)
                    .bind(user.coaching_persona.as_str())
                    .bind(user.manages_roster)
                    .bind(&user.timezone)
                    .bind(&user.theme)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to update user: {e}")))?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("User {}", user.id)));
                }
                Ok(())
            }

            async fn get(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<User>> {
                let row = sqlx::query(GET_USER_IN_TENANT_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get user by id+tenant: {e}"))
                    })?;
                row.as_ref().map(Self::user_row).transpose()
            }

            async fn get_global(&self, user_id: Uuid) -> AppResult<Option<User>> {
                let row = sqlx::query(GET_USER_BY_ID_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get user by id: {e}")))?;
                row.as_ref().map(Self::user_row).transpose()
            }

            async fn get_global_many(&self, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, User>> {
                if user_ids.is_empty() {
                    return Ok(HashMap::new());
                }
                let sql = users_by_ids_sql(user_ids.len());
                let mut query = sqlx::query(&sql);
                for id in user_ids {
                    query = query.bind($ids::bind(*id));
                }
                let rows = query
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to batch-get users: {e}")))?;

                let mut users = HashMap::with_capacity(rows.len());
                for row in &rows {
                    let user = Self::user_row(row)?;
                    users.insert(user.id, user);
                }
                Ok(users)
            }

            async fn get_by_email(&self, email: &str) -> AppResult<Option<User>> {
                let row = sqlx::query(GET_USER_BY_EMAIL_SQL)
                    .bind(email)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get user by email: {e}")))?;
                row.as_ref().map(Self::user_row).transpose()
            }

            async fn get_by_email_required(&self, email: &str) -> AppResult<User> {
                self.get_by_email(email)
                    .await?
                    .ok_or_else(|| AppError::not_found(format!("User with email: {email}")))
            }

            async fn get_by_firebase_uid(&self, firebase_uid: &str) -> AppResult<Option<User>> {
                let row = sqlx::query(GET_USER_BY_FIREBASE_UID_SQL)
                    .bind(firebase_uid)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get user by firebase_uid: {e}"))
                    })?;
                row.as_ref().map(Self::user_row).transpose()
            }

            async fn update_last_active(&self, user_id: Uuid) -> AppResult<()> {
                sqlx::query(UPDATE_LAST_ACTIVE_SQL)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update last active: {e}"))
                    })?;
                Ok(())
            }

            async fn count(&self) -> AppResult<i64> {
                let row = sqlx::query(COUNT_USERS_SQL)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get user count: {e}")))?;
                row.try_get("count")
                    .map_err(|e| AppError::database(format!("Failed to get count: {e}")))
            }

            async fn get_by_status(
                &self,
                status: &str,
                tenant_id: Option<TenantId>,
            ) -> AppResult<Vec<User>> {
                let status = user_status_filter(status)?;
                // Pending users have no tenant_users entry (assigned on approval),
                // so the tenant join is skipped for that status.
                let rows = match tenant_id {
                    Some(tid) if status != "pending" => {
                        sqlx::query(USERS_BY_STATUS_IN_TENANT_SQL)
                            .bind(status)
                            .bind(tid)
                            .fetch_all(self.pool())
                            .await
                    }
                    _ => {
                        sqlx::query(USERS_BY_STATUS_SQL)
                            .bind(status)
                            .fetch_all(self.pool())
                            .await
                    }
                }
                .map_err(|e| AppError::database(format!("Failed to get users by status: {e}")))?;

                rows.iter().map(Self::user_row).collect()
            }

            async fn get_by_status_cursor(
                &self,
                status: &str,
                params: &PaginationParams,
            ) -> AppResult<CursorPage<User>> {
                let status = user_status_filter(status)?;
                // One more than asked for tells whether a next page exists.
                let fetch_limit = i64::try_from(params.limit + 1)
                    .map_err(|_| AppError::invalid_input("Pagination limit too large"))?;

                let rows = if let Some(ref cursor) = params.cursor {
                    let (timestamp, id) = cursor
                        .decode()
                        .ok_or_else(|| AppError::invalid_input("Invalid cursor format"))?;
                    sqlx::query(USERS_BY_STATUS_AFTER_CURSOR_SQL)
                        .bind(status)
                        .bind(timestamp)
                        .bind($ids::bind_text(&id)?)
                        .bind(fetch_limit)
                        .fetch_all(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to get users by status (cursor): {e}"
                            ))
                        })?
                } else {
                    sqlx::query(USERS_BY_STATUS_PAGE_SQL)
                        .bind(status)
                        .bind(fetch_limit)
                        .fetch_all(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to get users by status (first page): {e}"
                            ))
                        })?
                };

                let mut users: Vec<User> =
                    rows.iter().map(Self::user_row).collect::<AppResult<_>>()?;
                let has_more = users.len() > params.limit;
                users.truncate(params.limit);
                let next_cursor = if has_more {
                    users
                        .last()
                        .map(|user| Cursor::new(user.created_at, &user.id.to_string()))
                } else {
                    None
                };

                Ok(CursorPage::new(users, next_cursor, None, has_more))
            }

            async fn update_status(
                &self,
                user_id: Uuid,
                new_status: UserStatus,
                approved_by: Option<Uuid>,
            ) -> AppResult<User> {
                // The approver and the moment are recorded on activation only.
                let (approved_by, approved_at) = if new_status == UserStatus::Active {
                    (approved_by, Some(Utc::now()))
                } else {
                    (None, None)
                };

                let result = sqlx::query(UPDATE_USER_STATUS_SQL)
                    .bind(user_status_to_str(&new_status))
                    .bind($ids::bind_opt(approved_by))
                    .bind(approved_at)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update user status: {e}"))
                    })?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("User with ID: {user_id}")));
                }

                self.get_global(user_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("User after status update"))
            }

            async fn set_admin_status(&self, user_id: Uuid, is_admin: bool) -> AppResult<User> {
                let current = self
                    .get_global(user_id)
                    .await?
                    .ok_or_else(|| AppError::not_found(format!("User with ID: {user_id}")))?;

                // A super-admin is never demoted through this path.
                if !is_admin && matches!(current.role, UserRole::SuperAdmin) {
                    return Err(AppError::invalid_input(
                        "Cannot demote super-admin via set_admin_status; use direct DB update",
                    ));
                }

                let new_role = if is_admin {
                    match current.role {
                        UserRole::SuperAdmin => "super_admin",
                        _ => "admin",
                    }
                } else {
                    "user"
                };

                let result = sqlx::query(SET_USER_ADMIN_STATUS_SQL)
                    .bind(is_admin)
                    .bind(new_role)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to set admin status: {e}")))?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("User with ID: {user_id}")));
                }

                self.get_global(user_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("User after admin status update"))
            }

            async fn list_admins(&self) -> AppResult<Vec<User>> {
                let rows = sqlx::query(LIST_ADMIN_USERS_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list admin users: {e}")))?;
                rows.iter().map(Self::user_row).collect()
            }

            async fn update_tenant_id(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()> {
                let result = sqlx::query(SET_USER_TENANT_SQL)
                    .bind(tenant_id.to_string())
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update user tenant ID: {e}"))
                    })?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("User with ID: {user_id}")));
                }

                sqlx::query(ADD_TENANT_MEMBER_SQL)
                    .bind($ids::bind(Uuid::new_v4()))
                    .bind(tenant_id)
                    .bind($ids::bind(user_id))
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert tenant_users entry: {e}"))
                    })?;

                Ok(())
            }

            async fn update_password(&self, user_id: Uuid, password_hash: &str) -> AppResult<()> {
                let result = sqlx::query(UPDATE_USER_PASSWORD_SQL)
                    .bind(password_hash)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update user password: {e}"))
                    })?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("User with ID: {user_id}")));
                }
                Ok(())
            }

            async fn update_display_name(
                &self,
                user_id: Uuid,
                display_name: &str,
            ) -> AppResult<User> {
                let result = sqlx::query(UPDATE_USER_DISPLAY_NAME_SQL)
                    .bind(display_name)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update user display name: {e}"))
                    })?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("User with ID: {user_id}")));
                }

                self.get_global(user_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("User after display name update"))
            }

            async fn delete(&self, user_id: Uuid) -> AppResult<()> {
                let result = sqlx::query(DELETE_USER_SQL)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to delete user: {e}")))?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("User {user_id} not found")));
                }
                Ok(())
            }

            async fn get_first_admin_user(&self) -> AppResult<Option<User>> {
                let row = sqlx::query(FIRST_ADMIN_USER_SQL)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get first admin user: {e}"))
                    })?;
                row.as_ref().map(Self::user_row).transpose()
            }

            async fn update_analytics_consent(
                &self,
                user_id: Uuid,
                enabled: bool,
            ) -> AppResult<()> {
                preferences::update_analytics_consent(self.pool(), user_id, enabled).await
            }

            async fn update_locale(&self, user_id: Uuid, locale: &str) -> AppResult<()> {
                preferences::update_locale(self.pool(), user_id, locale).await
            }

            async fn set_coaching_persona(
                &self,
                user_id: Uuid,
                persona: CoachingPersona,
            ) -> AppResult<()> {
                preferences::set_coaching_persona(self.pool(), user_id, persona).await
            }

            async fn set_manages_roster(
                &self,
                user_id: Uuid,
                manages_roster: bool,
            ) -> AppResult<()> {
                preferences::set_manages_roster(self.pool(), user_id, manages_roster).await
            }

            async fn set_timezone(&self, user_id: Uuid, timezone: &str) -> AppResult<()> {
                preferences::set_timezone(self.pool(), user_id, timezone).await
            }

            async fn set_theme(&self, user_id: Uuid, theme: Option<&str>) -> AppResult<()> {
                preferences::set_theme(self.pool(), user_id, theme).await
            }

            async fn set_tier(&self, user_id: Uuid, tier: UserTier) -> AppResult<User> {
                let result = sqlx::query(SET_USER_TIER_SQL)
                    .bind(tier.as_str())
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to set user tier: {e}")))?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("User with ID: {user_id}")));
                }

                self.get_global(user_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("User after tier update"))
            }
        }
    };
}
pub(crate) use impl_user_repository;
