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
//! A listing that finds no effort does not answer: empty, it is no word that
//! the athlete never rode the segment, so the stored details answer with how
//! much of the window they have read.
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
//! the athlete's own export ([`segment_keys`]), read through the one store
//! read ([`visit_stored_workouts`]): filtered as this transport and the
//! athlete's consents let a model see them, merged so the copies of one
//! activity are one, and matched to the sport asked. An activity's efforts
//! are read once a detail read of any copy read them. The efforts on the
//! segment are listed once each.
//!
//! **Coverage.** `window.total` is the activities in the window,
//! `window.returned` those whose segments were read ([`segments_read_coverage`]).
//! When fewer were read, the window says why ([`segment_completeness`]): a detail
//! walk is asked to read the rest and is reading them, a walk that ended
//! lately could not read some, or no backend the athlete holds reads
//! segment efforts. A window that starts before the stored Strava history
//! reaches says, first, that the Strava history walk is still copying it
//! in: the one owed, or the one the ask starts when none is
//! ([`history_ladder`]).
//! Rows the transport withheld, and any call over an external transport,
//! ask no walk of either kind.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Months, NaiveDate, Utc};
use chrono_tz::Tz;
use pierre_core::civil_time::{self, clock_date, resolve_zone};
use pierre_core::constants::oauth::providers::STRAVA;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coverage::{Completeness, IncompleteReason, Pending};
use pierre_core::models::TenantId;
use pierre_core::text_fold::fold_for_match;
use pierre_mcp_schema::PropertySchema;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::activity_origin;
use pierre_providers::core::{SegmentEffortHit, SegmentEffortWindow};
use pierre_services::training_history_read::{history_source_order, user_timezone, ComputeBackend};
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
use crate::activity_backfill::provider_tenant_id_str;
use crate::activity_fetch::{history_ladder, SourceHistory};
use crate::activity_query::predicate::{export_only, segments_read};
use crate::activity_query::{
    segment_keys, visit_stored_workouts, Athlete, CivilWindow, KeySet, ReadGate, SegmentCounts,
    SegmentMatch,
};
use crate::annotations::{annotations, Effect, Reach};
use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::implementations::data_helpers::segments_read_coverage;
use crate::implementations::fitness_support::activity_matches_sport;
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};

/// The tool's name.
const TOOL: &str = "get_segment_history";

/// How far back a window reaches when the call names no `from`.
const DEFAULT_YEARS: u32 = 10;

// ============================================================================
// The ask
// ============================================================================

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
    let midnight = |day: NaiveDate| civil_time::day_start_utc(day, zone);
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
/// refusal remembered, any failure of the listing, or a listing that found no
/// effort. An empty listing is not taken as the athlete never having ridden
/// the segment: the stored details say how much of the window they have read
/// instead of certifying none.
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
        Ok(hits) => listed(athlete.user_id, &backend, &hits, ask, zone),
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
/// the backend that listed it. `None` for a listing that found no effort,
/// which the stored details answer instead ([`live_listing`]).
fn listed(
    user_id: Uuid,
    backend: &str,
    hits: &[SegmentEffortHit],
    ask: &SegmentAsk,
    zone: Tz,
) -> Option<SegmentHistoryResult> {
    if hits.is_empty() {
        info!(%user_id, backend, "Segment history: the live listing found no effort; the stored details answer with what they have read");
        return None;
    }
    let activities: HashSet<&str> = hits.iter().map(|hit| hit.activity_id.as_str()).collect();
    let read = activities.len();
    let efforts = hits
        .iter()
        .map(|hit| effort_row(backend, backend, &hit.activity_id, None, &hit.effort))
        .collect();
    Some(answer(
        efforts,
        AnsweredBy::LiveSegmentHistory,
        segments_read_coverage(None, read, (ask.from, ask.to), Completeness::Complete, None),
        zone,
    ))
}

// ============================================================================
// The stored details
// ============================================================================

