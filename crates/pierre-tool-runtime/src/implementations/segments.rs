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
//! a non-subscriber. An external-service failure of the listing is
//! remembered for the athlete for a day ([`REFUSAL_MEMO`], [`provider_refused`]),
//! so a non-subscriber spends one listing a day, not one a call.
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
//! segment efforts. A window that starts before the stored history reaches
//! says the history walk is still copying it in, first. Rows the transport
//! withheld ask no walk.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration as StdDuration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Months, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use lru::LruCache;
use pierre_core::civil_time::{clock_date, local_date, resolve_zone};
use pierre_core::constants::oauth::providers::{companion_keys, STRAVA};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::coverage::{Completeness, Coverage, IncompleteReason, Pending};
use pierre_core::models::{Activity, SegmentEffort, TenantId};
use pierre_core::text_fold::fold_for_match;
use pierre_core::untrusted::display_line;
use pierre_database::repositories::{CachedRowCursor, CachedRowsQuery};
use pierre_mcp_schema::PropertySchema;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::activity_origin;
use pierre_providers::core::{SegmentEffortHit, SegmentEffortWindow};
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_services::training_history_read::{
    history_source_order, history_store_keys, user_timezone,
};
use pierre_tools_core::ToolResult;
use serde::Serialize;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::activity_backfill::detail_walk::{
    ask_detail_walk, detail_walk_pending, segment_detail_backend, DetailWalkAsk,
};
use crate::activity_backfill::provider_tenant_id_str;
use crate::activity_fetch::history_depth_covered;
use crate::annotations::{annotations, Effect, Reach};
use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::implementations::activity_summary::SEGMENT_NAME_MAX_CHARS;
use crate::implementations::data_helpers::segments_read_coverage;
use crate::implementations::fitness_support::filter_activities_by_sport_type;
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;
use crate::training_history_compute::history_walk_pending;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};

/// The tool's name.
const TOOL: &str = "get_segment_history";

/// How far back a window reaches when the call names no `from`.
const DEFAULT_YEARS: u32 = 10;

/// Stored rows one read of the window takes.
const STORE_PAGE_ROWS: i64 = 500;

/// Stored rows one call reads at most: past it the window is answered from
/// the newest rows read and says it stopped short ([`IncompleteReason::ReadCapped`]).
const MAX_WINDOW_ROWS: usize = 20_000;

/// How long a failure of the live listing ([`provider_refused`]) keeps the
/// athlete off it.
const REFUSAL_MEMO_TTL: StdDuration = StdDuration::from_hours(24);

/// Athletes the refusal memo holds at most; the least recently refused goes
/// first.
const REFUSAL_MEMO_ATHLETES: NonZeroUsize = match NonZeroUsize::new(10_000) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

/// When each athlete's live listing through a backend was last refused, by
/// `(athlete, backend)`: in process, bounded, read against
/// [`REFUSAL_MEMO_TTL`].
static REFUSAL_MEMO: LazyLock<Mutex<LruCache<(Uuid, String), Instant>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(REFUSAL_MEMO_ATHLETES)));

// ============================================================================
// The answer
// ============================================================================

/// Which of the two places answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnsweredBy {
    /// The provider's live listing of the segment's efforts: complete.
    LiveSegmentHistory,
    /// The segment efforts stored with the athlete's activities.
    StoredDetails,
}

/// One effort on a matched segment.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SegmentHistoryEffort {
    /// The service the effort came from, as the athlete knows it (`strava`).
    pub provider: String,
    /// The service that recorded it, when the provider relays another's.
    pub source: String,
    /// The provider's id of the activity the effort was made in.
    pub activity_id: String,
    /// That activity's title, as one line; absent from a live listing, which
    /// names only the activity's id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_name: Option<String>,
    /// When the effort started (RFC 3339, UTC).
    pub start_date: String,
    /// Seconds from the segment's start to its end.
    pub elapsed_time: u64,
    /// Seconds moving on the segment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moving_time: Option<u64>,
    /// The segment's length, in meters.
    pub distance: f64,
    /// Average heart rate over the effort (bpm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_heart_rate: Option<u32>,
    /// Average power over the effort (watts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_watts: Option<u32>,
    /// The effort's rank among the athlete's own on the segment when it was
    /// made (1 = their best then).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_rank: Option<u32>,
    /// The effort's rank on the segment's overall leaderboard when it was
    /// made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kom_rank: Option<u32>,
    /// The segment's id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    /// The segment's name, as one line: whoever created the segment wrote it.
    pub segment_name: String,
}

