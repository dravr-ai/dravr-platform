// ABOUTME: The detail walk: one durable job per athlete that reads the segment efforts of a window's stored activities
// ABOUTME: Newest first, ten activities a step, through a backend that reads them, within its own day budget; told once when it ends

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Detail walks (carnet#1483).
//!
//! A segment's history is the efforts of the athlete's stored activities,
//! and only a detail read of an activity carries its efforts. The live path
//! details the last days through the session, which reads none. A detail
//! walk reads them for a window: a [`BackfillJobKind::DetailWalk`] job,
//! recorded before it runs, leased while it runs, resumed by the sweep.
//!
//! **Which backend.** Any backend the athlete holds live, grant or session,
//! whose descriptor declares segment efforts on a detail read
//! ([`segment_detail_backend`]); today only the Strava grant does. It is read
//! exactly as named, and the walk's row is filed under it, so a purge of it
//! deletes the walk with the rest of its data.
//!
//! **One step per run.** A step reads the stored activities of the window
//! below its row's cursor (`before_ts`), newest first, under the keys its
//! backend's history is held under ([`history_store_keys`]), never the
//! athlete's own export. It takes whole seconds of starts at a time, so the
//! session's and the grant's copies of one activity, which share it, are
//! never split by a page's edge: a full page's oldest second is left to the
//! next read. It matches the walk's sport ([`filter_activities_by_sport_type`](crate::implementations::fitness_support::filter_activities_by_sport_type)),
//! counts the copies of one activity as one ([`native_activity_key`](pierre_providers::deduplication::native_activity_key)), and
//! reads up to [`DETAIL_WALK_STEP_ACTIVITIES`] whose efforts no copy holds,
//! each through the backend in the walk's own day budget ([`in_detail_walk`](pierre_providers::request_budget::in_detail_walk)).
//! What it read is stored beside its row ([`store_detail_from_live_grant`](crate::activity_detail_store::store_detail_from_live_grant)):
//! the grant's answer under the grant's key, its copy written first, through
//! the walk's owed-write guard ([`owed_write`](super::owed_write)), when only the session listed
//! the activity. The cursor then moves to the oldest second it finished, and
//! the row records the step ([`JobProgress`]): activities read, activities
//! stored, the seconds one took.
//!
//! **What stops a step.** An activity the backend cannot read (gone, or any
//! other answer but the ones below) is skipped: the cursor moves past it, and
//! it is counted read, never stored. A spent budget pauses the walk until its
//! window ends; a step that read some first records them, and the next claim
//! is the one that pauses. A dead credential ends the walk incomplete. A
//! failed store retries the step a lease later, its attempt counted.
//!
//! **Where it ends.** When its cursor passes the window's floor (`after_ts`).
//! The row is kept as walked with its window, when it ended and its counts:
//! an ask within [`DETAIL_WALK_STANDS_MS`] over that window reads nothing
//! again ([`DetailWalkAsked::RecentlyWalked`]), the activities the walk could
//! not read reported unread, never re-read at the athlete's cost. The athlete is
//! told once, in their personal thread, how many activities had their
//! efforts read, when that is any
//! ([`BackfillNotifier::push_detail_walk_complete`](crate::runtime::BackfillNotifier::push_detail_walk_complete)).

mod candidates;
mod step;

use std::sync::Arc;

use pierre_core::constants::oauth::providers::STRAVA_ARCHIVE;
use pierre_core::errors::AppResult;
use pierre_core::models::coverage::{IncompleteReason, Pending};
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    BackfillJobKind, BackfillJobState, DetailWalkRow, JobProgress, WalkWiden,
};
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::field_support::ActivityFields;
use pierre_providers::request_budget::DETAIL_WALK_DAILY_CALLS;
use pierre_services::training_history_read::{history_source_order, history_store_keys};
use tracing::{info, warn};
use uuid::Uuid;

use super::{enqueue, now_ms, provider_tenant_id_str, ActivityBackfillJob, Enqueued};
use crate::protocol::AuthService;
use crate::runtime::ToolRuntime;
use candidates::Athlete;
pub(super) use step::run_detail_walk;

/// Activities whose efforts one step reads, at most: the whole seconds it
/// takes stop once they hold this many.
pub const DETAIL_WALK_STEP_ACTIVITIES: usize = 10;

