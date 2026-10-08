// ABOUTME: Repository trait, shared statements and body for coach-access requests — open one, read it, decide it
// ABOUTME: One SQL text per operation; each backend shell supplies how it binds a user uuid and reads one back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach-access requests (carnet#738).
//!
//! A coach whose onboarding group came back coachless asks for coach access in
//! one tap; a super-admin grants or declines it. The table is scoped by
//! `user_id`: coach access (`manages_roster`) belongs to the account, not to a
//! tenant, and the admin queue is the global operator's. `group_tenant_id`
//! only locates the group a grant attaches the coach to.
//!
//! At most one request per user is pending, held by a partial unique index, so
//! [`CoachAccessRequestRepository::open`] is idempotent: a second tap affects
//! no row and the caller reads back the request already waiting.
//!
//! `user_id` and `decided_by` are `uuid` on Postgres and `TEXT` on `SQLite`;
//! they bind and read through the shell's codec (`$ids`). `id` and `group_id`
//! are `TEXT` on both. `$n` placeholders throughout: sqlx accepts them on
//! `SQLite` as well as Postgres.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CoachAccessRequest, CoachAccessStatus};
use uuid::Uuid;

/// Coach-access requests and their decisions.
#[async_trait]
pub trait CoachAccessRequestRepository: Send + Sync {
    /// Open a pending request. `false` when the user already has one pending,
    /// which is kept.
    async fn open(&self, request: &CoachAccessRequest) -> AppResult<bool>;
    /// The request with this id, if any.
    async fn get(&self, id: Uuid) -> AppResult<Option<CoachAccessRequest>>;
    /// The user's most recent request, whatever its status.
    async fn latest_for_user(&self, user_id: Uuid) -> AppResult<Option<CoachAccessRequest>>;
    /// Every request in `status`, oldest first.
    async fn list_by_status(&self, status: CoachAccessStatus)
        -> AppResult<Vec<CoachAccessRequest>>;
    /// Record a super-admin's decision on a pending request. `false` when the
    /// request is not pending (already decided, or unknown), which is left
    /// as it is.
    async fn decide(
        &self,
        id: Uuid,
        status: CoachAccessStatus,
        decided_by: Option<Uuid>,
        decided_at: DateTime<Utc>,
    ) -> AppResult<bool>;
}

/// Open a request; a second pending one for the same user hits the partial
/// unique index and affects no row.
pub(crate) const OPEN_REQUEST_SQL: &str = r"
            INSERT INTO coach_access_requests
                (id, user_id, group_id, group_tenant_id, status, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT DO NOTHING
            ";

/// The columns every read decodes.
macro_rules! coach_access_request_columns {
    () => {
        "id, user_id, group_id, group_tenant_id, status, created_at, decided_at, decided_by"
    };
}

/// One request by id.
pub(crate) const GET_REQUEST_SQL: &str = concat!(
    "SELECT ",
    coach_access_request_columns!(),
    " FROM coach_access_requests WHERE id = $1"
);

/// The user's newest request.
pub(crate) const LATEST_FOR_USER_SQL: &str = concat!(
    "SELECT ",
    coach_access_request_columns!(),
    " FROM coach_access_requests WHERE user_id = $1 ORDER BY created_at DESC, id DESC LIMIT 1"
);

/// Every request in one status, oldest first.
pub(crate) const LIST_BY_STATUS_SQL: &str = concat!(
    "SELECT ",
    coach_access_request_columns!(),
    " FROM coach_access_requests WHERE status = $1 ORDER BY created_at ASC, id ASC"
);

/// Decide a request that is still pending; matches nothing otherwise.
pub(crate) const DECIDE_REQUEST_SQL: &str = r"
            UPDATE coach_access_requests
            SET status = $2, decided_at = $3, decided_by = $4
            WHERE id = $1 AND status = 'pending'
            ";

/// Parse a text id column.
fn parse_id(col: &str, raw: &str) -> AppResult<Uuid> {
    Uuid::parse_str(raw).map_err(|e| AppError::database(format!("Invalid {col} '{raw}': {e}")))
}

/// Decode the backend-neutral columns of a request row; the two user uuids
/// arrive already read through the backend's codec. `try_get` throughout,
/// never `Row::get`, so a corrupt row surfaces as a recoverable error.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// an id that is not a uuid, or a status other than the three stored.
pub(crate) fn request_from_row<R>(
    row: &R,
    user_id: Uuid,
    decided_by: Option<Uuid>,
) -> AppResult<CoachAccessRequest>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column =
        |col: &str, e: sqlx::Error| AppError::database(format!("coach_access_requests {col}: {e}"));
    let id: String = row.try_get("id").map_err(|e| column("id", e))?;
    let group_id: Option<String> = row.try_get("group_id").map_err(|e| column("group_id", e))?;
    let status: String = row.try_get("status").map_err(|e| column("status", e))?;
    Ok(CoachAccessRequest {
        id: parse_id("id", &id)?,
        user_id,
        group_id: group_id
            .as_deref()
            .map(|raw| parse_id("group_id", raw))
            .transpose()?,
        group_tenant_id: row
            .try_get("group_tenant_id")
            .map_err(|e| column("group_tenant_id", e))?,
        status: CoachAccessStatus::from_stored(&status)
            .ok_or_else(|| AppError::database(format!("Unknown coach access status: {status}")))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
        decided_at: row
            .try_get("decided_at")
            .map_err(|e| column("decided_at", e))?,
        decided_by,
    })
}

