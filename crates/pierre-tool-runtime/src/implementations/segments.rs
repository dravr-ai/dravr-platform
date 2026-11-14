// ABOUTME: get_segment_history — every effort the athlete made on one segment, from Strava's live listing or the stored details
// ABOUTME: A name folds accents and case; what is unread asks a detail walk; Strava's efforts stay on Dravr's own surfaces

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `get_segment_history`
//!
//! One segment's history: every effort the athlete made on it in a window,
//! with the activity each was made in (carnet#1483). Two places hold it.
//!
//! **The live listing.** When the call names the segment by id and no sport,
//! and the athlete holds a live backend that lists a segment's efforts
//! (`ProviderCapabilities::SEGMENT_EFFORT_HISTORY`: the Strava grant), the
//! listing answers, complete, in the foreground budget. It stores nothing.
//! Any failure falls through to the stored details: a transport the
//! provider's terms keep its data off (the governed provider refuses before
//! any request), a dead grant, a spent budget, Strava refusing the listing to
//! a non-subscriber. Only that last refusal is remembered, for the athlete
//! and the backend, for a day ([`refusal`]), so a
//! non-subscriber spends one listing a day, not one a call; a mistyped
//! segment id or a failing Strava never keeps the athlete off it.
//!
//! **The stored details.** Every stored copy of the athlete's Strava
//! activities in the window, under the keys their history is held under and
//! the athlete's own export ([`segment_keys`]), filtered as this transport
//! and the athlete's consents let a model see them, matched to the sport
//! asked, and merged so the copies of one activity are one
//! ([`merge_duplicates`]). An activity's efforts are read once a detail read
//! of any copy read them. The efforts on the segment are listed once each.
//!
//! **Coverage.** `window.total` is the activities in the window,
//! `window.returned` those whose segments were read ([`segments_read_coverage`]).
//! When fewer were read, the window says why ([`coverage_of`]): a detail
//! walk is asked to read the rest and is reading them, a walk that ended
//! lately could not read some, or no backend the athlete holds reads
//! segment efforts. A window that starts before the stored Strava history
//! reaches says, first, that the Strava history walk is still copying it
//! in: the one owed, or the one the ask starts when none is ([`history_owed`]).
//! Rows the transport withheld, and any call over an external transport,
//! ask no walk of either kind.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Months, NaiveDate, Utc};
use chrono_tz::Tz;
use pierre_core::civil_time::{clock_date, resolve_zone};
use pierre_core::constants::oauth::providers::{companion_keys, STRAVA, STRAVA_ARCHIVE};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coverage::{Completeness, IncompleteReason, Pending};
use pierre_core::models::{Activity, SegmentEffort, TenantId};
use pierre_core::text_fold::fold_for_match;
use pierre_database::repositories::{CachedRowCursor, CachedRowsQuery};
use pierre_mcp_schema::PropertySchema;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::activity_origin;
use pierre_providers::core::{SegmentEffortHit, SegmentEffortWindow};
use pierre_providers::deduplication::{merge_duplicates, one_activity_key, DedupConfig};
use pierre_services::training_history_read::{
    history_source_order, history_store_keys, user_timezone,
};
use pierre_tools_core::ToolResult;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

pub mod answer;
mod refusal;

use answer::{answer, effort_row, AnsweredBy, SegmentHistoryResult};
use refusal::{provider_refused, refused_lately, remember_refusal};

use crate::activity_backfill::detail_walk::{
    ask_detail_walk, detail_walk_pending, live_backend_serving, segment_detail_serving,
    DetailWalkAsk, DetailWalkAsked, Serving,
};
use crate::activity_backfill::history_walk::start_history_walk;
use crate::activity_backfill::provider_tenant_id_str;
use crate::activity_fetch::{history_depth_covered, history_walk_owed};
use crate::annotations::{annotations, Effect, Reach};
use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::implementations::data_helpers::segments_read_coverage;
use crate::implementations::fitness_support::filter_activities_by_sport_type;
use crate::protocol::UniversalExecutor;
use crate::runtime::{ToolRuntime, WalkTrigger};
use crate::training_history_compute::owed_history_walk_pending;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};

/// The tool's name.
const TOOL: &str = "get_segment_history";

/// How far back a window reaches when the call names no `from`.
const DEFAULT_YEARS: u32 = 10;

/// Stored rows one read of the window takes.
const STORE_PAGE_ROWS: i64 = 500;

/// Activities one call reads at most, each counted once whatever copies of
/// it are stored ([`WindowRows`]): past it the window is answered from the
/// newest activities read and says it stopped short
/// ([`IncompleteReason::ReadCapped`]).
const MAX_WINDOW_ACTIVITIES: usize = 20_000;

