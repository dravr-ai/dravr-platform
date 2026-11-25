// ABOUTME: Repository trait, statements and shared body for a consent grant's receipts, the oauth_grant_events outbox
// ABOUTME: The record_log worker baselines, logs and signs them; the athlete reads their own; emitted per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A consent grant's receipts (carnet#1544, design row 34).
//!
//! The rows are written by the grant writes themselves, in their
//! transaction (`tokens/grant_writes.rs`); this repository reads them and
//! drains them as an outbox. The row is the outbox: `logged_at` is set once
//! the receipt's SHA-256 is appended to the athlete's barrage `record_log`,
//! `jws` once the checkpoint signer has signed the payload.
//!
//! The outbox lists and the baseline select across tenants: they are system
//! sweeps of the `record_log` worker. Every write they lead to is keyed by
//! the `(tenant_id, user_id, id)` of the row read. The athlete's reads filter
//! `tenant_id` and `user_id`.

use std::fmt::Display;

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{GrantActor, GrantEvent, OAuthGrantEvent};
use sqlx::Row;

/// A consent grant's receipts.
#[async_trait]
pub trait OAuthGrantEventRepository: Send + Sync {
    /// Receipts whose `record_log` entry is not appended yet, oldest first,
    /// at most `limit`, in every tenant.
    async fn list_unlogged(&self, limit: u32) -> AppResult<Vec<OAuthGrantEvent>>;

    /// Record that receipt `id` of `user_id` in `tenant_id` was appended to
    /// the `record_log` at `at`. Returns whether a row was marked.
    async fn mark_logged(
        &self,
        tenant_id: &str,
        user_id: &str,
        id: &str,
        at: Timestamp,
    ) -> AppResult<bool>;

    /// Receipts not signed yet, oldest first, at most `limit`, in every tenant.
    async fn list_unsigned(&self, limit: u32) -> AppResult<Vec<OAuthGrantEvent>>;

    /// Count one failed attempt to log or sign receipt `id`, owned by
    /// `user_id` in `tenant_id`, at `at`: the attempts it has now made, at
    /// [`MAX_GRANT_RECEIPT_ATTEMPTS`] poisoned and skipped by both scans
    /// from then on; `None` when no unpoisoned receipt matched.
    async fn record_failure(
        &self,
        tenant_id: &str,
        user_id: &str,
        id: &str,
        at: Timestamp,
    ) -> AppResult<Option<u32>>;

    /// Store the compact JWS over receipt `id` of `user_id` in `tenant_id`;
    /// a receipt already signed keeps its JWS. Returns whether one was set.
    async fn set_jws(&self, tenant_id: &str, user_id: &str, id: &str, jws: &str)
        -> AppResult<bool>;

