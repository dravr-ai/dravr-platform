// ABOUTME: Which stored activities a detail walk reads: a window's rows taken by whole seconds, newest first
// ABOUTME: Of the walk's sport, the copies of one activity as one, only those no copy of which holds its efforts

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    CachedActivityRow, CachedRowCursor, CachedRowsQuery, WalkTried,
};
use pierre_providers::deduplication::one_activity_key;
use tracing::warn;
use uuid::Uuid;

use super::DETAIL_WALK_STEP_ACTIVITIES;
use crate::implementations::fitness_support::filter_activities_by_sport_type;
use crate::runtime::ToolRuntime;

/// Stored rows one read of the window takes.
const WALK_PAGE_ROWS: i64 = 200;

/// Reads of the window one step makes looking for activities to read before
/// it records where it got to.
const WALK_PAGES_PER_STEP: usize = 20;

/// Seconds in a day, as a span of starts.
const DAY_SECS: i64 = 86_400;

/// Stored rows one page of the look for the other copies of a step's
/// activities takes: the look reads every row a day either side of the
/// activities it reads, a page at a time.
const TWIN_LOOKUP_ROWS: i64 = 1_000;

/// The athlete a walk reads for, and the runtime it reads through.
pub(super) struct Athlete<'run> {
    pub(super) runtime: &'run Arc<dyn ToolRuntime>,
    pub(super) user_id: Uuid,
    pub(super) tenant_id: TenantId,
}

/// One activity of a second of the window: its copies under the walk's keys.
#[derive(Debug)]
pub(super) struct Unread {
    pub(super) copies: Vec<CachedActivityRow>,
}

impl Unread {
    /// When its first copy among those of its second reached the store.
    fn first_stored_ms(&self) -> Option<i64> {
        self.copies.iter().map(stored_ms).fold(None, earliest)
    }
}

/// The activities started within one second of the window whose efforts no
/// copy holds.
#[derive(Debug)]
pub(super) struct Second {
    /// The second (unix seconds).
    pub(super) at: i64,
    /// Its activities to read.
    pub(super) unread: Vec<Unread>,
}

/// The rows of the walk's sport that started in one second of the window.
type SecondRows = (i64, Vec<CachedActivityRow>);

/// One read of the window's stored rows below `top`: its rows of the walk's
/// sport by the second they started in, whole seconds only, newest first,
/// and whether it reached the window's floor.
#[derive(Debug)]
struct Page {
    seconds: Vec<SecondRows>,
    reached_floor: bool,
}

