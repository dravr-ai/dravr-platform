// ABOUTME: Which cached rows a rows read takes: a window, the keys it names, and the keyset cursor it pages below
// ABOUTME: Re-exported from activity_cache, whose get_cached_activity_rows reads them newest first

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};

#[cfg(doc)]
use super::ActivityCacheRepository;
use super::CachedActivityRow;

/// Which cached rows [`ActivityCacheRepository::get_cached_activity_rows`]
/// reads: the user's rows in the tenant whose start lies in `[start, end]`,
/// newest first, at most `limit` of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedRowsQuery<'a> {
    /// The window's oldest start.
    pub start: DateTime<Utc>,
    /// The window's newest start.
    pub end: DateTime<Utc>,
    /// At most this many rows.
    pub limit: i64,
    /// Only rows filed under these provider keys; `None` reads every key,
    /// and an empty set reads none.
    pub keys: Option<&'a [&'a str]>,
    /// Only rows that come after this one in the read's order: the next page.
    pub below: Option<CachedRowCursor>,
}

impl<'a> CachedRowsQuery<'a> {
    /// Every key's rows in `[start, end]`, from the newest, at most `limit`.
    #[must_use]
    pub const fn window(start: DateTime<Utc>, end: DateTime<Utc>, limit: i64) -> Self {
        Self {
            start,
            end,
            limit,
            keys: None,
            below: None,
        }
    }

    /// The same read over the rows filed under `keys` only.
    #[must_use]
    pub const fn under_keys(mut self, keys: &'a [&'a str]) -> Self {
        self.keys = Some(keys);
        self
    }

    /// The same read's next page: the rows after `cursor`.
    #[must_use]
    pub fn below(mut self, cursor: CachedRowCursor) -> Self {
        self.below = Some(cursor);
        self
    }
}

/// Where a page of a rows read ended.
///
/// The read's order is start, then provider key, then activity id, each
/// descending. The key and the id break
/// the tie between rows that share a start, the two copies of one activity a
/// session and a grant file under their own keys among them, so a page's edge
/// never skips or repeats one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedRowCursor {
    /// The last row's start.
    pub start_date: DateTime<Utc>,
    /// The key the last row is filed under.
    pub provider: String,
    /// The last row's activity id.
    pub activity_id: String,
}

impl CachedRowCursor {
    /// The cursor the next page reads below: just after `row`.
    #[must_use]
    pub fn after(row: &CachedActivityRow) -> Self {
        Self {
            start_date: row.activity.start_date(),
            provider: row.provider.clone(),
            activity_id: row.activity.id().to_owned(),
        }
    }
}
