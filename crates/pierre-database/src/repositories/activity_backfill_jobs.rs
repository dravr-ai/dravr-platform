// ABOUTME: Repository trait for activity jobs still owed — one per (user, provider, kind), recorded before the spawn
// ABOUTME: The SQL is written once here and both backends emit their impl from it; a lapsed lease is what the resume sweep claims

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::repositories::memory_extraction_jobs::JobLease;

/// How a job row's columns decode
mod rows;
pub(crate) use rows::{backfill_job_from_row, job_progress_from_row};
/// The statements that record an ask
mod record_statements;
pub(crate) use record_statements::{
    REBIND_LIVE_READ_SQL, RECORD_BACKFILL_JOB_SQL, RECORD_LIVE_READ_SQL,
};
/// What a history walk's API phase is reopened from
mod walk_rows;
pub use walk_rows::{HistoryWalkRowState, LeasedWalkRow, PurgedHistoryWalk, ReopenFrom};
/// The statements that move a history walk down its window, through its phases
mod walk_statements;
pub(crate) use walk_statements::{
    END_HISTORY_WALK_SQL, HAND_OFF_HISTORY_WALK_SQL, HISTORY_WALK_ROW_SQL, LEASED_WALK_ROW_SQL,
    PURGED_HISTORY_WALKS_SQL, RECORD_JOB_PROGRESS_SQL, REOPEN_HISTORY_WALK_SQL,
    SETTLE_PURGED_HISTORY_WALK_SQL,
};

/// A detail walk asked again while one is owed
mod detail_walk;
pub(crate) use detail_walk::{
    detail_walk_ended_from_row, detail_walk_row_from_row, CREDIT_DETAIL_WALK_SQL,
    DETAIL_WALK_ROW_SQL, END_DETAIL_WALK_SQL, SET_DETAIL_WALK_TRIED_SQL, WIDEN_DETAIL_WALK_SQL,
};
pub use detail_walk::{DetailWalkEnd, DetailWalkEnded, DetailWalkRow, WalkTried, WalkWiden};
/// The statements of a row's life past its record
mod job_statements;
pub(crate) use job_statements::{
    BACKFILL_JOB_LEASE_SQL, BACKFILL_JOB_STATE_SQL, END_BACKFILL_JOB_SQL, FINISH_BACKFILL_JOB_SQL,
    FINISH_LIVE_READ_SQL, JOB_PROGRESS_SQL, REAP_EXHAUSTED_BACKFILL_JOBS_SQL,
    RELEASE_BACKFILL_JOB_CLAIM_SQL, RELEASE_BACKFILL_JOB_LEASE_SQL, RENEW_BACKFILL_JOB_LEASE_SQL,
    TAKE_OVER_IDLE_LIVE_READ_SQL,
};
/// The `outcome` column: how an ended row ended
mod outcome;
pub use outcome::BackfillJobState;
pub(crate) use outcome::{
    backfill_job_state_of, incomplete_outcome, phase_purged_outcome, walked_outcome,
};
/// What a job row is for, and the kinds the claim and the reap take
mod kind;
pub(crate) use kind::known_backfill_job_kinds;
pub use kind::BackfillJobKind;

/// How far a job has got, and what a step adds to it
mod progress;
pub use progress::{JobFloor, JobProgress, JobProgressStep};
/// The repository body every backend emits
mod repository_impl;
pub(crate) use repository_impl::impl_activity_backfill_job_repository;
/// Where a history walk's paused wellness copy resumes, and whether its first
/// fill completed (carnet#1661)
mod wellness_statements;
pub(crate) use wellness_statements::{
    MARK_WELLNESS_FILLED_SQL, PAUSE_WELLNESS_COPY_SQL, WELLNESS_FILLED_SQL, WELLNESS_RESUME_DAY_SQL,
};