/// How long a detail walk that ended walked stands for its window (ms).
///
/// An ask within it over that window records no fresh walk, which would
/// only spend the athlete's day re-reading the activities the backend could
/// not read. A week: a ride deleted or made private since is read again
/// soon enough.
pub const DETAIL_WALK_STANDS_MS: i64 = 7 * 86_400_000;

/// Seconds in a day.
const SECS_PER_DAY: f64 = 86_400.0;

/// The backend a detail walk of `provider`'s history reads for the athlete.
///
/// The first live one among the backends holding that history, grant or
/// session, whose descriptor declares segment efforts on a detail read.
/// `None` when the athlete holds none: no walk can read them.
pub async fn segment_detail_backend(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> Option<String> {
    let registry = runtime.provider_registry();
    history_source_order(runtime.repos(), user_id, tenant_id, provider)
        .await
        .into_iter()
        .filter(|source| source.live)
        .find(|source| {
            registry
                .get_descriptor(&source.backend)
                .is_some_and(|descriptor| {
                    descriptor
                        .activity_fields()
                        .detail_only
                        .contains(ActivityFields::SEGMENT_EFFORTS)
                })
        })
        .map(|source| source.backend)
}

/// The window a detail walk is asked to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetailWalkAsk<'ask> {
    /// The window's floor (unix seconds), included.
    pub after_ts: i64,
    /// The window's top (unix seconds), excluded.
    pub before_ts: i64,
    /// The sport the ask names; `None` for every sport.
    pub sport: Option<&'ask str>,
}

/// What asking for a detail walk came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailWalkAsked {
    /// A walk was recorded and started, counting this many activities to read.
    Started {
        /// Activities of the window whose efforts no copy holds.
        units_total: i64,
    },
    /// A walk was already owed: its floor was lowered to the ask's, and its
    /// total recounted.
    Widened,
    /// Every activity of the window has its efforts read: nothing to walk.
    NothingUnread,
    /// A walk over the window ended walked within
    /// [`DETAIL_WALK_STANDS_MS`]: it tried every activity in it, so none is
    /// read again. These are the window's activities whose efforts it could
    /// not read.
    RecentlyWalked {
        /// Activities of the window whose efforts no copy holds.
        unreadable: i64,
    },
    /// The store or the ledger could not be read or written (logged).
    NotRecorded,
    /// The backend's budget leaves the athlete's walk no call a day (an app
    /// registered at under 8 a day): no walk is recorded, which would only
    /// pause until each day's end, forever. No backend can read the
    /// window's efforts within its budget.
    NoBudget,
}

impl DetailWalkAsked {
    /// Why a segments window the ask answers is short of its efforts: a
    /// walk recorded or widened is still reading them
    /// ([`IncompleteReason::SegmentDetailWalking`]); one that ended lately
    /// left some unread ([`IncompleteReason::SegmentDetailPartial`]); a
    /// budget with no call a day for a walk reads none
    /// ([`IncompleteReason::SegmentDetailUnavailable`]).
    /// `None` when nothing is left to read, or when nothing could be
    /// recorded: the caller's own count says whether the window is short.
    #[must_use]
    pub fn segment_reason(self) -> Option<IncompleteReason> {
        match self {
            Self::Started { .. } | Self::Widened => Some(IncompleteReason::SegmentDetailWalking),
            Self::RecentlyWalked { unreadable } => Some(IncompleteReason::SegmentDetailPartial {
                unreadable: u64::try_from(unreadable).unwrap_or(0),
            }),
            Self::NoBudget => Some(IncompleteReason::SegmentDetailUnavailable),
            Self::NothingUnread | Self::NotRecorded => None,
        }
    }
}