// ============================================================================
// The ask
// ============================================================================

/// Which segment the call asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SegmentMatch {
    /// Its id, exactly.
    Id(String),
    /// Every segment whose folded name holds this folded text.
    Name(String),
}

impl SegmentMatch {
    fn matches(&self, effort: &SegmentEffort) -> bool {
        match self {
            Self::Id(id) => effort.segment_id.as_deref() == Some(id.as_str()),
            Self::Name(folded) => fold_for_match(&effort.name).contains(folded.as_str()),
        }
    }
}

/// What the call asks, parsed.
#[derive(Debug, Clone)]
struct SegmentAsk {
    segment: SegmentMatch,
    /// The first day of the window, the athlete's.
    from: NaiveDate,
    /// The last day of the window, the athlete's, included.
    to: NaiveDate,
    /// The sport asked, as the caller spelled it.
    sport: Option<String>,
}

/// A non-empty string argument.
fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn date_arg(args: &Value, key: &str) -> AppResult<Option<NaiveDate>> {
    text_arg(args, key)
        .map(|raw| {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .map_err(|e| AppError::invalid_input(format!("{key} must be YYYY-MM-DD: {e}")))
        })
        .transpose()
}

/// Parse the call: exactly one of `segment_id` and `segment_name`, the
/// window (`to` defaults to the athlete's `today`, `from` to
/// [`DEFAULT_YEARS`] before it), and the sport.
fn parse_ask(args: &Value, today: NaiveDate) -> AppResult<SegmentAsk> {
    // A model may pass the id as the number `segment_efforts` showed it.
    let numeric_id = args
        .get("segment_id")
        .and_then(Value::as_u64)
        .map(|id| id.to_string());
    let segment_id = text_arg(args, "segment_id")
        .map(str::to_owned)
        .or(numeric_id);
    let segment = match (segment_id, text_arg(args, "segment_name")) {
        (Some(id), None) => SegmentMatch::Id(id),
        (None, Some(name)) => {
            let folded = fold_for_match(name);
            if folded.is_empty() {
                return Err(AppError::invalid_input(
                    "segment_name holds no letters to match; pass segment_id or segment_name",
                ));
            }
            SegmentMatch::Name(folded)
        }
        _ => {
            return Err(AppError::invalid_input(
                "pass exactly one of segment_id or segment_name",
            ))
        }
    };
    let to = date_arg(args, "to")?.unwrap_or(today);
    let from = date_arg(args, "from")?.unwrap_or_else(|| {
        to.checked_sub_months(Months::new(DEFAULT_YEARS * 12))
            .unwrap_or(NaiveDate::MIN)
    });
    if to < from {
        return Err(AppError::invalid_input("from must not be after to"));
    }
    Ok(SegmentAsk {
        segment,
        from,
        to,
        sport: text_arg(args, "sport_type").map(str::to_owned),
    })
}

/// The window `[from 00:00, to + 1 00:00)` on the athlete's clock, as UTC
/// instants.
fn window_instants(ask: &SegmentAsk, zone: Tz) -> (DateTime<Utc>, DateTime<Utc>) {
    use chrono::TimeZone;

    let midnight = |day: NaiveDate| {
        day.and_hms_opt(0, 0, 0).map_or_else(Utc::now, |naive| {
            zone.from_local_datetime(&naive).earliest().map_or_else(
                || Utc.from_utc_datetime(&naive),
                |at| at.with_timezone(&Utc),
            )
        })
    };
    let end = ask.to.succ_opt().map_or_else(Utc::now, midnight);
    (midnight(ask.from), end)
}

// ============================================================================
// The live listing
// ============================================================================

/// The live backend the athlete holds that lists a segment's efforts, among
/// those holding their Strava history.
async fn listing_backend(runtime: &Arc<dyn ToolRuntime>, athlete: Athlete) -> Option<String> {
    live_backend_serving(
        runtime,
        athlete.user_id,
        athlete.tenant_id,
        STRAVA,
        |descriptor| descriptor.capabilities().supports_segment_effort_history(),
    )
    .await
}