/// Where a history walk goes on once its session phase found nothing more:
/// the API grant beside the session, paged strictly below the oldest row the
/// session reached (carnet#1482).
///
/// Written over a running walk's row
/// ([`ActivityBackfillJobRepository::hand_off_history_walk`]) or a walk the
/// session ended alone ([`ActivityBackfillJobRepository::reopen_history_walk`]).
/// The row keeps its key, the session's: every reader of the walk reads it
/// there, owed until the API phase ends too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkHandOff {
    /// The backend the walk reads from now on: the row's `walk_backend`.
    pub backend: String,
    /// The API phase reads strictly below this (unix seconds); `None` reads
    /// from the head of the list, when the session stored nothing.
    pub before_ts: Option<i64>,
    /// Months the walk covered, from now down to `before_ts`: the row's
    /// [`JobProgress::units_done`] is set to it, since the API phase pages
    /// again the years the session's last step found empty.
    pub units_done: i64,
    /// Activities the session's last step wrote, added to the row's
    /// `rows_landed`.
    pub rows_landed: i64,
    /// Activities one API request asks for: the row's `fetch_limit`.
    pub fetch_limit: i64,
}

/// One backfill owed: the window that was asked for and where its finished
/// answer is expected.
#[derive(Clone, PartialEq)]
pub struct ActivityBackfillJobRow {
    /// Job id (UUID string), minted by the recorder.
    pub id: String,
    /// Tenant that owns the cache rows.
    pub tenant_id: TenantId,
    /// Athlete whose history is being backfilled.
    pub user_id: Uuid,
    /// Backend provider slug (already resolved to the sciotte mirror if any).
    pub provider: String,
    /// What the job is for, as the ask that started it recorded.
    pub kind: BackfillJobKind,
    /// The requested window's deep `after` (unix seconds). A walk's floor: a
    /// [`BackfillJobKind::DetailWalk`] asked again while owed lowers it
    /// ([`ActivityBackfillJobRepository::widen_detail_walk`]), and a step's
    /// progress write never touches it.
    pub after_ts: Option<i64>,
    /// The requested window's `before` (unix seconds). A walk's cursor: it
    /// starts at the window's top and each step moves it down past the rows
    /// it settled, so a walk read newest first under it always ends.
    pub before_ts: Option<i64>,
    /// The requested fetch limit.
    pub fetch_limit: Option<i64>,
    /// Pierre conversation the completion notice goes back to, if any.
    pub conversation_id: Option<String>,
    /// Unix milliseconds when the ask recorded it.
    pub created_at_ms: i64,
    /// Unix milliseconds until which one runner holds it; `0` when free.
    pub leased_until_ms: i64,
    /// Runs started so far, including the one this claim represents.
    pub attempts: i64,
    /// The token of the claim (or the recording) that holds the row; renewing
    /// and finishing it require this token. `None` on a row recorded before
    /// tokens existed, which only a claim can take.
    pub lease_token: Option<String>,
    /// How far the job has got.
    pub progress: JobProgress,
    /// The conversation turn a live read answers.
    pub turn_id: Option<String>,
    /// The user message that asked a live read.
    pub asked_message_id: Option<String>,
    /// The backend a history walk's current phase reads: `None` reads
    /// `provider`, the walk's own source; the API grant once the session's
    /// phase handed off below the oldest row it reached ([`WalkHandOff`]).
    pub walk_backend: Option<String>,
    /// The sport type a [`BackfillJobKind::DetailWalk`] reads, `None` for
    /// every sport (and for every other kind).
    pub walk_sport: Option<String>,
}

/// The ask a live read's row was bound to when it finished.
///
/// This is where its follow-up goes. A newer ask while the read was owed
/// rebound the row ([`ActivityBackfillJobRepository::record_backfill_job`]),
/// so this is read back from the row, never from the run that started it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LiveReadAsk {
    /// The conversation the follow-up answers into, if any.
    pub conversation_id: Option<String>,
    /// The turn that asked last.
    pub turn_id: Option<String>,
    /// The user message that asked last.
    pub asked_message_id: Option<String>,
}

/// Whether an attempt holds an owed row's lease right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobLeaseState {
    /// A live attempt holds the lease: the job is running.
    Leased,
    /// The row is owed but no attempt holds it: a failed attempt gave its
    /// lease back, or the one that held it died. Nothing runs it until the
    /// resume sweep, or the next ask of a live read, takes it.
    Idle,
}

