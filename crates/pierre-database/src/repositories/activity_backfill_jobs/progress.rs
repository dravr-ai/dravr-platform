// ABOUTME: How far a job row has got, and what one step adds to it: units done, rows landed, the pace, the floor learned
// ABOUTME: Re-exported from activity_backfill_jobs, whose record_job_progress writes a step and whose reads report it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#[cfg(doc)]
use super::{ActivityBackfillJobRepository, BackfillJobKind};

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