    /// The receipts of `user_id` in `tenant_id`, newest first, at most
    /// `limit` after skipping `offset`: of grant `client_grant_id` only when
    /// one is named. A grant since deleted with its client keeps its
    /// receipts here.
    async fn list_for_user(
        &self,
        tenant_id: &str,
        user_id: &str,
        client_grant_id: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<OAuthGrantEvent>>;

    /// Give up to `limit` un-revoked grants (live, or past their expiry)
    /// with no `issue` receipt one (by the AS, flagged a baseline, at their
    /// `granted_at`): grants given before receipts were, an expired one
    /// whose `expire` the sweep wrote first included. Returns the receipts
    /// written; 0 once none is left.
    async fn baseline_live_grants(&self, limit: u32) -> AppResult<u64>;
}

/// How many times the outbox tries to log or sign one receipt before it
/// poisons it and skips it from then on.
pub const MAX_GRANT_RECEIPT_ATTEMPTS: u32 = 5;

/// Unlogged receipts not poisoned, the least-tried first and then oldest
/// first (a grant's issue before its ending at the same time), capped at
/// `$1`; every tenant. A failing receipt sorts behind every one not yet
/// tried, so a page of failures never holds the rest back.
pub(crate) const LIST_UNLOGGED_GRANT_EVENTS_SQL: &str = r"
            SELECT id, tenant_id, user_id, client_grant_id, event, actor, at,
                   payload, payload_sha256, jws, logged_at
            FROM oauth_grant_events
            WHERE logged_at IS NULL AND poisoned_at IS NULL
            ORDER BY attempts, at, CASE event WHEN 'issue' THEN 0 ELSE 1 END, id
            LIMIT $1
            ";

/// Mark one receipt logged.
pub(crate) const MARK_GRANT_EVENT_LOGGED_SQL: &str = r"
            UPDATE oauth_grant_events SET logged_at = $4
            WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND logged_at IS NULL
            ";

/// Unsigned receipts not poisoned, the least-tried first and then oldest
/// first, capped at `$1`; every tenant.
pub(crate) const LIST_UNSIGNED_GRANT_EVENTS_SQL: &str = r"
            SELECT id, tenant_id, user_id, client_grant_id, event, actor, at,
                   payload, payload_sha256, jws, logged_at
            FROM oauth_grant_events
            WHERE jws IS NULL AND poisoned_at IS NULL
            ORDER BY attempts, at, CASE event WHEN 'issue' THEN 0 ELSE 1 END, id
            LIMIT $1
            ";

/// Count one failed outbox attempt on a receipt not yet poisoned, poisoning
/// it at `$5` (its last allowed attempt, `$4`): the attempts it has made.
pub(crate) const RECORD_GRANT_EVENT_FAILURE_SQL: &str = r"
            UPDATE oauth_grant_events
            SET attempts = attempts + 1,
                poisoned_at = CASE WHEN attempts + 1 >= $4 THEN $5 ELSE NULL END
            WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND poisoned_at IS NULL
            RETURNING attempts
            ";

/// Store one receipt's JWS.
pub(crate) const SET_GRANT_EVENT_JWS_SQL: &str = r"
            UPDATE oauth_grant_events SET jws = $4
            WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND jws IS NULL
            ";

/// One athlete's receipts, newest first (a grant's ending before its issue
/// at the same time: `SQLite` stamps whole seconds).
pub(crate) const LIST_USER_GRANT_EVENTS_SQL: &str = r"
            SELECT id, tenant_id, user_id, client_grant_id, event, actor, at,
                   payload, payload_sha256, jws, logged_at
            FROM oauth_grant_events
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY at DESC, CASE event WHEN 'issue' THEN 0 ELSE 1 END DESC, id DESC
            LIMIT $3 OFFSET $4
            ";

/// One athlete's receipts of one grant, newest first.
pub(crate) const LIST_USER_GRANT_EVENTS_OF_GRANT_SQL: &str = r"
            SELECT id, tenant_id, user_id, client_grant_id, event, actor, at,
                   payload, payload_sha256, jws, logged_at
            FROM oauth_grant_events
            WHERE tenant_id = $1 AND user_id = $2 AND client_grant_id = $3
            ORDER BY at DESC, CASE event WHEN 'issue' THEN 0 ELSE 1 END DESC, id DESC
            LIMIT $4 OFFSET $5
            ";

/// The error for a column of this table that would not decode.
fn grant_event_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to read oauth_grant_events.{name}: {e}"))
}

/// Decode one `oauth_grant_events` row.
pub(crate) fn grant_event_from_row<R>(row: &R) -> AppResult<OAuthGrantEvent>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Timestamp: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<Timestamp>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let text = |name: &str| -> AppResult<String> {
        row.try_get(name)
            .map_err(|e| grant_event_column_error(name, e))
    };
    Ok(OAuthGrantEvent {
        id: text("id")?,
        tenant_id: text("tenant_id")?,
        user_id: text("user_id")?,
        client_grant_id: text("client_grant_id")?,
        event: GrantEvent::parse(&text("event")?)?,
        actor: GrantActor::parse(&text("actor")?)?,
        at: row
            .try_get("at")
            .map_err(|e| grant_event_column_error("at", e))?,
        payload: text("payload")?,
        payload_sha256: text("payload_sha256")?,
        jws: row
            .try_get("jws")
            .map_err(|e| grant_event_column_error("jws", e))?,
        logged_at: row
            .try_get("logged_at")
            .map_err(|e| grant_event_column_error("logged_at", e))?,
    })
}