/// Ask for a detail walk of `ask`'s window through `backend` for the
/// athlete.
///
/// A walk owed for the backend is widened to the ask's floor
/// ([`WalkWiden`]); its cursor stays where its steps moved it, and a newer
/// top is the next walk's once this one ends. With none owed, a walk is
/// recorded over the window, counted at the activities in it whose efforts
/// no copy holds, unless there are none, or unless a walk that ended walked
/// within [`DETAIL_WALK_STANDS_MS`] covers the window
/// ([`DetailWalkAsked::RecentlyWalked`]): after an older one, one over
/// another sport, one whose floor sits above the ask's, one an unread
/// activity of the ask started above the top of or after the end of, or one
/// that ended any other way, the window is walked afresh.
pub async fn ask_detail_walk(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    ask: DetailWalkAsk<'_>,
) -> DetailWalkAsked {
    let athlete = Athlete {
        runtime,
        user_id,
        tenant_id,
    };
    if walk_daily_calls(&athlete, backend).await == Some(0) {
        info!(user_id = %user_id, backend, "Detail walk: the backend's budget leaves a walk no call a day; none is recorded");
        return DetailWalkAsked::NoBudget;
    }
    let Ok(row) = walk_row(&athlete, backend).await else {
        return DetailWalkAsked::NotRecorded;
    };
    if let Some(row) = row
        .as_ref()
        .filter(|row| row.state == BackfillJobState::Owed)
    {
        return widen(&athlete, backend, row, ask).await;
    }
    let keys = walk_keys(backend);
    let Ok(unread) = athlete
        .unread_between(&keys, ask.after_ts, ask.before_ts, ask.sport)
        .await
    else {
        return DetailWalkAsked::NotRecorded;
    };
    let units_total = i64::try_from(unread.len()).unwrap_or(i64::MAX);
    if units_total == 0 {
        return DetailWalkAsked::NothingUnread;
    }
    if row
        .as_ref()
        .is_some_and(|row| stands_for(row, ask, &unread, now_ms()))
    {
        info!(user_id = %user_id, backend, unreadable = units_total, "Detail walk: a walk over the window ended lately; nothing is read again");
        return DetailWalkAsked::RecentlyWalked {
            unreadable: units_total,
        };
    }
    record_walk(&athlete, backend, ask, units_total).await
}

/// Record and start a walk of `ask`'s window through `backend`, counted at
/// `units_total` activities to read; when another ask recorded one between
/// this one's read and its record, widen that one instead.
async fn record_walk(
    athlete: &Athlete<'_>,
    backend: &str,
    ask: DetailWalkAsk<'_>,
    units_total: i64,
) -> DetailWalkAsked {
    let (user_id, tenant_id) = (athlete.user_id, athlete.tenant_id);
    let job = ActivityBackfillJob {
        resources: Arc::clone(athlete.runtime),
        user_id,
        tenant_id,
        tenant_id_str: provider_tenant_id_str(tenant_id),
        provider_name: backend.to_owned(),
        query_params: ActivityQueryParams {
            after: Some(ask.after_ts),
            before: Some(ask.before_ts),
            limit: None,
            offset: None,
        },
        // No conversation waits on it: the athlete's personal thread hears
        // the end.
        pierre_conversation_id: None,
        kind: BackfillJobKind::DetailWalk,
        turn_id: None,
        asked_message_id: None,
        walk_sport: ask.sport.map(str::to_owned),
    };
    let progress = JobProgress {
        units_total: Some(units_total),
        ..JobProgress::default()
    };
    match enqueue(job, None, progress).await {
        Enqueued::Started => {
            info!(user_id = %user_id, backend, units_total, "Detail walk: asked");
            return DetailWalkAsked::Started { units_total };
        }
        Enqueued::Unrecorded => return DetailWalkAsked::NotRecorded,
        Enqueued::InFlightHere(..) | Enqueued::Owed(..) => {}
    }
    // Recorded by another ask between the read and the record: this one
    // widens it.
    match owed_walk(athlete, backend).await {
        Ok(Some(row)) => widen(athlete, backend, &row, ask).await,
        _ => DetailWalkAsked::NotRecorded,
    }
}

/// Whether the walk `row` records, which ended walked within
/// [`DETAIL_WALK_STANDS_MS`] of `now_ms`, covers `ask`: its window reaches
/// from at or below the ask's floor, over the ask's sport or every sport,
/// and every activity of the ask left `unread` (the newest start of each)
/// started below its top and before it ended. A default window ends at the
/// athlete's next midnight, a day later each day: the walk stands for it
/// while nothing unread is newer than what the walk could have tried.
fn stands_for(row: &DetailWalkRow, ask: DetailWalkAsk<'_>, unread: &[i64], now_ms: i64) -> bool {
    let (Some(ended_ms), Some(top)) = (row.ended_at_ms, row.window_top_ts) else {
        return false;
    };
    let tried_below = top.min(ended_ms.div_euclid(1_000));
    row.state == BackfillJobState::Walked
        && now_ms.saturating_sub(ended_ms) < DETAIL_WALK_STANDS_MS
        && row.after_ts.is_none_or(|floor| floor <= ask.after_ts)
        && unread.iter().all(|newest| *newest < tried_below)
        && (row.walk_sport.is_none() || row.walk_sport.as_deref() == ask.sport)
}

