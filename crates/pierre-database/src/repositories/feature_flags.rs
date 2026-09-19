// ABOUTME: Feature-flags repository trait, shared statements and body — tenant defaults + per-user overrides
// ABOUTME: Resolution precedence: per-user override > tenant default > compile-time default
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Feature flags, written once.
//!
//! Two tables of the same shape — `tenant_feature_defaults` keyed by tenant
//! and `user_feature_overrides` keyed by user — each with a list, an upsert
//! and a delete. Resolution lives in the trait's default method; the
//! statements below are raw row CRUD.
//!
//! The two backends differ in one respect only: the id columns are `uuid`
//! on Postgres and `TEXT` on `SQLite`. A uuid is therefore bound through the
//! macro's `$bind_id` function (native on Postgres, hyphenated text on
//! `SQLite`) and `updated_by` is read back through `$text`, the cast that
//! turns the column into text on Postgres and is empty on `SQLite`, so one
//! row parser decodes both. `enabled` binds and decodes as a `bool` on both:
//! sqlx writes it as the integer 0/1 the `SQLite` schema's `CHECK` accepts.
//! `updated_at` binds and decodes as `DateTime<Utc>` on both: RFC 3339 text
//! on `SQLite`, byte-identical to the `to_rfc3339()` the rows were written
//! with, and `TIMESTAMPTZ` on Postgres.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::feature_flags::FeatureKey;
use std::collections::HashMap;
use std::str::FromStr;
use uuid::Uuid;

/// A single stored feature-flag row.
///
/// Returned by [`FeatureFlagsRepository::list_tenant_defaults`] for tenant
/// scope and by [`FeatureFlagsRepository::list_user_overrides`] for per-user
/// scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureFlagRow {
    /// The feature being flagged.
    pub feature_key: FeatureKey,
    /// Stored value at this scope (tenant or user).
    pub enabled: bool,
    /// Last write timestamp.
    pub updated_at: DateTime<Utc>,
    /// Admin user who last wrote the row (audit trail). `None` when the
    /// referenced admin has been deleted (FK `ON DELETE SET NULL`).
    pub updated_by: Option<Uuid>,
}

/// CRUD for `tenant_feature_defaults` and `user_feature_overrides`.
///
/// Resolution semantics: `resolve_for_user` returns the effective value for
/// every known [`FeatureKey`] using the precedence
/// **per-user override > tenant default > compile-time default**.
///
/// Admin endpoints layer tenant-membership checks on top; this trait does
/// raw row CRUD only.
#[async_trait]
pub trait FeatureFlagsRepository: Send + Sync {
    /// Every stored tenant default for `tenant_id`, in `feature_key` order.
    /// Empty vec when nothing is configured.
    async fn list_tenant_defaults(&self, tenant_id: Uuid) -> AppResult<Vec<FeatureFlagRow>>;

    /// Insert-or-update one tenant default. `updated_at` is set to now;
    /// `updated_by` is the admin acting (audit), or `None` for system calls.
    async fn set_tenant_default(
        &self,
        tenant_id: Uuid,
        feature_key: FeatureKey,
        enabled: bool,
        updated_by: Option<Uuid>,
    ) -> AppResult<()>;

    /// Remove one tenant-default row. Returns `true` when a row was deleted,
    /// `false` when none matched (caller can treat both as success).
    async fn clear_tenant_default(
        &self,
        tenant_id: Uuid,
        feature_key: FeatureKey,
    ) -> AppResult<bool>;

    /// Every stored per-user override for `user_id`, in `feature_key` order.
    async fn list_user_overrides(&self, user_id: Uuid) -> AppResult<Vec<FeatureFlagRow>>;

    /// Insert-or-update one per-user override.
    async fn set_user_override(
        &self,
        user_id: Uuid,
        feature_key: FeatureKey,
        enabled: bool,
        updated_by: Option<Uuid>,
    ) -> AppResult<()>;

    /// Remove one per-user override row.
    async fn clear_user_override(&self, user_id: Uuid, feature_key: FeatureKey) -> AppResult<bool>;

