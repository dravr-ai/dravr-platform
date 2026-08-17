// ABOUTME: CommitmentRepository trait — persistence for athlete commitments and their swept verdicts
// ABOUTME: Dual SQLite/Postgres impls live in database/ and backends/postgres/. Tenant-scoped except where noted.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::commitments::{Commitment, CommitmentOutcome, CommitmentStatus};

/// The verdict a sweep reached for one commitment.
///
/// Bundled into a struct rather than a wide argument list, matching the
/// repository's param-struct convention.
pub struct SweptVerdict<'a> {
    /// Owning tenant — every write is scoped to it.
    pub tenant_id: &'a str,
    /// The commitment being closed out.
    pub commitment_id: &'a str,
    /// What the count came to.
    pub outcome: CommitmentOutcome,
    /// How many matching sessions were counted.
    pub completed_sessions: u32,
    /// When the sweep ran.
    pub at: DateTime<Utc>,
}

/// Persistence for athlete commitments.
///
/// Commitments are **tenant-scoped**: every query carries `tenant_id` in its
/// `WHERE` clause. The two deliberate exceptions are [`CommitmentRepository::due_commitments`]
/// and [`CommitmentRepository::unreported_commitments`], the background sweeper's
/// scans — one task runs per server, so those span every tenant and carry the
/// per-row `tenant_id` forward into all subsequent writes.
///
/// `coach_id` / `conversation_id` / `sport` are stored as `''` for absent
/// rather than `NULL`, because the duplicate guard on insert compares them and
/// `NULL`s compare distinct. The repository maps `Option<String>` <-> `''` at
/// the boundary for all three.
#[async_trait]
pub trait CommitmentRepository: Send + Sync {
    /// Record a commitment the coach confirmed with the athlete.
    ///
    /// Deduplicates in SQL: a second identical open commitment (same tenant,
    /// user, coach, sport, target and window end) is silently dropped, so an
    /// athlete re-affirming the same promise mid-window cannot end up with two
    /// rows that both sweep and both report. Returns `true` when a row was
    /// actually inserted.
    async fn insert_commitment(&self, commitment: &Commitment) -> AppResult<bool>;

    /// A user's still-open commitments, soonest window first. Tenant-scoped;
    /// `limit` is clamped by the caller so the prompt block stays bounded.
    async fn list_open_commitments(
        &self,
        tenant_id: &str,
        user_id: &str,
        limit: i64,
    ) -> AppResult<Vec<Commitment>>;

    /// Open commitments whose window has closed (`status = 'open' AND
    /// window_end <= now_epoch`), oldest first. **Cross-tenant** system scan run
    /// by the background sweeper.
    async fn due_commitments(&self, now_epoch: i64, limit: i64) -> AppResult<Vec<Commitment>>;

    /// Labeled commitments whose verdict has not yet reached the athlete
    /// (`status = 'labeled'`), oldest sweep first. **Cross-tenant** system scan
    /// run by the background reporter.
    async fn unreported_commitments(&self, limit: i64) -> AppResult<Vec<Commitment>>;

