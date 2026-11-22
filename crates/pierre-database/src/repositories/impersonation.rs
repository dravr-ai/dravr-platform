// ABOUTME: Repository trait, statements and shared body for admin impersonation session audit records
// ABOUTME: Who acted as whom and when; written once, emitted for each backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Display;

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::permissions::impersonation::{BoundCredential, ImpersonationSession};
use uuid::Uuid;

/// Impersonation session management repository.
///
/// A permission surface: only an admin reaches the handlers that
/// write here, and every read is keyed by the session id or by the two
/// user ids the row names, so a caller can only see sessions they are a
/// party to unless they hold the admin role that lists them all.
#[async_trait]
pub trait ImpersonationRepository: Send + Sync {
    /// Create a new impersonation session for audit trail
    async fn create_session(&self, session: &ImpersonationSession) -> AppResult<()>;
    /// Get impersonation session by ID
    async fn get_session(&self, session_id: &str) -> AppResult<Option<ImpersonationSession>>;
    /// Get active impersonation session where user is impersonator or target
    async fn get_active_session(&self, user_id: Uuid) -> AppResult<Option<ImpersonationSession>>;
    /// The newest open session `impersonator_id` started with `credential`
    /// ([`ImpersonationSession::bound_credential`]); never a web session.
    async fn get_active_bound_session(
        &self,
        impersonator_id: Uuid,
        credential: &BoundCredential,
    ) -> AppResult<Option<ImpersonationSession>>;
    /// Whether the OAuth grant `grant_id` (a refresh-token family) that
    /// `user_id` holds still has a token unexpired at `now`, revoked or not.
    ///
    /// Revoked is deliberately not read: a rotation consumes the presented
    /// token before it stores the successor, so a family between the two
    /// reads as wholly revoked. A revocation ends the sessions bound to it
    /// where it is written (`support_binding`); this answers what no write
    /// marks, a grant that ran out or was deleted.
    async fn grant_unexpired(
        &self,
        user_id: Uuid,
        grant_id: &str,
        now: Timestamp,
    ) -> AppResult<bool>;
    /// Whether the OAuth grant `grant_id` that `user_id` holds has a token
    /// neither revoked nor expired at `now`: the grant may start a support
    /// session. Strict where [`Self::grant_unexpired`] is not, because a
    /// start refused mid-rotation is retried and ends nothing.
    async fn grant_live(&self, user_id: Uuid, grant_id: &str, now: Timestamp) -> AppResult<bool>;
    /// End an impersonation session
    async fn end_session(&self, session_id: &str) -> AppResult<()>;
    /// End all active impersonation sessions for an impersonator
    async fn end_all_sessions(&self, impersonator_id: Uuid) -> AppResult<u64>;
    /// One page of every session, newest first by `(started_at, id)`: the rows
    /// strictly after `before` in that order, or the newest when it is `None`.
    /// A keyset, so a session sharing the page boundary's start is never skipped.
    async fn list_sessions_page(
        &self,
        before: Option<&SessionCursor>,
        limit: u32,
    ) -> AppResult<Vec<ImpersonationSession>>;
}

/// Where a page of [`ImpersonationRepository::list_sessions_page`] resumes:
/// the last row of the previous page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCursor {
    /// That row's `started_at`.
    pub started_at: Timestamp,
    /// That row's id, which orders the sessions sharing that start.
    pub id: String,
}

/// The ten columns every read of `impersonation_sessions` returns, in the
/// order the macro's `session_from_row` reads them. One list, so a column
/// added to [`ImpersonationSession`] reaches every read at once.
macro_rules! session_columns {
    () => {
        "id, impersonator_id, target_user_id, reason, started_at, ended_at, is_active, created_at, \
         grant_id, api_key_id"
    };
}
pub(crate) use session_columns;

/// Record a session.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends and cannot drift between
/// them. The two user ids bind through the backend's uuid codec (see
/// [`super::uuid_columns`]); every other bind is a plain `&str`,
/// `Option<&str>`, `bool` or `Timestamp` both drivers encode alike —
/// `is_active` is `BOOLEAN` on Postgres and a 0/1 `INTEGER` on `SQLite`,
/// and a `bool` binds and decodes as both.
pub(crate) const CREATE_SESSION_SQL: &str = concat!(
    "INSERT INTO impersonation_sessions (",
    session_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
);

/// One session by its id.
pub(crate) const GET_SESSION_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM impersonation_sessions WHERE id = $1"
);