    /// Effective flag map for a user: user-override > tenant-default >
    /// `FeatureKey::default_enabled()`. The returned map always contains
    /// every variant in `FeatureKey::ALL`.
    ///
    /// Default impl performs two reads (`list_user_overrides`,
    /// `list_tenant_defaults`) then merges; backends inherit unless they
    /// have a SQL-side optimisation.
    async fn resolve_for_user(
        &self,
        tenant_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<HashMap<FeatureKey, bool>> {
        let tenant = self.list_tenant_defaults(tenant_id).await?;
        let user = self.list_user_overrides(user_id).await?;

        let mut effective: HashMap<FeatureKey, bool> = FeatureKey::ALL
            .iter()
            .map(|k| (*k, k.default_enabled()))
            .collect();

        for row in tenant {
            effective.insert(row.feature_key, row.enabled);
        }
        for row in user {
            effective.insert(row.feature_key, row.enabled);
        }

        Ok(effective)
    }
}

/// The columns every read decodes, with `$text` appended to `updated_by` so
/// the audit uuid arrives as text on both backends.
macro_rules! feature_flag_columns {
    ($text:literal) => {
        concat!(
            "feature_key, enabled, updated_at, updated_by",
            $text,
            " AS updated_by"
        )
    };
}
pub(crate) use feature_flag_columns;

/// Every stored tenant default for `$1`, in `feature_key` order.
macro_rules! list_tenant_defaults_sql {
    ($text:literal) => {
        concat!(
            "
            SELECT ",
            feature_flag_columns!($text),
            "
            FROM tenant_feature_defaults
            WHERE tenant_id = $1
            ORDER BY feature_key
            "
        )
    };
}
pub(crate) use list_tenant_defaults_sql;

/// Every stored per-user override for `$1`, in `feature_key` order.
macro_rules! list_user_overrides_sql {
    ($text:literal) => {
        concat!(
            "
            SELECT ",
            feature_flag_columns!($text),
            "
            FROM user_feature_overrides
            WHERE user_id = $1
            ORDER BY feature_key
            "
        )
    };
}
pub(crate) use list_user_overrides_sql;

/// Insert-or-update one tenant default.
pub(crate) const SET_TENANT_DEFAULT_SQL: &str = r"
            INSERT INTO tenant_feature_defaults
                (tenant_id, feature_key, enabled, updated_at, updated_by)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (tenant_id, feature_key) DO UPDATE SET
                enabled = EXCLUDED.enabled,
                updated_at = EXCLUDED.updated_at,
                updated_by = EXCLUDED.updated_by
            ";

/// Remove one tenant-default row.
pub(crate) const CLEAR_TENANT_DEFAULT_SQL: &str = r"
            DELETE FROM tenant_feature_defaults
            WHERE tenant_id = $1 AND feature_key = $2
            ";

/// Insert-or-update one per-user override.
pub(crate) const SET_USER_OVERRIDE_SQL: &str = r"
            INSERT INTO user_feature_overrides
                (user_id, feature_key, enabled, updated_at, updated_by)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (user_id, feature_key) DO UPDATE SET
                enabled = EXCLUDED.enabled,
                updated_at = EXCLUDED.updated_at,
                updated_by = EXCLUDED.updated_by
            ";

/// Remove one per-user override row.
pub(crate) const CLEAR_USER_OVERRIDE_SQL: &str = r"
            DELETE FROM user_feature_overrides
            WHERE user_id = $1 AND feature_key = $2
            ";

/// Decode one flag row, or `None` for a key this build no longer knows: a
/// row for a deleted key still exists, and dropping it at the read site
/// lets the caller fall back to the compile-time default. `try_get`
/// throughout, never `Row::get`, so a corrupt row surfaces as a recoverable
/// error rather than a panic.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded
/// or when `updated_by` does not hold a uuid.
pub(crate) fn flag_from_row<R>(row: &R) -> AppResult<Option<FeatureFlagRow>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let key_raw: String = row
        .try_get("feature_key")
        .map_err(|e| AppError::database(format!("feature flag feature_key: {e}")))?;
    let Ok(feature_key) = FeatureKey::from_str(&key_raw) else {
        return Ok(None);
    };
    let updated_by: Option<String> = row
        .try_get("updated_by")
        .map_err(|e| AppError::database(format!("feature flag updated_by: {e}")))?;
    let updated_by = updated_by
        .map(|s| {
            Uuid::parse_str(&s)
                .map_err(|e| AppError::database(format!("Invalid updated_by uuid '{s}': {e}")))
        })
        .transpose()?;
    Ok(Some(FeatureFlagRow {
        feature_key,
        enabled: row
            .try_get("enabled")
            .map_err(|e| AppError::database(format!("feature flag enabled: {e}")))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| AppError::database(format!("feature flag updated_at: {e}")))?,
        updated_by,
    }))
}

