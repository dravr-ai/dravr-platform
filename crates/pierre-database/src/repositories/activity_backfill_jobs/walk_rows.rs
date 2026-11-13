// ABOUTME: What a history walk's API phase is reopened from: a session walked alone, or a walk a purge left owed
// ABOUTME: Each reopen names which, so a walk a whole-provider purge settled is never reopened as a purged one

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::{phase_purged_outcome, walked_outcome};

/// Which history walk row a reopen of its API phase may take
/// ([`super::ActivityBackfillJobRepository::reopen_history_walk`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReopenFrom {
    /// A session walked alone, with no API phase on it
    /// ([`super::BackfillJobState::Walked`], no grant named): a grant now
    /// live beside it is owed that phase.
    WalkedAlone,
    /// A walk a purge of its grant left owed a reopen
    /// ([`super::BackfillJobState::PhasePurged`]), and only while it still
    /// is: one a whole-provider purge has settled since is walked alone, and
    /// its resolution leaves it alone.
    PhasePurged,
}

impl ReopenFrom {
    /// The `outcome` the row to reopen carries.
    pub(crate) const fn outcome(self) -> &'static str {
        match self {
            Self::WalkedAlone => walked_outcome(),
            Self::PhasePurged => phase_purged_outcome(),
        }
    }
}
