// ABOUTME: A detail walk's ask while one is owed widens its floor only; its end lands only on the floor it walked to
// ABOUTME: The cursor a step moved stays, the total is recounted, two different sports read every sport

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::{BackfillJobState, JobProgress};

/// What a newer ask widens an owed detail walk to
/// ([`super::ActivityBackfillJobRepository::widen_detail_walk`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkWiden {
    /// The new ask's window floor (unix seconds): the row's `after_ts`
    /// becomes the lower of the two, never the higher.
    pub after_ts: i64,
    /// The new ask's sport type; `None` for every sport. The row keeps its
    /// sport only when the two are the same one.
    pub walk_sport: Option<String>,
    /// Activities the widened window leaves unread, counted by the asker:
    /// the row's [`super::JobProgress::units_total`] becomes what it has done
    /// plus these. A step running while the asker counts can skew the total
    /// by at most the activities that one step walks, either way: what it
    /// stored before the count is left out but still added to the done
    /// count when it lands, and what it walks after is counted unread and
    /// done at once. The total is what the progress shows, never what ends
    /// the walk: its cursor passing the floor does
    /// ([`super::ActivityBackfillJobRepository::end_detail_walk`]).
    pub units_left: i64,
}

/// Widens the floor of the detail walk owed for the pair (`$4` its kind) and
/// nothing else, in the row's own tenant. Its cursor, `before_ts`, is left
/// where its steps moved it; a newer top is the next walk's, once this one
/// ends. A NULL floor reads no lower bound, the widest there is, and stays.
/// A sport kept only when both asks name the same one: a NULL on either side
/// reads every sport. The widen matches the row whatever lease holds it: a
/// step's progress write keeps `after_ts`, and the walk's end lands only on
/// the floor its step walked to ([`END_DETAIL_WALK_SQL`]), so a floor
/// lowered under a running step is walked by the steps after it.
pub(crate) const WIDEN_DETAIL_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET after_ts = CASE WHEN after_ts IS NOT NULL AND $5 < after_ts THEN $5 ELSE after_ts END, \
         months_total = months_done + $6, \
         walk_sport = CASE WHEN walk_sport = $7 THEN walk_sport ELSE NULL END \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4 \
       AND outcome IS NULL";

/// The last step of a detail walk, the one whose cursor passed the floor it
/// claimed ([`super::ActivityBackfillJobRepository::end_detail_walk`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailWalkEnd {
    /// Where the step's cursor stopped (unix seconds): the row's last
    /// `before_ts`.
    pub before_ts: i64,
    /// Activities this step walked, added to the row's
    /// [`super::JobProgress::units_done`].
    pub units_done: i64,
    /// Activities this step stored, added to the row's
    /// [`super::JobProgress::rows_landed`].
    pub rows_landed: i64,
    /// Seconds per activity this step measured; `None` keeps the stored one.
    pub secs_per_unit: Option<f64>,
    /// The floor the step walked down to, as its claim read the row's
    /// `after_ts`; `None` when the walk had none.
    pub walked_floor: Option<i64>,
}

/// Adds a detail walk's last step to its progress and keeps the row,
/// released, as walked (`$8`): its window's floor, what it read and what it
/// stored stay on it. Only while the floor is still the one the step walked
/// to (`$7`): an ask that widened it meanwhile
/// ([`WIDEN_DETAIL_WALK_SQL`], which matches the row whatever lease holds it)
/// left the walk rows below that floor to read, so the row stays owed and
/// the step writes its progress instead. A widen only ever lowers a floor
/// that is there, so a floor equal to the claimed one never moved.
pub(crate) const END_DETAIL_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET before_ts = $3, months_done = months_done + $4, rows_landed = rows_landed + $5, \
         secs_per_unit = COALESCE($6, secs_per_unit), outcome = $8, \
         leased_until_ms = 0, lease_token = NULL \
     WHERE id = $1 AND lease_token = $2 AND outcome IS NULL \
       AND (after_ts = $7 OR (after_ts IS NULL AND $7 IS NULL)) \
     RETURNING months_total, months_done, rows_landed, secs_per_unit, created_at_ms";

/// How a detail walk ended ([`super::ActivityBackfillJobRepository::end_detail_walk`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailWalkEnded {
    /// The progress the row ended on, its last step included.
    pub progress: JobProgress,
    /// When the ask that recorded the walk was made (unix ms): what keys the
    /// walk's one completion post, so a later walk posts again.
    pub created_at_ms: i64,
}

/// The detail walk filed under a pair, owed or ended, as it stands
/// ([`super::ActivityBackfillJobRepository::detail_walk_row`]).
#[derive(Debug, Clone, PartialEq)]
pub struct DetailWalkRow {
    /// Owed, or how it ended.
    pub state: BackfillJobState,
    /// The window's floor (unix seconds); `None` for no lower bound.
    pub after_ts: Option<i64>,
    /// The walk's cursor (unix seconds): every activity started at or after
    /// it was walked. An ended walk's sits at its floor.
    pub before_ts: Option<i64>,
    /// The sport the walk reads; `None` for every sport.
    pub walk_sport: Option<String>,
    /// When the ask that recorded it was made (unix ms).
    pub created_at_ms: i64,
    /// How far it got.
    pub progress: JobProgress,
}

/// The detail walk (`$4`) filed under the pair in the tenant, whatever its
/// state.
pub(crate) const DETAIL_WALK_ROW_SQL: &str = "SELECT outcome, after_ts, before_ts, walk_sport, \
            created_at_ms, months_total, months_done, rows_landed, secs_per_unit \
     FROM activity_backfill_jobs \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4";
