// ABOUTME: The outcome column of a job row: owed while NULL, else how the row ended or what a purge left it owing
// ABOUTME: The states a row reads as, one decoder of the column, and the values each state is stored as

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

/// What the ledger holds for one `(user, provider, kind)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillJobState {
    /// A job is owed: recorded, running or waiting for the resume sweep.
    Owed,
    /// The job ended before what it was asked for landed, and its row was
    /// kept to say so ([`super::ActivityBackfillJobRepository::end_backfill_job_incomplete`]):
    /// a history walk whose session expired mid-copy.
    Incomplete,
    /// A walk reached the end of what it walks, and its row was kept to say
    /// so: a history walk the end of the athlete's history
    /// ([`super::ActivityBackfillJobRepository::end_history_walk`]), a detail walk
    /// its window's floor ([`super::ActivityBackfillJobRepository::end_detail_walk`]).
    Walked,
    /// A purge of the grant a history walk's API phase was paging took that
    /// phase while it was owed. The row is still the walk of the pair: a step
    /// may still hold its lease and be paging the grant, so no walk of the
    /// grant starts beside it, and the step records nothing on the row and
    /// leaves none of what it read in the store (it checks the row around its
    /// write, [`super::ActivityBackfillJobRepository::leased_walk_row`]). Once no
    /// step holds the lease, the phase is reopened below the session's own
    /// rows when a grant is live, or the row is kept as
    /// [`Self::Walked`] when none is
    /// ([`super::ActivityBackfillJobRepository::reopen_history_walk`],
    /// [`super::ActivityBackfillJobRepository::settle_purged_history_walk`]); the
    /// resume sweep does it for a row whose step died
    /// ([`super::ActivityBackfillJobRepository::purged_history_walks`]). A
    /// whole-provider purge of that grant settles it as walked at once.
    PhasePurged,
}

/// The `outcome` an ended row carries in [`BackfillJobState::Incomplete`].
const OUTCOME_INCOMPLETE: &str = "incomplete";

/// The `outcome` an ended row carries in [`BackfillJobState::Walked`].
const OUTCOME_WALKED: &str = "walked";

/// The `outcome` a row carries in [`BackfillJobState::PhasePurged`]. Not an
/// end: only the walk's resolution moves it on, never a step's write, the
/// claim or the reap.
const OUTCOME_PHASE_PURGED: &str = "phase_purged";

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