/// The live listing's answer, or `None` when it cannot answer and the stored
/// details must: no segment id, a sport asked, no backend that lists, a
/// refusal remembered, or any failure of the listing.
async fn live_listing(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    ask: &SegmentAsk,
    zone: Tz,
) -> Option<SegmentHistoryResult> {
    let SegmentMatch::Id(segment_id) = &ask.segment else {
        return None;
    };
    if ask.sport.is_some() {
        return None;
    }
    let backend = listing_backend(runtime, athlete).await?;
    let memo_key = (athlete.user_id, backend.clone());
    if refused_lately(&memo_key, Instant::now()) {
        return None;
    }
    let executor = UniversalExecutor::new(Arc::clone(runtime));
    let tenant = provider_tenant_id_str(athlete.tenant_id);
    let provider = match executor
        .auth_service
        .create_exact_provider(&backend, athlete.user_id, tenant.as_deref())
        .await
    {
        Ok(provider) => provider,
        Err(response) => {
            info!(user_id = %athlete.user_id, backend, error = ?response.error, "Segment history: the listing backend could not be authenticated; the stored details answer");
            return None;
        }
    };
    let window = SegmentEffortWindow {
        from: ask.from,
        to: ask.to,
    };
    match provider.segment_effort_history(segment_id, window).await {
        Ok(hits) => Some(listed(&backend, &hits, ask, zone)),
        Err(e) => {
            if provider_refused(&e) {
                remember_refusal(memo_key, Instant::now());
            }
            info!(user_id = %athlete.user_id, backend, error = %e, "Segment history: the live listing failed; the stored details answer");
            None
        }
    }
}

/// The live listing's hits as the answer: complete, every effort named by
/// the backend that listed it.
fn listed(
    backend: &str,
    hits: &[SegmentEffortHit],
    ask: &SegmentAsk,
    zone: Tz,
) -> SegmentHistoryResult {
    let activities: HashSet<&str> = hits.iter().map(|hit| hit.activity_id.as_str()).collect();
    let read = activities.len();
    let efforts = hits
        .iter()
        .map(|hit| effort_row(backend, backend, &hit.activity_id, None, &hit.effort))
        .collect();
    answer(
        efforts,
        AnsweredBy::LiveSegmentHistory,
        segments_read_coverage(None, read, (ask.from, ask.to), Completeness::Complete, None),
        zone,
    )
}

// ============================================================================
// The stored details
// ============================================================================

/// The athlete a call reads: the caller, or the coached athlete the dispatch
/// chokepoint resolved.
#[derive(Debug, Clone, Copy)]
struct Athlete {
    user_id: Uuid,
    tenant_id: TenantId,
}

/// The keys the athlete's Strava activities are stored under: every key
/// their history is held under, and their own export.
fn segment_keys() -> Vec<&'static str> {
    let mut keys = history_store_keys(STRAVA);
    keys.extend(companion_keys(STRAVA));
    keys
}

/// The stored rows one call reads, up to its cap of activities.
///
/// The cap counts activities, not rows: the session's, the grant's and the
/// export's copies of one ride are one ([`one_activity_key`]), so an
/// athlete whose rides each sit under two or three keys reads as deep a
/// window as one whose rides sit under one. A row naming no activity its
/// copies share counts as its own. Memory stays bounded all the same: no
/// activity is stored under more than one row per key, so the rows read stop
/// at `cap x keys` ([`Self::row_cap`]) whatever they hold.
struct WindowRows {
    rows: Vec<Activity>,
    /// The activities the rows hold, by the key their copies share.
    activities: HashSet<(String, String)>,
    /// Activities the read takes at most.
    cap: usize,
    /// Rows the read takes at most: `cap` activities' copies under every key.
    row_cap: usize,
}

impl WindowRows {
    /// A read of at most `cap` activities, each stored under at most
    /// `copies` keys.
    fn new(cap: usize, copies: usize) -> Self {
        Self {
            rows: Vec::new(),
            activities: HashSet::new(),
            cap,
            row_cap: cap.saturating_mul(copies.max(1)),
        }
    }

    /// Take one page of rows; whether the read is full.
    fn take(&mut self, page: impl IntoIterator<Item = Activity>) -> bool {
        for row in page {
            self.activities.insert(one_activity_key(&row));
            self.rows.push(row);
        }
        self.activities.len() >= self.cap || self.rows.len() >= self.row_cap
    }
}

/// The stored window, as this transport may serve it.
struct StoredWindow {
    /// Its activities, one per workout, of the sport asked.
    activities: Vec<Activity>,
    /// Whether a gate withheld any stored row from this call, or the call is
    /// served over an external transport, which withholds every Strava row.
    withheld: bool,
    /// Whether the read stopped at [`MAX_WINDOW_ACTIVITIES`].
    capped: bool,
    /// Its activities whose efforts were not read and that only the
    /// athlete's own export holds, which no walk reads.
    export_only: usize,
}

