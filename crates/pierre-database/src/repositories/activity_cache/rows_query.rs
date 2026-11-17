// ABOUTME: Which cached rows a rows read takes: a window, the keys and activity ids it names, and the cursor it pages below
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
pub struct CachedRowsQuery<'keys> {
    /// The window's oldest start.
    pub start: DateTime<Utc>,
    /// The window's newest start.
    pub end: DateTime<Utc>,
    /// At most this many rows.
    pub limit: i64,
    /// Only rows filed under these provider keys; `None` reads every key,
    /// and an empty set reads none.
    pub keys: Option<&'keys [&'keys str]>,
    /// Only rows of these activity ids, whatever key each is filed under;
    /// `None` reads every id, and an empty set reads none.
    pub ids: Option<&'keys [&'keys str]>,
    /// Only rows that come after this one in the read's order: the next page.
    pub below: Option<CachedRowCursor>,
}

/// The first instant both engines store and order correctly:
/// 0001-01-01T00:00:00Z. PostgreSQL refuses a `timestamptz` before 4713 BC
/// ("timestamp out of range"), and SQLite compares the RFC 3339 text, which
/// sorts wrongly outside four-digit years.
const STORABLE_FIRST_SECS: i64 = -62_135_596_800;

/// The last second both engines store and order correctly:
/// 9999-12-31T23:59:59Z.
const STORABLE_LAST_SECS: i64 = 253_402_300_799;

/// `instant`, moved inside the span both engines store, so an open-ended
/// window (a chrono bound like `DateTime::<Utc>::MIN_UTC`) reads every row
/// instead of failing on PostgreSQL.
fn storable(instant: DateTime<Utc>) -> DateTime<Utc> {
    let first =
        DateTime::from_timestamp(STORABLE_FIRST_SECS, 0).unwrap_or(DateTime::<Utc>::MIN_UTC);
    let last = DateTime::from_timestamp(STORABLE_LAST_SECS, 0).unwrap_or(DateTime::<Utc>::MAX_UTC);
    instant.clamp(first, last)
}

impl<'keys> CachedRowsQuery<'keys> {
    /// Every key's rows in `[start, end]`, from the newest, at most `limit`.
    /// Bounds past what the store can hold are moved to its edges, which
    /// selects the same rows.
    #[must_use]
    pub fn window(start: DateTime<Utc>, end: DateTime<Utc>, limit: i64) -> Self {
        Self {
            start: storable(start),
            end: storable(end),
            limit,
            keys: None,
            ids: None,
            below: None,
        }
    }

    /// The same read over the rows filed under `keys` only.
    #[must_use]
    pub const fn under_keys(mut self, keys: &'keys [&'keys str]) -> Self {
        self.keys = Some(keys);
        self
    }

    /// The same read over the rows of `ids` only: every copy of each named
    /// activity, whatever key it is filed under, and no other row.
    #[must_use]
    pub const fn with_ids(mut self, ids: &'keys [&'keys str]) -> Self {
        self.ids = Some(ids);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_ended_window_reads_inside_what_both_engines_store() {
        let query = CachedRowsQuery::window(DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC, 10);
        assert_eq!(
            query.start.to_rfc3339(),
            "0001-01-01T00:00:00+00:00",
            "bug: a window from chrono's first instant reaches PostgreSQL out of range"
        );
        assert_eq!(
            query.end.to_rfc3339(),
            "9999-12-31T23:59:59+00:00",
            "bug: a window to chrono's last instant reaches the store out of range"
        );
        let now = Utc::now();
        assert_eq!(
            CachedRowsQuery::window(now, now, 1).start,
            now,
            "bug: an ordinary bound is moved"
        );
    }
}