/// Which rows a resume sweep may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillJobClaim {
    /// The moment the claim is made.
    pub now_ms: i64,
    /// Only rows recorded before `now_ms - queued_older_than_ms` are stale
    /// enough to take: a fresh row is one its own spawn is still running.
    pub queued_older_than_ms: i64,
    /// How long the claim holds the row.
    pub lease_ms: i64,
    /// Rows at or past this many attempts are left alone.
    pub max_attempts: i64,
    /// At most this many rows per sweep.
    pub limit: i64,
}

/// The ledger of backfills owed.
#[async_trait]
pub trait ActivityBackfillJobRepository: Send + Sync {
    /// Record a job before its spawn, leased to the recording instance until
    /// `leased_until_ms`. Idempotent on `(user_id, provider, kind)`: returns
    /// `true` when this call inserted the row, `false` when one of the kind
    /// was already owed — the same job, already on file. A row that only
    /// records how an earlier job ended ([`BackfillJobState::Incomplete`],
    /// [`BackfillJobState::Walked`]) is replaced by an ask in its own tenant:
    /// it owes nothing. An ask from another tenant leaves it, and returns
    /// `false`; so does a history walk in [`BackfillJobState::PhasePurged`],
    /// which a step may still hold.
    ///
    /// A [`BackfillJobKind::LiveRead`] asked again while one is owed still
    /// returns `false`, but rebinds the owed row to the new ask's
    /// conversation, turn and message, so the follow-up answers the newest
    /// question.
    async fn record_backfill_job(&self, row: &ActivityBackfillJobRow) -> AppResult<bool>;

    /// Rebind the [`BackfillJobKind::LiveRead`] owed for `ask`'s
    /// `(user_id, provider)` in its tenant to `ask`'s conversation, turn and
    /// message, recording nothing when none is owed. `false` when no row was
    /// rebound — none owed, or `ask` has no conversation.
    async fn rebind_live_read(&self, ask: &ActivityBackfillJobRow) -> AppResult<bool>;

    /// Atomically lease up to `claim.limit` rows whose lease has lapsed, that
    /// were recorded before `now_ms - queued_older_than_ms`, whose retry is
    /// due ([`Self::release_backfill_job_lease`]), and whose attempts are
    /// below the cap — oldest first, incrementing `attempts`.
    /// The returned rows carry the incremented count.
    async fn claim_stale_backfill_jobs(
        &self,
        claim: BackfillJobClaim,
    ) -> AppResult<Vec<ActivityBackfillJobRow>>;

    /// Extend the lease a running backfill holds. Returns `false` when the
    /// row is gone or another claim holds it.
    async fn renew_backfill_job_lease(
        &self,
        lease: &JobLease,
        leased_until_ms: i64,
    ) -> AppResult<bool>;

    /// The backfill reached a terminal outcome the athlete has been told
    /// about (completed, or reconnect needed): delete the row, if this lease
    /// still holds it. `false` when another claim took it.
    async fn finish_backfill_job(&self, lease: &JobLease) -> AppResult<bool>;

    /// A live read landed: delete its row, if this lease still holds it, and
    /// return the ask the row was bound to at that instant. One statement, so
    /// an ask that rebinds the row either lands before it (and is returned)
    /// or after it (and records a read of its own). `None` when another claim
    /// took the row.
    async fn finish_live_read(&self, lease: &JobLease) -> AppResult<Option<LiveReadAsk>>;

    /// The job ended before what it was asked for landed: keep its row, if
    /// this lease still holds it, as [`BackfillJobState::Incomplete`] —
    /// released, never claimed or reaped again. `false` when another claim
    /// took it.
    async fn end_backfill_job_incomplete(&self, lease: &JobLease) -> AppResult<bool>;

