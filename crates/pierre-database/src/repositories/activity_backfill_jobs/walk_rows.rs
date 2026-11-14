// ABOUTME: How the ledger reads a history walk's row: its state, a purged walk to resolve, the row a step's lease took
// ABOUTME: And what a reopen of its API phase starts from, so a walk a whole-provider purge settled is never reopened as purged

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::TenantId;
use uuid::Uuid;

use super::{phase_purged_outcome, walked_outcome, BackfillJobState};

/// What the ledger holds for one history walk row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryWalkRowState {
    /// Owed, or how it ended.
    pub state: BackfillJobState,
    /// The API grant the walk reads once its session handed off
    /// ([`super::WalkHandOff`]); `None` while it reads the key it is filed under.
    pub walk_backend: Option<String>,
}

/// A history walk in [`BackfillJobState::PhasePurged`] no step holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgedHistoryWalk {
    /// Athlete whose walk it is.
    pub user_id: Uuid,
    /// Tenant the row is filed under.
    pub tenant_id: TenantId,
    /// The key the walk is filed under: its session.
    pub provider: String,
}

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

/// The history walk row a step's lease was taken on, as it stands now
/// ([`super::ActivityBackfillJobRepository::leased_walk_row`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasedWalkRow {
    /// The key the walk is filed under: its session, or the grant walked
    /// alone.
    pub provider: String,
    /// Owed, or how it ended or was settled.
    pub state: BackfillJobState,
    /// The API grant the walk reads once its session handed off.
    pub walk_backend: Option<String>,
    /// Whether the step's lease still holds the row.
    pub leased_here: bool,
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