    /// Record a sweep verdict, moving the row `open -> labeled`.
    ///
    /// The status predicate is part of the `UPDATE`, so a racing second sweep
    /// affects zero rows and returns `false` rather than overwriting a verdict
    /// or double-reporting. Tenant-scoped.
    async fn record_commitment_verdict(&self, verdict: &SweptVerdict<'_>) -> AppResult<bool>;

    /// Mark a verdict as delivered, moving the row `labeled -> reported`.
    ///
    /// Conditional on the row still being `labeled`, so two reporter passes
    /// racing over one row deliver once. Tenant-scoped.
    async fn mark_commitment_reported(
        &self,
        tenant_id: &str,
        commitment_id: &str,
        at: DateTime<Utc>,
    ) -> AppResult<bool>;

    /// Close a commitment out without a verdict reaching the athlete — the
    /// activity data never caught up, or the verdict went stale before any
    /// delivery route opened. Tenant-scoped.
    async fn expire_commitment(&self, tenant_id: &str, commitment_id: &str) -> AppResult<bool>;

    /// Retract an open or labeled commitment at the athlete's request.
    /// Tenant-scoped. Returns `false` when it was already closed.
    async fn cancel_commitment(&self, tenant_id: &str, commitment_id: &str) -> AppResult<bool>;

    /// When this user last had a commitment verdict delivered to them, if ever.
    ///
    /// Backs the cadence cap: accountability lands when it is noticed, not when
    /// it arrives every hour, so the reporter holds a verdict rather than
    /// stacking a second message onto the same day.
    async fn last_commitment_report(
        &self,
        tenant_id: &str,
        user_id: &str,
    ) -> AppResult<Option<DateTime<Utc>>>;
}

/// Build a complete `SELECT` over `athlete_commitments` at compile time.
///
/// The column list lives here and nowhere else, so a column added to
/// [`CommitmentRow`] reaches every read at once and the two backends cannot
/// drift apart on one. Assembled by `concat!` rather than `format!` — the whole
/// statement is a literal by the time it reaches sqlx, so no query text is ever
/// built at runtime and every value still binds as `$n`.
macro_rules! select_commitments {
    ($tail:literal) => {
        concat!(
            "SELECT id, tenant_id, user_id, coach_id, conversation_id, statement, sport, ",
            "target_sessions, window_start, window_end, status, outcome, ",
            "completed_sessions, swept_at, reported_at, created_at, updated_at ",
            "FROM athlete_commitments ",
            $tail
        )
    };
}

/// Insert guarded by a duplicate check on the open-commitment key.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, and every bind here is a plain `&str`/`i64`, so one statement
/// serves both backends and cannot drift between them. The verdict columns are
/// literal `NULL` because a freshly recorded commitment has no verdict by
/// construction.
pub(crate) const INSERT_COMMITMENT_SQL: &str = r"
    INSERT INTO athlete_commitments (
        id, tenant_id, user_id, coach_id, conversation_id, statement, sport,
        target_sessions, window_start, window_end, status, outcome, completed_sessions,
        swept_at, reported_at, created_at, updated_at
    )
    SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NULL, NULL, NULL, NULL, $12, $13
    WHERE NOT EXISTS (
        SELECT 1 FROM athlete_commitments
        WHERE tenant_id = $2 AND user_id = $3 AND coach_id = $4
          AND sport = $7 AND target_sessions = $8 AND window_end = $10
          AND status = 'open'
    )
";

/// The sweeper's cross-tenant scan for commitments whose window has closed.
pub(crate) const DUE_COMMITMENTS_SQL: &str = select_commitments!(
    "WHERE status = 'open' AND window_end <= $1 ORDER BY window_end ASC LIMIT $2"
);

/// The reporter's cross-tenant scan for verdicts still waiting on a route.
pub(crate) const UNREPORTED_COMMITMENTS_SQL: &str =
    select_commitments!("WHERE status = 'labeled' ORDER BY swept_at ASC LIMIT $1");

/// A user's open commitments, soonest window first.
pub(crate) const LIST_OPEN_COMMITMENTS_SQL: &str = select_commitments!(
    "WHERE tenant_id = $1 AND user_id = $2 AND status = 'open' ORDER BY window_end ASC LIMIT $3"
);

/// Record the sweep verdict. The `status = 'open'` predicate makes a racing
/// second sweep a zero-row no-op instead of a double count.
pub(crate) const RECORD_VERDICT_SQL: &str = r"
    UPDATE athlete_commitments
    SET status = 'labeled', outcome = $1, completed_sessions = $2, swept_at = $3, updated_at = $3
    WHERE id = $4 AND tenant_id = $5 AND status = 'open'
";

/// Mark the verdict delivered. Conditional on `labeled` so two reporter passes
/// racing over one row deliver exactly once.
pub(crate) const MARK_REPORTED_SQL: &str = r"
    UPDATE athlete_commitments
    SET status = 'reported', reported_at = $1, updated_at = $1
    WHERE id = $2 AND tenant_id = $3 AND status = 'labeled'
";

/// Close a commitment out without delivering a verdict.
pub(crate) const EXPIRE_COMMITMENT_SQL: &str = r"
    UPDATE athlete_commitments
    SET status = 'expired', updated_at = $1
    WHERE id = $2 AND tenant_id = $3 AND status IN ('open', 'labeled')
";

/// Retract a commitment at the athlete's request.
pub(crate) const CANCEL_COMMITMENT_SQL: &str = r"
    UPDATE athlete_commitments
    SET status = 'cancelled', updated_at = $1
    WHERE id = $2 AND tenant_id = $3 AND status IN ('open', 'labeled')
";

/// Most recent verdict delivery for a user — the cadence-cap input.
pub(crate) const LAST_REPORT_SQL: &str = r"
    SELECT MAX(reported_at) AS last_reported
    FROM athlete_commitments
    WHERE tenant_id = $1 AND user_id = $2 AND reported_at IS NOT NULL
";

/// Convert epoch seconds to a UTC timestamp.
fn epoch_to_dt(secs: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(secs, 0)
}

/// Clamp a DB counter (`i64`, always non-negative in our schema) into the `u32`
/// the domain type uses. Saturating rather than `as` so a corrupt huge value can
/// never wrap to a small one.
fn clamp_count(v: i64) -> u32 {
    u32::try_from(v.max(0)).unwrap_or(u32::MAX)
}

/// Map a `''`-for-absent query key back to `Option<String>`.
fn empty_to_none(v: String) -> Option<String> {
    (!v.is_empty()).then_some(v)
}

/// Primitive column values for one `athlete_commitments` row.
///
/// Both backends extract primitives with `try_get` and hand them here, so the
/// enum parsing and the `''` <-> `None` mapping live in exactly one place
/// instead of drifting between `SQLite` and Postgres.
pub(crate) struct CommitmentRow {
    /// `id` column.
    pub id: String,
    /// `tenant_id` column.
    pub tenant_id: String,
    /// `user_id` column.
    pub user_id: String,
    /// `coach_id` column (`''` = no coach on the turn).
    pub coach_id: String,
    /// `conversation_id` column (`''` = not raised in a Pierre conversation).
    pub conversation_id: String,
    /// `statement` column.
    pub statement: String,
    /// `sport` column (`''` = any sport counts).
    pub sport: String,
    /// `target_sessions` column.
    pub target_sessions: i64,
    /// `window_start` epoch seconds.
    pub window_start: i64,
    /// `window_end` epoch seconds.
    pub window_end: i64,
    /// `status` column.
    pub status: String,
    /// `outcome` column, or `None` until swept.
    pub outcome: Option<String>,
    /// `completed_sessions` column, or `None` until swept.
    pub completed_sessions: Option<i64>,
    /// `swept_at` epoch seconds, or `None` until swept.
    pub swept_at: Option<i64>,
    /// `reported_at` epoch seconds, or `None` until reported.
    pub reported_at: Option<i64>,
    /// `created_at` epoch seconds.
    pub created_at: i64,
    /// `updated_at` epoch seconds.
    pub updated_at: i64,
}

/// Build a [`Commitment`] from extracted row primitives.
///
/// Unknown `status` / `outcome` strings are an error rather than a lenient
/// fallback: a row whose lifecycle state cannot be read is invisible to every
/// scan, and silently defaulting it to `open` would make the sweeper re-count a
/// commitment it had already reported.
pub(crate) fn commitment_from_row(row: CommitmentRow) -> AppResult<Commitment> {
    let status = CommitmentStatus::parse(&row.status)
        .ok_or_else(|| AppError::database(format!("commitment {}: unknown status", row.id)))?;
    let outcome = match row.outcome.as_deref() {
        None => None,
        Some(raw) => Some(CommitmentOutcome::parse(raw).ok_or_else(|| {
            AppError::database(format!("commitment {}: unknown outcome", row.id))
        })?),
    };
    Ok(Commitment {
        id: row.id,
        tenant_id: row.tenant_id,
        user_id: row.user_id,
        coach_id: empty_to_none(row.coach_id),
        conversation_id: empty_to_none(row.conversation_id),
        statement: row.statement,
        sport: empty_to_none(row.sport),
        target_sessions: clamp_count(row.target_sessions),
        window_start: epoch_to_dt(row.window_start).unwrap_or_else(Utc::now),
        window_end: epoch_to_dt(row.window_end).unwrap_or_else(Utc::now),
        status,
        outcome,
        completed_sessions: row.completed_sessions.map(clamp_count),
        swept_at: row.swept_at.and_then(epoch_to_dt),
        reported_at: row.reported_at.and_then(epoch_to_dt),
        created_at: epoch_to_dt(row.created_at).unwrap_or_else(Utc::now),
        updated_at: epoch_to_dt(row.updated_at).unwrap_or_else(Utc::now),
    })
}