/// The detail walk owed for `backend`, if one is; `Err` (logged) when the
/// ledger could not be read.
async fn owed_walk(athlete: &Athlete<'_>, backend: &str) -> Result<Option<DetailWalkRow>, ()> {
    Ok(walk_row(athlete, backend)
        .await?
        .filter(|row| row.state == BackfillJobState::Owed))
}

/// The detail walk filed under `backend`, owed or ended, if one is; `Err`
/// (logged) when the ledger could not be read.
async fn walk_row(athlete: &Athlete<'_>, backend: &str) -> Result<Option<DetailWalkRow>, ()> {
    match athlete
        .runtime
        .repos()
        .activity_backfill_jobs
        .detail_walk_row(athlete.user_id, athlete.tenant_id, backend)
        .await
    {
        Ok(row) => Ok(row),
        Err(e) => {
            warn!(user_id = %athlete.user_id, backend, error = %e, "Detail walk: the ledger could not be read");
            Err(())
        }
    }
}

/// Widen the walk owed for `backend` (`row`) to `ask`'s floor, its total
/// recounted at what it has read plus what is unread below its cursor in
/// the widened window.
async fn widen(
    athlete: &Athlete<'_>,
    backend: &str,
    row: &DetailWalkRow,
    ask: DetailWalkAsk<'_>,
) -> DetailWalkAsked {
    let floor = row
        .after_ts
        .map_or(ask.after_ts, |after| after.min(ask.after_ts));
    let cursor = row.before_ts.unwrap_or(ask.before_ts);
    let sport = if row.walk_sport.as_deref() == ask.sport {
        ask.sport
    } else {
        None
    };
    let keys = walk_keys(backend);
    let Ok(unread) = athlete.unread_between(&keys, floor, cursor, sport).await else {
        return DetailWalkAsked::NotRecorded;
    };
    let units_left = i64::try_from(unread.len()).unwrap_or(i64::MAX);
    let widen = WalkWiden {
        after_ts: ask.after_ts,
        walk_sport: ask.sport.map(str::to_owned),
        units_left,
    };
    match athlete
        .runtime
        .repos()
        .activity_backfill_jobs
        .widen_detail_walk(athlete.user_id, athlete.tenant_id, backend, &widen)
        .await
    {
        Ok(true) => DetailWalkAsked::Widened,
        Ok(false) => {
            info!(user_id = %athlete.user_id, backend, "Detail walk: the owed walk ended before the ask could widen it");
            DetailWalkAsked::NotRecorded
        }
        Err(e) => {
            warn!(user_id = %athlete.user_id, backend, error = %e, "Detail walk: the owed walk could not be widened");
            DetailWalkAsked::NotRecorded
        }
    }
}

