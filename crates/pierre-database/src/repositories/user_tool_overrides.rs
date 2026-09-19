// ABOUTME: Repository trait, statements and shared body for per-user admin tool overrides (allow/deny one tool for one user)
// ABOUTME: Overlay consulted by ToolSelectionService above tenant plan + tenant overrides; written once, emitted per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Per-user admin tool override.
///
/// Records that an operator explicitly enabled or disabled a single MCP tool
/// for one user, independent of the user's tenant plan or any tenant-level
/// tool override. `ToolSelectionService` consults it as a per-request overlay
/// above the (cached) tenant computation: a user override wins over plan
/// restriction, tenant override, and catalog default. A globally-disabled tool
/// (`PIERRE_DISABLED_TOOLS`) stays off and is never resurrected here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserToolOverride {
    /// User the override applies to.
    pub user_id: Uuid,
    /// Catalogued MCP tool name being overridden.
    pub tool_name: String,
    /// `true` force-enables the tool for this user, `false` force-disables it.
    pub is_enabled: bool,
    /// Admin user who set the override (audit trail). `None` for service
    /// tokens that do not map to a user UUID.
    pub set_by: Option<Uuid>,
    /// Operator-facing note explaining why the override exists.
    pub reason: Option<String>,
    /// First-set timestamp.
    pub created_at: DateTime<Utc>,
    /// Most-recent update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// CRUD for `user_tool_overrides` — the per-user tool allow/deny layer applied
/// as an overlay on top of the tenant tool-selection computation.
///
/// A permission surface. The table carries no `tenant_id`; every statement
/// is scoped by `user_id` (and `tool_name`), the strictly narrower key, and
/// the admin handler checks the caller's standing before reaching here.
#[async_trait]
pub trait UserToolOverrideRepository: Send + Sync {
    /// Fetch the override for one `(user, tool)`, or `None` if unset.
    async fn get(&self, user_id: Uuid, tool_name: &str) -> AppResult<Option<UserToolOverride>>;

    /// All overrides for a user (the overlay map). Empty when none are set.
    async fn list_for_user(&self, user_id: Uuid) -> AppResult<Vec<UserToolOverride>>;

    /// Insert or update the override for one `(user, tool)`. `created_at` is
    /// preserved on update; `updated_at` is always bumped to the call time.
    async fn upsert(&self, row: &UserToolOverride) -> AppResult<()>;

    /// Remove the override so the tool reverts to plan/tenant/default.
    /// Returns `true` when a row was removed.
    async fn delete(&self, user_id: Uuid, tool_name: &str) -> AppResult<bool>;
}

/// The columns every read of `user_tool_overrides` returns, in the order
/// the macro's `tool_override_from_row` reads them. One list, so a column added to
/// [`UserToolOverride`] reaches both reads at once.
macro_rules! tool_override_columns {
    () => {
        "user_id, tool_name, is_enabled, set_by, reason, created_at, updated_at"
    };
}

/// The override for one `(user, tool)`.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends and cannot drift between
/// them. The ids bind through the backend's uuid codec (see
/// [`super::uuid_columns`]); every other bind is a plain `&str`,
/// `Option<&str>`, `bool` or `DateTime<Utc>` both drivers encode alike —
/// `is_enabled` is `BOOLEAN` on Postgres and a 0/1 `INTEGER` on `SQLite`,
/// and a `bool` binds and decodes as both.
pub(crate) const GET_TOOL_OVERRIDE_SQL: &str = concat!(
    "SELECT ",
    tool_override_columns!(),
    " FROM user_tool_overrides WHERE user_id = $1 AND tool_name = $2"
);

/// Every override for one user, in no particular order: the caller folds
/// them into a map keyed by tool name.
pub(crate) const LIST_TOOL_OVERRIDES_SQL: &str = concat!(
    "SELECT ",
    tool_override_columns!(),
    " FROM user_tool_overrides WHERE user_id = $1"
);

