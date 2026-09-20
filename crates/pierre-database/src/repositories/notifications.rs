// ABOUTME: Repository trait, statements and shared body for OAuth completion notifications
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its user_id column needs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! OAuth completion notifications, written once.
//!
//! A row per completed OAuth flow, read back by the MCP notification
//! resources until it is marked read. `user_id` is a `uuid` column on
//! Postgres and `TEXT` on `SQLite`, so the shell hands the body its
//! [`uuid_columns`](super::uuid_columns) codec and the wire DTO's text form
//! is read through it. `success` binds and reads as `bool` (`BOOLEAN` on
//! Postgres, `INTEGER` 0/1 on `SQLite`); `created_at` binds as
//! [`DateTime<Utc>`](chrono::DateTime) and `read_at` is stamped by
//! `CURRENT_TIMESTAMP`, which sqlx decodes on both drivers. `expires_at` is
//! the caller's text on both.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use std::fmt::Write;

use pierre_core::models::OAuthNotification;
use uuid::Uuid;

/// OAuth notification repository
#[async_trait]
pub trait NotificationRepository: Send + Sync {
    /// Store OAuth completion notification for MCP resource delivery
    async fn store(
        &self,
        user_id: Uuid,
        provider: &str,
        success: bool,
        message: &str,
        expires_at: Option<&str>,
    ) -> AppResult<String>;
    /// Get unread OAuth notifications for a user
    async fn get_unread(&self, user_id: Uuid) -> AppResult<Vec<OAuthNotification>>;
    /// Mark OAuth notification as read
    async fn mark_read(&self, notification_id: &str, user_id: Uuid) -> AppResult<bool>;
    /// Mark all OAuth notifications as read for a user
    async fn mark_all_read(&self, user_id: Uuid) -> AppResult<u64>;
    /// Get all OAuth notifications for a user (read and unread)
    async fn get_all(&self, user_id: Uuid, limit: Option<i64>)
        -> AppResult<Vec<OAuthNotification>>;
}

/// Record one completed flow.
pub(crate) const STORE_OAUTH_NOTIFICATION_SQL: &str = r"
            INSERT INTO oauth_notifications (id, user_id, provider, success, message, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ";

/// The user's notifications nobody has marked read, newest first.
pub(crate) const UNREAD_OAUTH_NOTIFICATIONS_SQL: &str = r"
            SELECT id, user_id, provider, success, message, expires_at, created_at, read_at
            FROM oauth_notifications
            WHERE user_id = $1 AND read_at IS NULL
            ORDER BY created_at DESC
            ";

/// Mark one of the user's notifications read; a row already read is left as
/// it was, so the affected count says whether this call did it.
pub(crate) const MARK_OAUTH_NOTIFICATION_READ_SQL: &str = r"
            UPDATE oauth_notifications
            SET read_at = CURRENT_TIMESTAMP
            WHERE id = $1 AND user_id = $2 AND read_at IS NULL
            ";

/// Mark every unread notification of the user read.
pub(crate) const MARK_ALL_OAUTH_NOTIFICATIONS_READ_SQL: &str = r"
            UPDATE oauth_notifications
            SET read_at = CURRENT_TIMESTAMP
            WHERE user_id = $1 AND read_at IS NULL
            ";

/// Every notification of the user, read or not, newest first; the caller's
/// optional row cap is appended as a `LIMIT` clause.
pub(crate) const ALL_OAUTH_NOTIFICATIONS_SQL: &str = r"
            SELECT id, user_id, provider, success, message, expires_at, created_at, read_at
            FROM oauth_notifications
            WHERE user_id = $1
            ORDER BY created_at DESC
            ";

/// [`ALL_OAUTH_NOTIFICATIONS_SQL`] with the caller's row cap, when it gave one.
///
/// # Errors
/// Returns an internal error when the clause cannot be appended.
pub(crate) fn all_oauth_notifications_sql(limit: Option<i64>) -> AppResult<String> {
    let mut sql = ALL_OAUTH_NOTIFICATIONS_SQL.to_owned();
    if let Some(limit) = limit {
        write!(sql, " LIMIT {limit}")
            .map_err(|e| AppError::internal(format!("Format error: {e}")))?;
    }
    Ok(sql)
}

/// Decode one `oauth_notifications` row into the wire DTO. `user_id` is read
/// by the caller through its backend's codec, since the column is native uuid
/// on Postgres and text on `SQLite`; every other column decodes the same way
/// on both drivers. `try_get` throughout, never `Row::get`, so a corrupt row
/// surfaces as a recoverable error rather than a panic.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn oauth_notification_from_row<R>(
    row: &R,
    user_id: String,
) -> AppResult<OAuthNotification>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| {
        AppError::database(format!("Failed to get oauth_notifications.{col}: {e}"))
    };
    Ok(OAuthNotification {
        id: row.try_get("id").map_err(|e| column("id", e))?,
        user_id,
        provider: row.try_get("provider").map_err(|e| column("provider", e))?,
        success: row.try_get("success").map_err(|e| column("success", e))?,
        message: row.try_get("message").map_err(|e| column("message", e))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| column("expires_at", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
        read_at: row.try_get("read_at").map_err(|e| column("read_at", e))?,
    })
}

/// Emit the whole [`NotificationRepository`] implementation for one backend
/// type.
///
/// `$ids` is that backend's [`uuid_columns`](super::uuid_columns) codec, which
/// spells how the `user_id` column binds and reads back as the DTO's text.
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_notification_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl NotificationRepository for $ty {
            async fn store(
                &self,
                user_id: Uuid,
                provider: &str,
                success: bool,
                message: &str,
                expires_at: Option<&str>,
            ) -> AppResult<String> {
                let notification_id = Uuid::new_v4().to_string();

                sqlx::query(STORE_OAUTH_NOTIFICATION_SQL)
                    .bind(&notification_id)
                    .bind($ids::bind(user_id))
                    .bind(provider)
                    .bind(success)
                    .bind(message)
                    .bind(expires_at)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store OAuth notification: {e}"))
                    })?;

                Ok(notification_id)
            }

            async fn get_unread(&self, user_id: Uuid) -> AppResult<Vec<OAuthNotification>> {
                let rows = sqlx::query(UNREAD_OAUTH_NOTIFICATIONS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to query unread OAuth notifications: {e}"
                        ))
                    })?;
                debug!(
                    "Found {} unread notification rows for user {}",
                    rows.len(),
                    user_id
                );

                rows.iter()
                    .map(|row| oauth_notification_from_row(row, $ids::read_text(row, "user_id")?))
                    .collect()
            }

            async fn mark_read(&self, notification_id: &str, user_id: Uuid) -> AppResult<bool> {
                let result = sqlx::query(MARK_OAUTH_NOTIFICATION_READ_SQL)
                    .bind(notification_id)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to mark OAuth notification as read: {e}"
                        ))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn mark_all_read(&self, user_id: Uuid) -> AppResult<u64> {
                let result = sqlx::query(MARK_ALL_OAUTH_NOTIFICATIONS_READ_SQL)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to mark all OAuth notifications as read: {e}"
                        ))
                    })?;

                Ok(result.rows_affected())
            }

            async fn get_all(
                &self,
                user_id: Uuid,
                limit: Option<i64>,
            ) -> AppResult<Vec<OAuthNotification>> {
                let rows = sqlx::query(&all_oauth_notifications_sql(limit)?)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query all OAuth notifications: {e}"))
                    })?;

                rows.iter()
                    .map(|row| oauth_notification_from_row(row, $ids::read_text(row, "user_id")?))
                    .collect()
            }
        }
    };
}
pub(crate) use impl_notification_repository;