/// Read the stored window: every row under [`segment_keys`], newest first,
/// filtered as this call may see it, matched to the sport asked, and merged
/// so the copies of one workout are one.
async fn stored_window(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    (start, end): (DateTime<Utc>, DateTime<Utc>),
    sport: Option<&str>,
) -> AppResult<StoredWindow> {
    let keys = segment_keys();
    let end = end - Duration::nanoseconds(1);
    let mut rows = WindowRows::new(MAX_WINDOW_ACTIVITIES, keys.len());
    let mut below: Option<CachedRowCursor> = None;
    let capped = loop {
        let mut query = CachedRowsQuery::window(start, end, STORE_PAGE_ROWS).under_keys(&keys);
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
        if rows.take(page.into_iter().map(|row| row.activity)) {
            break true;
        }
        if last_page || below.is_none() {
            break false;
        }
    };
    let registry = runtime.provider_registry();
    let rows = rows.rows;
    let (kept, withheld) =
        ai_scope::tallied(async { ai_scope::filter_activities(registry.as_ref(), rows) }).await;
    // An external transport never sees Strava's rows, stored yet or not:
    // a walk it asked would land only what it may not read.
    let withheld_rows =
        withheld.dropped > 0 || withheld.off_interface > 0 || ai_scope::serving_external();
    ai_scope::record_withheld(withheld);
    let of_sport = filter_activities_by_sport_type(kept, sport);
    // The activities a copy of which a walk reads: any copy not the export.
    let walkable: HashSet<(String, String)> = of_sport
        .iter()
        .filter(|row| row.provider() != STRAVA_ARCHIVE)
        .map(one_activity_key)
        .collect();
    let (activities, _) = merge_duplicates(of_sport, &DedupConfig::from_env());
    let export_only = activities
        .iter()
        .filter(|activity| {
            activity.segment_efforts().is_none() && !walkable.contains(&one_activity_key(activity))
        })
        .count();
    Ok(StoredWindow {
        activities,
        withheld: withheld_rows,
        capped,
        export_only,
    })
}

/// The stored details' answer.
async fn stored_details(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    ask: &SegmentAsk,
    zone: Tz,
) -> AppResult<SegmentHistoryResult> {
    let instants = window_instants(ask, zone);
    let window = stored_window(runtime, athlete, instants, ask.sport.as_deref()).await?;
    let total = window.activities.len();
    let read = window
        .activities
        .iter()
        .filter(|activity| activity.segment_efforts().is_some())
        .count();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut efforts = Vec::new();
    for activity in &window.activities {
        let origin = activity_origin(activity.provider(), activity.source());
        let source = activity.source().unwrap_or(origin);
        for effort in activity.segment_efforts().into_iter().flatten() {
            if ask.segment.matches(effort) && seen.insert(effort.id.as_str()) {
                efforts.push(effort_row(
                    origin,
                    source,
                    activity.id(),
                    Some(activity.name()),
                    effort,
                ));
            }
        }
    }
    let shape = WindowShape {
        total,
        read,
        export_only: window.export_only,
        withheld: window.withheld,
        capped: window.capped,
        floor_ts: instants.0.timestamp(),
        top_ts: instants.1.timestamp(),
    };
    let (completeness, pending) = coverage_of(runtime, athlete, ask, shape).await;
    Ok(answer(
        efforts,
        AnsweredBy::StoredDetails,
        segments_read_coverage(Some(total), read, (ask.from, ask.to), completeness, pending),
        zone,
    ))
}

/// What the stored window held, for its coverage.
#[derive(Debug, Clone, Copy)]
struct WindowShape {
    /// Activities in the window.
    total: usize,
    /// Those whose segments were read.
    read: usize,
    /// Those whose segments were not read that only the athlete's own export
    /// holds, which no walk reads.
    export_only: usize,
    /// Whether a gate withheld any stored row from this call, or the call is
    /// served over an external transport, which withholds every Strava row.
    withheld: bool,
    /// Whether the read stopped at its cap ([`MAX_WINDOW_ACTIVITIES`]).
    capped: bool,
    /// The window's floor (unix seconds), included.
    floor_ts: i64,
    /// The window's top (unix seconds), excluded.
    top_ts: i64,
}

impl WindowShape {
    /// Activities whose segments were not read.
    fn unread(self) -> u64 {
        u64::try_from(self.total.saturating_sub(self.read)).unwrap_or(u64::MAX)
    }

    /// Complete when every activity was read; else short by the ones that
    /// were not, which nothing tried or is reading
    /// ([`IncompleteReason::SegmentDetailUnread`]).
    fn short_untried(self) -> Completeness {
        if self.unread() == 0 {
            Completeness::Complete
        } else {
            Completeness::Incomplete(IncompleteReason::SegmentDetailUnread)
        }
    }

