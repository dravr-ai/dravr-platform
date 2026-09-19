// ABOUTME: Repository trait, shared statements and body for the agent-athlete roster junction both backends serve
// ABOUTME: One SQL text per operation; each backend shell supplies how it binds a uuid and reads one back as text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Roster assignments, written once.
//!
//! Active rows are `revoked_at IS NULL`; a revoke stamps the row and leaves
//! it for audit, and the partial unique index refuses a second active row
//! for the same `(agent, athlete, tenant)`.
//!
//! The two backends differ in one respect only: every id column is a `uuid`
//! on Postgres and `TEXT` on `SQLite`. A uuid is therefore bound through the
//! macro's `$bind_id` function (native on Postgres, hyphenated text on
//! `SQLite`) and read back through `$text`, the cast that turns the column
//! into text on Postgres and is empty on `SQLite`, so one row parser decodes
//! both. `tenant_id` binds as a `TenantId`, whose own sqlx encoding already
//! follows that split. Timestamps bind and decode as `DateTime<Utc>` on
//! both: RFC 3339 text on `SQLite`, byte-identical to the `to_rfc3339()` the
//! rows were written with, and `TIMESTAMPTZ` on Postgres.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CoachAthleteAssignment, TenantId};
use uuid::Uuid;

/// 1:N agent → athlete roster assignment repository.
///
/// Backed by the `coach_athlete_assignments` table. All queries are
/// tenant-scoped — the route layer is responsible for verifying both
/// the agent and the athlete belong to the same tenant before calling
/// `assign_athlete`. Active assignments have `revoked_at IS NULL`.
#[async_trait]
pub trait RosterRepository: Send + Sync {
    /// List the active assignments where `coach_user_id` is coaching
    /// other users in `tenant_id`. Ordered by `assigned_at DESC` so the
    /// most recent assignments surface first.
    async fn list_athletes_for_coach(
        &self,
        coach_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAthleteAssignment>>;

    /// List the active assignments where `athlete_user_id` is being
    /// coached by other users in `tenant_id`. The inverse of
    /// `list_athletes_for_coach`, used by the athlete-side roster view.
    async fn list_coaches_for_athlete(
        &self,
        athlete_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAthleteAssignment>>;

    /// Insert a new assignment row. Returns `Ok(None)` when an active
    /// assignment for the same `(agent, athlete, tenant)` already exists
    /// (the unique partial index guards against duplicates). Caller
    /// MUST verify both users belong to `tenant_id` before invoking.
    async fn assign_athlete(
        &self,
        assignment: &CoachAthleteAssignment,
    ) -> AppResult<Option<CoachAthleteAssignment>>;

    /// Mark the active assignment for `(agent, athlete, tenant)` as
    /// revoked. Returns `true` when a row was updated, `false` when no
    /// active assignment matched. The audit row stays.
    async fn revoke_assignment(
        &self,
        coach_user_id: Uuid,
        athlete_user_id: Uuid,
        tenant_id: TenantId,
        revoked_by: Option<Uuid>,
    ) -> AppResult<bool>;

    /// `true` when an active assignment exists for the given triple.
    /// Used by route handlers to gate athlete-data reads.
    async fn is_athlete_managed_by(
        &self,
        coach_user_id: Uuid,
        athlete_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
}

/// The columns every read decodes, in the order [`assignment_from_row`]
/// reads them, with `$text` appended to each uuid column so it arrives as
/// text on both backends.
macro_rules! assignment_columns {
    ($text:literal) => {
        concat!(
            "id",
            $text,
            " AS id, coach_user_id",
            $text,
            " AS coach_user_id, athlete_user_id",
            $text,
            " AS athlete_user_id, tenant_id",
            $text,
            " AS tenant_id, assigned_by",
            $text,
            " AS assigned_by, assigned_at, revoked_at, revoked_by",
            $text,
            " AS revoked_by"
        )
    };
}
pub(crate) use assignment_columns;

/// Active assignments where `$1` is coaching, newest first.
macro_rules! list_athletes_for_coach_sql {
    ($text:literal) => {
        concat!(
            "
            SELECT ",
            assignment_columns!($text),
            "
            FROM coach_athlete_assignments
            WHERE coach_user_id = $1
              AND tenant_id = $2
              AND revoked_at IS NULL
            ORDER BY assigned_at DESC
            "
        )
    };
}
pub(crate) use list_athletes_for_coach_sql;

/// Active assignments where `$1` is being coached, newest first.
macro_rules! list_coaches_for_athlete_sql {
    ($text:literal) => {
        concat!(
            "
            SELECT ",
            assignment_columns!($text),
            "
            FROM coach_athlete_assignments
            WHERE athlete_user_id = $1
              AND tenant_id = $2
              AND revoked_at IS NULL
            ORDER BY assigned_at DESC
            "
        )
    };
}
pub(crate) use list_coaches_for_athlete_sql;

/// Insert an active row; the partial unique index turns a second active
/// assignment for the same triple into zero rows affected.
pub(crate) const ASSIGN_ATHLETE_SQL: &str = r"
            INSERT INTO coach_athlete_assignments
                (id, coach_user_id, athlete_user_id, tenant_id,
                 assigned_by, assigned_at, revoked_at, revoked_by)
            VALUES ($1, $2, $3, $4, $5, $6, NULL, NULL)
            ON CONFLICT DO NOTHING
            ";

/// Stamp the active row for the triple as revoked.
pub(crate) const REVOKE_ASSIGNMENT_SQL: &str = r"
            UPDATE coach_athlete_assignments
            SET revoked_at = $1,
                revoked_by = $2
            WHERE coach_user_id = $3
              AND athlete_user_id = $4
              AND tenant_id = $5
              AND revoked_at IS NULL
            ";

/// How many active rows the triple has — zero or one under the index.
pub(crate) const COUNT_ACTIVE_ASSIGNMENTS_SQL: &str = r"
            SELECT COUNT(*) FROM coach_athlete_assignments
            WHERE coach_user_id = $1
              AND athlete_user_id = $2
              AND tenant_id = $3
              AND revoked_at IS NULL
            ";

/// Decode one assignment row. Every uuid column arrives as text (see
/// [`assignment_columns`]) and is parsed here; `try_get` throughout, never
/// `Row::get`, so a corrupt row surfaces as a recoverable error rather than
/// a panic.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded
/// or does not hold a uuid.
pub(crate) fn assignment_from_row<R>(row: &R) -> AppResult<CoachAthleteAssignment>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let uuid = |col: &str| -> AppResult<Uuid> {
        let text: String = row
            .try_get(col)
            .map_err(|e| AppError::database(format!("coach_athlete_assignments {col}: {e}")))?;
        Uuid::parse_str(&text)
            .map_err(|e| AppError::database(format!("Invalid UUID in column {col}: {e}")))
    };
    let optional_uuid = |col: &str| -> AppResult<Option<Uuid>> {
        let text: Option<String> = row
            .try_get(col)
            .map_err(|e| AppError::database(format!("coach_athlete_assignments {col}: {e}")))?;
        text.map(|t| {
            Uuid::parse_str(&t)
                .map_err(|e| AppError::database(format!("Invalid UUID in column {col}: {e}")))
        })
        .transpose()
    };
    Ok(CoachAthleteAssignment {
        id: uuid("id")?,
        coach_user_id: uuid("coach_user_id")?,
        athlete_user_id: uuid("athlete_user_id")?,
        tenant_id: TenantId::from_uuid(uuid("tenant_id")?),
        assigned_by: optional_uuid("assigned_by")?,
        assigned_at: row.try_get("assigned_at").map_err(|e| {
            AppError::database(format!("coach_athlete_assignments assigned_at: {e}"))
        })?,
        revoked_at: row.try_get("revoked_at").map_err(|e| {
            AppError::database(format!("coach_athlete_assignments revoked_at: {e}"))
        })?,
        revoked_by: optional_uuid("revoked_by")?,
    })
}

/// Emit the whole [`RosterRepository`] implementation for one backend type.
///
/// `$bind_id` is the function turning a `Uuid` into whatever that backend's
/// uuid columns accept (the `bind` of its codec in [`super::uuid_columns`]);
/// `$text` is the cast that reads a uuid column back as text
/// (`"::text"` on Postgres, `""` on `SQLite`).
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_roster_repository {
    ($ty:ty, $bind_id:path, $text:literal) => {
        #[async_trait::async_trait]
        impl RosterRepository for $ty {
            async fn list_athletes_for_coach(
                &self,
                coach_user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Vec<CoachAthleteAssignment>> {
                let rows = sqlx::query(list_athletes_for_coach_sql!($text))
                    .bind($bind_id(coach_user_id))
                    .bind(tenant_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list athletes for coach: {e}"))
                    })?;

                rows.iter().map(assignment_from_row).collect()
            }

            async fn list_coaches_for_athlete(
                &self,
                athlete_user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Vec<CoachAthleteAssignment>> {
                let rows = sqlx::query(list_coaches_for_athlete_sql!($text))
                    .bind($bind_id(athlete_user_id))
                    .bind(tenant_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list coaches for athlete: {e}"))
                    })?;

                rows.iter().map(assignment_from_row).collect()
            }

            async fn assign_athlete(
                &self,
                assignment: &CoachAthleteAssignment,
            ) -> AppResult<Option<CoachAthleteAssignment>> {
                let result = sqlx::query(ASSIGN_ATHLETE_SQL)
                    .bind($bind_id(assignment.id))
                    .bind($bind_id(assignment.coach_user_id))
                    .bind($bind_id(assignment.athlete_user_id))
                    .bind(assignment.tenant_id)
                    .bind(assignment.assigned_by.map($bind_id))
                    .bind(assignment.assigned_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to assign athlete: {e}")))?;

                if result.rows_affected() == 0 {
                    // Active assignment for the same (agent, athlete, tenant) already exists.
                    return Ok(None);
                }
                Ok(Some(assignment.clone()))
            }

            async fn revoke_assignment(
                &self,
                coach_user_id: Uuid,
                athlete_user_id: Uuid,
                tenant_id: TenantId,
                revoked_by: Option<Uuid>,
            ) -> AppResult<bool> {
                let result = sqlx::query(REVOKE_ASSIGNMENT_SQL)
                    .bind(Utc::now())
                    .bind(revoked_by.map($bind_id))
                    .bind($bind_id(coach_user_id))
                    .bind($bind_id(athlete_user_id))
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to revoke assignment: {e}")))?;

                Ok(result.rows_affected() > 0)
            }

            async fn is_athlete_managed_by(
                &self,
                coach_user_id: Uuid,
                athlete_user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let count: i64 = sqlx::query_scalar(COUNT_ACTIVE_ASSIGNMENTS_SQL)
                    .bind($bind_id(coach_user_id))
                    .bind($bind_id(athlete_user_id))
                    .bind(tenant_id)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to check assignment: {e}")))?;

                Ok(count > 0)
            }
        }
    };
}
pub(crate) use impl_roster_repository;
