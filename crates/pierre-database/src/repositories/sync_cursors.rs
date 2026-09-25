// ABOUTME: Shared statements, row decoders and body for provider sync cursors and the connected-user roster the health sync runs over
// ABOUTME: One SQL text per operation; each backend shell supplies its type and how it reads user_oauth_tokens.tenant_id
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sync cursors, written once.
//!
//! `sync_state` keys on text ids on both backends and its timestamps bind as
//! [`DateTime<Utc>`], which sqlx-sqlite encodes to the bytes `to_rfc3339()`
//! writes. The one clause the two engines spell differently — how
//! `user_oauth_tokens.tenant_id` is read for [`ConnectedUserRow`] — is the
//! macro's second argument.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserId};

use super::health::{ConnectedUserRow, SyncCursorRow};
use super::health_persistence::column;

// ============================================================================
// sync_state
// ============================================================================

/// One provider/data-type cursor for a user under a tenant.
pub(crate) const GET_SYNC_CURSOR_SQL: &str = r"
            SELECT id, user_id, tenant_id, provider, data_type, cursor_value,
                   last_sync_at, last_sync_status, records_synced, error_message,
                   retry_count, next_retry_at
            FROM sync_state
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND data_type = $4
            ";

/// Insert or refresh a cursor; every mutable column is replaced on conflict.
pub(crate) const UPSERT_SYNC_CURSOR_SQL: &str = r"
            INSERT INTO sync_state (id, user_id, tenant_id, provider, data_type, cursor_value,
                last_sync_at, last_sync_status, records_synced, error_message, retry_count,
                next_retry_at, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT(user_id, tenant_id, provider, data_type) DO UPDATE SET
                cursor_value = EXCLUDED.cursor_value,
                last_sync_at = EXCLUDED.last_sync_at,
                last_sync_status = EXCLUDED.last_sync_status,
                records_synced = EXCLUDED.records_synced,
                error_message = EXCLUDED.error_message,
                retry_count = EXCLUDED.retry_count,
                next_retry_at = EXCLUDED.next_retry_at,
                updated_at = EXCLUDED.updated_at
            ";

/// Every cursor one user holds for one provider under one tenant.
pub(crate) const RESET_SYNC_CURSORS_SQL: &str = r"
            DELETE FROM sync_state
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ";

/// Every user holding a credential for a provider, with the credential kind.
///
/// `$tenant_col` is the one backend-specific clause on this table.
/// `user_oauth_tokens.tenant_id` is a `VARCHAR` on Postgres (the app binds
/// `tenant.id.to_string()`) while [`ConnectedUserRow::tenant_id`] is the
/// [`TenantId`] newtype whose Postgres decode is a native uuid, so reading
/// the column directly fails with "mismatched types" and silently kills the
/// whole scheduled health sync; Postgres passes `"tenant_id::uuid AS
/// tenant_id"` so the native decode works. `SQLite` stores the hyphenated
/// text the newtype decodes from and passes `"tenant_id"`.
macro_rules! list_connected_provider_users_sql {
    ($tenant_col:literal) => {
        concat!(
            "
            SELECT DISTINCT user_id, ",
            $tenant_col,
            ", token_type
            FROM user_oauth_tokens
            WHERE provider = $1
            "
        )
    };
}
pub(crate) use list_connected_provider_users_sql;

/// Decode a `sync_state` row.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn sync_cursor_from_row<R>(row: &R) -> AppResult<SyncCursorRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(SyncCursorRow {
        id: column(row, "id")?,
        user_id: column(row, "user_id")?,
        tenant_id: column(row, "tenant_id")?,
        provider: column(row, "provider")?,
        data_type: column(row, "data_type")?,
        cursor_value: column(row, "cursor_value")?,
        last_sync_at: column(row, "last_sync_at")?,
        last_sync_status: column(row, "last_sync_status")?,
        records_synced: column(row, "records_synced")?,
        error_message: column(row, "error_message")?,
        retry_count: column(row, "retry_count")?,
        next_retry_at: column(row, "next_retry_at")?,
    })
}

/// Decode one `user_oauth_tokens` identity row.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn connected_user_from_row<R>(row: &R) -> AppResult<ConnectedUserRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    UserId: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    TenantId: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(ConnectedUserRow {
        user_id: row
            .try_get("user_id")
            .map_err(|e| AppError::database(format!("decode user_id as UserId: {e}")))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| AppError::database(format!("decode tenant_id as TenantId: {e}")))?,
        token_type: row
            .try_get("token_type")
            .map_err(|e| AppError::database(format!("decode token_type as String: {e}")))?,
    })
}

/// Emit the [`SyncCursorRepository`] implementation for one backend type.
///
/// `$tenant_col` is the literal [`list_connected_provider_users_sql!`] takes:
/// `"tenant_id::uuid AS tenant_id"` on Postgres, `"tenant_id"` on `SQLite`.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
///
/// [`SyncCursorRepository`]: super::health::SyncCursorRepository
macro_rules! impl_sync_cursor_repository {
    ($ty:ty, $tenant_col:literal) => {
        #[async_trait::async_trait]
        impl SyncCursorRepository for $ty {
            async fn get_sync_cursor(
                &self,
                user_id: &str,
                tenant_id: &TenantId,
                provider: &str,
                data_type: &str,
            ) -> AppResult<Option<SyncCursorRow>> {
                let row = sqlx::query(GET_SYNC_CURSOR_SQL)
                    .bind(user_id)
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .bind(data_type)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get sync cursor: {e}")))?;

                row.as_ref().map(sync_cursor_from_row).transpose()
            }

            async fn upsert_sync_cursor(&self, cursor: &SyncCursorRow) -> AppResult<()> {
                let now = Utc::now();

                sqlx::query(UPSERT_SYNC_CURSOR_SQL)
                    .bind(&cursor.id)
                    .bind(&cursor.user_id)
                    .bind(&cursor.tenant_id)
                    .bind(&cursor.provider)
                    .bind(&cursor.data_type)
                    .bind(&cursor.cursor_value)
                    .bind(cursor.last_sync_at)
                    .bind(&cursor.last_sync_status)
                    .bind(cursor.records_synced)
                    .bind(&cursor.error_message)
                    .bind(cursor.retry_count)
                    .bind(cursor.next_retry_at)
                    .bind(now)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert sync cursor: {e}"))
                    })?;

                Ok(())
            }

            async fn list_connected_provider_users(
                &self,
                provider: &str,
            ) -> AppResult<Vec<ConnectedUserRow>> {
                let rows = sqlx::query(list_connected_provider_users_sql!($tenant_col))
                    .bind(provider)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list connected provider users: {e}"))
                    })?;

                rows.iter().map(connected_user_from_row).collect()
            }

            async fn reset_sync_cursors(
                &self,
                user_id: &str,
                tenant_id: &TenantId,
                provider: &str,
            ) -> AppResult<u64> {
                let result = sqlx::query(RESET_SYNC_CURSORS_SQL)
                    .bind(user_id)
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to reset sync cursors: {e}"))
                    })?;

                Ok(result.rows_affected())
            }
        }
    };
}
pub(crate) use impl_sync_cursor_repository;