    /// Short by the activities a walk that ended lately tried and could not
    /// read: every unread one but those only the export holds, counted over
    /// the activities `total` counts, so the note's figures add up
    /// ([`IncompleteReason::SegmentDetailPartial`]). With none, short by the
    /// export's alone ([`Self::short_untried`]).
    fn short_after_walk(self) -> Completeness {
        let tried = self.total.saturating_sub(self.read + self.export_only);
        if tried == 0 {
            return self.short_untried();
        }
        Completeness::Incomplete(IncompleteReason::SegmentDetailPartial {
            unreadable: u64::try_from(tried).unwrap_or(u64::MAX),
        })
    }
}

/// Whether the stored window answers the whole truth, and what is still owed
/// for it.
///
/// Rows a gate withheld ask nothing ([`settled_before_walk`]). Otherwise the
/// window's unread activities ask a detail walk ([`segments_owed`]), a
/// capped window's too: the walk counts the whole window itself, and an
/// athlete heavy enough to fill the cap is the one with the most to read.
/// The history walk is read, and the reasons are weighed ([`settle`]).
async fn coverage_of(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    ask: &SegmentAsk,
    shape: WindowShape,
) -> (Completeness, Option<Pending>) {
    if let Some(settled) = settled_before_walk(shape) {
        return settled;
    }
    let segments = segments_owed(runtime, athlete, ask, shape).await;
    let history = history_owed(runtime, athlete, shape.floor_ts).await;
    settle(shape, segments, history)
}

/// The coverage of a window that asks no walk: one a gate withheld rows of,
/// which is what this call sees, capped when the read stopped short, else
/// its unread activities named unread. `None` when the window may ask one.
fn settled_before_walk(shape: WindowShape) -> Option<(Completeness, Option<Pending>)> {
    if !shape.withheld {
        return None;
    }
    if shape.capped {
        return Some((Completeness::Incomplete(IncompleteReason::ReadCapped), None));
    }
    Some((shape.short_untried(), None))
}

/// Weigh the reasons a window that asked its walk is short, in order:
/// 1. A window the read stopped short of is capped: its counts are only the
///    newest activities', so it says so ([`IncompleteReason::ReadCapped`]), with
///    the history walk still copying, or else the detail walk reading, as
///    what is owed for it.
/// 2. A window deeper than the stored history reaches, while the history
///    walk is still copying it in, says so first
///    ([`IncompleteReason::HistoryWalkRunning`]).
/// 3. Otherwise the segment reason ([`segments_owed`]).
fn settle(
    shape: WindowShape,
    segments: (Completeness, Option<Pending>),
    history: Option<Pending>,
) -> (Completeness, Option<Pending>) {
    if shape.capped {
        return (
            Completeness::Incomplete(IncompleteReason::ReadCapped),
            history.or(segments.1),
        );
    }
    if let Some(history) = history {
        return (
            Completeness::Incomplete(IncompleteReason::HistoryWalkRunning),
            Some(history),
        );
    }
    segments
}

/// The Strava history walk still copying in a window deeper than the stored
/// Strava history reaches, or `None` when the history stands behind it.
///
/// A walk owed under any of Strava's history keys is reported with its
/// progress ([`history_walk_owed`]). With none owed, the walk is asked as a
/// read finding the depth missing asks it ([`start_history_walk`]), and a
/// started one is [`Pending::HistoryNotWalked`]: the window holds only what
/// the store holds so far, however complete its segments read. Strava's
/// alone: another provider's walk says nothing of a segment's window, and is
/// never started for one. `None` too when no walk can be started: the
/// athlete holds no live Strava source, or the walk reached the end of the
/// history, which then holds all there is.
async fn history_owed(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    floor_ts: i64,
) -> Option<Pending> {
    if history_depth_covered(
        runtime,
        STRAVA,
        athlete.user_id,
        athlete.tenant_id,
        floor_ts,
    )
    .await
    {
        return None;
    }
    if let Some(key) = history_walk_owed(runtime, STRAVA, athlete.user_id, athlete.tenant_id).await
    {
        return Some(
            owed_history_walk_pending(runtime, athlete.tenant_id, athlete.user_id, &key).await,
        );
    }
    start_history_walk(
        runtime,
        athlete.user_id,
        athlete.tenant_id,
        STRAVA,
        WalkTrigger::DepthMissing,
    )
    .await
    .then_some(Pending::HistoryNotWalked)
}

