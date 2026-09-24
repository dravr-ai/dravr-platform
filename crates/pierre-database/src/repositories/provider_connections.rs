// ABOUTME: Shared statements, row decoder and body for provider connections — one row per user, tenant and provider,
// ABOUTME: with the reauth lifecycle (needs_reauth, re-armed on reconnect, notified once) written once per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider connections, written once.
//!
//! `provider_connections` is the single source of truth for which providers
//! an athlete has connected. `user_id` is `TEXT` on both backends (the
//! orphan-reconcile migrations cast around that on Postgres), so a uuid binds
//! as its hyphenated text and is parsed on the way back on both drivers, and
//! no uuid codec is needed. Timestamps bind as [`DateTime<Utc>`] on both:
//! `TIMESTAMPTZ` on Postgres, RFC 3339 text on `SQLite`, which orders
//! correctly for the recency election because every value shares one offset
//! and width.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them.

use std::fmt::Display;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    ConnectionStatus, ConnectionType, ProviderAccountRole, ProviderConnection, ReauthMark,
};

use crate::column_decode::uuid_column;

/// Register or refresh a connection. A reconnect re-arms the row: `status`
/// back to `active`, the transition stamped, the last error and the
/// one-time notification marker cleared. The account role is cleared too: a
/// new login may be a different account, so its role is read again.
pub(crate) const REGISTER_CONNECTION_SQL: &str = r"
            INSERT INTO provider_connections (id, user_id, tenant_id, provider, connection_type, connected_at, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT(user_id, tenant_id, provider) DO UPDATE SET
                connection_type = EXCLUDED.connection_type,
                connected_at = EXCLUDED.connected_at,
                metadata = EXCLUDED.metadata,
                status = 'active',
                status_changed_at = EXCLUDED.connected_at,
                last_error = NULL,
                notified_at = NULL,
                account_role = NULL
            ";

/// Drop a connection.
pub(crate) const REMOVE_CONNECTION_SQL: &str =
    "DELETE FROM provider_connections WHERE user_id = $1 AND tenant_id = $2 AND provider = $3";

/// Drop a connection only when it is a delegated one, so ending a delegated
/// link can never delete a member's own connection to the same provider.
pub(crate) const REMOVE_DELEGATED_CONNECTION_SQL: &str = r"
            DELETE FROM provider_connections
             WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
               AND connection_type = 'delegated'
            ";

/// Record the kind of account a connection signed in with.
pub(crate) const SET_ACCOUNT_ROLE_SQL: &str = r"
            UPDATE provider_connections SET account_role = $1
             WHERE user_id = $2 AND tenant_id = $3 AND provider = $4
            ";

/// One read of the connection columns. `$filter` is the `WHERE` clause after
/// the `user_id = $1` every read carries; `$order` closes the statement.
macro_rules! connection_select_sql {
    ($filter:literal, $order:literal) => {
        concat!(
            "SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, status, metadata,
                    account_role
             FROM provider_connections
             WHERE user_id = $1",
            $filter,
            $order
        )
    };
}

/// A user's connections within one tenant, newest first.
pub(crate) const GET_FOR_USER_IN_TENANT_SQL: &str =
    connection_select_sql!(" AND tenant_id = $2", " ORDER BY connected_at DESC");

/// A user's connections across every tenant, newest first.
pub(crate) const GET_FOR_USER_SQL: &str = connection_select_sql!("", " ORDER BY connected_at DESC");

/// Whether the provider is connected in any tenant.
pub(crate) const IS_CONNECTED_SQL: &str =
    "SELECT COUNT(*) FROM provider_connections WHERE user_id = $1 AND provider = $2";

/// Touch-on-read; absence of the row is not an error.
pub(crate) const TOUCH_LAST_USED_SQL: &str = r"
            UPDATE provider_connections
               SET last_used_at = $1
             WHERE user_id = $2 AND tenant_id = $3 AND provider = $4
            ";

/// The most recently used usable connection within one tenant. The election:
/// health first, so a dead connection never shadows a healthy sibling; then
/// a coach account last, since it has no calendar of its own to serve; then
/// the freshest `last_used_at` with untouched rows last; then the freshest
/// `connected_at`.
pub(crate) const RESOLVE_MOST_RECENT_IN_TENANT_SQL: &str = connection_select_sql!(
    " AND tenant_id = $2",
    " ORDER BY CASE WHEN status = 'active' THEN 0 ELSE 1 END,
               CASE WHEN account_role = 'coach' THEN 1 ELSE 0 END,
               last_used_at DESC NULLS LAST, connected_at DESC LIMIT 1"
);

/// The most recently used usable connection across every tenant, elected as
/// [`RESOLVE_MOST_RECENT_IN_TENANT_SQL`] does.
pub(crate) const RESOLVE_MOST_RECENT_SQL: &str = connection_select_sql!(
    "",
    " ORDER BY CASE WHEN status = 'active' THEN 0 ELSE 1 END,
               CASE WHEN account_role = 'coach' THEN 1 ELSE 0 END,
               last_used_at DESC NULLS LAST, connected_at DESC LIMIT 1"
);

/// Flag a connection after a failure an attempt that began at `$6` observed.
/// Guarded so the transition timestamp reflects the first failure, not every
/// retry, and so a connection (re)connected or re-armed since `$6` stands: a
/// reconnect stamps `connected_at` and a re-arm `status_changed_at`, and the
/// failure is a verdict on the credential the attempt read, not on theirs. A
/// row that never changed status has no `status_changed_at`.
pub(crate) const MARK_NEEDS_REAUTH_SQL: &str = r"
            UPDATE provider_connections
               SET status = 'needs_reauth',
                   status_changed_at = $1,
                   last_error = $2
             WHERE user_id = $3 AND tenant_id = $4 AND provider = $5
               AND status != 'needs_reauth'
               AND connected_at <= $6
               AND (status_changed_at IS NULL OR status_changed_at <= $6)
            ";

/// The status of one connection, read after [`MARK_NEEDS_REAUTH_SQL`] changed
/// nothing to say why: no row, already flagged, or reconnected since.
pub(crate) const CONNECTION_STATUS_SQL: &str =
    "SELECT status FROM provider_connections WHERE user_id = $1 AND tenant_id = $2 AND provider = $3";

/// What [`MARK_NEEDS_REAUTH_SQL`] left the connection as: `flipped` when it
/// changed a row, else read from the status it has now. Unflipped, a status
/// that does not require re-authorizing can only be a connection the time
/// guard protected, since every other one would have flipped.
pub(crate) fn reauth_mark(flipped: bool, status_now: Option<&str>) -> ReauthMark {
    if flipped {
        return ReauthMark::Flagged;
    }
    match status_now.map(ConnectionStatus::from_str_value) {
        None => ReauthMark::NoConnection,
        Some(status) if status.requires_reauth() => ReauthMark::AlreadyFlagged,
        Some(_) => ReauthMark::ReconnectedSince,
    }
}

/// [`MARK_NEEDS_REAUTH_SQL`]'s flip, only while the connection's stored token
/// is still the row the refused refresh read: the same `id` (`$6`) and the same
/// `updated_at` (`$7`). A reconnect stores its token under a fresh `id`, and a
/// refresh that landed meanwhile rewrote the pair and `updated_at` under the
/// same `id`, so a refusal of a grant a reconnect replaced, or of a refresh
/// token another refresh already spent, changes nothing. `$7` is the value the
/// read decoded, bound back unchanged: `TIMESTAMPTZ` equality on Postgres, and
/// on `SQLite` the RFC 3339 text every writer binds, which a decode and an
/// encode return verbatim. The token's `user_id` is `uuid` on Postgres and
/// `TEXT` on `SQLite`, so it is cast down to the connection's `TEXT`; its `id`,
/// `tenant_id` and `provider` are text on both (`VARCHAR` on Postgres).
pub(crate) const MARK_NEEDS_REAUTH_IF_TOKEN_CURRENT_SQL: &str = r"
            UPDATE provider_connections
               SET status = 'needs_reauth',
                   status_changed_at = $1,
                   last_error = $2
             WHERE user_id = $3 AND tenant_id = $4 AND provider = $5
               AND status != 'needs_reauth'
               AND EXISTS (
                   SELECT 1 FROM user_oauth_tokens t
                   WHERE t.id = $6
                     AND t.updated_at = $7
                     AND CAST(t.user_id AS TEXT) = provider_connections.user_id
                     AND t.tenant_id = provider_connections.tenant_id
                     AND t.provider = provider_connections.provider)
            ";

/// Re-arm a connection after a successful (re)connect or refresh. Guarded so
/// a refresh of an already-healthy connection is a no-op.
pub(crate) const MARK_ACTIVE_SQL: &str = r"
            UPDATE provider_connections
               SET status = 'active',
                   status_changed_at = $1,
                   last_error = NULL,
                   notified_at = NULL
             WHERE user_id = $2 AND tenant_id = $3 AND provider = $4
               AND status != 'active'
            ";

/// Claim the one-time disconnect notification: only the first caller after
/// the transition affects a row.
pub(crate) const CLAIM_REAUTH_NOTIFICATION_SQL: &str = r"
            UPDATE provider_connections
               SET notified_at = $1
             WHERE user_id = $2 AND tenant_id = $3 AND provider = $4
               AND status = 'needs_reauth'
               AND notified_at IS NULL
            ";

fn column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to get provider_connections.{col}: {e}"))
}