impl Athlete<'_> {
    /// The athlete's rows under `keys` started in `[floor, top)`, newest
    /// first, at most `limit`, after `below` when a page ended there.
    async fn rows_between(
        &self,
        keys: &[&str],
        (floor, top): (i64, i64),
        limit: i64,
        below: Option<CachedRowCursor>,
    ) -> AppResult<Vec<CachedActivityRow>> {
        let (Some(start), Some(end)) = (
            Utc.timestamp_opt(floor, 0).single(),
            Utc.timestamp_opt(top, 0).single(),
        ) else {
            return Err(AppError::invalid_input(format!(
                "a detail walk window [{floor}, {top}) is out of range"
            )));
        };
        // `top` is excluded: the read's end is inclusive.
        let end: DateTime<Utc> = end - Duration::nanoseconds(1);
        if end < start {
            return Ok(Vec::new());
        }
        let query = CachedRowsQuery::window(start, end, limit).under_keys(keys);
        let query = match below {
            Some(cursor) => query.below(cursor),
            None => query,
        };
        self.runtime
            .repos()
            .activity_cache
            .get_cached_activity_rows(self.user_id, &self.tenant_id, &query)
            .await
    }

    /// Every activity under `keys` started in `[floor, top)` a copy of
    /// which holds its efforts, read a page at a time down to `floor`.
    async fn read_between(
        &self,
        keys: &[&str],
        window: (i64, i64),
    ) -> AppResult<HashSet<NativeKey>> {
        let mut read = HashSet::new();
        let mut below = None;
        loop {
            let rows = self
                .rows_between(keys, window, TWIN_LOOKUP_ROWS, below)
                .await?;
            let full = i64::try_from(rows.len()).unwrap_or(i64::MAX) >= TWIN_LOOKUP_ROWS;
            read.extend(rows.iter().filter(|row| row.segment_read).map(identity));
            match rows.last() {
                Some(last) if full => below = Some(CachedRowCursor::after(last)),
                _ => return Ok(read),
            }
        }
    }

    /// Read the athlete's rows under `keys` started in `[floor, top)`, at
    /// most [`WALK_PAGE_ROWS`], as whole seconds: a full page's oldest second
    /// may hold more rows past its edge, so it is left to the next read,
    /// unless it is the page's only one. Only rows of `sport` are kept, but
    /// every second the page read stays, so the cursor moves past it.
    async fn page(
        &self,
        keys: &[&str],
        floor: i64,
        top: i64,
        sport: Option<&str>,
    ) -> AppResult<Page> {
        let rows = self
            .rows_between(keys, (floor, top), WALK_PAGE_ROWS, None)
            .await?;
        let full = i64::try_from(rows.len()).unwrap_or(i64::MAX) >= WALK_PAGE_ROWS;
        let mut seconds = rows_by_second(rows, sport);
        if full && seconds.len() > 1 {
            seconds.pop();
        }
        Ok(Page {
            seconds,
            reached_floor: !full,
        })
    }

    /// The activities started in `[floor, top)` under `keys`, of `sport`, no
    /// copy of which holds its efforts: the copies of one activity are one
    /// ([`identity`]) wherever in the window each starts.
    pub(super) async fn unread_between(
        &self,
        keys: &[&str],
        floor: i64,
        top: i64,
        sport: Option<&str>,
    ) -> Result<Vec<UnreadActivity>, ()> {
        let mut read: HashMap<NativeKey, (bool, UnreadActivity)> = HashMap::new();
        let mut top = top;
        loop {
            let page = self.page(keys, floor, top, sport).await.map_err(|e| {
                warn!(user_id = %self.user_id, error = %e, "Detail walk: the store could not be read to count the window");
            })?;
            for (at, rows) in &page.seconds {
                for row in rows {
                    let (was_read, activity) = read.entry(identity(row)).or_insert((
                        false,
                        UnreadActivity {
                            newest: *at,
                            first_stored_ms: None,
                        },
                    ));
                    *was_read |= row.segment_read;
                    activity.newest = activity.newest.max(*at);
                    activity.first_stored_ms = earliest(activity.first_stored_ms, stored_ms(row));
                }
            }
            match page.seconds.last() {
                Some((oldest, _)) if !page.reached_floor => top = *oldest,
                _ => break,
            }
        }
        Ok(read
            .into_values()
            .filter_map(|(read, activity)| (!read).then_some(activity))
            .collect())
    }
}

/// One activity of a window no copy of which holds its efforts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UnreadActivity {
    /// The newest start among its copies (unix seconds): where a walk, going
    /// down, meets it first.
    pub(super) newest: i64,
    /// When its first copy reached the store (unix ms); `None` when no copy
    /// says.
    pub(super) first_stored_ms: Option<i64>,
}

/// When `row` was first written to the store (unix ms).
fn stored_ms(row: &CachedActivityRow) -> Option<i64> {
    row.stored_at.map(|at| at.timestamp_millis())
}