/// The segment reason of a window with unread activities, asking the walk
/// that reads them: with a live backend that reads segment efforts, a detail
/// walk of the window is asked, which is reading them
/// ([`IncompleteReason::SegmentDetailWalking`]), or ended lately leaving some
/// it tried unreadable ([`IncompleteReason::SegmentDetailPartial`], counted
/// apart from those only the export holds); what no walk tried or is
/// reading, those only the export holds or a window whose walk could not be
/// asked, is unread and never called unreadable
/// ([`IncompleteReason::SegmentDetailUnread`]); one connected but
/// unable to read them now (a credential to re-authorize, an app with no
/// call a day for a walk) is [`IncompleteReason::ProviderUnavailable`]; with
/// none connected, [`IncompleteReason::SegmentDetailUnavailable`]. An
/// athlete with no Strava connection and nothing stored reads complete and
/// empty. A capped window asks whatever its newest activities held: the
/// walk counts the rest.
async fn segments_owed(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    ask: &SegmentAsk,
    shape: WindowShape,
) -> (Completeness, Option<Pending>) {
    let backend = match segment_detail_serving(runtime, athlete.user_id, athlete.tenant_id, STRAVA)
        .await
    {
        Serving::Live(backend) => backend,
        Serving::Lapsed if shape.unread() == 0 && shape.total > 0 => {
            return (Completeness::Complete, None);
        }
        Serving::Lapsed => {
            return (
                Completeness::Incomplete(IncompleteReason::ProviderUnavailable),
                None,
            );
        }
        Serving::Absent => {
            if shape.unread() == 0 && (shape.total > 0 || !strava_connected(runtime, athlete).await)
            {
                return (Completeness::Complete, None);
            }
            return (
                Completeness::Incomplete(IncompleteReason::SegmentDetailUnavailable),
                None,
            );
        }
    };
    if complete_without_walk(shape) {
        return (Completeness::Complete, None);
    }
    let asked = ask_detail_walk(
        runtime,
        athlete.user_id,
        athlete.tenant_id,
        &backend,
        DetailWalkAsk {
            after_ts: shape.floor_ts,
            before_ts: shape.top_ts,
            sport: ask.sport.as_deref(),
        },
    )
    .await;
    match asked {
        DetailWalkAsked::Started { .. } | DetailWalkAsked::Widened => {
            walking(runtime, athlete, &backend).await
        }
        // Nothing was asked: a walk may still be owed and reading the window.
        DetailWalkAsked::NotRecorded => {
            let (completeness, pending) = walking(runtime, athlete, &backend).await;
            if pending.is_some() {
                (completeness, pending)
            } else {
                (shape.short_untried(), None)
            }
        }
        DetailWalkAsked::RecentlyWalked { .. } => (shape.short_after_walk(), None),
        // Nothing a walk reads is unread: the rest only the export holds.
        DetailWalkAsked::NothingUnread => (shape.short_untried(), None),
        // The backend the athlete connected cannot be read within its budget.
        DetailWalkAsked::NoBudget => (
            Completeness::Incomplete(IncompleteReason::ProviderUnavailable),
            None,
        ),
    }
}

/// A detail walk through `backend` reading the window, with its progress
/// when one is owed ([`detail_walk_pending`]).
async fn walking(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    backend: &str,
) -> (Completeness, Option<Pending>) {
    let pending = detail_walk_pending(runtime, athlete.user_id, athlete.tenant_id, backend)
        .await
        .unwrap_or_else(|e| {
            warn!(user_id = %athlete.user_id, backend, error = %e, "Segment history: the detail walk's progress could not be read");
            None
        });
    (
        Completeness::Incomplete(IncompleteReason::SegmentDetailWalking),
        pending,
    )
}

/// Whether the window is complete with no walk asked: every activity it read
/// was read, and the read was not capped. A capped window's newest activities say
/// nothing of the rest, which the walk counts.
fn complete_without_walk(shape: WindowShape) -> bool {
    shape.unread() == 0 && !shape.capped
}

/// Whether the athlete holds any live Strava backend, session or grant.
async fn strava_connected(runtime: &Arc<dyn ToolRuntime>, athlete: Athlete) -> bool {
    history_source_order(runtime.repos(), athlete.user_id, athlete.tenant_id, STRAVA)
        .await
        .iter()
        .any(|source| source.live)
}

// ============================================================================
// The tool
// ============================================================================

