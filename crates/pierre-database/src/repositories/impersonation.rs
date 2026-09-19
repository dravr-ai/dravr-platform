// ABOUTME: Repository trait, statements and shared body for super-admin impersonation session audit records
// ABOUTME: Who acted as whom and when; written once, emitted for each backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Display;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::permissions::impersonation::ImpersonationSession;
use uuid::Uuid;

/// Impersonation session management repository.
///
/// A permission surface: only a super admin reaches the handlers that
/// write here, and every read is keyed by the session id or by the two
/// user ids the row names, so a caller can only see sessions they are a
/// party to unless they hold the operator role that lists them all.
#[async_trait]
pub trait ImpersonationRepository: Send + Sync {
    /// Create a new impersonation session for audit trail
    async fn create_session(&self, session: &ImpersonationSession) -> AppResult<()>;
    /// Get impersonation session by ID
    async fn get_session(&self, session_id: &str) -> AppResult<Option<ImpersonationSession>>;
    /// Get active impersonation session where user is impersonator or target
    async fn get_active_session(&self, user_id: Uuid) -> AppResult<Option<ImpersonationSession>>;
    /// End an impersonation session
    async fn end_session(&self, session_id: &str) -> AppResult<()>;
    /// End all active impersonation sessions for an impersonator
    async fn end_all_sessions(&self, impersonator_id: Uuid) -> AppResult<u64>;
    /// List impersonation sessions with optional filters
    async fn list_sessions(
        &self,
        impersonator_id: Option<Uuid>,
        target_user_id: Option<Uuid>,
        active_only: bool,
        limit: u32,
    ) -> AppResult<Vec<ImpersonationSession>>;
}

/// The eight columns every read of `impersonation_sessions` returns, in the
/// order the macro's `session_from_row` reads them. One list, so a column
/// added to [`ImpersonationSession`] reaches every read at once.
macro_rules! session_columns {
    () => {
        "id, impersonator_id, target_user_id, reason, started_at, ended_at, is_active, created_at"
    };
}

/// Record a session.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends and cannot drift between
/// them. The two user ids bind through the backend's uuid codec (see
/// [`super::uuid_columns`]); every other bind is a plain `&str`,
/// `Option<&str>`, `bool` or `DateTime<Utc>` both drivers encode alike —
/// `is_active` is `BOOLEAN` on Postgres and a 0/1 `INTEGER` on `SQLite`,
/// and a `bool` binds and decodes as both.
pub(crate) const CREATE_SESSION_SQL: &str = concat!(
    "INSERT INTO impersonation_sessions (",
    session_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
);

/// One session by its id.
pub(crate) const GET_SESSION_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM impersonation_sessions WHERE id = $1"
);

/// The newest open session a user is a party to, as operator or as target.
/// `TRUE` is the spelling both engines accept for the boolean column.
pub(crate) const GET_ACTIVE_SESSION_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM impersonation_sessions \
     WHERE (impersonator_id = $1 OR target_user_id = $1) AND is_active = TRUE \
     ORDER BY started_at DESC LIMIT 1"
);

/// Close one session.
pub(crate) const END_SESSION_SQL: &str = "UPDATE impersonation_sessions \
     SET is_active = FALSE, ended_at = $1 \
     WHERE id = $2";

/// Close every open session an operator holds.
pub(crate) const END_ALL_SESSIONS_SQL: &str = "UPDATE impersonation_sessions \
     SET is_active = FALSE, ended_at = $1 \
     WHERE impersonator_id = $2 AND is_active = TRUE";

/// The audit list, newest first, with each filter optional.
///
/// One statement rather than a composed one: a NULL operator or target
/// filter matches every row, and `$3` is the `active_only` flag, so the
/// eight filter combinations the callers can ask for are one query text
/// with four binds.
pub(crate) const LIST_SESSIONS_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM impersonation_sessions \
     WHERE ($1 IS NULL OR impersonator_id = $1) \
       AND ($2 IS NULL OR target_user_id = $2) \
       AND ($3 = FALSE OR is_active = TRUE) \
     ORDER BY started_at DESC LIMIT $4"
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

            async fn end_session(&self, session_id: &str) -> AppResult<()> {
                sqlx::query(END_SESSION_SQL)
                    .bind(Utc::now())
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
                    .bind(Utc::now())
                    .bind($ids::bind(impersonator_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to end impersonation sessions: {e}"))
                    })?;

                Ok(result.rows_affected())
            }

            async fn list_sessions(
                &self,
                impersonator_id: Option<Uuid>,
                target_user_id: Option<Uuid>,
                active_only: bool,
                limit: u32,
            ) -> AppResult<Vec<ImpersonationSession>> {
                let rows = sqlx::query(LIST_SESSIONS_SQL)
                    .bind($ids::bind_opt(impersonator_id))
                    .bind($ids::bind_opt(target_user_id))
                    .bind(active_only)
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
