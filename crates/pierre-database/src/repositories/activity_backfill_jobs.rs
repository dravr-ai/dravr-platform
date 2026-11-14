// ABOUTME: Repository trait for activity jobs still owed — one per (user, provider, kind), recorded before the spawn
// ABOUTME: The SQL is written once here and both backends emit their impl from it; a lapsed lease is what the resume sweep claims

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use std::fmt;
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
pub use detail_walk::{DetailWalkEnd, WalkWiden};
pub(crate) use detail_walk::{END_DETAIL_WALK_SQL, WIDEN_DETAIL_WALK_SQL};
/// The statements of a row's life past its record
mod job_statements;
pub(crate) use job_statements::{
    BACKFILL_JOB_LEASE_SQL, BACKFILL_JOB_STATE_SQL, END_BACKFILL_JOB_SQL, FINISH_BACKFILL_JOB_SQL,
    FINISH_LIVE_READ_SQL, JOB_PROGRESS_SQL, REAP_EXHAUSTED_BACKFILL_JOBS_SQL,
    RELEASE_BACKFILL_JOB_CLAIM_SQL, RELEASE_BACKFILL_JOB_LEASE_SQL, RENEW_BACKFILL_JOB_LEASE_SQL,
    TAKE_OVER_IDLE_LIVE_READ_SQL,
};
/// What a job row is for.
///
mod kind;
pub(crate) use kind::known_backfill_job_kinds;
pub use kind::BackfillJobKind;

/// How far a job has got: what a walk reports while it runs.
///
/// A unit is what the job's kind walks: a month of history for a
/// [`BackfillJobKind::HistoryWalk`], an activity whose detail is read for a
/// [`BackfillJobKind::DetailWalk`]. The columns keep their first names,
/// `months_total` and `months_done`, which an applied migration fixed.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JobProgress {
    /// Units the job has to cover; `None` while that is not known (a
    /// history walk's floor not learned yet).
    pub units_total: Option<i64>,
    /// Units covered so far.
    pub units_done: i64,
    /// Activities written so far.
    pub rows_landed: i64,
    /// Measured seconds per covered unit; `None` until a step measured one.
    pub secs_per_unit: Option<f64>,
}

/// One step a running job finished
/// ([`ActivityBackfillJobRepository::record_job_progress`]): what it adds to
/// the row's [`JobProgress`] and where the next step starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JobProgressStep {
    /// The next step reads below this (unix seconds): the row's new `before_ts`.
    pub before_ts: i64,
    /// Units this step covered, added to [`JobProgress::units_done`].
    pub units_done: i64,
    /// Activities this step wrote, added to [`JobProgress::rows_landed`].
    pub rows_landed: i64,
    /// Seconds per unit this step measured; `None` keeps the stored one.
    pub secs_per_unit: Option<f64>,
    /// The floor the job learned on this step — the oldest instant it reads
    /// to (unix seconds, the row's new `after_ts`) and the units from its
    /// start down to it ([`JobProgress::units_total`]); `None` keeps both.
    pub floor: Option<JobFloor>,
}

/// Where a walk's history ends, once a step has learned it: a Strava
/// account's creation, or the hard floor when the profile names none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobFloor {
    /// The oldest instant the job reads to (unix seconds).
    pub after_ts: i64,
    /// Months from the job's start down to `after_ts`, the walk's
    /// [`JobProgress::units_total`]; `None` keeps the stored one, when the
    /// floor is only the hard one and the walk ends on its empty years
    /// before it.
    pub units_total: Option<i64>,
}

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

/// Prints every field but the lease token, which grants the right to renew
/// and finish the row and so stays out of logs and panics.
impl fmt::Debug for ActivityBackfillJobRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActivityBackfillJobRow")
            .field("id", &self.id)
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .field("provider", &self.provider)
            .field("kind", &self.kind)
            .field("after_ts", &self.after_ts)
            .field("before_ts", &self.before_ts)
            .field("fetch_limit", &self.fetch_limit)
            .field("conversation_id", &self.conversation_id)
            .field("created_at_ms", &self.created_at_ms)
            .field("leased_until_ms", &self.leased_until_ms)
            .field("attempts", &self.attempts)
            .field("lease_token", &"[redacted]")
            .field("progress", &self.progress)
            .field("turn_id", &self.turn_id)
            .field("asked_message_id", &self.asked_message_id)
            .field("walk_backend", &self.walk_backend)
            .field("walk_sport", &self.walk_sport)
            .finish()
    }
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

