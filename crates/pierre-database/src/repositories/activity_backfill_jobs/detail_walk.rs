// ABOUTME: A detail walk's ask while one is owed widens its floor only; its end lands only on the floor it walked to
// ABOUTME: The cursor a step moved stays, the total is recounted, two different sports read every sport

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use super::{backfill_job_state_of, job_progress_from_row, BackfillJobState, JobProgress};

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
///
/// A walk of one sport widened to every sport left the other sports'
/// activities above its cursor unread: its cursor goes back to its window's
/// top, and the range an earlier walk tried, which was that sport's alone,
/// is dropped. The step that holds it meanwhile would write its own cursor
/// over that, so its lease is taken back with it: the step's writes, which
/// all name its lease, then land nothing, and the sweep runs the walk on
/// from the top.
pub(crate) const WIDEN_DETAIL_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET after_ts = CASE WHEN after_ts IS NOT NULL AND $5 < after_ts THEN $5 ELSE after_ts END, \
         months_total = months_done + $6, \
         before_ts = CASE WHEN walk_sport IS NOT NULL AND ($7 IS NULL OR walk_sport <> $7) \
                          THEN window_top_ts ELSE before_ts END, \
         lease_token = CASE WHEN walk_sport IS NOT NULL AND ($7 IS NULL OR walk_sport <> $7) \
                            THEN NULL ELSE lease_token END, \
         leased_until_ms = CASE WHEN walk_sport IS NOT NULL AND ($7 IS NULL OR walk_sport <> $7) \
                                THEN 0 ELSE leased_until_ms END, \
         tried_floor_ts = CASE WHEN walk_sport IS NOT NULL AND ($7 IS NULL OR walk_sport <> $7) \
                               THEN NULL ELSE tried_floor_ts END, \
         tried_top_ts = CASE WHEN walk_sport IS NOT NULL AND ($7 IS NULL OR walk_sport <> $7) \
                             THEN NULL ELSE tried_top_ts END, \
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
    /// When the walk ended (unix ms): an ask within a week over the window
    /// it walked records no fresh walk.
    pub ended_at_ms: i64,
}

/// Adds a detail walk's last step to its progress and keeps the row,
/// released, as walked (`$8`) at `$9`: its window's floor and top, what it
/// read and what it stored stay on it. Only while the floor is still the one the step walked
/// to (`$7`): an ask that widened it meanwhile
/// ([`WIDEN_DETAIL_WALK_SQL`], which matches the row whatever lease holds it)
/// left the walk rows below that floor to read, so the row stays owed and
/// the step writes its progress instead. A widen only ever lowers a floor
/// that is there, so a floor equal to the claimed one never moved.
pub(crate) const END_DETAIL_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET before_ts = $3, months_done = months_done + $4, rows_landed = rows_landed + $5, \
         secs_per_unit = COALESCE($6, secs_per_unit), outcome = $8, ended_at_ms = $9, \
         leased_until_ms = 0, lease_token = NULL \
     WHERE id = $1 AND lease_token = $2 AND outcome IS NULL \
       AND (after_ts = $7 OR (after_ts IS NULL AND $7 IS NULL)) \
     RETURNING months_total, months_done, rows_landed, secs_per_unit, created_at_ms";

/// Adds the activities a detail walk's step stored (`$5`) to the done and
/// stored counts of its walk (`$1`, the row's id, of kind `$4`, the user's
/// `$2` in tenant `$3`), whatever lease holds the row now: once a widen to
/// every sport took the step's lease back, its own progress write lands
/// nothing, and what it stored is read, so no later step counts it. Only
/// while that same walk (asked at `$6`) is still owed: a walk recorded over
/// the row since never read them.
pub(crate) const CREDIT_DETAIL_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET months_done = months_done + $5, rows_landed = rows_landed + $5 \
     WHERE id = $1 AND user_id = $2 AND tenant_id = $3 AND kind = $4 \
       AND created_at_ms = $6 AND outcome IS NULL";

/// How a detail walk ended ([`super::ActivityBackfillJobRepository::end_detail_walk`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailWalkEnded {
    /// The progress the row ended on, its last step included.
    pub progress: JobProgress,
    /// When the ask that recorded the walk was made (unix ms): what keys the
    /// walk's one completion post, so a later walk posts again.
    pub created_at_ms: i64,
}

