// ABOUTME: Repository trait, statements and shared body for per-user rate-limit overrides (exemption pattern)
// ABOUTME: Row presence wins over UserTier::monthly_limit(); written once, emitted for each backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use uuid::Uuid;

/// Per-user rate-limit override row (industry-standard exemption pattern).
///
/// When a row exists for a user, its values win over the tier-keyed default
/// computed from `UserTier::monthly_limit()` and friends. `None` on either
/// limit field means "unlimited for this dimension" (same semantics as the
/// existing `Enterprise.monthly_limit()` returning `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRateLimitOverride {
    /// User the override applies to.
    pub user_id: Uuid,
    /// Custom daily request cap. `None` = unlimited daily.
    pub daily_limit: Option<u32>,
    /// Custom monthly request cap. `None` = unlimited monthly.
    pub monthly_limit: Option<u32>,
    /// Operator-facing note explaining why the override exists.
    pub note: Option<String>,
    /// Admin user who set the override (audit trail).
    pub set_by: Option<Uuid>,
    /// First-set timestamp.
    pub set_at: DateTime<Utc>,
    /// Most-recent update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// CRUD for `user_rate_limit_overrides`, the per-user exemption table.
///
/// The auth middleware enforces a row and `compute_user_rate_limits` (in
/// `pierre-services`) reports it, both through `UserRequestLimits::resolve`
/// (in `pierre-auth`), before falling back to the tier default.
///
/// A quota surface. The table carries no `tenant_id`; every statement is
/// scoped by `user_id`, the strictly narrower key, and the admin handler
/// checks the caller's standing before reaching here.
#[async_trait]
pub trait UserRateLimitOverrideRepository: Send + Sync {
    /// Fetch the override row for a user, or `None` if no override is set
    /// (tier default applies).
    async fn get(&self, user_id: Uuid) -> AppResult<Option<UserRateLimitOverride>>;

    /// Insert or update the override row for a user. `set_at` is preserved
    /// on update; `updated_at` is always bumped to the call time.
    async fn upsert(&self, row: &UserRateLimitOverride) -> AppResult<()>;

    /// Remove the override row so the user reverts to the tier default.
    /// Returns `true` when a row was removed, `false` when no override
    /// existed.
    async fn delete(&self, user_id: Uuid) -> AppResult<bool>;
}

/// The override row for one user.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends and cannot drift between
/// them. The id binds through the backend's uuid codec (see
/// [`super::uuid_columns`]); every other bind is a plain `Option<i32>`,
/// `Option<&str>` or `DateTime<Utc>` both drivers encode alike.
pub(crate) const GET_RATE_LIMIT_OVERRIDE_SQL: &str = r"
            SELECT user_id, daily_limit, monthly_limit, note, set_by, set_at, updated_at
            FROM user_rate_limit_overrides
            WHERE user_id = $1
            ";

/// Insert, or on conflict update everything but `set_at`.
///
/// `set_at` and `updated_at` both bind the call time (`$6`); the `DO UPDATE`
/// never names `set_at`, so an existing row keeps its first-set timestamp
/// while `updated_at` advances. Limits bind as `i32`: the column is a 4-byte
/// `INTEGER` on Postgres, and a cap above `i32::MAX` saturates rather than
/// failing the write.
pub(crate) const UPSERT_RATE_LIMIT_OVERRIDE_SQL: &str = r"
            INSERT INTO user_rate_limit_overrides
                (user_id, daily_limit, monthly_limit, note, set_by, set_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $6)
            ON CONFLICT (user_id) DO UPDATE SET
                daily_limit = EXCLUDED.daily_limit,
                monthly_limit = EXCLUDED.monthly_limit,
                note = EXCLUDED.note,
                set_by = EXCLUDED.set_by,
                updated_at = EXCLUDED.updated_at
            ";

/// Drop the override so the tier default applies again.
pub(crate) const DELETE_RATE_LIMIT_OVERRIDE_SQL: &str =
    "DELETE FROM user_rate_limit_overrides WHERE user_id = $1";

/// The error for a column of this table that would not decode.
pub(crate) fn rate_limit_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to read rate-limit override {name}: {e}"))
}

/// A stored cap as the domain sees it: a negative value (which the schema
/// never writes) reads as unlimited rather than wrapping.
pub(crate) fn limit_from_column(raw: Option<i32>) -> Option<u32> {
    raw.and_then(|v| u32::try_from(v).ok())
}

/// A domain cap as the column stores it, saturating at the column's width.
pub(crate) fn limit_to_column(limit: u32) -> i32 {
    i32::try_from(limit).unwrap_or(i32::MAX)
}

/// Emit the whole [`UserRateLimitOverrideRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type and its uuid codec, and sqlx resolves the driver from
/// `self.pool()` per expansion.
///
/// `$ids` is the codec in [`super::uuid_columns`] for how that backend's
/// `user_id` and `set_by` columns bind and read.
macro_rules! impl_user_rate_limit_override_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl UserRateLimitOverrideRepository for $ty {
            async fn get(&self, user_id: Uuid) -> AppResult<Option<UserRateLimitOverride>> {
                let row = sqlx::query(GET_RATE_LIMIT_OVERRIDE_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch rate-limit override: {e}"))
                    })?;

                let Some(row) = row else { return Ok(None) };

                Ok(Some(UserRateLimitOverride {
                    user_id: $ids::read(&row, "user_id")?,
                    daily_limit: limit_from_column(
                        row.try_get("daily_limit")
                            .map_err(|e| rate_limit_column_error("daily_limit", e))?,
                    ),
                    monthly_limit: limit_from_column(
                        row.try_get("monthly_limit")
                            .map_err(|e| rate_limit_column_error("monthly_limit", e))?,
                    ),
                    note: row
                        .try_get("note")
                        .map_err(|e| rate_limit_column_error("note", e))?,
                    set_by: $ids::read_opt(&row, "set_by")?,
                    set_at: row
                        .try_get("set_at")
                        .map_err(|e| rate_limit_column_error("set_at", e))?,
                    updated_at: row
                        .try_get("updated_at")
                        .map_err(|e| rate_limit_column_error("updated_at", e))?,
                }))
            }

            async fn upsert(&self, row: &UserRateLimitOverride) -> AppResult<()> {
                sqlx::query(UPSERT_RATE_LIMIT_OVERRIDE_SQL)
                    .bind($ids::bind(row.user_id))
                    .bind(row.daily_limit.map(limit_to_column))
                    .bind(row.monthly_limit.map(limit_to_column))
                    .bind(row.note.as_deref())
                    .bind($ids::bind_opt(row.set_by))
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert rate-limit override: {e}"))
                    })?;

                Ok(())
            }

            async fn delete(&self, user_id: Uuid) -> AppResult<bool> {
                let res = sqlx::query(DELETE_RATE_LIMIT_OVERRIDE_SQL)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete rate-limit override: {e}"))
                    })?;
                Ok(res.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_user_rate_limit_override_repository;
