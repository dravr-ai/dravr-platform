// ABOUTME: Repository trait, statements and shared body for per-user admin tier overrides (anti-clobber marker)
// ABOUTME: Row presence makes the billing webhook skip set_tier/set_plan; written once, emitted per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserTier;
use uuid::Uuid;

/// Per-user admin tier override marker.
///
/// When a row exists for a user, an operator has manually set the user's
/// billing tier outside the Stripe loop (QA, comp accounts, manual
/// overrides). While the row is present the Stripe webhook MUST NOT change
/// `users.tier` or the tenant plan — it still upserts the subscription row
/// and logs that it skipped the tier flip. No row means the webhook drives
/// the tier as usual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTierOverride {
    /// User the override applies to.
    pub user_id: Uuid,
    /// Tier the operator pinned the user to.
    pub tier: UserTier,
    /// Operator-facing note explaining why the override exists.
    pub note: Option<String>,
    /// Admin user who set the override (audit trail). `None` for service
    /// tokens that do not map to a user UUID.
    pub set_by: Option<Uuid>,
    /// First-set timestamp.
    pub set_at: DateTime<Utc>,
    /// Most-recent update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// CRUD for `user_tier_overrides` — the marker consulted by the billing
/// webhook before applying a Stripe-driven tier change.
///
/// A permission surface. The table carries no `tenant_id`; every statement
/// is scoped by `user_id`, the strictly narrower key, and the admin handler
/// checks the caller's standing before reaching here.
#[async_trait]
pub trait UserTierOverrideRepository: Send + Sync {
    /// Fetch the override row for a user, or `None` if no override is set
    /// (the webhook drives the tier).
    async fn get(&self, user_id: Uuid) -> AppResult<Option<UserTierOverride>>;

    /// Insert or update the override row for a user. `set_at` is preserved
    /// on update; `updated_at` is always bumped to the call time.
    async fn upsert(&self, row: &UserTierOverride) -> AppResult<()>;

    /// Remove the override row so the webhook drives the tier again.
    /// Returns `true` when a row was removed, `false` when no override
    /// existed.
    async fn delete(&self, user_id: Uuid) -> AppResult<bool>;
}

/// The override row for one user.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends and cannot drift between
/// them. The id binds through the backend's uuid codec (see
/// [`super::uuid_columns`]); every other bind is a plain `&str`,
/// `Option<&str>` or `DateTime<Utc>` both drivers encode alike.
pub(crate) const GET_TIER_OVERRIDE_SQL: &str = r"
            SELECT user_id, tier, note, set_by, set_at, updated_at
            FROM user_tier_overrides
            WHERE user_id = $1
            ";

/// Insert, or on conflict update everything but `set_at`.
///
/// `set_at` and `updated_at` both bind the call time (`$5`); the `DO UPDATE`
/// never names `set_at`, so an existing row keeps its first-set timestamp
/// while `updated_at` advances.
pub(crate) const UPSERT_TIER_OVERRIDE_SQL: &str = r"
            INSERT INTO user_tier_overrides
                (user_id, tier, note, set_by, set_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $5)
            ON CONFLICT (user_id) DO UPDATE SET
                tier = EXCLUDED.tier,
                note = EXCLUDED.note,
                set_by = EXCLUDED.set_by,
                updated_at = EXCLUDED.updated_at
            ";

/// Drop the marker so the webhook drives the tier again.
pub(crate) const DELETE_TIER_OVERRIDE_SQL: &str =
    "DELETE FROM user_tier_overrides WHERE user_id = $1";

/// The error for a column of this table that would not decode.
pub(crate) fn tier_override_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to read tier override {name}: {e}"))
}

/// Parse the stored tier, rejecting a value the application no longer
/// knows rather than substituting a default.
///
/// # Errors
/// Returns a database error naming the stored text when it is not a tier.
pub(crate) fn tier_from_column(raw: &str) -> AppResult<UserTier> {
    raw.parse()
        .map_err(|e| AppError::database(format!("Invalid stored tier '{raw}': {e}")))
}

/// Emit the whole [`UserTierOverrideRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type and its uuid codec, and sqlx resolves the driver from
/// `self.pool()` per expansion.
///
/// `$ids` is the codec in [`super::uuid_columns`] for how that backend's
/// `user_id` and `set_by` columns bind and read.
macro_rules! impl_user_tier_override_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl UserTierOverrideRepository for $ty {
            async fn get(&self, user_id: Uuid) -> AppResult<Option<UserTierOverride>> {
                let row = sqlx::query(GET_TIER_OVERRIDE_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch tier override: {e}"))
                    })?;

                let Some(row) = row else { return Ok(None) };

                let tier_raw: String = row
                    .try_get("tier")
                    .map_err(|e| tier_override_column_error("tier", e))?;
                Ok(Some(UserTierOverride {
                    user_id: $ids::read(&row, "user_id")?,
                    tier: tier_from_column(&tier_raw)?,
                    note: row
                        .try_get("note")
                        .map_err(|e| tier_override_column_error("note", e))?,
                    set_by: $ids::read_opt(&row, "set_by")?,
                    set_at: row
                        .try_get("set_at")
                        .map_err(|e| tier_override_column_error("set_at", e))?,
                    updated_at: row
                        .try_get("updated_at")
                        .map_err(|e| tier_override_column_error("updated_at", e))?,
                }))
            }

            async fn upsert(&self, row: &UserTierOverride) -> AppResult<()> {
                sqlx::query(UPSERT_TIER_OVERRIDE_SQL)
                    .bind($ids::bind(row.user_id))
                    .bind(row.tier.as_str())
                    .bind(row.note.as_deref())
                    .bind($ids::bind_opt(row.set_by))
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert tier override: {e}"))
                    })?;

                Ok(())
            }

            async fn delete(&self, user_id: Uuid) -> AppResult<bool> {
                let res = sqlx::query(DELETE_TIER_OVERRIDE_SQL)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete tier override: {e}"))
                    })?;
                Ok(res.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_user_tier_override_repository;
