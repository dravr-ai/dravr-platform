// ABOUTME: The one store read behind every count: civil-year chunks, keyset-paged, gated first, then merged into workouts
// ABOUTME: No row cap; a chunk's slack rows serve the merge only, so a straddling workout and a withheld copy count once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The read.
//!
//! A window is read in civil-year chunks, newest first, each clipped to the
//! window asked ([`chunks`]), so a twelve-week read pages twelve weeks and an
//! all-time read holds one year's rows at a time whatever the history's
//! depth. Each chunk is paged without a row cap, gated ([`ReadGate`]), merged
//! into workouts, and the workouts whose merged start falls in the chunk are
//! handed to the visitor.
//!
//! A chunk is read [`READ_EDGE_SLACK_DAYS`] wider on each side, so the copies
//! of a workout that straddles its edge merge whole. The slack is for the
//! merge only: a row's gate tally and its key's copies count only in the
//! chunk its own start falls in.

use std::collections::BTreeMap;
use std::sync::Arc;

use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp, Zone};
use pierre_core::civil_time::{day_start_utc, local_date};
use pierre_core::errors::AppResult;
use pierre_database::repositories::{CachedActivityRow, CachedRowCursor, CachedRowsQuery};
use pierre_providers::ai_scope;
use pierre_services::workouts::{filter_activity_rows, served_rows, workouts, Workout};

use super::{segment_keys, Athlete, ReadGate};
use crate::runtime::ToolRuntime;

/// Stored rows one page of a chunk takes.
const STORE_PAGE_ROWS: i64 = 500;

/// Days a chunk's read reaches past each edge so the copies of a workout
/// that straddles it merge whole: the slack the training history's window
/// read takes, for the same reason.
const READ_EDGE_SLACK_DAYS: i64 = 2;

/// The window a read covers, on the athlete's calendar.
#[derive(Debug, Clone)]
pub struct CivilWindow {
    /// The first day, included; `None` reads from the oldest stored start
    /// under the keys read.
    pub from: Option<CivilDate>,
    /// The last day, included.
    pub to: CivilDate,
    /// The athlete's zone.
    pub zone: Zone,
}

/// The stored keys a read takes rows under.
#[derive(Debug, Clone, Copy)]
pub enum KeySet<'keys> {
    /// Every key the athlete holds rows under.
    All,
    /// These keys alone.
    Only(&'keys [&'keys str]),
}

/// One stored key's part of a read: its rows whose own start is in the
/// window, served to this reader, before the merge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyReach {
    /// Rows read.
    pub copies: u64,
    /// The oldest row's civil day.
    pub oldest: Option<CivilDate>,
    /// The newest row's civil day.
    pub newest: Option<CivilDate>,
}

impl KeyReach {
    fn add(&mut self, day: CivilDate) {
        self.copies = self.copies.saturating_add(1);
        self.oldest = Some(self.oldest.map_or(day, |oldest| oldest.min(day)));
        self.newest = Some(self.newest.map_or(day, |newest| newest.max(day)));
    }
}

/// What a read reached.
#[derive(Debug, Clone, Default)]
pub struct ReadReach {
    /// Each key's rows read, by key.
    pub by_key: BTreeMap<String, KeyReach>,
    /// Whether the gate withheld any row under a key a detail walk reads
    /// segments from ([`segment_keys`]).
    pub segment_rows_withheld: bool,
}

impl ReadReach {
    /// The oldest civil day any row read starts on.
    #[must_use]
    pub fn oldest(&self) -> Option<CivilDate> {
        self.by_key.values().filter_map(|reach| reach.oldest).min()
    }
}

/// The civil-year chunks of `[floor, to]`, newest first, each clipped to it.
pub(crate) fn chunks(floor: CivilDate, to: CivilDate) -> Vec<(CivilDate, CivilDate)> {
    let mut out = Vec::new();
    if floor > to {
        return out;
    }
    let mut year = to.year();
    while year >= floor.year() {
        let (Some(first), Some(last)) = (
            CivilDate::from_ymd(year, 1, 1).ok(),
            CivilDate::from_ymd(year, 12, 31).ok(),
        ) else {
            break;
        };
        out.push((first.max(floor), last.min(to)));
        year -= 1;
    }
    out
}

/// The instants a chunk `[first, last]` reads, widened by the slack on each
/// side; the end is the last instant before the day after `last` (plus the
/// slack) begins. At either end of the calendar the slack stops there.
pub(crate) fn read_bounds(
    (first, last): (CivilDate, CivilDate),
    zone: &Zone,
) -> (Timestamp, Timestamp) {
    let slack = Elapsed::days(READ_EDGE_SLACK_DAYS);
    let start = day_start_utc(first, zone)
        .checked_sub(slack)
        .unwrap_or(Timestamp::MIN);
    let end = last
        .succ()
        .map_or_else(Timestamp::now, |next| day_start_utc(next, zone))
        .checked_add(slack - Elapsed::nanoseconds(1))
        .unwrap_or(Timestamp::MAX);
    (start, end)
}

