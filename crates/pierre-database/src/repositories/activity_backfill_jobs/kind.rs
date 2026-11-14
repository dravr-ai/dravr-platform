// ABOUTME: The kinds of job the activity backfill ledger runs, and the SQL list the claim and the reap filter on
// ABOUTME: A kind a newer build wrote is left to that build, so a rolling deploy never fails a shared batch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use std::fmt;

/// The kind of job one ledger row runs.
///
/// Each kind is its own row beside the others for one `(user, provider)`: a
/// history walk, a live read and a backfill never share a slot. Stored as
/// [`Self::as_str`]; a value this build does not know is a decode error,
/// never a silent default, which is why the claim and the reap take only
/// [`known_backfill_job_kinds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackfillJobKind {
    /// A chat's or the capture rail's deep window.
    Backfill,
    /// The whole-history walk, back to the account's floor.
    HistoryWalk,
    /// A chat read that outran its turn budget. A newer ask while it is owed
    /// rebinds the turn it answers instead of being refused.
    LiveRead,
    /// The walk that reads, newest first, the stored activities of a window
    /// whose segment efforts were never read, through a backend that reads
    /// them. Its unit is an activity, its cursor the row's `before_ts`.
    DetailWalk,
}

impl BackfillJobKind {
    /// Every kind this build runs.
    pub const ALL: [Self; 4] = [
        Self::Backfill,
        Self::HistoryWalk,
        Self::LiveRead,
        Self::DetailWalk,
    ];

    /// The `kind` column value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backfill => "backfill",
            Self::HistoryWalk => "history_walk",
            Self::LiveRead => "live_read",
            Self::DetailWalk => "detail_walk",
        }
    }

    /// The kind a stored `kind` value names.
    ///
    /// # Errors
    ///
    /// A value this build does not know.
    pub fn parse(stored: &str) -> AppResult<Self> {
        match stored {
            "backfill" => Ok(Self::Backfill),
            "history_walk" => Ok(Self::HistoryWalk),
            "live_read" => Ok(Self::LiveRead),
            "detail_walk" => Ok(Self::DetailWalk),
            other => Err(AppError::database(format!(
                "activity_backfill_jobs kind is not one this build knows: {other}"
            ))),
        }
    }
}

/// The `kind` values this build runs, as an SQL list. The claim and the reap
/// take only these, so a row a newer build wrote during a rolling deploy is
/// left for that build instead of failing the whole batch it was claimed
/// with. [`BackfillJobKind::ALL`] and the test beside it keep the two in step.
macro_rules! known_backfill_job_kinds {
    () => {
        "('backfill', 'history_walk', 'live_read', 'detail_walk')"
    };
}
pub(crate) use known_backfill_job_kinds;

impl fmt::Display for BackfillJobKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::BackfillJobKind;

    /// The claim and the reap filter on the SQL list, so a kind added to the
    /// enum without the list would be recorded and never run.
    #[test]
    fn the_sql_kind_list_names_every_kind_this_build_runs() {
        let listed: Vec<&str> = known_backfill_job_kinds!()
            .trim_matches(|c| c == '(' || c == ')')
            .split(", ")
            .map(|quoted| quoted.trim_matches('\''))
            .collect();
        let runs: Vec<&str> = BackfillJobKind::ALL
            .iter()
            .map(|kind| kind.as_str())
            .collect();
        assert_eq!(
            listed, runs,
            "bug: the SQL kind list drifted from the kinds this build runs"
        );
        for kind in BackfillJobKind::ALL {
            let parsed = BackfillJobKind::parse(kind.as_str())
                .expect("bug: a kind this build runs does not parse back");
            assert_eq!(parsed, kind, "bug: {kind} does not round-trip");
            match kind {
                // A new variant fails this match until ALL and the list name it.
                BackfillJobKind::Backfill
                | BackfillJobKind::HistoryWalk
                | BackfillJobKind::LiveRead
                | BackfillJobKind::DetailWalk => {}
            }
        }
    }
}