/// The stored details' answer: the window read through the one store read
/// under [`segment_keys`], each workout of the sport asked counted, its
/// efforts on the segment listed once each.
async fn stored_details(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    ask: &SegmentAsk,
    zone: Tz,
) -> AppResult<SegmentHistoryResult> {
    let keys = segment_keys();
    let window = CivilWindow {
        from: Some(ask.from),
        to: ask.to,
        zone,
    };
    let mut counts = SegmentCounts::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut efforts = Vec::new();
    let reach = visit_stored_workouts(
        runtime,
        athlete,
        window,
        KeySet::Only(&keys),
        ReadGate::Model,
        &mut |workouts| {
            let of_sport = workouts.iter().filter(|workout| {
                ask.sport
                    .as_deref()
                    .is_none_or(|sport| activity_matches_sport(&workout.session, sport))
            });
            for workout in of_sport {
                counts.total += 1;
                if segments_read(workout) {
                    counts.read += 1;
                } else if export_only(workout) {
                    counts.export_only += 1;
                }
                let activity = &workout.session;
                let origin = activity_origin(activity.provider(), activity.source());
                let source = activity.source().unwrap_or(origin);
                for effort in activity.segment_efforts().into_iter().flatten() {
                    if ask.segment.matches(effort) && seen.insert(effort.id.clone()) {
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
        },
    )
    .await?;
    // An external transport never sees Strava's rows, stored yet or not:
    // a walk it asked would land only what it may not read.
    counts.withheld = reach.segment_rows_withheld || ai_scope::serving_external();
    let (start, end) = window_instants(ask, zone);
    let (completeness, pending) = segment_completeness(
        runtime,
        athlete,
        start.timestamp(),
        end.timestamp(),
        ask.sport.as_deref(),
        counts,
    )
    .await;
    Ok(answer(
        efforts,
        AnsweredBy::StoredDetails,
        segments_read_coverage(
            Some(counts.total),
            counts.read,
            (ask.from, ask.to),
            completeness,
            pending,
        ),
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

/// Whether a window's segment population ([`SegmentCounts`]) answers the
/// whole truth, and what is still owed for it.
///
/// Rows a gate withheld ask nothing ([`settled_before_walk`]). Otherwise the
/// window's unread activities ask a detail walk ([`segments_owed`]) of
/// `sport` (every sport when `None`), the Strava history walk is read on the
/// one ladder ([`history_owed`]), and the reasons are weighed ([`settle`]).
/// `get_segment_history` and a `query_activities` count with a segment
/// settle their coverage here alike.
pub(crate) async fn segment_completeness(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    floor_ts: i64,
    top_ts: i64,
    sport: Option<&str>,
    counts: SegmentCounts,
) -> (Completeness, Option<Pending>) {
    let shape = WindowShape {
        total: counts.total,
        read: counts.read,
        export_only: counts.export_only,
        withheld: counts.withheld,
        floor_ts,
        top_ts,
    };
    if let Some(settled) = settled_before_walk(shape) {
        return settled;
    }
    let segments = segments_owed(runtime, athlete, sport, shape).await;
    let history = history_owed(runtime, athlete, shape.floor_ts).await;
    settle(segments, history)
}

/// The coverage of a window that asks no walk: one a gate withheld rows of,
/// which is what this call sees, its unread activities named unread. `None`
/// when the window may ask one.
fn settled_before_walk(shape: WindowShape) -> Option<(Completeness, Option<Pending>)> {
    shape.withheld.then(|| (shape.short_untried(), None))
}

/// Weigh the reasons a window that asked its walk is short, in order:
/// 1. A window deeper than the stored history reaches, while the history
///    walk is still copying it in, says so first
///    ([`IncompleteReason::HistoryWalkRunning`]).
/// 2. Otherwise the segment reason ([`segments_owed`]).
fn settle(
    segments: (Completeness, Option<Pending>),
    history: Option<Pending>,
) -> (Completeness, Option<Pending>) {
    if let Some(history) = history {
        return (
            Completeness::Incomplete(IncompleteReason::HistoryWalkRunning),
            Some(history),
        );
    }
    segments
}

/// The Strava history walk still copying in a window deeper than the stored
/// Strava history reaches, or `None` when the history stands behind it: the
/// one history ladder ([`history_ladder`]) asked of Strava.
///
/// A walk owed is reported with its progress. With none owed and none ever
/// asked, the walk is asked as a read finding the depth missing asks it,
/// and a started one is [`Pending::HistoryNotWalked`]: the window holds only
/// what the store holds so far, however complete its segments read.
/// Strava's alone: another provider's walk says nothing of a segment's
/// window, and is never started for one. `None` too when no walk can be
/// started: the athlete holds no live Strava source, or the walk reached the
/// end of the history, which then holds all there is.
async fn history_owed(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    floor_ts: i64,
) -> Option<Pending> {
    let strava = ComputeBackend {
        slug: STRAVA.to_owned(),
        requires_reauth: false,
        capturable: true,
    };
    match history_ladder(runtime, athlete, &strava, Some(floor_ts), true).await {
        SourceHistory::Covered | SourceHistory::Short => None,
        SourceHistory::Walking(pending) => Some(pending),
        SourceHistory::Asked => Some(Pending::HistoryNotWalked),
    }
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
/// empty.
async fn segments_owed(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    sport: Option<&str>,
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
    if shape.unread() == 0 {
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
            sport,
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

    fn shape(total: usize, read: usize, withheld: bool) -> WindowShape {
        WindowShape {
            total,
            read,
            export_only: 0,
            withheld,
            floor_ts: 0,
            top_ts: 86_400,
        }
    }

    #[test]
    fn a_walked_window_counts_unreadable_only_what_a_walk_tried() {
        let walked = |export_only| WindowShape {
            export_only,
            ..shape(10, 4, false)
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
            shape(10, 10, false).short_after_walk(),
            Completeness::Complete,
            "bug: a window of read activities reads short"
        );
    }

    #[test]
    fn only_a_window_with_rows_withheld_settles_before_any_walk() {
        assert_eq!(
            settled_before_walk(shape(20_000, 9_000, false)),
            None,
            "bug: a window of served rows asks no walk"
        );
        assert_eq!(
            settled_before_walk(shape(10, 4, true)),
            Some((
                Completeness::Incomplete(IncompleteReason::SegmentDetailUnread),
                None
            )),
            "bug: withheld rows ask a walk, or read as could not be read"
        );
        assert_eq!(
            settled_before_walk(shape(10, 10, true)),
            Some((Completeness::Complete, None)),
            "bug: a withheld window whose served activities were all read reads short"
        );
    }

    #[test]
    fn the_history_walk_comes_before_the_segment_reason() {
        let walking = Pending::detail_walk_running(3, Some(40), None);
        let copying = Pending::running(2, Some(12), None);
        let segments = (
            Completeness::Incomplete(IncompleteReason::SegmentDetailWalking),
            Some(walking),
        );
        assert_eq!(
            settle(segments, Some(copying)),
            (
                Completeness::Incomplete(IncompleteReason::HistoryWalkRunning),
                Some(copying)
            ),
            "bug: the history walk does not come first"
        );
        assert_eq!(
            settle(segments, None),
            segments,
            "bug: the segment reason is lost"
        );
    }
}