/// Whether a start falls in the chunk `[first, last]` on the athlete's
/// calendar.
pub(crate) fn in_chunk(
    start: Timestamp,
    (first, last): (CivilDate, CivilDate),
    zone: &Zone,
) -> bool {
    (first..=last).contains(&local_date(start, zone))
}

/// The oldest civil day a row under `keys` starts on, or `None` when they
/// hold none.
pub(crate) async fn oldest_stored(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    keys: KeySet<'_>,
    zone: &Zone,
) -> AppResult<Option<CivilDate>> {
    let cache = &runtime.repos().activity_cache;
    let held: Vec<String> = match keys {
        KeySet::All => cache
            .count_by_provider(athlete.user_id, &athlete.tenant_id)
            .await?
            .into_iter()
            .map(|(key, _)| key)
            .collect(),
        KeySet::Only(keys) => keys.iter().map(|key| (*key).to_owned()).collect(),
    };
    let mut oldest: Option<Timestamp> = None;
    for key in &held {
        if let Some(start) = cache
            .oldest_activity_start(athlete.user_id, &athlete.tenant_id, key)
            .await?
        {
            oldest = Some(oldest.map_or(start, |seen| seen.min(start)));
        }
    }
    Ok(oldest.map(|start| local_date(start, zone)))
}

/// The days a read pages: the window, narrowed to the stored history when it
/// spans more than one civil year, so a window reaching past either end of
/// that history never pages the empty years beyond it (a date argument may
/// name any year the calendar holds). `None` when the window can hold no
/// stored row.
async fn paged_span(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    window: CivilWindow,
    keys: KeySet<'_>,
) -> AppResult<Option<(CivilDate, CivilDate)>> {
    let floor = match window.from {
        Some(from) => from,
        None => match oldest_stored(runtime, athlete, keys, &window.zone).await? {
            Some(oldest) => oldest,
            None => return Ok(None),
        },
    };
    if floor > window.to {
        return Ok(None);
    }
    if floor.year() == window.to.year() {
        return Ok(Some((floor, window.to)));
    }
    let oldest = match window.from {
        Some(_) => oldest_stored(runtime, athlete, keys, &window.zone).await?,
        None => Some(floor),
    };
    // The newest row under any key bounds the newest under these keys.
    let newest = runtime
        .repos()
        .activity_cache
        .latest_activity_start(athlete.user_id, &athlete.tenant_id)
        .await?
        .map(|start| local_date(start, &window.zone));
    let (Some(oldest), Some(newest)) = (oldest, newest) else {
        return Ok(None);
    };
    let (first, last) = (floor.max(oldest), window.to.min(newest));
    Ok((first <= last).then_some((first, last)))
}

/// Every stored row under `keys` in `[start, end]`, newest first, paged.
async fn chunk_rows(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    keys: KeySet<'_>,
    (start, end): (Timestamp, Timestamp),
) -> AppResult<Vec<CachedActivityRow>> {
    let mut rows = Vec::new();
    let mut below: Option<CachedRowCursor> = None;
    loop {
        let mut query = CachedRowsQuery::window(start, end, STORE_PAGE_ROWS);
        if let KeySet::Only(keys) = keys {
            query = query.under_keys(keys);
        }
        if let Some(cursor) = below.take() {
            query = query.below(cursor);
        }
        let page = runtime
            .repos()
            .activity_cache
            .get_cached_activity_rows(athlete.user_id, &athlete.tenant_id, &query)
            .await?;
        let last_page = i64::try_from(page.len()).unwrap_or(i64::MAX) < STORE_PAGE_ROWS;
        below = page.last().map(CachedRowCursor::after);
        rows.extend(page);
        if last_page || below.is_none() {
            return Ok(rows);
        }
    }
}

/// The rows of one chunk a reader is served: its own rows, gated in a
/// scope whose tally is recorded, then its slack rows, gated in one whose
/// tally is dropped, so a row in two chunks' slack is tallied once. Whether
/// a row under a segment key was withheld is told apart.
async fn gated(
    runtime: &Arc<dyn ToolRuntime>,
    gate: ReadGate,
    own: Vec<CachedActivityRow>,
    slack: Vec<CachedActivityRow>,
) -> (Vec<CachedActivityRow>, bool) {
    let terms = runtime.provider_registry().as_ref();
    match gate {
        ReadGate::Display => {
            let mut kept = served_rows(terms, own);
            kept.extend(served_rows(terms, slack));
            (kept, false)
        }
        ReadGate::Model | ReadGate::Roster => {
            let keys = segment_keys();
            let (segment_rows, other_rows): (Vec<_>, Vec<_>) = own
                .into_iter()
                .partition(|row| keys.contains(&row.provider.as_str()));
            let (mut kept, segment_tally) =
                ai_scope::tallied(async { filter_activity_rows(terms, segment_rows) }).await;
            let segment_withheld = !segment_tally.is_empty();
            ai_scope::record_withheld(segment_tally);
            let (other_kept, other_tally) =
                ai_scope::tallied(async { filter_activity_rows(terms, other_rows) }).await;
            ai_scope::record_withheld(other_tally);
            kept.extend(other_kept);
            let (slack_kept, _) =
                ai_scope::tallied(async { filter_activity_rows(terms, slack) }).await;
            kept.extend(slack_kept);
            (kept, segment_withheld)
        }
    }
}