/// Emit the whole [`CoachAccessRequestRepository`] implementation for one
/// backend type. `$row` is the driver's row type and `$ids` the backend's
/// codec in [`super::uuid_columns`]. The body names its consts and helpers
/// unqualified, so the invoking shell must `use` every one of them.
macro_rules! impl_coach_access_request_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        impl $ty {
            /// Decode a request row through the backend's uuid codec.
            fn coach_access_row(row: &$row) -> AppResult<CoachAccessRequest> {
                request_from_row(
                    row,
                    $ids::read(row, "user_id")?,
                    $ids::read_opt(row, "decided_by")?,
                )
            }
        }

        #[async_trait::async_trait]
        impl CoachAccessRequestRepository for $ty {
            async fn open(&self, request: &CoachAccessRequest) -> AppResult<bool> {
                let result = sqlx::query(OPEN_REQUEST_SQL)
                    .bind(request.id.to_string())
                    .bind($ids::bind(request.user_id))
                    .bind(request.group_id.map(|id| id.to_string()))
                    .bind(request.group_tenant_id.as_deref())
                    .bind(request.status.as_str())
                    .bind(request.created_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to open coach access request: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn get(&self, id: Uuid) -> AppResult<Option<CoachAccessRequest>> {
                let row = sqlx::query(GET_REQUEST_SQL)
                    .bind(id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read coach access request: {e}"))
                    })?;
                row.map(|row| Self::coach_access_row(&row)).transpose()
            }

            async fn latest_for_user(
                &self,
                user_id: Uuid,
            ) -> AppResult<Option<CoachAccessRequest>> {
                let row = sqlx::query(LATEST_FOR_USER_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read coach access request: {e}"))
                    })?;
                row.map(|row| Self::coach_access_row(&row)).transpose()
            }

            async fn list_by_status(
                &self,
                status: CoachAccessStatus,
            ) -> AppResult<Vec<CoachAccessRequest>> {
                let rows = sqlx::query(LIST_BY_STATUS_SQL)
                    .bind(status.as_str())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list coach access requests: {e}"))
                    })?;
                rows.iter().map(Self::coach_access_row).collect()
            }

            async fn decide(
                &self,
                id: Uuid,
                status: CoachAccessStatus,
                decided_by: Option<Uuid>,
                decided_at: DateTime<Utc>,
            ) -> AppResult<bool> {
                let result = sqlx::query(DECIDE_REQUEST_SQL)
                    .bind(id.to_string())
                    .bind(status.as_str())
                    .bind(decided_at)
                    .bind($ids::bind_opt(decided_by))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to decide coach access request: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }
        }
    };
}
pub(crate) use impl_coach_access_request_repository;