/// Decode one `provider_connections` row. `try_get` throughout, never
/// `Row::get`, and no defaulted timestamp: a stored value that will not
/// decode is a database error on both drivers, never `Utc::now()`.
/// `connection_type` outside the model's vocabulary reads as `Manual`, as
/// both backends always did. `account_role` outside its vocabulary is an
/// error: the column's CHECK admits only the two roles.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// when `user_id` is not a uuid, or when `account_role` names no role.
pub(crate) fn connection_from_row<R>(row: &R) -> AppResult<ProviderConnection>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let user_id: String = row
        .try_get("user_id")
        .map_err(|e| column_error("user_id", e))?;
    let connection_type: String = row
        .try_get("connection_type")
        .map_err(|e| column_error("connection_type", e))?;
    let status: String = row
        .try_get("status")
        .map_err(|e| column_error("status", e))?;
    let account_role: Option<String> = row
        .try_get("account_role")
        .map_err(|e| column_error("account_role", e))?;
    let account_role = account_role
        .map(|role| {
            ProviderAccountRole::from_str_opt(&role).ok_or_else(|| {
                AppError::database(format!(
                    "provider_connections.account_role holds unknown value `{role}`"
                ))
            })
        })
        .transpose()?;
    Ok(ProviderConnection {
        id: row.try_get("id").map_err(|e| column_error("id", e))?,
        user_id: uuid_column("provider_connections.user_id", &user_id)?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column_error("tenant_id", e))?,
        provider: row
            .try_get("provider")
            .map_err(|e| column_error("provider", e))?,
        connection_type: ConnectionType::from_str_value(&connection_type)
            .unwrap_or(ConnectionType::Manual),
        connected_at: row
            .try_get("connected_at")
            .map_err(|e| column_error("connected_at", e))?,
        last_used_at: row
            .try_get("last_used_at")
            .map_err(|e| column_error("last_used_at", e))?,
        status: ConnectionStatus::from_str_value(&status),
        metadata: row
            .try_get("metadata")
            .map_err(|e| column_error("metadata", e))?,
        account_role,
    })
}