/// One segment the call matched.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct MatchedSegment {
    /// The service the segment is on (`strava`).
    pub provider: String,
    /// The segment's id, when its efforts carry one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The segment's name, as one line.
    pub name: String,
}

/// One segment's efforts on one service, summed.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SegmentSummary {
    /// The service the efforts came from (`strava`).
    pub provider: String,
    /// The segment's id, when its efforts carry one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    /// The segment's name, as one line.
    pub segment_name: String,
    /// Efforts listed.
    pub count: usize,
    /// Efforts listed per calendar year, in the athlete's timezone.
    pub by_year: BTreeMap<String, usize>,
    /// The fastest effort's elapsed time, in seconds.
    pub best_elapsed_time: u64,
    /// When the fastest effort started (RFC 3339, UTC).
    pub best_start_date: String,
}

/// What `get_segment_history` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SegmentHistoryResult {
    /// Every effort on the matched segments, newest first.
    pub efforts: Vec<SegmentHistoryEffort>,
    /// The distinct segments matched.
    pub segments: Vec<MatchedSegment>,
    /// The efforts summed per segment and service.
    pub summary: Vec<SegmentSummary>,
    /// Where the efforts were read: the provider's live listing, or the
    /// stored details of the athlete's activities.
    pub answered_by: AnsweredBy,
    /// Whether the efforts are all there are in the window: `window.total` is
    /// the activities in it, `window.returned` the activities whose segments
    /// were read; with `incomplete_reason` and the walk still `pending` when
    /// some were not, and the `note` saying what may be claimed.
    pub coverage: Coverage,
}

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

/// Whether Strava refused `key`'s live listing within [`REFUSAL_MEMO_TTL`]
/// of `now`.
fn refused_lately(key: &(Uuid, String), now: Instant) -> bool {
    REFUSAL_MEMO
        .lock()
        .ok()
        .and_then(|mut memo| memo.get(key).copied())
        .is_some_and(|at| now.saturating_duration_since(at) < REFUSAL_MEMO_TTL)
}

/// Remember that Strava refused `key`'s live listing at `now`.
fn remember_refusal(key: (Uuid, String), now: Instant) {
    if let Ok(mut memo) = REFUSAL_MEMO.lock() {
        memo.put(key, now);
    }
}

/// Whether `error` keeps the athlete off the listing for a day: an external
/// service error, which is how every non-`401` refusal Strava answers
/// reaches here (a `402` or `403` to a non-subscriber, a `5xx`, a `404`),
/// and also a request that could not be sent and an open circuit breaker.
/// The error carries no status to tell those apart, so all of them are
/// remembered: the stored details still answer. A gate that refused before
/// any request (this transport, the athlete's consent), a spent budget or
/// rate limit, a dead grant and an id that is not a number are not
/// remembered: none says the athlete cannot list.
fn provider_refused(error: &AppError) -> bool {
    error.code == ErrorCode::ExternalServiceError
}