fn string_property(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

fn input_schema() -> pierre_mcp_schema::JsonSchema {
    let mut properties: HashMap<String, PropertySchema> = HashMap::new();
    properties.insert(
        "segment_id".to_owned(),
        string_property("Segment id, from an activity's segment_efforts. This or segment_name."),
    );
    properties.insert(
        "segment_name".to_owned(),
        string_property(
            "Segment name or part of it; case and accents ignored. This or segment_id.",
        ),
    );
    properties.insert(
        "from".to_owned(),
        string_property("YYYY-MM-DD; default ten years before to."),
    );
    properties.insert(
        "to".to_owned(),
        string_property("YYYY-MM-DD, included; default today."),
    );
    properties.insert(
        "sport_type".to_owned(),
        string_property("Only this sport (Ride, Run)."),
    );
    object_schema(properties, None)
}

/// Tool listing every effort the athlete made on one segment.
#[derive(Debug)]
pub struct GetSegmentHistoryTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetSegmentHistoryTool {
    fn definition(&self) -> Tool {
        answers_with::<SegmentHistoryResult>(tool_definition(
            "get_segment_history",
            "A segment's efforts (a named climb or road stretch) in a window, each with its \
             activity; summary has every effort's count per year and best time; efforts lists \
             50 at most (best, ranked, newest). coverage.window.total counts the window's \
             activities, returned those whose segments were read; when incomplete, say how many \
             were read, not a final count or best.",
            input_schema(),
            annotations(
                "Get segment history",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let athlete = Athlete {
                user_id: context.user_id,
                tenant_id: TenantId::from_uuid(context.require_tenant()?),
            };
            let runtime = &context.resources;
            let zone = resolve_zone(
                user_timezone(runtime.repos(), athlete.user_id)
                    .await?
                    .as_deref(),
            );
            let ask = parse_ask(&args, clock_date(Utc::now(), zone))?;
            let payload = match live_listing(runtime, athlete, &ask, zone).await {
                Some(listed) => listed,
                None => stored_details(runtime, athlete, &ask, zone).await?,
            };
            ok_typed(TOOL, payload)
        }
        .await;
        tool_result_to_response(result)
    }
}

crate::declare_security!(GetSegmentHistoryTool => empty);

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use pierre_core::models::{ActivityBuilder, SportType};
    use serde_json::json;

    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap_or(NaiveDate::MIN)
    }

    /// The ride started at `at` as `key` lists it.
    fn copy(key: &str, at: i64) -> Activity {
        let built = ActivityBuilder::new(
            at.to_string(),
            "Morning ride",
            SportType::Ride,
            Utc.timestamp_opt(at, 0).single().unwrap_or_default(),
            3_600,
            key,
        );
        if key == "sciotte" {
            built.source(STRAVA).build()
        } else {
            built.build()
        }
    }

    #[test]
    fn the_read_cap_counts_activities_not_their_copies() {
        // Three activities, each listed by the session and by the grant, as
        // the newest-first read meets them.
        let page = |ats: &[i64]| -> Vec<Activity> {
            ats.iter()
                .flat_map(|&at| [copy("sciotte", at), copy(STRAVA, at)])
                .collect()
        };
        let mut rows = WindowRows::new(3, 3);
        assert!(
            !rows.take(page(&[3_000, 2_000])),
            "bug: two activities' four copies fill a read capped at three activities"
        );
        assert!(
            rows.take(page(&[1_000])),
            "bug: the third activity does not fill a read capped at three"
        );
        assert_eq!(rows.rows.len(), 6, "bug: a copy read was dropped");
    }

    #[test]
    fn the_read_still_stops_at_its_row_bound() {
        // Rows naming no shared activity count each as its own; and a
        // window whose copies outnumber its keys stops at cap x keys rows.
        let mut rows = WindowRows::new(3, 2);
        assert!(
            !rows.take([copy("uploaded", 1), copy(STRAVA, 2), copy("sciotte", 2)]),
            "bug: two activities, one with two copies, fill a read capped at three"
        );
        assert!(
            rows.take([copy("uploaded", 2)]),
            "bug: a row naming no shared activity is not counted as one"
        );
        let mut copies = WindowRows::new(2, 2);
        assert!(
            copies.take((0..4).map(|_| copy(STRAVA, 7))),
            "bug: rows past cap x keys do not stop the read"
        );
    }

    #[test]
    fn a_window_defaults_to_ten_years_up_to_the_athletes_today() {
        let ask = parse_ask(&json!({ "segment_id": "611413" }), today());
        let ask = ask.unwrap_or_else(|e| panic!("bug: a bare id is a whole ask: {e}"));
        assert_eq!(ask.to, today(), "bug: the window ends today");
        assert_eq!(
            ask.from,
            NaiveDate::from_ymd_opt(2016, 11, 14).unwrap_or(NaiveDate::MIN),
            "bug: the window starts ten years back"
        );
        assert_eq!(
            ask.segment,
            SegmentMatch::Id("611413".to_owned()),
            "bug: a bare id is not asked as that id"
        );
    }

    #[test]
    fn a_name_is_matched_folded() {
        let ask = parse_ask(&json!({ "segment_name": "  LA Bête " }), today());
        let ask = ask.unwrap_or_else(|e| panic!("bug: a name is a whole ask: {e}"));
        assert_eq!(
            ask.segment,
            SegmentMatch::Name("la bete".to_owned()),
            "bug: a name is not trimmed and folded of its case and accents"
        );
    }

    #[test]
    fn a_window_that_ends_before_it_starts_is_refused() {
        let refused = parse_ask(
            &json!({ "segment_id": "1", "from": "2026-02-01", "to": "2026-01-01" }),
            today(),
        );
        assert!(refused.is_err(), "bug: an inverted window was accepted");
    }

    fn shape(total: usize, read: usize, withheld: bool, capped: bool) -> WindowShape {
        WindowShape {
            total,
            read,
            export_only: 0,
            withheld,
            capped,
            floor_ts: 0,
            top_ts: 86_400,
        }
    }

    #[test]
    fn a_walked_window_counts_unreadable_only_what_a_walk_tried() {
        let walked = |export_only| WindowShape {
            export_only,
            ..shape(10, 4, false, false)
        };
        assert_eq!(
            walked(2).short_after_walk(),
            Completeness::Incomplete(IncompleteReason::SegmentDetailPartial { unreadable: 4 }),
            "bug: the export-only activities are counted among those a walk could not read"
        );
        assert_eq!(
            walked(6).short_after_walk(),
            Completeness::Incomplete(IncompleteReason::SegmentDetailUnread),
            "bug: a window short only by its export reads as one a walk could not read"
        );
        assert_eq!(
            shape(10, 10, false, false).short_after_walk(),
            Completeness::Complete,
            "bug: a window of read activities reads short"
        );
    }

    #[test]
    fn a_capped_window_still_asks_the_walk_unless_rows_were_withheld() {
        assert_eq!(
            settled_before_walk(shape(20_000, 20_000, false, true)),
            None,
            "bug: a capped window whose newest rows are all read asks no walk"
        );
        assert_eq!(
            settled_before_walk(shape(20_000, 9_000, true, true)),
            Some((Completeness::Incomplete(IncompleteReason::ReadCapped), None)),
            "bug: a capped window of withheld rows asks a walk or reads uncapped"
        );
        assert_eq!(
            settled_before_walk(shape(10, 4, true, false)),
            Some((
                Completeness::Incomplete(IncompleteReason::SegmentDetailUnread),
                None
            )),
            "bug: withheld rows ask a walk, or read as could not be read"
        );
    }

    #[test]
    fn only_an_uncapped_window_of_read_activities_is_complete_without_a_walk() {
        assert!(
            complete_without_walk(shape(12, 12, false, false)),
            "bug: an uncapped window with every activity read asks a walk"
        );
        assert!(
            !complete_without_walk(shape(20_000, 20_000, false, true)),
            "bug: a capped window whose newest rows are read asks no walk"
        );
        assert!(
            !complete_without_walk(shape(12, 3, false, false)),
            "bug: a window with unread activities asks no walk"
        );
    }

    #[test]
    fn a_capped_window_reads_capped_and_carries_what_is_owed_for_it() {
        let walking = Pending::detail_walk_running(3, Some(40), None);
        let copying = Pending::running(2, Some(12), None);
        let segments = (
            Completeness::Incomplete(IncompleteReason::SegmentDetailWalking),
            Some(walking),
        );
        assert_eq!(
            settle(shape(20_000, 9_000, false, true), segments, None),
            (
                Completeness::Incomplete(IncompleteReason::ReadCapped),
                Some(walking)
            ),
            "bug: a capped window drops the walk it asked"
        );
        assert_eq!(
            settle(shape(20_000, 9_000, false, true), segments, Some(copying)),
            (
                Completeness::Incomplete(IncompleteReason::ReadCapped),
                Some(copying)
            ),
            "bug: a capped window drops the history walk"
        );
        assert_eq!(
            settle(shape(40, 9, false, false), segments, Some(copying)),
            (
                Completeness::Incomplete(IncompleteReason::HistoryWalkRunning),
                Some(copying)
            ),
            "bug: the history walk does not come first"
        );
        assert_eq!(
            settle(shape(40, 9, false, false), segments, None),
            segments,
            "bug: the segment reason is lost"
        );
    }
}