/// Emit the whole [`OAuthGrantEventRepository`] implementation for one
/// backend type; sqlx resolves the driver from `self.pool()`. The baseline
/// writes through the grant writes' `baseline_grant_rows`, emitted for the
/// same type by `impl_client_grant_writes!`.
macro_rules! impl_grant_event_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl OAuthGrantEventRepository for $ty {
            async fn list_unlogged(&self, limit: u32) -> AppResult<Vec<OAuthGrantEvent>> {
                let rows = sqlx::query(LIST_UNLOGGED_GRANT_EVENTS_SQL)
                    .bind(i64::from(limit))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list unlogged grant receipts: {e}"))
                    })?;
                rows.iter().map(grant_event_from_row).collect()
            }

            async fn mark_logged(
                &self,
                tenant_id: &str,
                user_id: &str,
                id: &str,
                at: Timestamp,
            ) -> AppResult<bool> {
                let marked = sqlx::query(MARK_GRANT_EVENT_LOGGED_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(id)
                    .bind(at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to mark a grant receipt logged: {e}"))
                    })?;
                Ok(marked.rows_affected() > 0)
            }

            async fn list_unsigned(&self, limit: u32) -> AppResult<Vec<OAuthGrantEvent>> {
                let rows = sqlx::query(LIST_UNSIGNED_GRANT_EVENTS_SQL)
                    .bind(i64::from(limit))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list unsigned grant receipts: {e}"))
                    })?;
                rows.iter().map(grant_event_from_row).collect()
            }

            async fn record_failure(
                &self,
                tenant_id: &str,
                user_id: &str,
                id: &str,
                at: Timestamp,
            ) -> AppResult<Option<u32>> {
                let attempts = sqlx::query_scalar::<_, i32>(RECORD_GRANT_EVENT_FAILURE_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(id)
                    .bind(i64::from(MAX_GRANT_RECEIPT_ATTEMPTS))
                    .bind(at)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to count a grant receipt's failed attempt: {e}"
                        ))
                    })?;
                Ok(attempts.map(|count| u32::try_from(count).unwrap_or(0)))
            }

            async fn set_jws(
                &self,
                tenant_id: &str,
                user_id: &str,
                id: &str,
                jws: &str,
            ) -> AppResult<bool> {
                let set = sqlx::query(SET_GRANT_EVENT_JWS_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(id)
                    .bind(jws)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store a grant receipt's JWS: {e}"))
                    })?;
                Ok(set.rows_affected() > 0)
            }

            async fn list_for_user(
                &self,
                tenant_id: &str,
                user_id: &str,
                client_grant_id: Option<&str>,
                limit: u32,
                offset: u32,
            ) -> AppResult<Vec<OAuthGrantEvent>> {
                let query = match client_grant_id {
                    Some(client_grant_id) => sqlx::query(LIST_USER_GRANT_EVENTS_OF_GRANT_SQL)
                        .bind(tenant_id)
                        .bind(user_id)
                        .bind(client_grant_id),
                    None => sqlx::query(LIST_USER_GRANT_EVENTS_SQL)
                        .bind(tenant_id)
                        .bind(user_id),
                };
                let rows = query
                    .bind(i64::from(limit))
                    .bind(i64::from(offset))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list grant receipts: {e}"))
                    })?;
                rows.iter().map(grant_event_from_row).collect()
            }

            async fn baseline_live_grants(&self, limit: u32) -> AppResult<u64> {
                self.baseline_grant_rows(limit).await
            }
        }
    };
}
pub(crate) use impl_grant_event_repository;
