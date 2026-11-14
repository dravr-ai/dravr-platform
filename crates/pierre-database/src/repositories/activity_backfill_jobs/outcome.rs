// ABOUTME: The outcome column of a job row: owed while NULL, else how the row ended or what a purge left it owing
// ABOUTME: One decoder of the column, and the values each state is stored as, for both backends' statements

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use super::BackfillJobState;

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