/// What the ledger holds for one `(user, provider, kind)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillJobState {
    /// A job is owed: recorded, running or waiting for the resume sweep.
    Owed,
    /// The job ended before what it was asked for landed, and its row was
    /// kept to say so ([`ActivityBackfillJobRepository::end_backfill_job_incomplete`]):
    /// a history walk whose session expired mid-copy.
    Incomplete,
    /// A walk reached the end of what it walks, and its row was kept to say
    /// so: a history walk the end of the athlete's history
    /// ([`ActivityBackfillJobRepository::end_history_walk`]), a detail walk
    /// its window's floor ([`ActivityBackfillJobRepository::end_detail_walk`]).
    Walked,
    /// A purge of the grant a history walk's API phase was paging took that
    /// phase while it was owed. The row is still the walk of the pair: a step
    /// may still hold its lease and be paging the grant, so no walk of the
    /// grant starts beside it, and the step records nothing on the row and
    /// leaves none of what it read in the store (it checks the row around its
    /// write, [`ActivityBackfillJobRepository::leased_walk_row`]). Once no
    /// step holds the lease, the phase is reopened below the session's own
    /// rows when a grant is live, or the row is kept as
    /// [`Self::Walked`] when none is
    /// ([`ActivityBackfillJobRepository::reopen_history_walk`],
    /// [`ActivityBackfillJobRepository::settle_purged_history_walk`]); the
    /// resume sweep does it for a row whose step died
    /// ([`ActivityBackfillJobRepository::purged_history_walks`]). A
    /// whole-provider purge of that grant settles it as walked at once.
    PhasePurged,
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

/// The `outcome` an ended row carries in [`BackfillJobState::Incomplete`].
const OUTCOME_INCOMPLETE: &str = "incomplete";

/// The `outcome` an ended row carries in [`BackfillJobState::Walked`].
const OUTCOME_WALKED: &str = "walked";

/// The `outcome` a row carries in [`BackfillJobState::PhasePurged`]. Not an
/// end: only the walk's resolution moves it on, never a step's write, the
/// claim or the reap.
const OUTCOME_PHASE_PURGED: &str = "phase_purged";

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
    /// it stored. Returns the progress the row ended on, or `None` when the
    /// row was not ended: this lease no longer holds it, or an ask widened
    /// its floor below `end.walked_floor` meanwhile
    /// ([`Self::widen_detail_walk`]). In the second case the lease still
    /// holds the owed row, so the step writes its progress
    /// ([`Self::record_job_progress`]) and the next step walks on down.
    async fn end_detail_walk(
        &self,
        lease: &JobLease,
        end: &DetailWalkEnd,
    ) -> AppResult<Option<JobProgress>>;

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

/// The state an `outcome` column reads as.
pub(crate) fn backfill_job_state_of(outcome: Option<&str>) -> AppResult<BackfillJobState> {
    match outcome {
        None => Ok(BackfillJobState::Owed),
        Some(OUTCOME_INCOMPLETE) => Ok(BackfillJobState::Incomplete),
        Some(OUTCOME_WALKED) => Ok(BackfillJobState::Walked),
        Some(OUTCOME_PHASE_PURGED) => Ok(BackfillJobState::PhasePurged),
        Some(other) => Err(AppError::database(format!(
            "activity_backfill_jobs outcome is not one this build knows: {other}"
        ))),
    }
}

/// The `outcome` [`BackfillJobState::Incomplete`] is stored as.
pub(crate) const fn incomplete_outcome() -> &'static str {
    OUTCOME_INCOMPLETE
}

/// The `outcome` [`BackfillJobState::Walked`] is stored as.
pub(crate) const fn walked_outcome() -> &'static str {
    OUTCOME_WALKED
}

/// The `outcome` [`BackfillJobState::PhasePurged`] is stored as.
pub(crate) const fn phase_purged_outcome() -> &'static str {
    OUTCOME_PHASE_PURGED
}