    /// Give back a claim that could not run: a run for the pair is already
    /// in flight on this instance, or a request budget refused the step. The
    /// row is released and the attempt the claim counted is returned, so
    /// waiting never spends the job's attempts. `pause_until_ms` makes the
    /// sweep's next claim wait until that instant (unix ms), the end of the
    /// refusing window; `None` keeps the retry the row had. `false` when this
    /// lease no longer holds it.
    async fn release_backfill_job_claim(
        &self,
        lease: &JobLease,
        pause_until_ms: Option<i64>,
    ) -> AppResult<bool>;

    /// A failed attempt gives its lease back and keeps the attempt it
    /// counted: the row reads [`JobLeaseState::Idle`] at once, for an ask to
    /// take over, while the sweep's bounded retries wait until `retry_at_ms`.
    /// `false` when this lease no longer holds the row.
    async fn release_backfill_job_lease(
        &self,
        lease: &JobLease,
        retry_at_ms: i64,
    ) -> AppResult<bool>;

    /// Take the [`BackfillJobKind::LiveRead`] owed for `ask`'s
    /// `(user_id, provider)` in its tenant when no attempt holds it at
    /// `ask.created_at_ms` ([`JobLeaseState::Idle`]): lease it under `ask`'s
    /// token until `ask.leased_until_ms`, read `ask`'s window, and count one
    /// more attempt. The follow-up moves to `ask` when it has a
    /// conversation; an ask with none leaves the row bound to the ask it was
    /// owed to. Returns the lease and the attempts it now counts; `None`
    /// when an attempt holds the row, or none is owed in the tenant.
    async fn take_over_idle_live_read(
        &self,
        ask: &ActivityBackfillJobRow,
    ) -> AppResult<Option<(JobLease, i64)>>;

    /// Whether an attempt holds the lease of the owed `kind` job for
    /// `(user_id, provider)` in `tenant_id` at `now_ms`; `None` when no job
    /// of the kind is owed.
    async fn backfill_job_lease_state(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        kind: BackfillJobKind,
        now_ms: i64,
    ) -> AppResult<Option<JobLeaseState>>;