/// Emit the whole [`ProviderConnectionRepository`](super::ProviderConnectionRepository)
/// implementation for one backend type.
///
/// No per-engine argument: every column involved has the same shape on both
/// backends (see the module doc). The body is written once here; each
/// backend's shell invokes it with its own type, and sqlx resolves the driver
/// from `self.pool()` per expansion. The body names its consts, helpers and
/// types unqualified, so the invoking shell must `use` every one of them.
macro_rules! impl_provider_connection_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl ProviderConnectionRepository for $ty {
            async fn register_connection(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                connection_type: &ConnectionType,
                metadata: Option<&str>,
            ) -> AppResult<()> {
                sqlx::query(REGISTER_CONNECTION_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .bind(connection_type.as_str())
                    .bind(Utc::now())
                    .bind(metadata)
                    .execute(self.pool())
                    .await?;

                Ok(())
            }

            async fn remove_connection(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<()> {
                sqlx::query(REMOVE_CONNECTION_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await?;

                Ok(())
            }

            async fn remove_delegated_connection(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(REMOVE_DELEGATED_CONNECTION_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await?;

                Ok(result.rows_affected() > 0)
            }

            async fn set_account_role(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                role: ProviderAccountRole,
            ) -> AppResult<bool> {
                let result = sqlx::query(SET_ACCOUNT_ROLE_SQL)
                    .bind(role.as_str())
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await?;

                Ok(result.rows_affected() > 0)
            }

            async fn get_for_user(
                &self,
                user_id: Uuid,
                tenant_id: Option<TenantId>,
            ) -> AppResult<Vec<ProviderConnection>> {
                let rows = match tenant_id {
                    Some(tid) => {
                        sqlx::query(GET_FOR_USER_IN_TENANT_SQL)
                            .bind(user_id.to_string())
                            .bind(tid.to_string())
                            .fetch_all(self.pool())
                            .await?
                    }
                    None => {
                        sqlx::query(GET_FOR_USER_SQL)
                            .bind(user_id.to_string())
                            .fetch_all(self.pool())
                            .await?
                    }
                };

                rows.iter().map(connection_from_row).collect()
            }

            async fn is_connected(&self, user_id: Uuid, provider: &str) -> AppResult<bool> {
                let count: i64 = sqlx::query_scalar(IS_CONNECTED_SQL)
                    .bind(user_id.to_string())
                    .bind(provider)
                    .fetch_one(self.pool())
                    .await?;

                Ok(count > 0)
            }

            async fn touch_last_used(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<()> {
                sqlx::query(TOUCH_LAST_USED_SQL)
                    .bind(Utc::now())
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await?;

                Ok(())
            }

            async fn resolve_most_recent(
                &self,
                user_id: Uuid,
                tenant_id: Option<TenantId>,
            ) -> AppResult<Option<ProviderConnection>> {
                let row = match tenant_id {
                    Some(tid) => {
                        sqlx::query(RESOLVE_MOST_RECENT_IN_TENANT_SQL)
                            .bind(user_id.to_string())
                            .bind(tid.to_string())
                            .fetch_optional(self.pool())
                            .await?
                    }
                    None => {
                        sqlx::query(RESOLVE_MOST_RECENT_SQL)
                            .bind(user_id.to_string())
                            .fetch_optional(self.pool())
                            .await?
                    }
                };

                row.map(|row| connection_from_row(&row)).transpose()
            }

            async fn mark_needs_reauth(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                error_code: Option<&str>,
                attempt_started_at: DateTime<Utc>,
            ) -> AppResult<ReauthMark> {
                let flipped = sqlx::query(MARK_NEEDS_REAUTH_SQL)
                    .bind(Utc::now())
                    .bind(error_code)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .bind(attempt_started_at)
                    .execute(self.pool())
                    .await?
                    .rows_affected()
                    > 0;
                if flipped {
                    return Ok(reauth_mark(true, None));
                }
                let status_now: Option<String> = sqlx::query_scalar(CONNECTION_STATUS_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .fetch_optional(self.pool())
                    .await?;

                Ok(reauth_mark(false, status_now.as_deref()))
            }

            async fn mark_needs_reauth_if_token_current(
                &self,
                token: &UserOAuthToken,
                error_code: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(MARK_NEEDS_REAUTH_IF_TOKEN_CURRENT_SQL)
                    .bind(Utc::now())
                    .bind(error_code)
                    .bind(token.user_id.to_string())
                    .bind(&token.tenant_id)
                    .bind(&token.provider)
                    .bind(&token.id)
                    .bind(token.updated_at)
                    .execute(self.pool())
                    .await?;

                Ok(result.rows_affected() > 0)
            }

            async fn mark_active(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<()> {
                sqlx::query(MARK_ACTIVE_SQL)
                    .bind(Utc::now())
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await?;

                Ok(())
            }

            async fn claim_reauth_notification(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(CLAIM_REAUTH_NOTIFICATION_SQL)
                    .bind(Utc::now())
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await?;

                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_provider_connection_repository;
