// ABOUTME: What a merged wellness read asks for: which sources take part, and what of each row it keeps
// ABOUTME: The read's vocabulary only; every read itself stays in stored_health.rs, the one merge site

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Which stored rows a merged read takes part in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFilter<'a> {
    /// Every source the gates in force let the reader see, merged.
    All,
    /// Only the rows one source synced (the sleep tools' `sleep_provider`
    /// argument, matched without regard to case), merged.
    Service(&'a str),
    /// Only the rows one linked platform filed, each returned as it was
    /// stored and never merged with another: a coach link reads what that
    /// platform holds, not the athlete's other sources.
    Link(&'a str),
}

impl SourceFilter<'_> {
    /// Whether a row synced by `source_name` takes part.
    pub(super) fn admits(self, source_name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Service(source) | Self::Link(source) => source_name.eq_ignore_ascii_case(source),
        }
    }
}

/// What a merged read keeps of each stored row, before the rows merge: the
/// row with only what the reader may see, or `None` to leave it out.
///
/// Applied before the merge because a merged record cannot be cut down
/// afterwards: a primary picked from a row the reader may not see carries
/// that row's id, source and time into the record, and the next source's are
/// gone. [`as_stored`] keeps every row whole.
pub type RowProjection<T> = fn(T) -> Option<T>;

/// Each row as it was stored: the [`RowProjection`] of a read that shows the
/// athlete's records whole.
#[must_use]
pub const fn as_stored<T>(row: T) -> Option<T> {
    Some(row)
}