/// The range of a window an earlier detail walk that ended walked tried,
/// which a walk recorded after it reads nothing of again
/// ([`super::ActivityBackfillJobRepository::set_detail_walk_tried`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkTried {
    /// The range's floor (unix seconds); `None` for no lower bound.
    pub floor_ts: Option<i64>,
    /// The range's top (unix seconds), excluded.
    pub top_ts: i64,
    /// When the earlier walk was asked (unix ms): an activity first stored
    /// after it was never in its reach.
    pub asked_ms: i64,
    /// When the oldest walk whose tries the range carries ended (unix ms):
    /// the range stands for a week from it, no longer.
    pub ended_ms: i64,
}

/// Records on the ended detail walk of the pair (`$4` its kind) the range a
/// walk recorded over it next reads nothing of again (`$5`–`$8`, all NULL
/// for none), in the row's own tenant. The record that replaces the ended
/// row leaves these columns as they are, so the new walk carries them.
pub(crate) const SET_DETAIL_WALK_TRIED_SQL: &str = "UPDATE activity_backfill_jobs \
     SET tried_floor_ts = $5, tried_top_ts = $6, tried_asked_ms = $7, tried_ended_ms = $8 \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4 \
       AND outcome IS NOT NULL";

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
    /// The window's top (unix seconds), excluded, as the ask that recorded
    /// the walk named it: a widen never moves it, and it stays once the walk
    /// ends.
    pub window_top_ts: Option<i64>,
    /// When the walk ended walked (unix ms); `None` while it is owed, or
    /// when it ended any other way.
    pub ended_at_ms: Option<i64>,
    /// The sport the walk reads; `None` for every sport.
    pub walk_sport: Option<String>,
    /// When the ask that recorded it was made (unix ms).
    pub created_at_ms: i64,
    /// The range an earlier walk tried, which this one reads nothing of
    /// again; `None` when there is none.
    pub tried: Option<WalkTried>,
    /// How far it got.
    pub progress: JobProgress,
}

/// The detail walk (`$4`) filed under the pair in the tenant, whatever its
/// state.
pub(crate) const DETAIL_WALK_ROW_SQL: &str = "SELECT outcome, after_ts, before_ts, walk_sport, \
            window_top_ts, ended_at_ms, created_at_ms, months_total, months_done, rows_landed, \
            secs_per_unit, tried_floor_ts, tried_top_ts, tried_asked_ms, tried_ended_ms \
     FROM activity_backfill_jobs \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4";

fn column_error(name: &str, e: &sqlx::Error) -> AppError {
    AppError::database(format!("activity_backfill_jobs {name}: {e}"))
}

/// The [`DetailWalkEnded`] an end's `RETURNING` row reads as.
pub(crate) fn detail_walk_ended_from_row<R>(row: &R) -> AppResult<DetailWalkEnded>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<i64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(DetailWalkEnded {
        progress: job_progress_from_row(row)?,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(|e| column_error("created_at_ms", &e))?,
    })
}

/// The [`DetailWalkRow`] a [`DETAIL_WALK_ROW_SQL`] row reads as.
pub(crate) fn detail_walk_row_from_row<R>(row: &R) -> AppResult<DetailWalkRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<i64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |name: &'static str| move |e: sqlx::Error| column_error(name, &e);
    let outcome: Option<String> = row.try_get("outcome").map_err(column("outcome"))?;
    Ok(DetailWalkRow {
        state: backfill_job_state_of(outcome.as_deref())?,
        after_ts: row.try_get("after_ts").map_err(column("after_ts"))?,
        before_ts: row.try_get("before_ts").map_err(column("before_ts"))?,
        walk_sport: row.try_get("walk_sport").map_err(column("walk_sport"))?,
        window_top_ts: row
            .try_get("window_top_ts")
            .map_err(column("window_top_ts"))?,
        ended_at_ms: row.try_get("ended_at_ms").map_err(column("ended_at_ms"))?,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(column("created_at_ms"))?,
        tried: walk_tried_from_row(row)?,
        progress: job_progress_from_row(row)?,
    })
}

/// The [`WalkTried`] a [`DETAIL_WALK_ROW_SQL`] row carries: one when its
/// top, its ask and its end are all set.
fn walk_tried_from_row<R>(row: &R) -> AppResult<Option<WalkTried>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<i64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |name: &'static str| -> AppResult<Option<i64>> {
        row.try_get(name).map_err(|e| column_error(name, &e))
    };
    let (floor_ts, top_ts, asked_ms, ended_ms) = (
        column("tried_floor_ts")?,
        column("tried_top_ts")?,
        column("tried_asked_ms")?,
        column("tried_ended_ms")?,
    );
    Ok(match (top_ts, asked_ms, ended_ms) {
        (Some(top_ts), Some(asked_ms), Some(ended_ms)) => Some(WalkTried {
            floor_ts,
            top_ts,
            asked_ms,
            ended_ms,
        }),
        _ => None,
    })
}