/// Emit the whole [`ActivityBackfillJobRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type and its lock clause for [`claim_backfill_jobs_sql!`],
/// and sqlx resolves the driver from `self.pool()`.
///
/// The body names most of its consts, helpers and types unqualified, so the
/// invoking shell must `use` every one of those; the rest it reaches through
/// `$crate::repositories::activity_backfill_jobs::…`.
macro_rules! impl_activity_backfill_job_repository {
    ($ty:ty, $lock:literal) => {
        #[async_trait::async_trait]
        impl ActivityBackfillJobRepository for $ty {
            async fn record_backfill_job(&self, row: &ActivityBackfillJobRow) -> AppResult<bool> {
                let live_read = row.kind == BackfillJobKind::LiveRead;
                let sql = if live_read {
                    RECORD_LIVE_READ_SQL
                } else {
                    RECORD_BACKFILL_JOB_SQL
                };
                let query = sqlx::query(sql)
                    .bind(&row.id)
                    .bind(row.tenant_id)
                    .bind(row.user_id.to_string())
                    .bind(&row.provider)
                    .bind(row.after_ts)
                    .bind(row.before_ts)
                    .bind(row.fetch_limit)
                    .bind(&row.conversation_id)
                    .bind(row.created_at_ms)
                    .bind(row.leased_until_ms)
                    .bind(row.attempts)
                    .bind(&row.lease_token)
                    .bind(row.kind.as_str())
                    .bind(row.progress.units_total)
                    .bind(row.progress.units_done)
                    .bind(row.progress.rows_landed)
                    .bind(row.progress.secs_per_unit)
                    .bind(&row.turn_id)
                    .bind(&row.asked_message_id)
                    .bind(&row.walk_sport);
                let query = if live_read {
                    query
                } else {
                    query.bind($crate::repositories::activity_backfill_jobs::phase_purged_outcome())
                };
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to record backfill job: {e}"))
                };
                if live_read {
                    let written = query.fetch_optional(self.pool()).await.map_err(failed)?;
                    let written_id: Option<String> = written
                        .map(|written| written.try_get("id"))
                        .transpose()
                        .map_err(failed)?;
                    return Ok(written_id.as_deref() == Some(row.id.as_str()));
                }
                let result = query.execute(self.pool()).await.map_err(failed)?;
                Ok(result.rows_affected() == 1)
            }

            async fn rebind_live_read(&self, ask: &ActivityBackfillJobRow) -> AppResult<bool> {
                let rebound =
                    sqlx::query($crate::repositories::activity_backfill_jobs::REBIND_LIVE_READ_SQL)
                        .bind(ask.user_id.to_string())
                        .bind(ask.tenant_id)
                        .bind(&ask.provider)
                        .bind(BackfillJobKind::LiveRead.as_str())
                        .bind(&ask.conversation_id)
                        .bind(&ask.turn_id)
                        .bind(&ask.asked_message_id)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to rebind live read: {e}"))
                        })?;
                Ok(rebound.rows_affected() == 1)
            }

            async fn claim_stale_backfill_jobs(
                &self,
                claim: BackfillJobClaim,
            ) -> AppResult<Vec<ActivityBackfillJobRow>> {
                let rows = sqlx::query(claim_backfill_jobs_sql!($lock))
                    .bind(claim.now_ms.saturating_add(claim.lease_ms))
                    .bind(claim.now_ms)
                    .bind(claim.now_ms.saturating_sub(claim.queued_older_than_ms))
                    .bind(claim.max_attempts)
                    .bind(claim.limit)
                    .bind(uuid::Uuid::new_v4().to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim backfill jobs: {e}"))
                    })?;
                rows.iter().map(backfill_job_from_row).collect()
            }

            async fn renew_backfill_job_lease(
                &self,
                lease: &JobLease,
                leased_until_ms: i64,
            ) -> AppResult<bool> {
                let result = sqlx::query(RENEW_BACKFILL_JOB_LEASE_SQL)
                    .bind(leased_until_ms)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to renew backfill job lease: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn finish_backfill_job(&self, lease: &JobLease) -> AppResult<bool> {
                let done = sqlx::query(FINISH_BACKFILL_JOB_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to finish backfill job: {e}"))
                    })?;
                Ok(done.rows_affected() == 1)
            }

            async fn finish_live_read(
                &self,
                lease: &JobLease,
            ) -> AppResult<Option<$crate::repositories::activity_backfill_jobs::LiveReadAsk>> {
                let failed =
                    |e: sqlx::Error| AppError::database(format!("Failed to finish live read: {e}"));
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::FINISH_LIVE_READ_SQL)
                        .bind(&lease.id)
                        .bind(&lease.token)
                        .fetch_optional(self.pool())
                        .await
                        .map_err(failed)?;
                row.map(|row| {
                    Ok($crate::repositories::activity_backfill_jobs::LiveReadAsk {
                        conversation_id: row.try_get("conversation_id").map_err(failed)?,
                        turn_id: row.try_get("turn_id").map_err(failed)?,
                        asked_message_id: row.try_get("asked_message_id").map_err(failed)?,
                    })
                })
                .transpose()
            }

            async fn end_backfill_job_incomplete(&self, lease: &JobLease) -> AppResult<bool> {
                let ended = sqlx::query(END_BACKFILL_JOB_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(incomplete_outcome())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to end backfill job: {e}")))?;
                Ok(ended.rows_affected() == 1)
            }

            async fn release_backfill_job_claim(
                &self,
                lease: &JobLease,
                pause_until_ms: Option<i64>,
            ) -> AppResult<bool> {
                let released = sqlx::query(RELEASE_BACKFILL_JOB_CLAIM_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(pause_until_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to release backfill job claim: {e}"))
                    })?;
                Ok(released.rows_affected() == 1)
            }

            async fn release_backfill_job_lease(
                &self,
                lease: &JobLease,
                retry_at_ms: i64,
            ) -> AppResult<bool> {
                let released = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::RELEASE_BACKFILL_JOB_LEASE_SQL,
                )
                .bind(&lease.id)
                .bind(&lease.token)
                .bind(retry_at_ms)
                .execute(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to release backfill job lease: {e}"))
                })?;
                Ok(released.rows_affected() == 1)
            }

            async fn take_over_idle_live_read(
                &self,
                ask: &ActivityBackfillJobRow,
            ) -> AppResult<Option<(JobLease, i64)>> {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to take over an idle live read: {e}"))
                };
                let Some(token) = ask.lease_token.clone() else {
                    return Ok(None);
                };
                let row = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::TAKE_OVER_IDLE_LIVE_READ_SQL,
                )
                .bind(ask.user_id.to_string())
                .bind(ask.tenant_id)
                .bind(&ask.provider)
                .bind(BackfillJobKind::LiveRead.as_str())
                .bind(ask.after_ts)
                .bind(ask.before_ts)
                .bind(ask.fetch_limit)
                .bind(&ask.conversation_id)
                .bind(&ask.turn_id)
                .bind(&ask.asked_message_id)
                .bind(ask.leased_until_ms)
                .bind(&token)
                .bind(ask.created_at_ms)
                .fetch_optional(self.pool())
                .await
                .map_err(failed)?;
                row.map(|row| {
                    let id: String = row.try_get("id").map_err(failed)?;
                    let attempts: i64 = row.try_get("attempts").map_err(failed)?;
                    Ok((JobLease { id, token }, attempts))
                })
                .transpose()
            }

            async fn backfill_job_lease_state(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                kind: BackfillJobKind,
                now_ms: i64,
            ) -> AppResult<Option<$crate::repositories::activity_backfill_jobs::JobLeaseState>>
            {
                use $crate::repositories::activity_backfill_jobs::JobLeaseState;
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read backfill job lease: {e}"))
                };
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::BACKFILL_JOB_LEASE_SQL)
                        .bind(user_id.to_string())
                        .bind(tenant_id)
                        .bind(provider)
                        .bind(kind.as_str())
                        .fetch_optional(self.pool())
                        .await
                        .map_err(failed)?;
                row.map(|row| {
                    let leased_until_ms: i64 = row.try_get("leased_until_ms").map_err(failed)?;
                    Ok(if leased_until_ms > now_ms {
                        JobLeaseState::Leased
                    } else {
                        JobLeaseState::Idle
                    })
                })
                .transpose()
            }

            async fn history_walk_row(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<Option<$crate::repositories::activity_backfill_jobs::HistoryWalkRowState>>
            {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read history walk row: {e}"))
                };
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::HISTORY_WALK_ROW_SQL)
                        .bind(user_id.to_string())
                        .bind(tenant_id)
                        .bind(provider)
                        .bind(BackfillJobKind::HistoryWalk.as_str())
                        .fetch_optional(self.pool())
                        .await
                        .map_err(failed)?;
                row.map(|row| {
                    let outcome: Option<String> = row.try_get("outcome").map_err(failed)?;
                    Ok($crate::repositories::activity_backfill_jobs::HistoryWalkRowState {
                        state: backfill_job_state_of(outcome.as_deref())?,
                        walk_backend: row.try_get("walk_backend").map_err(failed)?,
                    })
                })
                .transpose()
            }

            async fn leased_walk_row(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                lease: &JobLease,
                kind: BackfillJobKind,
            ) -> AppResult<Option<$crate::repositories::activity_backfill_jobs::LeasedWalkRow>>
            {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read a leased walk row: {e}"))
                };
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::LEASED_WALK_ROW_SQL)
                        .bind(&lease.id)
                        .bind(user_id.to_string())
                        .bind(tenant_id)
                        .bind(kind.as_str())
                        .fetch_optional(self.pool())
                        .await
                        .map_err(failed)?;
                row.map(|row| {
                    let outcome: Option<String> = row.try_get("outcome").map_err(failed)?;
                    let token: Option<String> = row.try_get("lease_token").map_err(failed)?;
                    Ok($crate::repositories::activity_backfill_jobs::LeasedWalkRow {
                        provider: row.try_get("provider").map_err(failed)?,
                        state: backfill_job_state_of(outcome.as_deref())?,
                        walk_backend: row.try_get("walk_backend").map_err(failed)?,
                        leased_here: token.as_deref() == Some(lease.token.as_str()),
                    })
                })
                .transpose()
            }

            async fn record_job_progress(
                &self,
                lease: &JobLease,
                step: JobProgressStep,
            ) -> AppResult<bool> {
                let recorded = sqlx::query(RECORD_JOB_PROGRESS_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(step.before_ts)
                    .bind(step.units_done)
                    .bind(step.rows_landed)
                    .bind(step.secs_per_unit)
                    .bind(step.floor.map(|floor| floor.after_ts))
                    .bind(step.floor.and_then(|floor| floor.units_total))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record backfill job progress: {e}"))
                    })?;
                Ok(recorded.rows_affected() == 1)
            }

            async fn widen_detail_walk(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                widen: &$crate::repositories::activity_backfill_jobs::WalkWiden,
            ) -> AppResult<bool> {
                let widened =
                    sqlx::query($crate::repositories::activity_backfill_jobs::WIDEN_DETAIL_WALK_SQL)
                        .bind(user_id.to_string())
                        .bind(tenant_id)
                        .bind(provider)
                        .bind(BackfillJobKind::DetailWalk.as_str())
                        .bind(widen.after_ts)
                        .bind(widen.units_left)
                        .bind(&widen.walk_sport)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to widen a detail walk: {e}"))
                        })?;
                Ok(widened.rows_affected() == 1)
            }

            async fn end_detail_walk(
                &self,
                lease: &JobLease,
                end: &$crate::repositories::activity_backfill_jobs::DetailWalkEnd,
            ) -> AppResult<Option<JobProgress>> {
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::END_DETAIL_WALK_SQL)
                        .bind(&lease.id)
                        .bind(&lease.token)
                        .bind(end.before_ts)
                        .bind(end.units_done)
                        .bind(end.rows_landed)
                        .bind(end.secs_per_unit)
                        .bind(end.walked_floor)
                        .bind(walked_outcome())
                        .fetch_optional(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to end a detail walk: {e}"))
                        })?;
                row.as_ref().map(job_progress_from_row).transpose()
            }

            async fn end_history_walk(
                &self,
                lease: &JobLease,
                step: JobProgressStep,
            ) -> AppResult<Option<JobProgress>> {
                let row = sqlx::query(END_HISTORY_WALK_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(step.before_ts)
                    .bind(step.units_done)
                    .bind(step.rows_landed)
                    .bind(step.secs_per_unit)
                    .bind(step.floor.map(|floor| floor.after_ts))
                    .bind(step.floor.and_then(|floor| floor.units_total))
                    .bind(walked_outcome())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to end history walk: {e}")))?;
                row.as_ref().map(job_progress_from_row).transpose()
            }

            async fn hand_off_history_walk(
                &self,
                lease: &JobLease,
                hand_off: &$crate::repositories::activity_backfill_jobs::WalkHandOff,
            ) -> AppResult<bool> {
                let handed = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::HAND_OFF_HISTORY_WALK_SQL,
                )
                .bind(&lease.id)
                .bind(&lease.token)
                .bind(&hand_off.backend)
                .bind(hand_off.before_ts)
                .bind(hand_off.units_done)
                .bind(hand_off.rows_landed)
                .bind(hand_off.fetch_limit)
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to hand off history walk: {e}")))?;
                Ok(handed.rows_affected() == 1)
            }

            async fn reopen_history_walk(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                from: $crate::repositories::activity_backfill_jobs::ReopenFrom,
                hand_off: &$crate::repositories::activity_backfill_jobs::WalkHandOff,
                now_ms: i64,
            ) -> AppResult<bool> {
                let reopened = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::REOPEN_HISTORY_WALK_SQL,
                )
                .bind(user_id.to_string())
                .bind(tenant_id)
                .bind(provider)
                .bind(BackfillJobKind::HistoryWalk.as_str())
                .bind(&hand_off.backend)
                .bind(hand_off.before_ts)
                .bind(hand_off.units_done)
                .bind(hand_off.fetch_limit)
                .bind(from.outcome())
                .bind(now_ms)
                .bind($crate::repositories::activity_backfill_jobs::phase_purged_outcome())
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to reopen history walk: {e}")))?;
                Ok(reopened.rows_affected() == 1)
            }

            async fn settle_purged_history_walk(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                now_ms: i64,
            ) -> AppResult<bool> {
                let settled = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::SETTLE_PURGED_HISTORY_WALK_SQL,
                )
                .bind(user_id.to_string())
                .bind(tenant_id)
                .bind(provider)
                .bind(BackfillJobKind::HistoryWalk.as_str())
                .bind(walked_outcome())
                .bind($crate::repositories::activity_backfill_jobs::phase_purged_outcome())
                .bind(now_ms)
                .execute(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to settle a purged history walk: {e}"))
                })?;
                Ok(settled.rows_affected() == 1)
            }

            async fn purged_history_walks(
                &self,
                now_ms: i64,
                limit: i64,
            ) -> AppResult<Vec<$crate::repositories::activity_backfill_jobs::PurgedHistoryWalk>>
            {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read purged history walks: {e}"))
                };
                let rows = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::PURGED_HISTORY_WALKS_SQL,
                )
                .bind(BackfillJobKind::HistoryWalk.as_str())
                .bind($crate::repositories::activity_backfill_jobs::phase_purged_outcome())
                .bind(now_ms)
                .bind(limit)
                .fetch_all(self.pool())
                .await
                .map_err(failed)?;
                rows.iter()
                    .map(|row| {
                        let user_id: String = row.try_get("user_id").map_err(failed)?;
                        Ok($crate::repositories::activity_backfill_jobs::PurgedHistoryWalk {
                            user_id: Uuid::parse_str(&user_id).map_err(|e| {
                                AppError::database(format!(
                                    "purged history walk user_id is not a UUID: {e}"
                                ))
                            })?,
                            tenant_id: row.try_get("tenant_id").map_err(failed)?,
                            provider: row.try_get("provider").map_err(failed)?,
                        })
                    })
                    .collect()
            }

            async fn job_progress(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                kind: BackfillJobKind,
            ) -> AppResult<Option<JobProgress>> {
                let row = sqlx::query(JOB_PROGRESS_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id)
                    .bind(provider)
                    .bind(kind.as_str())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read backfill job progress: {e}"))
                    })?;
                row.as_ref().map(job_progress_from_row).transpose()
            }

            async fn backfill_job_state(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                kind: BackfillJobKind,
            ) -> AppResult<Option<BackfillJobState>> {
                let row = sqlx::query(BACKFILL_JOB_STATE_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id)
                    .bind(provider)
                    .bind(kind.as_str())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read backfill job state: {e}"))
                    })?;
                row.map(|row| {
                    let outcome: Option<String> = row.try_get("outcome").map_err(|e| {
                        AppError::database(format!("activity_backfill_jobs outcome: {e}"))
                    })?;
                    backfill_job_state_of(outcome.as_deref())
                })
                .transpose()
            }

            async fn reap_exhausted_backfill_jobs(
                &self,
                now_ms: i64,
                max_attempts: i64,
            ) -> AppResult<Vec<ActivityBackfillJobRow>> {
                let rows = sqlx::query(REAP_EXHAUSTED_BACKFILL_JOBS_SQL)
                    .bind(max_attempts)
                    .bind(now_ms)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to reap exhausted backfill jobs: {e}"))
                    })?;
                rows.iter().map(backfill_job_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_activity_backfill_job_repository;