/// The newest open session a user is a party to, as admin or as target.
/// `TRUE` is the spelling both engines accept for the boolean column.
pub(crate) const GET_ACTIVE_SESSION_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM impersonation_sessions \
     WHERE (impersonator_id = $1 OR target_user_id = $1) AND is_active = TRUE \
     ORDER BY started_at DESC LIMIT 1"
);

/// The newest open session an admin started with one OAuth grant.
pub(crate) const GET_ACTIVE_GRANT_SESSION_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM impersonation_sessions \
     WHERE impersonator_id = $1 AND grant_id = $2 AND is_active = TRUE \
     ORDER BY started_at DESC LIMIT 1"
);

/// The newest open session an admin started with one of their API keys.
pub(crate) const GET_ACTIVE_API_KEY_SESSION_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM impersonation_sessions \
     WHERE impersonator_id = $1 AND api_key_id = $2 AND is_active = TRUE \
     ORDER BY started_at DESC LIMIT 1"
);

/// One token of a refresh family the user holds, unrevoked and unexpired at
/// `$3`.
///
/// SECURITY: no `tenant_id` filter, by design. The family id is a server-minted
/// UUID the caller holds only in its own signed token's `grant_id` claim, and
/// the read is narrowed to that token's user, as the family-keyed statements
/// in `tokens.rs` are.
pub(crate) const GRANT_LIVE_SQL: &str = "SELECT family_id FROM oauth2_refresh_tokens \
     WHERE family_id = $1 AND user_id = $2 AND revoked = FALSE AND expires_at > $3 \
     LIMIT 1";

/// One token of a refresh family the user holds, unexpired at `$3`. Every
/// `oauth2_refresh_tokens` id column is `TEXT` on both backends. Unscoped by
/// tenant for the reason [`GRANT_LIVE_SQL`] gives.
pub(crate) const GRANT_UNEXPIRED_SQL: &str = "SELECT family_id FROM oauth2_refresh_tokens \
     WHERE family_id = $1 AND user_id = $2 AND expires_at > $3 \
     LIMIT 1";

/// Close one session.
pub(crate) const END_SESSION_SQL: &str = "UPDATE impersonation_sessions \
     SET is_active = FALSE, ended_at = $1 \
     WHERE id = $2";

/// Close every open session an admin holds.
pub(crate) const END_ALL_SESSIONS_SQL: &str = "UPDATE impersonation_sessions \
     SET is_active = FALSE, ended_at = $1 \
     WHERE impersonator_id = $2 AND is_active = TRUE";

/// Close every open session on one account: what a write that gives the
/// account the admin role runs, since an admin is never impersonated.
pub(crate) const END_SESSIONS_OF_TARGET_SQL: &str = "UPDATE impersonation_sessions \
     SET is_active = FALSE, ended_at = $1 \
     WHERE target_user_id = $2 AND is_active = TRUE";

/// A page of every session, newest first by `(started_at, id)`. `$1` NULL
/// reads from the newest; otherwise the rows strictly after `($1, $2)`.
pub(crate) const LIST_SESSIONS_PAGE_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM impersonation_sessions \
     WHERE ($1 IS NULL OR started_at < $1 OR (started_at = $1 AND id < $2)) \
     ORDER BY started_at DESC, id DESC LIMIT $3"
);

/// The error for a column of this table that would not decode.
pub(crate) fn session_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to get column '{name}': {e}"))
}