/// The one store read: every chunk's gated, merged workouts, newest chunk
/// first, each workout handed to `visit` once, in the chunk its merged
/// start falls in.
///
/// # Errors
///
/// Returns the store's error when a read fails.
pub async fn visit_stored_workouts(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    window: CivilWindow,
    keys: KeySet<'_>,
    gate: ReadGate,
    visit: &mut (dyn FnMut(&[Workout<'_>]) + Send),
) -> AppResult<ReadReach> {
    let mut reach = ReadReach::default();
    let Some((first, last)) = paged_span(runtime, athlete, window.clone(), keys).await? else {
        return Ok(reach);
    };
    for chunk in chunks(first, last) {
        let rows = chunk_rows(runtime, athlete, keys, read_bounds(chunk, &window.zone)).await?;
        let (own, slack): (Vec<_>, Vec<_>) = rows
            .into_iter()
            .partition(|row| in_chunk(row.activity.start_date(), chunk, &window.zone));
        let (kept, segment_withheld) = gated(runtime, gate, own, slack).await;
        reach.segment_rows_withheld |= segment_withheld;
        let own_kept = kept
            .iter()
            .filter(|row| in_chunk(row.activity.start_date(), chunk, &window.zone));
        for row in own_kept {
            reach
                .by_key
                .entry(row.provider.clone())
                .or_default()
                .add(local_date(row.activity.start_date(), &window.zone));
        }
        let merged: Vec<Workout<'_>> = workouts(&kept)
            .into_iter()
            .filter(|workout| in_chunk(workout.session.start_date(), chunk, &window.zone))
            .collect();
        visit(&merged);
    }
    Ok(reach)
}

#[cfg(test)]
mod tests {
    use pierre_core::civil_time::resolve_zone;

    use super::*;

    fn ymd(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate::from_ymd(year, month, day)
            .ok()
            .unwrap_or(CivilDate::MIN)
    }

    #[test]
    fn a_window_is_read_in_civil_years_newest_first_clipped_to_it() {
        assert_eq!(
            chunks(ymd(2024, 3, 5), ymd(2026, 2, 1)),
            vec![
                (ymd(2026, 1, 1), ymd(2026, 2, 1)),
                (ymd(2025, 1, 1), ymd(2025, 12, 31)),
                (ymd(2024, 3, 5), ymd(2024, 12, 31)),
            ],
            "bug: the chunks are not the window's civil years, clipped, newest first"
        );
        assert_eq!(
            chunks(ymd(2026, 8, 24), ymd(2026, 11, 16)),
            vec![(ymd(2026, 8, 24), ymd(2026, 11, 16))],
            "bug: twelve weeks are not read as one twelve-week chunk"
        );
        assert!(
            chunks(ymd(2026, 2, 1), ymd(2026, 1, 1)).is_empty(),
            "bug: an inverted window reads a chunk"
        );
    }

    #[test]
    fn a_chunk_reads_two_days_past_each_edge_on_the_athletes_clock() {
        let montreal = resolve_zone(Some("America/Montreal"));
        let (start, end) = read_bounds((ymd(2025, 1, 1), ymd(2025, 12, 31)), &montreal);
        assert_eq!(
            start,
            Timestamp::parse_from_rfc3339("2024-12-30T05:00:00Z")
                .ok()
                .unwrap_or_default(),
            "bug: the read does not start two days before Montreal's new year"
        );
        assert_eq!(
            end + Elapsed::nanoseconds(1),
            Timestamp::parse_from_rfc3339("2026-01-03T05:00:00Z")
                .ok()
                .unwrap_or_default(),
            "bug: the read does not end two days after Montreal's new year's eve"
        );
    }

    #[test]
    fn a_chunk_on_the_calendars_first_day_reads_from_its_first_instant() {
        let tokyo = resolve_zone(Some("Asia/Tokyo"));
        let (start, _) = read_bounds((CivilDate::MIN, CivilDate::MIN), &tokyo);
        assert_eq!(
            start,
            Timestamp::MIN,
            "bug: the slack before the calendar's first day is not cut at its first instant"
        );
    }

    #[test]
    fn a_late_ride_on_new_years_eve_is_in_the_old_years_chunk() {
        let montreal = resolve_zone(Some("America/Montreal"));
        // 23:30 on Dec 31 in Montreal is 04:30 UTC on Jan 1.
        let ride = Timestamp::parse_from_rfc3339("2026-01-01T04:30:00Z")
            .ok()
            .unwrap_or_default();
        assert!(
            in_chunk(ride, (ymd(2025, 1, 1), ymd(2025, 12, 31)), &montreal),
            "bug: a ride at 23:30 local on Dec 31 is not in its own year"
        );
        assert!(
            !in_chunk(ride, (ymd(2026, 1, 1), ymd(2026, 12, 31)), &montreal),
            "bug: a ride at 23:30 local on Dec 31 counts in the next year"
        );
    }
}