/// The live backend the athlete holds that lists a segment's efforts, among
/// those holding their Strava history.
async fn listing_backend(runtime: &Arc<dyn ToolRuntime>, athlete: Athlete) -> Option<String> {
    let registry = runtime.provider_registry();
    history_source_order(runtime.repos(), athlete.user_id, athlete.tenant_id, STRAVA)
        .await
        .into_iter()
        .filter(|source| source.live)
        .find(|source| {
            registry
                .get_descriptor(&source.backend)
                .is_some_and(|descriptor| {
                    descriptor.capabilities().supports_segment_effort_history()
                })
        })
        .map(|source| source.backend)
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

/// The stored window, as this transport may serve it.
struct StoredWindow {
    /// Its activities, one per workout, of the sport asked.
    activities: Vec<Activity>,
    /// Whether a gate withheld any stored row from this call.
    withheld: bool,
    /// Whether the read stopped at [`MAX_WINDOW_ROWS`].
    capped: bool,
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
    let mut rows = Vec::new();
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
        rows.extend(page.into_iter().map(|row| row.activity));
        if rows.len() >= MAX_WINDOW_ROWS {
            break true;
        }
        if last_page || below.is_none() {
            break false;
        }
    };
    let registry = runtime.provider_registry();
    let (kept, withheld) =
        ai_scope::tallied(async { ai_scope::filter_activities(registry.as_ref(), rows) }).await;
    let withheld_rows = withheld.dropped > 0 || withheld.off_interface > 0;
    ai_scope::record_withheld(withheld);
    let of_sport = filter_activities_by_sport_type(kept, sport);
    let (activities, _) = merge_duplicates(of_sport, &DedupConfig::from_env());
    Ok(StoredWindow {
        activities,
        withheld: withheld_rows,
        capped,
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
    /// Whether a gate withheld any stored row from this call.
    withheld: bool,
    /// Whether the read stopped at its row cap.
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
    /// were not, which nothing is reading.
    fn short_by_unread(self) -> Completeness {
        if self.unread() == 0 {
            Completeness::Complete
        } else {
            Completeness::Incomplete(IncompleteReason::SegmentDetailPartial {
                unreadable: self.unread(),
            })
        }
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
    Some((shape.short_by_unread(), None))
}

/// Weigh the reasons a window that asked its walk is short, in order:
/// 1. A window the read stopped short of is capped: its counts are only the
///    newest rows', so it says so ([`IncompleteReason::ReadCapped`]), with
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

/// The history walk still copying in a window deeper than the stored
/// history reaches, or `None` when the history stands behind it.
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
    match history_walk_pending(runtime, athlete.tenant_id, athlete.user_id).await {
        Ok(pending) => pending,
        Err(e) => {
            warn!(user_id = %athlete.user_id, error = %e, "Segment history: the history walk could not be read; the window reads as stored");
            None
        }
    }
}

/// The segment reason of a window with unread activities, asking the walk
/// that reads them: with a backend that reads segment efforts, a detail walk
/// of the window is asked, which is reading them
/// ([`IncompleteReason::SegmentDetailWalking`]) or ended lately leaving some
/// unread ([`IncompleteReason::SegmentDetailPartial`]); with none,
/// [`IncompleteReason::SegmentDetailUnavailable`]. An athlete with no Strava
/// connection and nothing stored reads complete and empty. A capped window
/// asks whatever its newest rows held: the walk counts the rest.
async fn segments_owed(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    ask: &SegmentAsk,
    shape: WindowShape,
) -> (Completeness, Option<Pending>) {
    let Some(backend) =
        segment_detail_backend(runtime, athlete.user_id, athlete.tenant_id, STRAVA).await
    else {
        if shape.unread() == 0 && (shape.total > 0 || !strava_connected(runtime, athlete).await) {
            return (Completeness::Complete, None);
        }
        return (
            Completeness::Incomplete(IncompleteReason::SegmentDetailUnavailable),
            None,
        );
    };
    if shape.unread() == 0 && !shape.capped {
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
    match asked.segment_reason() {
        Some(IncompleteReason::SegmentDetailWalking) => {
            let pending =
                detail_walk_pending(runtime, athlete.user_id, athlete.tenant_id, &backend)
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
        Some(reason) => (Completeness::Incomplete(reason), None),
        // Nothing the walk can read is unread: the rest are held only in the
        // athlete's export, or the walk could not be recorded.
        None => (shape.short_by_unread(), None),
    }
}

/// Whether the athlete holds any live Strava backend, session or grant.
async fn strava_connected(runtime: &Arc<dyn ToolRuntime>, athlete: Athlete) -> bool {
    history_source_order(runtime.repos(), athlete.user_id, athlete.tenant_id, STRAVA)
        .await
        .iter()
        .any(|source| source.live)
}

// ============================================================================
// The payload
// ============================================================================

/// One effort as the answer lists it.
fn effort_row(
    provider: &str,
    source: &str,
    activity_id: &str,
    activity_name: Option<&str>,
    effort: &SegmentEffort,
) -> SegmentHistoryEffort {
    SegmentHistoryEffort {
        provider: provider.to_owned(),
        source: source.to_owned(),
        activity_id: activity_id.to_owned(),
        activity_name: activity_name.map(|name| display_line(name, SEGMENT_NAME_MAX_CHARS)),
        start_date: effort.start_date.to_rfc3339(),
        elapsed_time: effort.elapsed_time,
        moving_time: effort.moving_time,
        distance: effort.distance,
        average_heart_rate: effort.average_heart_rate,
        average_watts: effort.average_watts,
        pr_rank: effort.pr_rank,
        kom_rank: effort.kom_rank,
        segment_id: effort.segment_id.clone(),
        segment_name: display_line(&effort.name, SEGMENT_NAME_MAX_CHARS),
    }
}

/// The answer over `efforts`, newest first, with the segments they are on
/// and their summary per segment and service.
fn answer(
    mut efforts: Vec<SegmentHistoryEffort>,
    answered_by: AnsweredBy,
    coverage: Coverage,
    zone: Tz,
) -> SegmentHistoryResult {
    efforts.sort_by(|a, b| b.start_date.cmp(&a.start_date));
    let mut groups: BTreeMap<(String, String), Vec<&SegmentHistoryEffort>> = BTreeMap::new();
    for effort in &efforts {
        let segment = effort
            .segment_id
            .clone()
            .unwrap_or_else(|| effort.segment_name.clone());
        groups
            .entry((effort.provider.clone(), segment))
            .or_default()
            .push(effort);
    }
    let mut segments = Vec::with_capacity(groups.len());
    let mut summary = Vec::with_capacity(groups.len());
    for ((provider, _), group) in &groups {
        let Some(first) = group.first() else {
            continue;
        };
        segments.push(MatchedSegment {
            provider: provider.clone(),
            id: first.segment_id.clone(),
            name: first.segment_name.clone(),
        });
        summary.push(summarize(provider, group, zone));
    }
    SegmentHistoryResult {
        efforts,
        segments,
        summary,
        answered_by,
        coverage,
    }
}

/// One segment's efforts on one service, summed.
fn summarize(provider: &str, group: &[&SegmentHistoryEffort], zone: Tz) -> SegmentSummary {
    let mut by_year: BTreeMap<String, usize> = BTreeMap::new();
    for effort in group {
        let year = DateTime::parse_from_rfc3339(&effort.start_date)
            .map(|at| {
                local_date(at.with_timezone(&Utc), zone)
                    .format("%Y")
                    .to_string()
            })
            .unwrap_or_default();
        *by_year.entry(year).or_default() += 1;
    }
    let best = group
        .iter()
        .min_by_key(|effort| effort.elapsed_time)
        .copied();
    SegmentSummary {
        provider: provider.to_owned(),
        segment_id: best.and_then(|effort| effort.segment_id.clone()),
        segment_name: best
            .map(|effort| effort.segment_name.clone())
            .unwrap_or_default(),
        count: group.len(),
        by_year,
        best_elapsed_time: best.map_or(0, |effort| effort.elapsed_time),
        best_start_date: best
            .map(|effort| effort.start_date.clone())
            .unwrap_or_default(),
    }
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
    let properties: HashMap<String, PropertySchema> = [
        (
            "segment_id",
            "Segment id, from an activity's segment_efforts. This or segment_name.",
        ),
        (
            "segment_name",
            "Segment name or part of it; case and accents ignored. This or segment_id.",
        ),
        ("from", "YYYY-MM-DD; default ten years before to."),
        ("to", "YYYY-MM-DD, included; default today."),
        ("sport_type", "Only this sport (Ride, Run)."),
    ]
    .into_iter()
    .map(|(name, description)| (name.to_owned(), string_property(description)))
    .collect();
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
            "Every effort the athlete made on one segment (a named climb or stretch of road) in \
             a window, with each effort's activity, the count per year and the best time. \
             coverage.window.total counts the window's activities, returned those whose \
             segments were read; when incomplete, say how many were read, not a final count or \
             best.",
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
    use serde_json::json;

    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap_or(NaiveDate::MIN)
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
        assert_eq!(ask.segment, SegmentMatch::Id("611413".to_owned()));
    }

    #[test]
    fn a_name_is_matched_folded() {
        let ask = parse_ask(&json!({ "segment_name": "  LA Bête " }), today());
        let ask = ask.unwrap_or_else(|e| panic!("bug: a name is a whole ask: {e}"));
        assert_eq!(ask.segment, SegmentMatch::Name("la bete".to_owned()));
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
            withheld,
            capped,
            floor_ts: 0,
            top_ts: 86_400,
        }
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
                Completeness::Incomplete(IncompleteReason::SegmentDetailPartial { unreadable: 6 }),
                None
            )),
            "bug: withheld rows ask a walk"
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

    #[test]
    fn a_refusal_strava_answered_is_remembered_for_a_day() {
        let key = (Uuid::new_v4(), STRAVA.to_owned());
        let at = Instant::now();
        assert!(!refused_lately(&key, at), "bug: a fresh athlete is refused");
        remember_refusal(key.clone(), at);
        assert!(
            refused_lately(&key, at + StdDuration::from_hours(23)),
            "bug: a refusal is forgotten within the day"
        );
        assert!(
            !refused_lately(&key, at + REFUSAL_MEMO_TTL),
            "bug: a refusal outlives its day"
        );
    }

    #[test]
    fn only_an_answer_strava_gave_is_remembered() {
        assert!(provider_refused(&AppError::external_service(
            STRAVA,
            "strava API request failed with status 402 Payment Required"
        )));
        for gate in [
            AppError::unavailable_over_transport("this data is not available over this interface"),
            AppError::not_found("strava data is withheld from AI"),
            AppError::provider_auth_required(STRAVA),
            AppError::invalid_input("a Strava segment id is a number"),
        ] {
            assert!(
                !provider_refused(&gate),
                "bug: a refusal no listing answered keeps the athlete off it: {gate}"
            );
        }
    }
}