/// Emit the whole [`ImpersonationRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, its driver's row type and its uuid codec, and sqlx resolves
/// the driver from `self.pool()` per expansion.
///
/// `$row` is the driver's row type and `$ids` the codec in
/// [`super::uuid_columns`] for how that backend's `impersonator_id` and
/// `target_user_id` columns bind and read; the row parser is emitted inside
/// the macro because those two reads are the one thing in it that differs
/// per driver.
macro_rules! impl_impersonation_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// Decode one session row via `try_get` only, so a corrupt row
        /// surfaces as a recoverable error rather than a panic.
        fn session_from_row(row: &$row) -> AppResult<ImpersonationSession> {
            Ok(ImpersonationSession {
                id: row
                    .try_get("id")
                    .map_err(|e| session_column_error("id", e))?,
                impersonator_id: $ids::read(row, "impersonator_id")?,
                target_user_id: $ids::read(row, "target_user_id")?,
                reason: row
                    .try_get("reason")
                    .map_err(|e| session_column_error("reason", e))?,
                started_at: row
                    .try_get("started_at")
                    .map_err(|e| session_column_error("started_at", e))?,
                ended_at: row
                    .try_get("ended_at")
                    .map_err(|e| session_column_error("ended_at", e))?,
                is_active: row
                    .try_get("is_active")
                    .map_err(|e| session_column_error("is_active", e))?,
                created_at: row
                    .try_get("created_at")
                    .map_err(|e| session_column_error("created_at", e))?,
                grant_id: row
                    .try_get("grant_id")
                    .map_err(|e| session_column_error("grant_id", e))?,
                api_key_id: row
                    .try_get("api_key_id")
                    .map_err(|e| session_column_error("api_key_id", e))?,
            })
        }

        #[async_trait::async_trait]
        impl ImpersonationRepository for $ty {
            async fn create_session(&self, session: &ImpersonationSession) -> AppResult<()> {
                sqlx::query(CREATE_SESSION_SQL)
                    .bind(&session.id)
                    .bind($ids::bind(session.impersonator_id))
                    .bind($ids::bind(session.target_user_id))
                    .bind(session.reason.as_deref())
                    .bind(session.started_at)
                    .bind(session.ended_at)
                    .bind(session.is_active)
                    .bind(session.created_at)
                    .bind(session.grant_id.as_deref())
                    .bind(session.api_key_id.as_deref())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create impersonation session: {e}"))
                    })?;

                Ok(())
            }

            async fn get_session(
                &self,
                session_id: &str,
            ) -> AppResult<Option<ImpersonationSession>> {
                let row = sqlx::query(GET_SESSION_SQL)
                    .bind(session_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get impersonation session: {e}"))
                    })?;

                row.as_ref().map(session_from_row).transpose()
            }

            async fn get_active_session(
                &self,
                user_id: Uuid,
            ) -> AppResult<Option<ImpersonationSession>> {
                let row = sqlx::query(GET_ACTIVE_SESSION_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to get active impersonation session: {e}"
                        ))
                    })?;

                row.as_ref().map(session_from_row).transpose()
            }

            async fn get_active_bound_session(
                &self,
                impersonator_id: Uuid,
                credential: &BoundCredential,
            ) -> AppResult<Option<ImpersonationSession>> {
                let (sql, id) = match credential {
                    BoundCredential::Grant(id) => (GET_ACTIVE_GRANT_SESSION_SQL, id),
                    BoundCredential::ApiKey(id) => (GET_ACTIVE_API_KEY_SESSION_SQL, id),
                };
                let row = sqlx::query(sql)
                    .bind($ids::bind(impersonator_id))
                    .bind(id.as_str())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to get the active bound impersonation session: {e}"
                        ))
                    })?;

                row.as_ref().map(session_from_row).transpose()
            }

            async fn grant_unexpired(
                &self,
                user_id: Uuid,
                grant_id: &str,
                now: Timestamp,
            ) -> AppResult<bool> {
                let row = sqlx::query(GRANT_UNEXPIRED_SQL)
                    .bind(grant_id)
                    .bind(user_id.to_string())
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to read the grant a support session is bound to: {e}"
                        ))
                    })?;

                Ok(row.is_some())
            }

            async fn grant_live(
                &self,
                user_id: Uuid,
                grant_id: &str,
                now: Timestamp,
            ) -> AppResult<bool> {
                let row = sqlx::query(GRANT_LIVE_SQL)
                    .bind(grant_id)
                    .bind(user_id.to_string())
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to read the grant a support session would bind to: {e}"
                        ))
                    })?;

                Ok(row.is_some())
            }

            async fn end_session(&self, session_id: &str) -> AppResult<()> {
                sqlx::query(END_SESSION_SQL)
                    .bind(Timestamp::now())
                    .bind(session_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to end impersonation session: {e}"))
                    })?;

                Ok(())
            }

            async fn end_all_sessions(&self, impersonator_id: Uuid) -> AppResult<u64> {
                let result = sqlx::query(END_ALL_SESSIONS_SQL)
                    .bind(Timestamp::now())
                    .bind($ids::bind(impersonator_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to end impersonation sessions: {e}"))
                    })?;

                Ok(result.rows_affected())
            }

            async fn list_sessions_page(
                &self,
                before: Option<&SessionCursor>,
                limit: u32,
            ) -> AppResult<Vec<ImpersonationSession>> {
                let rows = sqlx::query(LIST_SESSIONS_PAGE_SQL)
                    .bind(before.map(|cursor| cursor.started_at))
                    .bind(before.map(|cursor| cursor.id.as_str()))
                    .bind(i64::from(limit))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list impersonation sessions: {e}"))
                    })?;

                rows.iter().map(session_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_impersonation_repository;