    /// The history walk row filed under `(user_id, provider)` in
    /// `tenant_id`: its state and the grant it reads once handed off, or
    /// `None` when the ledger holds none.
    async fn history_walk_row(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<HistoryWalkRowState>>;

    /// The `kind` walk row `lease` was taken on, `user_id`'s in
    /// `tenant_id`, as it stands now, the lease or not; `None` when it is
    /// gone, or is of another kind. What a step reads around its write to
    /// the store.
    async fn leased_walk_row(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        lease: &JobLease,
        kind: BackfillJobKind,
    ) -> AppResult<Option<LeasedWalkRow>>;

    /// A step of a job that walks down its window landed: add `step` to the
    /// row's [`JobProgress`] and keep the row owed for the next step, read
    /// below `step.before_ts`, with its attempts reset and its lease released
    /// so the resume sweep runs that step. `false` when this lease no longer
    /// holds the row.
    async fn record_job_progress(&self, lease: &JobLease, step: JobProgressStep)
        -> AppResult<bool>;

    /// A [`BackfillJobKind::DetailWalk`] asked again while one is owed for
    /// `(user_id, provider)` in `tenant_id`: widen its floor down to
    /// `widen.after_ts`, recount its total, and read every sport when the two
    /// asks name different ones ([`WalkWiden`]). Its cursor is left where its
    /// steps moved it. `false` when no such walk is owed in the tenant: it
    /// ended meanwhile, and the ask records a fresh one.
    async fn widen_detail_walk(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        widen: &WalkWiden,
    ) -> AppResult<bool>;

    /// A [`BackfillJobKind::DetailWalk`]'s last step passed the floor it
    /// walked to: add `end` to the row's progress and keep the row, released,
    /// as [`BackfillJobState::Walked`], with its floor, what it read and what
    /// it stored. Returns the progress the row ended on and when its walk
    /// was asked ([`DetailWalkEnded`]), or `None` when the row was not
    /// ended: this lease no longer holds it, or an ask widened its floor
    /// below `end.walked_floor` meanwhile ([`Self::widen_detail_walk`]). In
    /// the second case the lease still holds the owed row, so the step
    /// writes its progress ([`Self::record_job_progress`]) and the next step
    /// walks on down.
    async fn end_detail_walk(
        &self,
        lease: &JobLease,
        end: &DetailWalkEnd,
    ) -> AppResult<Option<DetailWalkEnded>>;

    /// Add `stored` activities a [`BackfillJobKind::DetailWalk`] step stored
    /// after its lease was taken back to the done and stored counts of its
    /// walk (row `row_id`, asked at `asked_at_ms`), while it is still owed
    /// ([`CREDIT_DETAIL_WALK_SQL`]). `false` when that walk is not.
    async fn credit_detail_walk(
        &self,
        row_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        asked_at_ms: i64,
        stored: i64,
    ) -> AppResult<bool>;

    /// The [`BackfillJobKind::DetailWalk`] filed under `(user_id, provider)`
    /// in `tenant_id`, owed or ended, or `None` when the ledger holds none.
    async fn detail_walk_row(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<DetailWalkRow>>;

    /// Record on the ended [`BackfillJobKind::DetailWalk`] filed under
    /// `(user_id, provider)` in `tenant_id` the range a walk recorded over it
    /// next reads nothing of again, or none. `false` when no ended row
    /// stands: an owed one keeps the range it was recorded with.
    async fn set_detail_walk_tried(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        tried: Option<&WalkTried>,
    ) -> AppResult<bool>;

    /// The day the wellness copy of the walk `lease` holds resumes from
    /// (carnet#1661); `None` when it copies its whole window, or the lease no
    /// longer holds the row. A session-read cap that refuses a day pauses the
    /// copy there ([`Self::pause_wellness_copy`]); the walk's end and a new
    /// ask of the row clear it.
    async fn wellness_resume_day(
        &self,
        lease: &JobLease,
        tenant_id: TenantId,
    ) -> AppResult<Option<NaiveDate>>;

    /// Pause the walk's wellness copy at `resume_day` until `pause_until_ms`
    /// (unix ms): the row is released with its attempt returned, so a cap
    /// never spends the walk's attempts. `false` when this lease no longer
    /// holds it.
    async fn pause_wellness_copy(
        &self,
        lease: &JobLease,
        tenant_id: TenantId,
        resume_day: NaiveDate,
        pause_until_ms: i64,
    ) -> AppResult<bool>;

    /// Whether the walk `lease` holds completed its first wellness fill
    /// (carnet#1661): a later copy counts in the ordinary background caps,
    /// not the first fill's own day. `false` when it has not, or the lease no
    /// longer holds the row.
    async fn wellness_first_filled(&self, lease: &JobLease, tenant_id: TenantId)
        -> AppResult<bool>;

    /// Record that the walk `lease` holds completed its first wellness fill
    /// at `filled_at_ms` (unix ms), keeping the first time it did; a new ask
    /// of the row keeps it. `false` when this lease no longer holds it.
    async fn mark_wellness_first_filled(
        &self,
        lease: &JobLease,
        tenant_id: TenantId,
        filled_at_ms: i64,
    ) -> AppResult<bool>;

    /// A history walk's last step landed: add `step` to the row's progress
    /// and keep the row, released, as [`BackfillJobState::Walked`] — never
    /// claimed or reaped again. Returns the progress the row ended on, or
    /// `None` when this lease no longer holds the row.
    async fn end_history_walk(
        &self,
        lease: &JobLease,
        step: JobProgressStep,
    ) -> AppResult<Option<JobProgress>>;

    /// A history walk's session phase found nothing more and a live API grant
    /// stands beside it: keep the row owed under its key, reading
    /// `hand_off.backend` from now on, below `hand_off.before_ts`, with its
    /// floor to learn again from the grant's profile, its attempts reset and
    /// its lease released so the resume sweep runs the first API step.
    /// `false` when this lease no longer holds the row.
    async fn hand_off_history_walk(
        &self,
        lease: &JobLease,
        hand_off: &WalkHandOff,
    ) -> AppResult<bool>;

    /// A history walk of `provider` owes its API phase to the grant now live
    /// beside it: the row of `(user_id, provider)` in `tenant_id` is owed
    /// again as [`Self::hand_off_history_walk`] leaves it, its retry due at
    /// once. Its count of rows starts over: the session's end was told
    /// already, so the API phase's message counts what it brings in, and none
    /// is sent when that is nothing. Only a row in the state `from` names, and
    /// only once no step holds its lease at `now_ms`; `false` otherwise.
    async fn reopen_history_walk(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        from: ReopenFrom,
        hand_off: &WalkHandOff,
        now_ms: i64,
    ) -> AppResult<bool>;

    /// The history walk of `(user_id, provider)` in `tenant_id` in
    /// [`BackfillJobState::PhasePurged`] has no live grant to reopen its
    /// phase through: keep it as [`BackfillJobState::Walked`], its session
    /// walked alone, which a grant connected later reopens. Only once no step
    /// holds its lease at `now_ms`; `false` when no such row stands.
    async fn settle_purged_history_walk(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        now_ms: i64,
    ) -> AppResult<bool>;

    /// Up to `limit` history walks in [`BackfillJobState::PhasePurged`] whose
    /// lease has lapsed at `now_ms`, oldest first: the step that held each
    /// one stopped, or died, without resolving it. Across every tenant, for
    /// the resume sweep.
    async fn purged_history_walks(
        &self,
        now_ms: i64,
        limit: i64,
    ) -> AppResult<Vec<PurgedHistoryWalk>>;

    /// How far the `kind` job for `(user_id, provider)` in `tenant_id` has
    /// got, owed or ended, or `None` when the ledger holds no such row.
    async fn job_progress(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        kind: BackfillJobKind,
    ) -> AppResult<Option<JobProgress>>;

    /// What the ledger holds for the `kind` job of `(user_id, provider)` in
    /// `tenant_id`, or `None` when it holds nothing.
    async fn backfill_job_state(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        kind: BackfillJobKind,
    ) -> AppResult<Option<BackfillJobState>>;

    /// Delete every owed row at or past the attempt cap whose lease has
    /// lapsed: the run that was to finish it died too, no claim will take it
    /// again, and while it stands `record_backfill_job` refuses every new ask
    /// for the pair. Returns the rows removed, as they stood: a live read's
    /// still names the ask it owes a follow-up.
    async fn reap_exhausted_backfill_jobs(
        &self,
        now_ms: i64,
        max_attempts: i64,
    ) -> AppResult<Vec<ActivityBackfillJobRow>>;
}

/// The claim: one statement, so the lease and the attempt bump land
/// together and the subquery picks the rows under the same lock. The
/// backend's lock clause — `FOR UPDATE SKIP LOCKED` on `PostgreSQL`, nothing
/// on `SQLite` — is spliced in by the backend shell.
macro_rules! claim_backfill_jobs_sql {
    ($lock:literal) => {
        concat!(
            "UPDATE activity_backfill_jobs \
             SET leased_until_ms = $1, attempts = attempts + 1, lease_token = $6 \
             WHERE id IN ( \
                 SELECT j.id FROM activity_backfill_jobs j \
                 WHERE j.leased_until_ms <= $2 \
                   AND j.next_attempt_ms <= $2 \
                   AND j.created_at_ms < $3 \
                   AND j.attempts < $4 \
                   AND j.outcome IS NULL \
                   AND j.kind IN ",
            $crate::repositories::activity_backfill_jobs::known_backfill_job_kinds!(),
            " \
                 ORDER BY j.created_at_ms ASC \
                 LIMIT $5 ",
            $lock,
            ") \
             RETURNING id, tenant_id, user_id, provider, after_ts, before_ts, fetch_limit, \
                       conversation_id, created_at_ms, leased_until_ms, attempts, lease_token, \
                       kind, months_total, months_done, rows_landed, secs_per_unit, turn_id, \
                       asked_message_id, walk_backend, walk_sport"
        )
    };
}
pub(crate) use claim_backfill_jobs_sql;