/// The earlier of two instants, either of which may be unknown.
fn earliest(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Whether an earlier walk that ended walked tried an activity a walk meets
/// first at `start` (unix seconds) whose first copy reached the store at
/// `first_stored_ms`: it started within the range that walk tried
/// (`tried`), and was in the store before the second that walk was asked
/// in. One the store got after that, or one whose arrival nothing records,
/// the earlier walk never had in reach. The second, not the instant: `SQLite`
/// keeps a row's first write to the second, so one stored in the second of
/// the ask is taken as stored after it, and read rather than reported
/// unreadable.
///
/// The one test a fresh walk's count ([`super::ask_detail_walk`]) and its
/// steps ([`take_seconds`]) both apply, so a walk counts what it reads.
pub(super) fn tried_by(tried: &WalkTried, start: i64, first_stored_ms: Option<i64>) -> bool {
    tried.floor_ts.is_none_or(|floor| floor <= start)
        && start < tried.top_ts
        && first_stored_ms
            .is_some_and(|stored| stored < tried.asked_ms - tried.asked_ms.rem_euclid(1_000))
}

/// `rows`, newest first, by the second they started in, each second keeping
/// only its rows of `sport`.
fn rows_by_second(rows: Vec<CachedActivityRow>, sport: Option<&str>) -> Vec<SecondRows> {
    let mut seconds: Vec<SecondRows> = Vec::new();
    for row in rows {
        let at = row.activity.start_date().timestamp();
        if !matches!(seconds.last(), Some((last, _)) if *last == at) {
            seconds.push((at, Vec::new()));
        }
        let of_sport =
            !filter_activities_by_sport_type(vec![row.activity.clone()], sport).is_empty();
        if let (Some((_, rows)), true) = (seconds.last_mut(), of_sport) {
            rows.push(row);
        }
    }
    seconds
}

/// The service and its own activity id the copies of one activity share.
type NativeKey = (String, String);

/// What makes `row` one activity with its other copies
/// ([`one_activity_key`]): the service's own activity id, the same under the
/// session's key and the grant's whatever start each lists it at.
fn identity(row: &CachedActivityRow) -> NativeKey {
    one_activity_key(&row.activity)
}

/// The activities among one second's `rows` no copy of which in that second
/// holds its efforts; a copy elsewhere is the step's to look for
/// ([`drop_read_twins`]).
fn unread_of(rows: Vec<CachedActivityRow>) -> Vec<Unread> {
    let mut activities: Vec<(NativeKey, Vec<CachedActivityRow>)> = Vec::new();
    for row in rows {
        let key = identity(&row);
        match activities.iter_mut().find(|(seen, _)| *seen == key) {
            Some((_, copies)) => copies.push(row),
            None => activities.push((key, vec![row])),
        }
    }
    activities
        .into_iter()
        .filter(|(_, copies)| !copies.iter().any(|copy| copy.segment_read))
        .map(|(_, copies)| Unread { copies })
        .collect()
}

/// The whole seconds a step takes below its cursor.
#[derive(Debug)]
pub(super) struct Taken {
    pub(super) seconds: Vec<Second>,
    /// Whether the seconds taken are every one down to the floor.
    pub(super) reached_floor: bool,
}

impl Taken {
    /// Where the cursor moves once every second taken is finished.
    pub(super) fn walked_to(&self, floor: i64, cursor: i64) -> i64 {
        if self.reached_floor {
            floor
        } else {
            self.seconds.last().map_or(cursor, |oldest| oldest.at)
        }
    }
}

/// Take whole seconds of the window below `cursor`, newest first, until
/// they hold [`DETAIL_WALK_STEP_ACTIVITIES`] activities to read, the floor
/// is reached, or [`WALK_PAGES_PER_STEP`] reads found too few. A second whose
/// activities would carry the step past its count is left to the next step,
/// unless the step holds none yet. An activity an earlier walk `tried`
/// ([`tried_by`]) is not one to read.
pub(super) async fn take_seconds(
    athlete: &Athlete<'_>,
    keys: &[&str],
    (floor, cursor): (i64, i64),
    sport: Option<&str>,
    tried: Option<&WalkTried>,
) -> Result<Taken, ()> {
    let mut taken = Taken {
        seconds: Vec::new(),
        reached_floor: false,
    };
    let mut unread = 0_usize;
    let mut top = cursor;
    for _ in 0..WALK_PAGES_PER_STEP {
        let page = athlete.page(keys, floor, top, sport).await.map_err(|e| {
            warn!(user_id = %athlete.user_id, error = %e, "Detail walk: the store could not be read for the step");
        })?;
        for (at, rows) in page.seconds {
            let second = Second {
                at,
                unread: unread_of(rows)
                    .into_iter()
                    .filter(|unread| {
                        !tried.is_some_and(|tried| tried_by(tried, at, unread.first_stored_ms()))
                    })
                    .collect(),
            };
            if unread >= DETAIL_WALK_STEP_ACTIVITIES && !second.unread.is_empty() {
                return drop_read_twins(athlete, keys, taken).await;
            }
            unread += second.unread.len();
            top = second.at;
            taken.seconds.push(second);
        }
        if page.reached_floor {
            taken.reached_floor = true;
            return drop_read_twins(athlete, keys, taken).await;
        }
    }
    drop_read_twins(athlete, keys, taken).await
}

/// Drop from `taken` every activity another copy of which, anywhere a day
/// either side of the activities taken, holds its efforts, and every second
/// copy of one activity the step took twice.
///
/// The copies of one activity can start in different seconds: a session's
/// list read can carry only the day, while the grant's names the start, and
/// the grant's copy the walk writes for a session-only ride carries the
/// grant's start. Within a second the copies are already one ([`unread_of`]).
async fn drop_read_twins(
    athlete: &Athlete<'_>,
    keys: &[&str],
    mut taken: Taken,
) -> Result<Taken, ()> {
    let starts = || {
        taken
            .seconds
            .iter()
            .filter(|second| !second.unread.is_empty())
            .map(|second| second.at)
    };
    let (Some(oldest), Some(newest)) = (starts().min(), starts().max()) else {
        return Ok(taken);
    };
    let read = athlete
        .read_between(
            keys,
            (
                oldest.saturating_sub(DAY_SECS),
                newest.saturating_add(DAY_SECS + 1),
            ),
        )
        .await
        .map_err(|e| {
            warn!(user_id = %athlete.user_id, error = %e, "Detail walk: the store could not be read for the step's other copies");
        })?;
    let mut seen: HashSet<NativeKey> = HashSet::new();
    for second in &mut taken.seconds {
        second.unread.retain(|unread| {
            unread
                .copies
                .first()
                .map(identity)
                .is_some_and(|key| !read.contains(&key) && seen.insert(key))
        });
    }
    Ok(taken)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{TimeZone, Utc};
    use pierre_core::models::{ActivityBuilder, SportType};
    use pierre_database::repositories::CachedActivityRow;

    use super::{rows_by_second, unread_of, Second};

    /// The unread activities of each second, by the first copy's id.
    fn unread_ids(seconds: &[Second]) -> HashMap<i64, Vec<String>> {
        seconds
            .iter()
            .map(|second| {
                let ids = second
                    .unread
                    .iter()
                    .filter_map(|unread| unread.copies.first())
                    .map(|copy| copy.activity.id().to_owned())
                    .collect();
                (second.at, ids)
            })
            .collect()
    }

    /// A row of `id` under `key`; a session's capture names Strava as its
    /// source, as sciotte's does.
    fn row(
        key: &str,
        id: &str,
        start: i64,
        sport: SportType,
        segment_read: bool,
    ) -> CachedActivityRow {
        let started = Utc.timestamp_opt(start, 0).single().expect("a start");
        let activity = ActivityBuilder::new(id, "Ride", sport, started, 3_600, key);
        let activity = if key == "sciotte" {
            activity.source("strava")
        } else {
            activity
        };
        CachedActivityRow {
            provider: key.to_owned(),
            activity: activity.build(),
            route: None,
            detail_read: segment_read,
            segment_read,
            stored_at: None,
        }
    }

    #[test]
    fn the_copies_of_one_activity_are_read_once_and_any_read_copy_reads_it() {
        let rows = vec![
            row("strava", "101", 2_000, SportType::Ride, false),
            row("sciotte", "101", 2_000, SportType::Ride, false),
            row("strava", "102", 1_000, SportType::Ride, true),
            row("sciotte", "102", 1_000, SportType::Ride, false),
            row("sciotte", "103", 1_000, SportType::Run, false),
        ];
        let seconds: Vec<Second> = rows_by_second(rows, Some("ride"))
            .into_iter()
            .map(|(at, rows)| Second {
                at,
                unread: unread_of(rows),
            })
            .collect();
        let ids = unread_ids(&seconds);
        assert_eq!(
            ids.get(&2_000).map(Vec::len),
            Some(1),
            "bug: the session's and the grant's copies of one ride are read twice: {ids:?}"
        );
        assert_eq!(
            ids.get(&1_000),
            Some(&Vec::new()),
            "bug: a ride a copy of which holds its efforts, or a run in a ride walk, is read: {ids:?}"
        );
    }
}