/// Decode every row a list query returned, dropping unknown keys.
///
/// # Errors
/// Returns the first row's decode error.
pub(crate) fn flags_from_rows<R>(rows: &[R]) -> AppResult<Vec<FeatureFlagRow>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(flag) = flag_from_row(row)? {
            out.push(flag);
        }
    }
    Ok(out)
}

/// Emit the whole [`FeatureFlagsRepository`] implementation for one backend
/// type; `resolve_for_user` stays the trait's default method.
///
/// `$bind_id` is the function turning a `Uuid` into whatever that backend's
/// uuid columns accept (the `bind` of its codec in [`super::uuid_columns`]);
/// `$text` is the cast that reads `updated_by` back as text
/// (`"::text"` on Postgres, `""` on `SQLite`).
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_feature_flags_repository {
    ($ty:ty, $bind_id:path, $text:literal) => {
        #[async_trait::async_trait]
        impl FeatureFlagsRepository for $ty {
            async fn list_tenant_defaults(
                &self,
                tenant_id: Uuid,
            ) -> AppResult<Vec<FeatureFlagRow>> {
                let rows = sqlx::query(list_tenant_defaults_sql!($text))
                    .bind($bind_id(tenant_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list tenant feature defaults: {e}"))
                    })?;
                flags_from_rows(&rows)
            }

            async fn set_tenant_default(
                &self,
                tenant_id: Uuid,
                feature_key: FeatureKey,
                enabled: bool,
                updated_by: Option<Uuid>,
            ) -> AppResult<()> {
                sqlx::query(SET_TENANT_DEFAULT_SQL)
                    .bind($bind_id(tenant_id))
                    .bind(feature_key.as_str())
                    .bind(enabled)
                    .bind(Utc::now())
                    .bind(updated_by.map($bind_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set tenant feature default: {e}"))
                    })?;
                Ok(())
            }

            async fn clear_tenant_default(
                &self,
                tenant_id: Uuid,
                feature_key: FeatureKey,
            ) -> AppResult<bool> {
                let res = sqlx::query(CLEAR_TENANT_DEFAULT_SQL)
                    .bind($bind_id(tenant_id))
                    .bind(feature_key.as_str())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to clear tenant feature default: {e}"))
                    })?;
                Ok(res.rows_affected() > 0)
            }

            async fn list_user_overrides(&self, user_id: Uuid) -> AppResult<Vec<FeatureFlagRow>> {
                let rows = sqlx::query(list_user_overrides_sql!($text))
                    .bind($bind_id(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list user feature overrides: {e}"))
                    })?;
                flags_from_rows(&rows)
            }

            async fn set_user_override(
                &self,
                user_id: Uuid,
                feature_key: FeatureKey,
                enabled: bool,
                updated_by: Option<Uuid>,
            ) -> AppResult<()> {
                sqlx::query(SET_USER_OVERRIDE_SQL)
                    .bind($bind_id(user_id))
                    .bind(feature_key.as_str())
                    .bind(enabled)
                    .bind(Utc::now())
                    .bind(updated_by.map($bind_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set user feature override: {e}"))
                    })?;
                Ok(())
            }

            async fn clear_user_override(
                &self,
                user_id: Uuid,
                feature_key: FeatureKey,
            ) -> AppResult<bool> {
                let res = sqlx::query(CLEAR_USER_OVERRIDE_SQL)
                    .bind($bind_id(user_id))
                    .bind(feature_key.as_str())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to clear user feature override: {e}"))
                    })?;
                Ok(res.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_feature_flags_repository;