/// The detail walk still owed for `backend`'s window, with its progress
/// ([`Pending::DetailWalkRunning`]), or `None` when none is owed.
///
/// Its ETA is the activities left at the slower of the pace its steps
/// measured and the one its budget allows: one activity a call, at most the
/// calls a day the athlete's walk window holds as the backend's budget sizes
/// it ([`walk_daily_calls`]).
///
/// # Errors
///
/// Returns [`AppError`] when the ledger cannot be read.
pub async fn detail_walk_pending(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> AppResult<Option<Pending>> {
    let row = runtime
        .repos()
        .activity_backfill_jobs
        .detail_walk_row(user_id, tenant_id, backend)
        .await?;
    let Some(row) = row.filter(|row| row.state == BackfillJobState::Owed) else {
        return Ok(None);
    };
    let athlete = Athlete {
        runtime,
        user_id,
        tenant_id,
    };
    let daily_calls = walk_daily_calls(&athlete, backend).await;
    Ok(Some(detail_walk_running(&row.progress, daily_calls)))
}

/// The calls a day the athlete's detail walk through `backend` may make, as
/// the budget its stored credential is counted in sizes the walk's window
/// ([`RequestBudget::detail_walk_daily_calls`](pierre_providers::request_budget::RequestBudget::detail_walk_daily_calls)).
/// `None` when nothing counts the backend's calls for the athlete, or its
/// budget has no walk window: the walk is then held to its background share
/// alone.
async fn walk_daily_calls(athlete: &Athlete<'_>, backend: &str) -> Option<u32> {
    AuthService::new(Arc::clone(athlete.runtime))
        .stored_request_budget(backend, athlete.user_id, athlete.tenant_id)
        .await?
        .detail_walk_daily_calls(backend)
        .await
}

/// A running detail walk's [`Pending`] from its recorded progress
/// ([`Pending::detail_walk_running`]), at `daily_calls` a day, or
/// [`DETAIL_WALK_DAILY_CALLS`] when no budget sizes the walk's day.
#[must_use]
pub fn detail_walk_running(progress: &JobProgress, daily_calls: Option<u32>) -> Pending {
    let units = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
    let day = daily_calls.unwrap_or(DETAIL_WALK_DAILY_CALLS).max(1);
    let budget_pace = SECS_PER_DAY / f64::from(day);
    let pace = progress
        .secs_per_unit
        .map_or(budget_pace, |measured| measured.max(budget_pace));
    Pending::detail_walk_running(
        units(progress.units_done),
        progress.units_total.map(units),
        Some(pace),
    )
}

/// The keys a walk through `backend` reads candidates under: every key its
/// history is held under, never the athlete's own Strava export, which no
/// API detail is ever written beside.
fn walk_keys(backend: &str) -> Vec<&str> {
    history_store_keys(backend)
        .into_iter()
        .filter(|key| *key != STRAVA_ARCHIVE)
        .collect()
}

#[cfg(test)]
mod tests {
    use pierre_core::models::coverage::{IncompleteReason, Pending};
    use pierre_database::repositories::JobProgress;

    use super::{detail_walk_running, DetailWalkAsked};

    #[test]
    fn a_walk_goes_no_faster_than_its_budget_allows() {
        let progress = |measured: f64| JobProgress {
            units_total: Some(60),
            units_done: 10,
            rows_landed: 8,
            secs_per_unit: Some(measured),
        };
        assert_eq!(
            detail_walk_running(&progress(5.0), None),
            Pending::DetailWalkRunning {
                units_done: 10,
                units_total: Some(60),
                eta_secs: Some(50 * 1_728),
            },
            "bug: 50 activities left at 50 a day is a day, however fast a step measured"
        );
        assert_eq!(
            detail_walk_running(&progress(3_000.0), None),
            Pending::DetailWalkRunning {
                units_done: 10,
                units_total: Some(60),
                eta_secs: Some(50 * 3_000),
            },
            "bug: a measured pace slower than the budget's is the walk's"
        );
        assert_eq!(
            detail_walk_running(&progress(5.0), Some(10)),
            Pending::DetailWalkRunning {
                units_done: 10,
                units_total: Some(60),
                eta_secs: Some(50 * 8_640),
            },
            "bug: a walk held to 10 a day is paced as if it had 50"
        );
    }

    #[test]
    fn an_ask_names_why_its_segments_window_is_short() {
        assert_eq!(
            DetailWalkAsked::Started { units_total: 4 }.segment_reason(),
            Some(IncompleteReason::SegmentDetailWalking)
        );
        assert_eq!(
            DetailWalkAsked::Widened.segment_reason(),
            Some(IncompleteReason::SegmentDetailWalking)
        );
        assert_eq!(
            DetailWalkAsked::RecentlyWalked { unreadable: 3 }.segment_reason(),
            Some(IncompleteReason::SegmentDetailPartial { unreadable: 3 }),
            "bug: a walk that ended lately reads as still walking"
        );
        assert_eq!(
            DetailWalkAsked::NoBudget.segment_reason(),
            Some(IncompleteReason::SegmentDetailUnavailable),
            "bug: a window no walk has a call to read reads as complete"
        );
        assert_eq!(DetailWalkAsked::NothingUnread.segment_reason(), None);
        assert_eq!(DetailWalkAsked::NotRecorded.segment_reason(), None);
    }
}
