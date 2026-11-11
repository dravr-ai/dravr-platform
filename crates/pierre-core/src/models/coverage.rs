// ABOUTME: Whether a served activity window is the whole truth for the window that was asked
// ABOUTME: An incomplete window names why, so no count or "nothing before" is stated from it as fact

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Window completeness.
//!
//! A count of activities is only as true as the window it was counted over.
//! The live replay of 2026-11-09 counted 43 trail runs since 2020 from a
//! 151-row cached window served while the live read ran on, and 599 four
//! minutes later from the 2,000 rows the read stopped at; each was stated as
//! fact. Every served window therefore says whether it is complete, and when
//! it is not, why.

use serde::Serialize;

/// Why a served window is not the whole truth for the window asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncompleteReason {
    /// The live read outlived the turn's budget; the cached window answered
    /// and the read runs on.
    LiveReadRunning,
    /// The read stopped at its row cap, so older activities were not read.
    ReadCapped,
    /// The provider could not be read; what it last synced answered.
    ProviderUnavailable,
}

impl IncompleteReason {
    /// The sentence a model reads after "This window is incomplete:".
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::LiveReadRunning => {
                "the live read of the provider is still running, so only the activities already synced are shown; the rest lands when it finishes"
            }
            Self::ReadCapped => {
                "the read stopped at its row cap, so activities older than the oldest shown were not read"
            }
            Self::ProviderUnavailable => {
                "the provider could not be read, so only what it last synced is shown"
            }
        }
    }
}

/// Whether a served window is the whole truth for the window asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// Every activity of the asked window was read.
    Complete,
    /// The served window is partial, for this reason.
    Incomplete(IncompleteReason),
}

impl Completeness {
    /// Whether the window is complete.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }

    /// Why the window is incomplete, when it is.
    #[must_use]
    pub const fn reason(self) -> Option<IncompleteReason> {
        match self {
            Self::Complete => None,
            Self::Incomplete(reason) => Some(reason),
        }
    }
}
