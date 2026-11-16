// ABOUTME: Repository traits for the user domain, with the shared body of UserRepository; its statements are in user_sql
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
use pierre_core::admin::models::DELETED_ACCOUNT_OPERATOR;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::permissions::UserRole;

use crate::backends::shared::enums::str_to_user_status;
use pierre_core::models::default_locale;
use pierre_core::models::TenantId;
use pierre_core::models::{
    CoachingPersona, OperatorRosterGrant, User, UserReference, UserStatus, UserTier,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use std::collections::HashMap;
use uuid::Uuid;

mod listing_status;
mod tokens;
pub use listing_status::ALL_USER_STATUSES;
pub(crate) use listing_status::{listing_status_filter, user_status_filter};
pub use tokens::{EmailVerificationRepository, PasswordResetRepository};

use super::admin_standing::{StandingWrite, WithdrawnAdminCredentials};

/// User account management repository
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Insert a new user account, its email stored normalized (trimmed, lowercase).
    ///
    /// Insert-only on both engines: a duplicate email in any casing is `invalid_input`,
    /// never a silent overwrite; to write a User over its row, call [`UserRepository::update`].
    ///
    /// The role decides who is an admin: `users.role` is written from
    /// `user.role`, and the database derives `users.is_admin` from it
    /// (20261114113701); no statement here names that column.
    ///
    /// The nil UUID is `invalid_input`: it stands for a deleted account
    /// (`DELETED_ACCOUNT_OPERATOR`) and is no account's id.
    async fn create(&self, user: &User) -> AppResult<Uuid>;
    /// Write a whole `User` onto its existing row, matched by id.
    ///
    /// Every mutable column is written, so the caller must hand a `User` it loaded
    /// and then modified. `email` and `created_at` are not writable. This is the
    /// escape hatch for a caller changing several fields at once (`pierre-cli
    /// user create --force`); a single-field change keeps its dedicated setter
    /// below, and a federated link its own write ([`Self::link_federated_identity`]).
    ///
    /// The role decides who is an admin, as [`UserRepository::create`] writes
    /// it. A write that would take the last active admin out of that standing
    /// (no longer an admin, deactivated or not active) is `invalid_input`; the
    /// check and the write are one transaction (see
    /// [`UserRepository::is_last_active_admin`]).
    ///
    /// The same transaction keeps the two credential rules of the role: a row
    /// left out of admin standing loses the device logins it approved and the
    /// impersonations it opened, which are returned, and a row left with the
    /// admin role is no longer impersonated by anyone.
    async fn update(&self, user: &User) -> AppResult<WithdrawnAdminCredentials>;
    /// Link a federated identity to the account `user_id`: one statement
    /// that writes what the link owns ([`FederatedLink`]) and stamps
    /// `last_active`, as every write on the row does.
    ///
    /// The account's role, status and `is_active` flag are not written, so
    /// the link cannot undo a suspension, a demotion or a promotion that
    /// landed since its caller read the account. The row comes back as the
    /// write left it, `None` for an id that is no account.
    async fn link_federated_identity(
        &self,
        user_id: Uuid,
        link: &FederatedLink<'_>,
    ) -> AppResult<Option<User>>;
    /// Get user by ID, scoped to a specific tenant for multi-tenant isolation
    async fn get(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<User>>;
    /// Get user by ID without tenant scoping (for system-level operations)
    async fn get_global(&self, user_id: Uuid) -> AppResult<Option<User>>;
    /// Batch-fetch users by ID without tenant scoping. Returns a map keyed by
    /// user id; ids with no matching row are omitted. Replaces per-id
    /// `get_global` loops with a single `WHERE id IN (...)` query.
    async fn get_global_many(&self, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, User>>;
    /// Get user by email address, compared case aside
    async fn get_by_email(&self, email: &str) -> AppResult<Option<User>>;
    /// Get user by email, case aside (required - fails if not found)
    async fn get_by_email_required(&self, email: &str) -> AppResult<User>;
    /// Get user by Firebase UID
    async fn get_by_firebase_uid(&self, firebase_uid: &str) -> AppResult<Option<User>>;
    /// Update user's last active timestamp
    async fn update_last_active(&self, user_id: Uuid) -> AppResult<()>;
    /// Get total number of users
    async fn count(&self) -> AppResult<i64>;
    /// How many accounts were created at or after `since`, whatever created
    /// them: the count the sign-up ceiling is measured against
    async fn count_created_since(&self, since: DateTime<Utc>) -> AppResult<i64>;
    /// Get users by status (pending, active, suspended), optionally scoped to a tenant
    async fn get_by_status(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<User>>;
    /// Get users by status with cursor-based pagination; the status
    /// [`ALL_USER_STATUSES`] pages every account. `tenant_id` keeps the
    /// members of one tenant, whatever the status; `None` pages every tenant
    async fn get_by_status_cursor(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<User>>;
    /// Update user status and approval information.
    ///
    /// Moving the last active admin out of `Active` is `invalid_input`: the
    /// platform always keeps one admin who can sign in and decide. The check
    /// and the write are one transaction.
    ///
    /// A move out of `Active` deactivates, in that transaction, the device
    /// logins the account approved and the impersonations it opened: whoever
    /// calls this, an account out of standing holds neither, and a later
    /// approval does not bring them back.
    async fn update_status(
        &self,
        user_id: Uuid,
        new_status: UserStatus,
        approved_by: Option<Uuid>,
    ) -> AppResult<StandingWrite>;
    /// [`UserRepository::update_status`] for an account whose status is still
    /// `from`. The check and the write are one statement, so of two
    /// decisions on one account only the first is written. `None`, with
    /// nothing written, when no account of that id holds `from`.
    async fn update_status_from(
        &self,
        user_id: Uuid,
        from: UserStatus,
        new_status: UserStatus,
        approved_by: Option<Uuid>,
    ) -> AppResult<Option<StandingWrite>>;
    /// Grant or revoke the admin role: granting writes the `Admin` role,
    /// revoking resets it to `User`, and the database keeps `users.is_admin`
    /// equal to it.
    /// Revoking it from the last active admin is `invalid_input`; the check
    /// and the write are one transaction.
    ///
    /// Revoking deactivates, in that transaction, the device logins the
    /// account approved and the impersonations it opened. Granting ends
    /// every impersonation open on the account: an admin is never
    /// impersonated, including one who became an admin under an open session.
    async fn set_admin_status(&self, user_id: Uuid, is_admin: bool) -> AppResult<StandingWrite>;
    /// Whether `user` is the only admin in good standing
    /// ([`User::is_active_admin`]): the one account that must not be demoted,
    /// suspended or deleted, since nobody would be left to manage the platform.
    /// `false` for anyone who is not an active admin.
    ///
    /// A read, so its answer can be stale by the time a caller acts on it:
    /// it is for refusing early, before anything irreversible. The writes
    /// that take an admin out of good standing ([`UserRepository::update`],
    /// [`UserRepository::update_status`], [`UserRepository::set_admin_status`]
    /// and the account deletion) decide again inside their own transaction,
    /// under a lock that makes two of them wait for each other.
    async fn is_last_active_admin(&self, user: &User) -> AppResult<bool> {
        let admins = self.list_admins().await?;
        Ok(is_last_active_admin_among(&admins, user.id))
    }
    /// Every user holding the admin role, ordered by email
    async fn list_admins(&self) -> AppResult<Vec<User>>;
    /// Update user's `tenant_id` to link them to a tenant (`tenant_id` should be UUID string)
    async fn update_tenant_id(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()>;
    /// Update user's password hash
    async fn update_password(&self, user_id: Uuid, password_hash: &str) -> AppResult<()>;
    /// Update user's display name
    async fn update_display_name(&self, user_id: Uuid, display_name: &str) -> AppResult<User>;
    /// The rows that reference this user without cascading, each of which would
    /// fail the account deletion (`AccountDeletionRepository::delete_account`):
    /// groups they own or coach, invites they created,
    /// and operator or audit attribution. Empty when the account can go.
    async fn deletion_blockers(&self, user_id: Uuid) -> AppResult<Vec<UserReference>>;
    /// Get the first admin user by creation date
    async fn get_first_admin_user(&self) -> AppResult<Option<User>>;
    /// Update user's analytics consent preference
    async fn update_analytics_consent(&self, user_id: Uuid, enabled: bool) -> AppResult<()>;
    /// The notice version this user accepted for `provider` (the backend the
    /// notice guards, e.g. `sciotte_coros`) and has not withdrawn, or `None`
    /// when no acceptance stands. A login to that backend is refused until it
    /// matches the current notice.
    async fn provider_terms_version(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> AppResult<Option<String>>;
    /// Record that this user accepted `provider`'s notice `version`, stamping
    /// the time and clearing any earlier withdrawal. Kept across disconnects:
    /// it is the account's answer to the notice, not part of any session.
    async fn record_provider_terms(
        &self,
        user_id: Uuid,
        provider: &str,
        version: &str,
    ) -> AppResult<()>;
    /// Withdraw this user's standing acceptance of `provider`'s notice,
    /// stamping the time; it reads as none until accepted again. Returns
    /// whether an acceptance was standing.
    async fn withdraw_provider_terms(&self, user_id: Uuid, provider: &str) -> AppResult<bool>;
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
    /// Record whether the user coaches others — the onboarding role answer.
    /// A role, not a style: it leaves `coaching_persona` alone (carnet#827).
    async fn set_coaches_others(&self, user_id: Uuid, coaches_others: bool) -> AppResult<()>;
    /// Whether the user coaches others; `false` for an unknown user.
    async fn coaches_others(&self, user_id: Uuid) -> AppResult<bool>;
    /// Set the user's `manages_roster` permission flag, which gates redeeming
    /// a coach invite (joining a coaching group as its human coach). Set to
    /// `true` when a TrainingPeaks connection reports a coach account.
    /// Independent from `coaching_persona` — see
    /// `Coaching Persona Architecture.md` §8 for the rationale.
    async fn set_manages_roster(&self, user_id: Uuid, manages_roster: bool) -> AppResult<()>;
    /// Grant or revoke `manages_roster` as an operator (the admin roster
    /// route, `pierre-cli user set --manages-roster`). A grant records
    /// `operator` and the time as an [`OperatorRosterGrant`], which
    /// [`Self::revoke_earned_manages_roster`] never takes back; a revoke
    /// clears the grant and its record together.
    async fn set_manages_roster_by_operator(
        &self,
        user_id: Uuid,
        manages_roster: bool,
        operator: Option<Uuid>,
    ) -> AppResult<()>;
    /// Take back a `manages_roster` grant no operator made, as the
    /// `TrainingPeaks` reconciler does when the coach connection that earned it
    /// stops earning it. Returns whether a grant was taken back.
    async fn revoke_earned_manages_roster(&self, user_id: Uuid) -> AppResult<bool>;
    /// The operator grant behind the user's `manages_roster`, if one holds.
    async fn manages_roster_operator_grant(
        &self,
        user_id: Uuid,
    ) -> AppResult<Option<OperatorRosterGrant>>;
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

/// What a first federated sign-in writes on the account it links to: the
/// columns the link owns ([`UserRepository::link_federated_identity`]).
#[derive(Debug, Clone, Copy)]
pub struct FederatedLink<'identity> {
    /// The Firebase UID the sign-in carries, when it carries one.
    pub firebase_uid: Option<&'identity str>,
    /// The sign-in provider, recorded as the account's `auth_provider`.
    pub auth_provider: &'identity str,
    /// The marker that replaces the password of an account whose earlier
    /// credentials are retired, `None` when they are kept. Retired, the
    /// Firebase UID becomes exactly `firebase_uid`; kept, a sign-in that
    /// carries none leaves an earlier one.
    pub retired_password_hash: Option<&'identity str>,
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
pub(crate) use user_columns;

/// Refuse to give an account the id that stands for none.
///
/// A deleted account's device logins are kept acting as
/// [`DELETED_ACCOUNT_OPERATOR`], the nil UUID, and the admin gate refuses
/// them because no account answers to that id. Every id assigned here is a
/// random version-4 UUID, so none does; this makes it a rule of the two
/// statements that insert a `users` row (the repository's create and the
/// seeder's upsert) instead of a habit of their callers.
pub(crate) fn refuse_the_id_of_no_account(id: Uuid) -> AppResult<()> {
    if id == DELETED_ACCOUNT_OPERATOR {
        return Err(AppError::invalid_input(
            "The nil UUID stands for a deleted account and is no account's id",
        ));
    }
    Ok(())
}

/// The refusal of a write that would leave the platform without an active
/// admin; `change` is what was asked of the last one.
pub(crate) fn last_admin_refusal(change: &str) -> AppError {
    AppError::invalid_input(format!("Cannot {change} the last active admin"))
}

/// Whether taking `user_id` out of good standing would leave `accounts`
/// without an active admin ([`User::is_active_admin`]): `user_id` is one, and
/// no other account is. `accounts` holds every admin; whoever else it holds
/// is ignored.
pub(crate) fn is_last_active_admin_among(accounts: &[User], user_id: Uuid) -> bool {
    let (target, others): (Vec<&User>, Vec<&User>) = accounts
        .iter()
        .filter(|account| account.is_active_admin())
        .partition(|account| account.id == user_id);
    !target.is_empty() && others.is_empty()
}

/// Every admin row and `$1`'s own, in id order, under `$lock`: the lock a
/// write takes before it removes `$1` from the admins in good standing.
///
/// `$lock` is `FOR NO KEY UPDATE` on `PostgreSQL`, so two such writes wait
/// for each other instead of each counting the other's admin as still
/// standing; the second one reads the rows as the first left them. It is the
/// lock a plain `UPDATE` of a non-key column takes, so it does not hold up an
/// insert that merely references one of these users, which `FOR UPDATE`
/// would. The id order is what keeps two of them from deadlocking, and `$1`
/// is locked even when it is not an admin so that it cannot become the last
/// one between this read and the write. `""` on `SQLite`, whose transaction
/// opens with `BEGIN IMMEDIATE` and so already holds the one write lock.
macro_rules! admin_removal_lock_sql {
    ($lock:literal) => {
        concat!(
            "SELECT ",
            $crate::repositories::users::user_columns!(""),
            " FROM users WHERE role = 'admin' OR id = $1 ORDER BY id ",
            $lock
        )
    };
}
pub(crate) use admin_removal_lock_sql;

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
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column =
        |col: &str, e: sqlx::Error| AppError::database(format!("Failed to get users.{col}: {e}"));
    let tier: String = row.try_get("tier").map_err(|e| column("tier", e))?;
    let user_status: String = row
        .try_get("user_status")
        .map_err(|e| column("user_status", e))?;
    // The role decides who is an admin. `users.is_admin` is neither read nor
    // written here: the database derives that column from the role on every
    // write (20261114113701), for the build a rollback would bring back. A
    // string this build does not know reads as a plain user.
    let role: String = row.try_get("role").map_err(|e| column("role", e))?;
    let role = UserRole::from_str_lossy(&role);

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
        is_active: row
            .try_get("is_active")
            .map_err(|e| column("is_active", e))?,
        user_status: str_to_user_status(&user_status),
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
/// on. `$begin` is the statement a write that can remove an admin opens its
/// transaction with (`BEGIN IMMEDIATE` on `SQLite`, which takes the write
/// lock before the admins are read; a plain `BEGIN` on `PostgreSQL`, where
/// the row lock does that job), and `$lock` is that row lock (see
/// `admin_removal_lock_sql!`). The body is written once here; each backend's
/// shell invokes it with its own type, and sqlx resolves the driver from
/// `self.pool()` per expansion. The single-column preference writes come
/// from `preferences`, the backend's expansion of the shared
/// [`user_preferences`](super::user_preferences) body.
macro_rules! impl_user_repository {
    ($ty:ty, $row:ty, $ids:ident, $duplicate:path, $begin:literal, $lock:literal) => {
        impl $ty {
            /// Decode a row through the backend's codec for its two uuid columns.
            fn user_row(row: &$row) -> AppResult<User> {
                user_from_row(
                    row,
                    $ids::read(row, "id")?,
                    $ids::read_opt(row, "approved_by")?,
                )
            }

            /// Refuse `change` when it would take `user_id`, the last admin
            /// in good standing, out of that standing.
            ///
            /// `conn` is the transaction the write then runs in. The admin
            /// rows and `user_id`'s own are locked here until it ends, so a
            /// second such write waits and decides on what this one left
            /// (see `admin_removal_lock_sql!`).
            ///
            /// # Errors
            /// Returns `invalid_input` for the last active admin, and a
            /// database error when the rows cannot be read.
            pub(crate) async fn refuse_removing_the_last_active_admin(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                user_id: Uuid,
                change: &str,
            ) -> AppResult<()> {
                let rows = sqlx::query(admin_removal_lock_sql!($lock))
                    .bind($ids::bind(user_id))
                    .fetch_all(conn)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to lock the admin accounts: {e}"))
                    })?;
                let accounts = rows
                    .iter()
                    .map(Self::user_row)
                    .collect::<AppResult<Vec<User>>>()?;
                if is_last_active_admin_among(&accounts, user_id) {
                    return Err(last_admin_refusal(change));
                }
                Ok(())
            }

            /// The one status write: `from` is the status the row must
            /// still hold, `None` for any. `Ok(None)` when no row matched,
            /// and then nothing was written.
            async fn write_status(
                &self,
                user_id: Uuid,
                from: Option<UserStatus>,
                new_status: UserStatus,
                approved_by: Option<Uuid>,
            ) -> AppResult<Option<StandingWrite>> {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to update user status: {e}"))
                };
                let mut tx = self.begin_admin_removal().await?;
                let activating = new_status == UserStatus::Active;
                // The approver and the moment are recorded on activation only.
                let (approved_by, approved_at) = if activating {
                    (approved_by, Some(Utc::now()))
                } else {
                    Self::refuse_removing_the_last_active_admin(&mut *tx, user_id, "suspend")
                        .await?;
                    (None, None)
                };

                let statement = if from.is_some() {
                    UPDATE_USER_STATUS_FROM_SQL
                } else {
                    UPDATE_USER_STATUS_SQL
                };
                let mut write = sqlx::query(statement)
                    .bind(user_status_to_str(&new_status))
                    .bind($ids::bind_opt(approved_by))
                    .bind(approved_at)
                    .bind($ids::bind(user_id));
                if let Some(from) = &from {
                    write = write.bind(user_status_to_str(from));
                }
                if write
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?
                    .rows_affected()
                    == 0
                {
                    return Ok(None);
                }
                let withdrawn = if activating {
                    WithdrawnAdminCredentials::default()
                } else {
                    Self::withdraw_admin_credentials(&mut *tx, user_id).await?
                };
                tx.commit().await.map_err(failed)?;

                let user = self
                    .get_global(user_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("User after status update"))?;
                Ok(Some(StandingWrite { user, withdrawn }))
            }

            /// Open the transaction a write that can remove an admin runs in.
            async fn begin_admin_removal(
                &self,
            ) -> AppResult<sqlx::Transaction<'static, <$row as sqlx::Row>::Database>> {
                self.pool()
                    .begin_with($begin)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to begin a user write: {e}")))
            }
        }

        #[async_trait::async_trait]
        impl UserRepository for $ty {
            async fn create(&self, user: &User) -> AppResult<Uuid> {
                $crate::repositories::users::refuse_the_id_of_no_account(user.id)?;
                let role = user.role;
                sqlx::query(CREATE_USER_SQL)
                    .bind($ids::bind(user.id))
                    .bind(normalize_email(&user.email))
                    .bind(&user.display_name)
                    .bind(&user.password_hash)
                    .bind(user.tier.as_str())
                    .bind(user.is_active)
                    .bind(user_status_to_str(&user.user_status))
                    .bind(role.as_str())
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

            async fn update(&self, user: &User) -> AppResult<WithdrawnAdminCredentials> {
                let role = user.role;
                let mut tx = self.begin_admin_removal().await?;
                if !user.is_active_admin() {
                    Self::refuse_removing_the_last_active_admin(
                        &mut *tx,
                        user.id,
                        "demote or deactivate",
                    )
                    .await?;
                }
                let result = sqlx::query(UPDATE_USER_SQL)
                    .bind($ids::bind(user.id))
                    .bind(&user.display_name)
                    .bind(&user.password_hash)
                    .bind(user.tier.as_str())
                    .bind(user.is_active)
                    .bind(user_status_to_str(&user.user_status))
                    .bind(role.as_str())
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
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to update user: {e}")))?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("User {}", user.id)));
                }
                let withdrawn = if user.is_active_admin() {
                    WithdrawnAdminCredentials::default()
                } else {
                    Self::withdraw_admin_credentials(&mut *tx, user.id).await?
                };
                if role.is_admin() {
                    Self::end_impersonations_of(&mut *tx, user.id).await?;
                }
                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("Failed to update user: {e}")))?;
                Ok(withdrawn)
            }

            async fn link_federated_identity(
                &self,
                user_id: Uuid,
                link: &FederatedLink<'_>,
            ) -> AppResult<Option<User>> {
                let row = sqlx::query(LINK_FEDERATED_IDENTITY_SQL)
                    .bind($ids::bind(user_id))
                    .bind(link.firebase_uid)
                    .bind(link.auth_provider)
                    .bind(link.retired_password_hash)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to link a federated identity: {e}"))
                    })?;
                row.as_ref().map(Self::user_row).transpose()
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
                    .bind(normalize_email(email))
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

            async fn count_created_since(&self, since: DateTime<Utc>) -> AppResult<i64> {
                let row = sqlx::query(COUNT_USERS_CREATED_SINCE_SQL)
                    .bind(since)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count recent accounts: {e}"))
                    })?;
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
                tenant_id: Option<TenantId>,
                params: &PaginationParams,
            ) -> AppResult<CursorPage<User>> {
                let status = listing_status_filter(status)?;
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
                        .bind(tenant_id)
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
                        .bind(tenant_id)
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
            ) -> AppResult<StandingWrite> {
                self.write_status(user_id, None, new_status, approved_by)
                    .await?
                    .ok_or_else(|| AppError::not_found(format!("User with ID: {user_id}")))
            }

            async fn update_status_from(
                &self,
                user_id: Uuid,
                from: UserStatus,
                new_status: UserStatus,
                approved_by: Option<Uuid>,
            ) -> AppResult<Option<StandingWrite>> {
                self.write_status(user_id, Some(from), new_status, approved_by)
                    .await
            }

            async fn set_admin_status(
                &self,
                user_id: Uuid,
                is_admin: bool,
            ) -> AppResult<StandingWrite> {
                let failed =
                    |e: sqlx::Error| AppError::database(format!("Failed to set admin status: {e}"));
                let mut tx = self.begin_admin_removal().await?;
                let new_role = if is_admin {
                    UserRole::Admin
                } else {
                    Self::refuse_removing_the_last_active_admin(&mut *tx, user_id, "demote")
                        .await?;
                    UserRole::User
                };

                let result = sqlx::query(SET_USER_ADMIN_STATUS_SQL)
                    .bind(new_role.as_str())
                    .bind($ids::bind(user_id))
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("User with ID: {user_id}")));
                }
                let withdrawn = if is_admin {
                    Self::end_impersonations_of(&mut *tx, user_id).await?;
                    WithdrawnAdminCredentials::default()
                } else {
                    Self::withdraw_admin_credentials(&mut *tx, user_id).await?
                };
                tx.commit().await.map_err(failed)?;

                let user = self
                    .get_global(user_id)
                    .await?
                    .ok_or_else(|| AppError::not_found("User after admin status update"))?;
                Ok(StandingWrite { user, withdrawn })
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

                (result.rows_affected() > 0)
                    .ok_or_else(|| AppError::not_found(format!("User with ID: {user_id}")))
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

            async fn deletion_blockers(&self, user_id: Uuid) -> AppResult<Vec<UserReference>> {
                let rows = sqlx::query(DELETION_BLOCKERS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read user deletion blockers: {e}"))
                    })?;

                rows.iter().map(user_reference_from_row).collect()
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

            async fn provider_terms_version(
                &self,
                user_id: Uuid,
                provider: &str,
            ) -> AppResult<Option<String>> {
                preferences::provider_terms_version(self.pool(), user_id, provider).await
            }

            async fn record_provider_terms(
                &self,
                user_id: Uuid,
                provider: &str,
                version: &str,
            ) -> AppResult<()> {
                preferences::record_provider_terms(self.pool(), user_id, provider, version).await
            }

            async fn withdraw_provider_terms(
                &self,
                user_id: Uuid,
                provider: &str,
            ) -> AppResult<bool> {
                preferences::withdraw_provider_terms(self.pool(), user_id, provider).await
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

            async fn set_coaches_others(
                &self,
                user_id: Uuid,
                coaches_others: bool,
            ) -> AppResult<()> {
                preferences::set_coaches_others(self.pool(), user_id, coaches_others).await
            }

            async fn coaches_others(&self, user_id: Uuid) -> AppResult<bool> {
                preferences::coaches_others(self.pool(), user_id).await
            }

            async fn set_manages_roster(
                &self,
                user_id: Uuid,
                manages_roster: bool,
            ) -> AppResult<()> {
                preferences::set_manages_roster(self.pool(), user_id, manages_roster).await
            }

            async fn set_manages_roster_by_operator(
                &self,
                user_id: Uuid,
                manages_roster: bool,
                operator: Option<Uuid>,
            ) -> AppResult<()> {
                preferences::set_manages_roster_by_operator(
                    self.pool(),
                    user_id,
                    manages_roster,
                    operator,
                )
                .await
            }

            async fn revoke_earned_manages_roster(&self, user_id: Uuid) -> AppResult<bool> {
                preferences::revoke_earned_manages_roster(self.pool(), user_id).await
            }

            async fn manages_roster_operator_grant(
                &self,
                user_id: Uuid,
            ) -> AppResult<Option<OperatorRosterGrant>> {
                preferences::manages_roster_operator_grant(self.pool(), user_id).await
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