/// Insert, or on conflict update everything but `created_at`.
///
/// `created_at` and `updated_at` both bind the call time (`$6`); the
/// `DO UPDATE` never names `created_at`, so an existing row keeps its
/// first-set timestamp while `updated_at` advances.
pub(crate) const UPSERT_TOOL_OVERRIDE_SQL: &str = r"
            INSERT INTO user_tool_overrides
                (user_id, tool_name, is_enabled, set_by, reason, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $6)
            ON CONFLICT (user_id, tool_name) DO UPDATE SET
                is_enabled = EXCLUDED.is_enabled,
                set_by = EXCLUDED.set_by,
                reason = EXCLUDED.reason,
                updated_at = EXCLUDED.updated_at
            ";

/// Drop the override so plan, tenant override and catalog default apply.
pub(crate) const DELETE_TOOL_OVERRIDE_SQL: &str =
    "DELETE FROM user_tool_overrides WHERE user_id = $1 AND tool_name = $2";

/// The error for a column of this table that would not decode.
pub(crate) fn tool_override_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to read tool override {name}: {e}"))
}

/// Emit the whole [`UserToolOverrideRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type and its uuid codec, and sqlx resolves the driver from
/// `self.pool()` per expansion.
///
/// `$row` is the driver's row type and `$ids` the codec in
/// [`super::uuid_columns`] for how that backend's `user_id` and `set_by`
/// columns bind and read; the row parser is emitted inside the macro because
/// those two reads are the one thing in it that differs per driver.
macro_rules! impl_user_tool_override_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// Decode one override row via `try_get` only, so a corrupt row
        /// surfaces as a recoverable error rather than a panic.
        fn tool_override_from_row(row: &$row) -> AppResult<UserToolOverride> {
            Ok(UserToolOverride {
                user_id: $ids::read(row, "user_id")?,
                tool_name: row
                    .try_get("tool_name")
                    .map_err(|e| tool_override_column_error("tool_name", e))?,
                is_enabled: row
                    .try_get("is_enabled")
                    .map_err(|e| tool_override_column_error("is_enabled", e))?,
                set_by: $ids::read_opt(row, "set_by")?,
                reason: row
                    .try_get("reason")
                    .map_err(|e| tool_override_column_error("reason", e))?,
                created_at: row
                    .try_get("created_at")
                    .map_err(|e| tool_override_column_error("created_at", e))?,
                updated_at: row
                    .try_get("updated_at")
                    .map_err(|e| tool_override_column_error("updated_at", e))?,
            })
        }

        #[async_trait::async_trait]
        impl UserToolOverrideRepository for $ty {
            async fn get(
                &self,
                user_id: Uuid,
                tool_name: &str,
            ) -> AppResult<Option<UserToolOverride>> {
                let row = sqlx::query(GET_TOOL_OVERRIDE_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tool_name)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch tool override: {e}"))
                    })?;

                row.as_ref().map(tool_override_from_row).transpose()
            }

            async fn list_for_user(&self, user_id: Uuid) -> AppResult<Vec<UserToolOverride>> {
                let rows = sqlx::query(LIST_TOOL_OVERRIDES_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list tool overrides: {e}"))
                    })?;

                rows.iter().map(tool_override_from_row).collect()
            }

            async fn upsert(&self, row: &UserToolOverride) -> AppResult<()> {
                sqlx::query(UPSERT_TOOL_OVERRIDE_SQL)
                    .bind($ids::bind(row.user_id))
                    .bind(&row.tool_name)
                    .bind(row.is_enabled)
                    .bind($ids::bind_opt(row.set_by))
                    .bind(row.reason.as_deref())
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert tool override: {e}"))
                    })?;

                Ok(())
            }

            async fn delete(&self, user_id: Uuid, tool_name: &str) -> AppResult<bool> {
                let res = sqlx::query(DELETE_TOOL_OVERRIDE_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tool_name)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete tool override: {e}"))
                    })?;
                Ok(res.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_user_tool_override_repository;
