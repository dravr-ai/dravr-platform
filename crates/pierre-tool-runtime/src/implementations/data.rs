// ABOUTME: Data access tools implementing the McpTool trait directly.
// ABOUTME: Inlines handler bodies for stored health-data queries; bridges fitness_api handlers.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Data Access Tools
//!
//! This module contains tools for accessing fitness data:
//! - `GetActivitiesTool` - Retrieve user activities with filtering and pagination
//! - `GetAthleteTool` - Get athlete profile information
//! - `GetStatsTool` - Get aggregated activity statistics
//! - `GetSleepSessionsTool` - Query stored sleep sessions
//! - `GetRecoveryMetricsTool` - Query stored recovery and readiness metrics
//! - `GetHealthSnapshotsTool` - Query stored health snapshots (body composition, vitals)
//! - `ListDataSourcesTool` - List connected data sources (devices and providers)
//!
//! Stored-data tools (sleep sessions, recovery metrics, health snapshots, data
//! sources) execute their query bodies inline. Provider-API tools (activities,
//! athlete, stats) bridge into the shared `fitness_api` handlers because those
//! handlers are also called directly from the chat pipeline prefetch stage.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use pierre_core::models::TenantId;
use serde_json::{json, Value};
use tracing::{debug, field, info, warn, Span};

use pierre_cache::{CacheKey, CacheResource};
use uuid::Uuid;

use crate::activity_backfill::{
    is_historical_backfill_window, spawn_activity_backfill, ActivityBackfillJob,
};
use crate::activity_fetch::read_cached_window;
use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::implementations::fitness_support::{
    build_activities_success_response, cache_activities_result, fetch_and_cache_athlete,
    fetch_and_cache_stats, filter_activities_by_sport_type, try_get_athlete_id_from_cache,
    try_get_cached_activities, try_get_cached_athlete, try_get_cached_stats,
    ActivitiesResponseParams, AnalysisType, CachedActivitiesParams, PaginationInfo,
};
use crate::implementations::handler_bridge;
use crate::protocol::provider_helpers::resolve_provider_for_tool;
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::config::fitness::activity_detail_threshold;
use pierre_core::errors::AppResult;
use pierre_database::repositories::BackfillCoverage;
use pierre_formatters::{format_output, OutputFormat};
use pierre_intelligence::physiological_constants::api_limits::{
    safe_limit_json_detailed, safe_limit_json_summary, safe_limit_toon_detailed,
    safe_limit_toon_summary, DEFAULT_ACTIVITY_LIMIT_U32, MAX_ACTIVITY_LIMIT,
};
use pierre_intelligence::weather::build_provider as build_weather_provider;
use pierre_intelligence::weather_cache_adapter::WeatherCacheRepoAdapter;
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_providers::backend_resolver;
use pierre_providers::core::ActivityQueryParams;
use pierre_services::weather_backfill;
use pierre_tools_core::ToolResult;
use pierre_weather::WeatherProvider;

/// Default lookback window when no explicit date range is supplied.
const DEFAULT_LOOKBACK_DAYS: i64 = 30;

/// When a historical `get_activities` query leaves `before` open, the backfill
/// gate checks cache coverage over `[after, after + 1 year]` so recent rows in
/// `[after, now]` don't mask a missing historical season.
const HISTORICAL_COVERAGE_BOUND_SECS: i64 = 365 * 24 * 60 * 60;

/// Read limit for the single deterministic durable-cache read on the historical
/// backfill path. Decoupled from the user's display limit so the cache read
/// returns the COMPLETE window (the display limit caps only the returned list
/// afterwards). Generous enough to cover a dense season (>=2 activities/day for
/// a year) without truncating the window read; the durable table is already
/// retention-bounded, so this only guards against an unbounded read.
const HISTORICAL_WINDOW_READ_LIMIT: usize = 2_000;

/// Read limit (as `usize`) for the historical backfill window read.
#[must_use]
const fn historical_window_read_limit() -> usize {
    HISTORICAL_WINDOW_READ_LIMIT
}

/// Default fetch limit a background backfill passes to the provider so the scrape
/// pages the WHOLE requested window instead of stopping at the user's display
/// limit. The sciotte date-bounded scrape stops on `in_window_count >= limit`
/// OR `oldest <= after`; with a small (display) limit it caps at the recent tail
/// and never reaches the window start. A generous limit makes `oldest <= after`
/// the binding condition, so the scrape pages the full season.
const DEFAULT_HISTORICAL_BACKFILL_FETCH_LIMIT: usize = 2_000;

/// Fetch limit a background backfill requests, from
/// `PIERRE_HISTORICAL_BACKFILL_FETCH_LIMIT` (falls back to
/// [`DEFAULT_HISTORICAL_BACKFILL_FETCH_LIMIT`]). Operators raise this for athletes
/// with very deep histories; zero/unparseable values fall back to the default.
fn historical_backfill_fetch_limit() -> usize {
    env::var("PIERRE_HISTORICAL_BACKFILL_FETCH_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_HISTORICAL_BACKFILL_FETCH_LIMIT)
}

/// Whether a cached historical window is deep enough to serve as-is.
///
/// Cached rows alone are not enough — a prior limit-capped backfill leaves only
/// the recent slice of a deep window. The window is covered when a backfill
/// reached at least as far back as `after_ts`, OR exhausted the provider feed
/// (`hit_feed_end`, so no older data exists). No coverage record ⇒ not covered.
/// `pub` so the gate decision is exercisable by the integration test suite.
#[must_use]
pub fn historical_depth_covered(coverage: Option<BackfillCoverage>, after_ts: i64) -> bool {
    coverage.is_some_and(|c| c.hit_feed_end || c.oldest_reached_ts <= after_ts)
}

/// Whether a `get_activities` query may use the `CacheKey` response cache.
///
/// One decision governs BOTH the read short-circuit and the write-through, so
/// the two never diverge. Two query shapes must bypass the response cache:
///
/// * `auto_promote_to_detail` — the cache key omits `mode`, so a cached summary
///   cannot satisfy a detail-promoted response, and a detail payload must not be
///   written under a key a summary request would later read.
/// * `is_historical` — a deep historical window must route through the
///   coverage-aware gate, never a TTL'd response that could keep serving a stale
///   slice after the coverage was purged. Excluding it from the WRITE too keeps
///   the cache from accumulating historical entries that are never read back.
///
/// `pub` so the read/write contract is exercisable by the integration test suite.
#[must_use]
pub fn response_cache_eligible(auto_promote_to_detail: bool, is_historical: bool) -> bool {
    !auto_promote_to_detail && !is_historical
}

/// Parse `start`/`end` RFC3339 args, defaulting to (now - 30d, now).
fn parse_date_range(args: &Value) -> (DateTime<Utc>, DateTime<Utc>) {
    let end = args
        .get("end")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(Utc::now, |dt| dt.with_timezone(&Utc));
    let start = args
        .get("start")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(
            || Utc::now() - Duration::days(DEFAULT_LOOKBACK_DAYS),
            |dt| dt.with_timezone(&Utc),
        );
    (start, end)
}

/// Pull the `format` arg, defaulting to JSON.
fn parse_output_format(args: &Value) -> OutputFormat {
    args.get("format")
        .and_then(Value::as_str)
        .map_or(OutputFormat::Json, OutputFormat::from_str_param)
}

/// Apply TOON formatting to a stored-data result payload, mirroring
/// `apply_format_to_response` from `fitness_api`. JSON is the no-op identity.
fn apply_format(payload: Value, data_key: &str, format: OutputFormat) -> Value {
    match format {
        OutputFormat::Json => payload,
        OutputFormat::Toon => match format_output(&payload, OutputFormat::Toon) {
            Ok(formatted) => json!({
                format!("{data_key}_toon"): formatted.data,
                "format": "toon",
            }),
            Err(e) => json!({
                data_key: payload,
                "format": "json",
                "format_fallback": true,
                "format_error": e.to_string(),
            }),
        },
    }
}

/// Annotations for read-only data retrieval tools
fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

// ============================================================================
// GetActivitiesTool - Retrieve user activities
// ============================================================================

/// Tool for retrieving user activities from fitness providers.
///
/// Supports filtering by sport type, date ranges, pagination, and
/// different output modes (summary/detailed) and formats (json/toon).
pub struct GetActivitiesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetActivitiesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Maximum number of activities to return (1-400). Use the smallest value that answers the question: 1 for 'last activity', 5-10 for 'this week', 20 for broader queries. Do NOT raise this to reach older/historical activities — the feed is newest-first; use `after`/`before` date bounds instead. Response includes has_more and pagination info for follow-up requests.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "offset".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Number of activities to skip for pagination.".to_owned()),
                ..Default::default()
            },
        );

        properties.insert(
            "sport_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by sport type (e.g., 'run', 'ride', 'swim'). Case-insensitive."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "before".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Unix timestamp (epoch seconds) upper bound — return only activities at or before this time. Pair with `after` to bound a specific year or date range (e.g. all of 2022 = after 1640995200, before 1672531200).".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "after".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Unix timestamp (epoch seconds) lower bound — return only activities at or after this time. REQUIRED for any year- or date-scoped question (e.g. 'my 2022 races', 'runs last spring'): set `after` to the start of the range and `before` to the end. Do NOT try to reach older activities by raising `limit` — the feed is newest-first and will not page back far; date filters are the only way to query history. Deep historical windows are served from cache or fetched in the background, so a first request may return a 'fetching, ask again shortly' status.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "mode".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output mode: 'summary' (default, minimal fields) or 'detailed' (full activity data).".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None, // All parameters are optional
        };

        tool_definition(
            "get_activities",
            "Retrieve user's fitness activities from connected providers. For a specific year or date range (e.g. '2022 races'), pass `after`/`before` epoch-second bounds — do NOT page recent activities via `limit` to reach old data. Supports sport-type filtering and pagination.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            // Provider arg is optional — when omitted, fall back to the user's
            // configured default. Use the resolved name for both span and log
            // rather than the raw arg string.
            let provider_name = match resolve_provider_for_tool(&args, &context).await {
                Ok(p) => p,
                Err(result) => return Ok(result),
            };

            let span = Span::current();
            span.record("provider", field::display(&provider_name));
            if let Some(tenant_id) = context.tenant_id {
                span.record("tenant_id", field::display(&tenant_id));
            }

            // notify: chat-triggered fetch of activities from a fitness provider.
            // The LLM invoking this tool counts as user_initiated — the user
            // asked the question that prompted the tool call.
            info!(
                target: "notify",
                event = "provider.fetch_started",
                provider = %provider_name,
                trigger = "user_initiated",
                "fetching activities from provider"
            );

            // Mode/format determine the safe default limit. Track whether the
            // caller explicitly picked a mode so we can auto-promote small-limit
            // queries to detail without overriding explicit choices.
            let mode_explicit = args.get("mode").is_some();
            let mode = args
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("summary");

            let analysis_type = args
                .get("analysis_type")
                .and_then(Value::as_str)
                .map_or_else(AnalysisType::default, |s| match s {
                    "weekly_summary" => AnalysisType::WeeklySummary,
                    "trend_analysis" => AnalysisType::TrendAnalysis,
                    "race_preparation" => AnalysisType::RacePreparation,
                    "recovery_assessment" => AnalysisType::RecoveryAssessment,
                    _ => AnalysisType::GeneralOverview,
                });

            let output_format = args
                .get("format")
                .and_then(Value::as_str)
                .map_or(OutputFormat::Json, OutputFormat::from_str_param);

            // Format-aware safe defaults — prevent LLM context overflow when
            // `limit` is not specified. Tunable via SAFE_LIMIT_* env vars.
            let format_aware_default = match (output_format, mode) {
                (OutputFormat::Toon, "summary") => safe_limit_toon_summary(),
                (OutputFormat::Toon, _) => safe_limit_toon_detailed(),
                (OutputFormat::Json, "summary") => safe_limit_json_summary(),
                (OutputFormat::Json, _) => safe_limit_json_detailed(),
            };

            let user_limit = args.get("limit").and_then(Value::as_u64);
            let limit = user_limit
                .and_then(|v| usize::try_from(v).ok())
                .unwrap_or(format_aware_default)
                .clamp(1, MAX_ACTIVITY_LIMIT);

            // Optional offset — MCP clients may send numbers as floats
            // (e.g. 100.0 instead of 100), accept both.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let offset = args.get("offset").and_then(|v| {
                v.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .or_else(|| v.as_f64().map(|f| f as usize))
            });

            // Optional before/after timestamps for time-based filtering. We do
            // NOT apply a default `after` — Strava silently flips ordering to
            // oldest-first when only `after` is set; pairing with `before=now`
            // upstream keeps the default newest-first behaviour.
            let before = args.get("before").and_then(Value::as_i64);
            let after = args.get("after").and_then(Value::as_i64);

            let sport_type_filter = args
                .get("sport_type")
                .and_then(Value::as_str)
                .map(str::to_owned);

            let query_params = ActivityQueryParams {
                limit: Some(limit),
                offset,
                before,
                after,
            };

            // Tenant ID strings for cache keys and downstream metadata.
            let tenant_uuid = context.tenant_id.unwrap_or_else(Uuid::nil);
            let tenant_id = TenantId::from(tenant_uuid);
            let tenant_id_str = context.tenant_id.map(|t| t.to_string());

            // ── Activity read model: three layers, two read paths — NOT three
            //    caches of the same data. (A tempting misread is "the response cache
            //    is redundant over the durable store, delete it" — it is load-bearing;
            //    see why below.) ────────────────────────────────────────────────────
            //
            //  1. CacheKey response cache — `context.resources.cache()`, the
            //     pierre-cache `Cache`: in-memory OR Redis per `REDIS_URL`. dev/prod
            //     run Redis, so it is SHARED across replicas and survives a redeploy —
            //     a TTL'd entry serves a freshly-rolled revision (TTL governs it, not
            //     the revision). Stores the raw `Vec<Activity>` keyed by (tenant, user,
            //     provider, page, before, after, sport_type) and re-runs
            //     `build_activities_success_response` per hit, so weather/mode/format
            //     stay fresh (no "stale formatted response" — only normal TTL age).
            //     Read on the RECENT path just below; on a MISS the recent path does a
            //     LIVE provider fetch — for sciotte a full ~tens-of-seconds Chrome
            //     scrape. This is the ONLY read-cache between a recent coaching turn
            //     and that scrape; a miss is a near-free `get -> None`, a hit saves the
            //     scrape (asymmetric payoff — low hit-rate does not mean low value).
            //
            //  2. Durable `activity_cache` (SQL rows) — the persistent store. Read
            //     ONLY in the historical branch (`read_cached_window`), written-through
            //     on the recent path. It does NOT backstop the recent read path, so it
            //     is not interchangeable with layer 1.
            //
            //  3. `activity_backfill_coverage` (SQL) — completeness signal ("is this
            //     deep window whole, or just its recent slice?"), read in the
            //     historical branch. The only defensible consolidation here is folding
            //     this into `activity_cache` as a per-window marker (2 tables -> 1) —
            //     not removing layer 1.
            //
            //  Recent window      -> layer 1 -> (miss) live provider fetch.
            //  Historical `after` -> the gate below: layers 2 + 3, coverage-aware.
            //
            // Cache key includes time filters to prevent stale-window hits.
            // Safe: limit is bounded by MAX_ACTIVITY_LIMIT which fits in u32.
            let per_page = u32::try_from(limit).unwrap_or(DEFAULT_ACTIVITY_LIMIT_U32);
            let offset_val = offset.unwrap_or(0);
            #[allow(clippy::cast_possible_truncation)]
            let page = if offset_val > 0 {
                (offset_val / limit + 1) as u32
            } else {
                1
            };
            let cache_key = CacheKey::new(
                tenant_id,
                context.user_id,
                provider_name.clone(),
                CacheResource::ActivityList {
                    page,
                    per_page,
                    before,
                    after,
                    sport_type: sport_type_filter.clone(),
                },
            );

            let create_pagination = |returned_count: usize| PaginationInfo {
                offset: offset.unwrap_or(0),
                limit,
                returned_count,
                has_more: returned_count == limit,
            };

            // Auto-promote small-limit queries (e.g. "my last activity") to
            // detailed mode — the provider list endpoint only returns a shallow
            // summary, so we round-trip each id through get_activity_detailed.
            // Callers who explicitly pass mode=summary retain the compact payload.
            let detail_threshold = activity_detail_threshold();
            let auto_promote_to_detail =
                !mode_explicit && detail_threshold > 0 && limit <= detail_threshold;

            // Weather provider constructed once per request — both cache and
            // live response paths reuse it for the temperature backfill pass.
            // None when WEATHER_BACKFILL_ENABLED=false.
            let weather_provider: Option<Arc<dyn WeatherProvider>> =
                if weather_backfill::is_enabled() {
                    let cache_store = Arc::new(WeatherCacheRepoAdapter::new(
                        context.resources.repos().weather_cache.clone(),
                    ));
                    Some(build_weather_provider(cache_store))
                } else {
                    None
                };

            let cache = context.resources.cache();

            // Resolve the user's IANA timezone once so activity start times can be
            // rendered in local time for display (start_date stays UTC for stable
            // windowing/sorting). Best effort: any lookup failure or unset timezone
            // falls back to UTC only. Used by both the cached and fresh paths.
            let user_timezone = context
                .resources
                .repos()
                .users
                .get_global(context.user_id)
                .await
                .ok()
                .flatten()
                .and_then(|u| u.timezone);

            // A deep historical `after` must reach the coverage-aware gate below,
            // never a cached response. The gate reads the durable window + backfill
            // coverage and re-scrapes a stale/partial slice; a CacheKey response
            // (shared across replicas, TTL'd) would short-circuit that and keep
            // serving the old slice even after the coverage was purged (the "2022
            // stuck at Jul–Dec after the coverage purge" bug). The gate still serves
            // covered windows from the durable cache, so this costs no extra scrape.
            let is_historical = after.is_some_and(is_historical_backfill_window);

            // Cache hit short-circuits the auth+fetch round-trip. Skip when
            // auto-promoting (the cache key omits mode, so a cached summary cannot
            // satisfy a detail-promoted response) or for a historical window (it
            // must route through the gate, not a stale cached response).
            if response_cache_eligible(auto_promote_to_detail, is_historical) {
                if let Some(cached_response) = try_get_cached_activities(CachedActivitiesParams {
                    cache,
                    cache_key: &cache_key,
                    user_uuid: context.user_id,
                    tenant_id: tenant_id_str.clone(),
                    provider_name: &provider_name,
                    mode,
                    output_format,
                    limit,
                    offset: offset.unwrap_or(0),
                    analysis_type,
                    weather_provider: weather_provider.clone(),
                    user_timezone: user_timezone.clone(),
                })
                .await
                {
                    return handler_bridge::map_universal_response(
                        "get_activities",
                        Ok(cached_response),
                    );
                }
            }

            // Cache miss path. A deep historical `after` must NOT scrape inline —
            // paging a provider's reverse-chronological feed back across years stalls
            // the turn and trips the sciotte navigation timeout. Serve such windows
            // from the durable activity cache; on a cold cache, kick off a bounded
            // background backfill and tell the caller to ask again shortly. The gate
            // fires ONLY for scrape-backed mirror providers (sciotte) — OAuth API
            // providers (Strava, Fitbit, …) fetch deep windows inline (fast API).
            let route_to_backfill = if is_historical {
                let backend = backend_resolver::resolve_backend(
                    &context.resources.repos().auth_repos(),
                    context.user_id,
                    context.tenant_id.map(TenantId::from),
                    &provider_name,
                )
                .await;
                backend_resolver::is_mirror_backend(&backend)
            } else {
                false
            };
            let (activities, provider) = if route_to_backfill {
                // Is the requested historical window actually cached, or only recent
                // rows that fall inside it? Read the durable window ONCE and derive
                // both the coverage decision and the served list from that single
                // result — issuing a second read with a different limit (coverage
                // probe vs serve) let the two diverge between awaits (concurrent
                // backfill/prune, or an open-`before` coverage bound that differs
                // from the serve window), so the same 2022 query could serve a full
                // 200 on one call and a partial 46 on the next. When the caller
                // leaves `before` open we bound the read to a year above `after` so
                // recent rows can't mask missing historical depth. The read limit is
                // the durable retention cap, NOT the user's display limit, so the
                // served set is the complete window; the display limit caps only the
                // returned length afterwards.
                let coverage_params = ActivityQueryParams {
                    after,
                    before: before.or_else(|| {
                        after.map(|a| a.saturating_add(HISTORICAL_COVERAGE_BOUND_SECS))
                    }),
                    limit: Some(historical_window_read_limit()),
                    offset,
                };
                let window = read_cached_window(
                    &context.resources,
                    &provider_name,
                    context.user_id,
                    tenant_id,
                    &coverage_params,
                )
                .await;
                // Depth coverage: cached rows alone aren't enough — a prior
                // limit-capped backfill leaves only the recent slice of a deep
                // window (the "2022 starts in July" bug). The window is covered
                // only when a backfill reached at least as deep as `after`, OR
                // exhausted the provider feed (no older data exists). Absent a
                // coverage record the window is treated as not-yet-backfilled.
                let after_ts = after.unwrap_or(0);
                let coverage = context
                    .resources
                    .repos()
                    .activity_cache
                    .get_backfill_coverage(context.user_id, &tenant_id, &provider_name)
                    .await
                    .unwrap_or(None);
                let depth_covered = historical_depth_covered(coverage, after_ts);
                let covered = window.is_some() && depth_covered;

                info!(
                    user_id = %context.user_id,
                    provider = %provider_name,
                    ?after,
                    ?before,
                    covered,
                    depth_covered,
                    window_len = window.as_ref().map_or(0, Vec::len),
                    "get_activities historical gate decision"
                );

                if let Some(served) = window.filter(|_| depth_covered) {
                    // The requested depth is cached AND a backfill confirmed it
                    // reaches `after` (or the feed end). Serve the COMPLETE window;
                    // the user's display limit is applied AFTER the sport filter
                    // (below), never here, so a sport-filtered ask like "my 2022
                    // runs" can't have its older runs displaced out of the limit
                    // window by other-sport activities that were never going to be
                    // returned (the "2022 runs stuck at 46" bug).
                    (served, None)
                } else {
                    // Cold cache OR a shallow (limit-capped / never-backfilled)
                    // window: page the WHOLE window in the background. The fetch
                    // limit is decoupled from the user's display limit so the
                    // sciotte date-bounded scrape pages until `oldest <= after`
                    // (or the feed end) instead of stopping at the recent tail
                    // once `in_window_count >= limit`. A partial cache self-heals:
                    // the completion push then re-answers with the full window.
                    let mut backfill_params = query_params.clone();
                    backfill_params.limit = Some(historical_backfill_fetch_limit());
                    let started = spawn_activity_backfill(ActivityBackfillJob {
                        resources: context.resources.clone(),
                        user_id: context.user_id,
                        tenant_id,
                        tenant_id_str: tenant_id_str.clone(),
                        provider_name: provider_name.clone(),
                        query_params: backfill_params,
                        pierre_conversation_id: context.conversation_id.clone(),
                    });
                    return Ok(ToolResult::ok(json!({
                        "status": "backfilling",
                        "provider": provider_name,
                        "backfill_started": started,
                        "message": format!(
                            "I'm pulling your older {provider_name} history now — this can take a \
                             minute. Ask me again shortly and it'll be ready."
                        ),
                    })));
                }
            } else {
                // Recent window, or a deep window on a fast OAuth API provider —
                // authenticate and fetch from the provider inline.
                let executor = UniversalExecutor::new(context.resources.clone());
                let provider = match executor
                    .auth_service
                    .create_authenticated_provider(
                        &provider_name,
                        context.user_id,
                        tenant_id_str.as_deref(),
                    )
                    .await
                {
                    Ok(provider) => provider,
                    Err(response) => {
                        let fallback_error = response
                            .error
                            .clone()
                            .unwrap_or_else(|| "get_activities authentication failed".to_owned());
                        let error_payload = response.result.unwrap_or_else(|| {
                            json!({
                                "error": fallback_error,
                            })
                        });
                        return Ok(ToolResult::error(error_payload));
                    }
                };

                match provider.get_activities_with_params(&query_params).await {
                    Ok(activities) => (activities, Some(provider)),
                    Err(e) => {
                        return Ok(ToolResult::error(json!({
                            "error": format!("Failed to fetch activities: {e}"),
                        })));
                    }
                }
            };

            // Write-through to the persistent activity cache so the snapshot
            // (stale-while-revalidate) path and later turns can serve these without
            // re-fetching. Best-effort: a cache failure never blocks the response.
            if context.tenant_id.is_some() && !activities.is_empty() {
                let cache_repo = context.resources.repos().activity_cache.clone();
                if let Err(e) = cache_repo
                    .upsert_activities(context.user_id, &tenant_id, &provider_name, &activities)
                    .await
                {
                    warn!(
                        user_id = %context.user_id,
                        provider = %provider_name,
                        error = %e,
                        "Activity cache: write-through from get_activities failed"
                    );
                }
            }

            // Apply sport_type filter server-side before any further work.
            let mut filtered_activities =
                filter_activities_by_sport_type(activities, sport_type_filter.as_deref());

            // Stable newest-first ordering matches the provider default and
            // mirrors what cached responses also produce.
            filtered_activities.sort_by_key(|a| Reverse(a.start_date()));

            // Apply the user's display limit AFTER the sport filter. The gate above
            // serves the COMPLETE window; truncating before this filter let other
            // -sport activities consume the limit budget and push older matching
            // activities out (e.g. a year of runs collapsing to the recent 46).
            // The live-fetch path already returns a provider-limited set, so this
            // is a no-op there. Logged so the served size is observable.
            info!(
                provider = %provider_name,
                filtered_len = filtered_activities.len(),
                limit,
                "get_activities served list (post sport-filter, pre display-limit)"
            );
            filtered_activities.truncate(limit);

            // Auto-promote small-limit queries to detailed by issuing
            // get_activity_detailed per id. N+1 fetch bounded by
            // detail_threshold (default 20). The _detailed variant parses the
            // richer detail-endpoint shape on providers that have one (Strava);
            // providers without a detail endpoint inherit the default impl that
            // forwards to get_activity.
            let effective_mode = if let (true, Some(provider)) = (
                auto_promote_to_detail && !filtered_activities.is_empty(),
                provider.as_ref(),
            ) {
                let mut detailed = Vec::with_capacity(filtered_activities.len());
                let original_count = filtered_activities.len();
                for activity in &filtered_activities {
                    match provider.get_activity_detailed(activity.id()).await {
                        Ok(detail) => detailed.push(detail),
                        Err(err) => {
                            warn!(
                                activity_id = %activity.id(),
                                error = %err,
                                "Detail fetch failed — retaining summary for this activity"
                            );
                            detailed.push(activity.clone());
                        }
                    }
                }
                debug!(
                    count = original_count,
                    threshold = detail_threshold,
                    "Auto-promoted get_activities to detailed (N+1 fetch)"
                );
                filtered_activities = detailed;
                "detailed"
            } else {
                mode
            };

            // Write-through under the SAME eligibility as the read above: skip the
            // auto-promoted detail payload (the key omits mode) and the historical
            // window (its serve never reads this cache, so a write here is dead —
            // it would only accrete never-read historical entries).
            if response_cache_eligible(auto_promote_to_detail, is_historical) {
                cache_activities_result(cache, &cache_key, &filtered_activities, per_page).await;
            }

            let pagination = create_pagination(filtered_activities.len());

            // Weather backfill — first call per (lat, lng, hour) bucket hits the
            // vendor API, all subsequent calls hit weather_cache.
            let backfill_temps = if let Some(provider) = weather_provider {
                weather_backfill::fill_activity_temperatures(&filtered_activities, provider).await
            } else {
                HashMap::new()
            };

            let response = build_activities_success_response(ActivitiesResponseParams {
                activities: &filtered_activities,
                user_uuid: context.user_id,
                tenant_id: tenant_id_str,
                provider_name: &provider_name,
                mode: effective_mode,
                output_format,
                pagination: Some(&pagination),
                analysis_type,
                backfill_temps: &backfill_temps,
                user_timezone,
            });

            handler_bridge::map_universal_response("get_activities", Ok(response))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetAthleteTool - Get athlete profile
// ============================================================================

/// Tool for retrieving the user's athlete profile from a fitness provider.
pub struct GetAthleteTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetAthleteTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };

        tool_definition(
            "get_athlete",
            "Retrieve the user's athlete profile from connected fitness providers including personal details and preferences",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let provider_name = match resolve_provider_for_tool(&args, &context).await {
                Ok(p) => p,
                Err(result) => return Ok(result),
            };

            let output_format = parse_output_format(&args);

            let tenant_id = TenantId::from(context.tenant_id.unwrap_or_else(Uuid::nil));
            let tenant_id_str = context.tenant_id.map(|t| t.to_string());

            let cache_key = CacheKey::new(
                tenant_id,
                context.user_id,
                provider_name.clone(),
                CacheResource::AthleteProfile,
            );

            let cache = context.resources.cache();

            // Cache hit short-circuits the provider auth + fetch round-trip.
            // `try_get_cached_athlete` already builds the formatted response
            // metadata (id/tenant/cached:true) — wrap it in the same
            // ToolResult shape `map_universal_response` produces on success.
            if let Some(cached_response) = handler_bridge::map_protocol_result(
                "get_athlete",
                try_get_cached_athlete(
                    cache,
                    &cache_key,
                    context.user_id,
                    tenant_id_str.as_ref(),
                    output_format,
                )
                .await,
            )? {
                return handler_bridge::map_universal_response("get_athlete", Ok(cached_response));
            }

            // Cache miss path — authenticate and fetch from provider.
            let executor = UniversalExecutor::new(context.resources.clone());
            let provider = match executor
                .auth_service
                .create_authenticated_provider(
                    &provider_name,
                    context.user_id,
                    tenant_id_str.as_deref(),
                )
                .await
            {
                Ok(provider) => provider,
                Err(response) => {
                    let fallback_error = response
                        .error
                        .clone()
                        .unwrap_or_else(|| "get_athlete authentication failed".to_owned());
                    let error_payload = response.result.unwrap_or_else(|| {
                        json!({
                            "error": fallback_error,
                        })
                    });
                    return Ok(ToolResult::error(error_payload));
                }
            };

            handler_bridge::map_universal_response(
                "get_athlete",
                fetch_and_cache_athlete(
                    provider.as_ref(),
                    cache,
                    &cache_key,
                    context.user_id,
                    tenant_id_str,
                    output_format,
                )
                .await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetStatsTool - Get activity statistics
// ============================================================================

/// Tool for retrieving aggregated activity statistics from a fitness provider.
pub struct GetStatsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetStatsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };

        tool_definition(
            "get_stats",
            "Retrieve aggregated activity statistics from a connected fitness provider. The top-level total_* fields are ALL-TIME / lifetime totals. When the provider supplies it (currently Strava), a `year_to_date` object holds CURRENT-CALENDAR-YEAR totals — use that for 'this year' / annual questions and never report the all-time totals as annual. If `year_to_date` is absent, the provider does not expose annual figures. IMPORTANT: Strava's ride and run totals here count ONLY the base sport type and EXCLUDE variant disciplines (VirtualRide, GravelRide, MountainBikeRide, EBikeRide; TrailRun, VirtualRun), so they undercount multi-discipline athletes. For a true cross-discipline total (e.g. 'total km cycling this year'), do NOT report this single ride/run figure — call get_activities for the period and sum distance across all related sport types.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let provider_name = match resolve_provider_for_tool(&args, &context).await {
                Ok(p) => p,
                Err(result) => return Ok(result),
            };

            let output_format = parse_output_format(&args);

            // get_stats needs an athlete_id (provider-specific u64) for its
            // cache key shape. The first lookup is cheap when an athlete profile
            // is already cached; otherwise we fall through to the live API path.
            let tenant_id = TenantId::from(context.tenant_id.unwrap_or_else(Uuid::nil));
            let tenant_id_str = context.tenant_id.map(|t| t.to_string());

            let athlete_cache_key = CacheKey::new(
                tenant_id,
                context.user_id,
                provider_name.clone(),
                CacheResource::AthleteProfile,
            );

            let cache = context.resources.cache();

            if let Some(athlete_id) = try_get_athlete_id_from_cache(cache, &athlete_cache_key).await
            {
                let stats_cache_key = CacheKey::new(
                    tenant_id,
                    context.user_id,
                    provider_name.clone(),
                    CacheResource::Stats { athlete_id },
                );

                if let Some(cached_response) = handler_bridge::map_protocol_result(
                    "get_stats",
                    try_get_cached_stats(
                        cache,
                        &stats_cache_key,
                        context.user_id,
                        tenant_id_str.as_ref(),
                        output_format,
                    )
                    .await,
                )? {
                    return handler_bridge::map_universal_response(
                        "get_stats",
                        Ok(cached_response),
                    );
                }
            }

            // Cache miss path — authenticate and fetch from provider.
            let executor = UniversalExecutor::new(context.resources.clone());
            let provider = match executor
                .auth_service
                .create_authenticated_provider(
                    &provider_name,
                    context.user_id,
                    tenant_id_str.as_deref(),
                )
                .await
            {
                Ok(provider) => provider,
                Err(response) => {
                    let fallback_error = response
                        .error
                        .clone()
                        .unwrap_or_else(|| "get_stats authentication failed".to_owned());
                    let error_payload = response.result.unwrap_or_else(|| {
                        json!({
                            "error": fallback_error,
                        })
                    });
                    return Ok(ToolResult::error(error_payload));
                }
            };

            handler_bridge::map_universal_response(
                "get_stats",
                fetch_and_cache_stats(
                    provider.as_ref(),
                    cache,
                    &athlete_cache_key,
                    tenant_id,
                    context.user_id,
                    &provider_name,
                    output_format,
                )
                .await,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Helper - shared schema builders for stored health-data tools
// ============================================================================

/// Build the standard date-range + format property set used by stored
/// health-data queries (sleep, recovery, snapshots). Inferred from the
/// handler bodies in `handlers/health_data.rs`, which read `start`, `end`,
/// and `format` from `request.parameters`.
fn date_range_properties() -> HashMap<String, PropertySchema> {
    let mut properties = HashMap::new();
    properties.insert(
        "start".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Start of the date range as RFC3339 timestamp. Defaults to 30 days ago.".to_owned(),
            ),
            ..Default::default()
        },
    );
    properties.insert(
        "end".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "End of the date range as RFC3339 timestamp. Defaults to now.".to_owned(),
            ),
            ..Default::default()
        },
    );
    properties.insert(
        "format".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Output format: 'json' (default) or 'toon' (token-efficient for LLMs).".to_owned(),
            ),
            ..Default::default()
        },
    );
    properties
}

// ============================================================================
// GetSleepSessionsTool - Query stored sleep sessions
// ============================================================================

/// Tool for querying stored sleep sessions from the database.
pub struct GetSleepSessionsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetSleepSessionsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
        };

        tool_definition(
            "get_sleep_sessions",
            "Get stored sleep sessions from the database",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = parse_output_format(&args);
            let tenant_id = TenantId::from(context.require_tenant()?);
            let (start, end) = parse_date_range(&args);

            match context
                .resources
                .repos()
                .sleep
                .get_sleep_sessions(context.user_id, &tenant_id, start, end)
                .await
            {
                Ok(sessions) => {
                    let payload = json!({
                        "count": sessions.len(),
                        "sessions": sessions,
                        "range": {
                            "start": start.to_rfc3339(),
                            "end": end.to_rfc3339(),
                        },
                    });
                    Ok(ToolResult::ok(apply_format(payload, "result", format)))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch sleep sessions: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetRecoveryMetricsTool - Query stored recovery and readiness metrics
// ============================================================================

/// Tool for querying stored recovery and readiness data from the database.
pub struct GetRecoveryMetricsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetRecoveryMetricsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
        };

        tool_definition(
            "get_recovery_metrics",
            "Get stored recovery and readiness metrics",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = parse_output_format(&args);
            let tenant_id = TenantId::from(context.require_tenant()?);
            let (start, end) = parse_date_range(&args);

            match context
                .resources
                .repos()
                .recovery
                .get_recovery_metrics(context.user_id, &tenant_id, start, end)
                .await
            {
                Ok(metrics) => {
                    let payload = json!({
                        "count": metrics.len(),
                        "metrics": metrics,
                        "range": {
                            "start": start.to_rfc3339(),
                            "end": end.to_rfc3339(),
                        },
                    });
                    Ok(ToolResult::ok(apply_format(payload, "result", format)))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch recovery metrics: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetHealthSnapshotsTool - Query stored health snapshots
// ============================================================================

/// Tool for querying stored body composition and vitals snapshots.
pub struct GetHealthSnapshotsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetHealthSnapshotsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
        };

        tool_definition(
            "get_health_snapshots",
            "Get stored health snapshots (body composition, vitals)",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = parse_output_format(&args);
            let tenant_id = TenantId::from(context.require_tenant()?);
            let (start, end) = parse_date_range(&args);

            match context
                .resources
                .repos()
                .health_snapshots
                .get_health_snapshots(context.user_id, &tenant_id, start, end)
                .await
            {
                Ok(snapshots) => {
                    let payload = json!({
                        "count": snapshots.len(),
                        "snapshots": snapshots,
                        "range": {
                            "start": start.to_rfc3339(),
                            "end": end.to_rfc3339(),
                        },
                    });
                    Ok(ToolResult::ok(apply_format(payload, "result", format)))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch health snapshots: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ListDataSourcesTool - List connected devices and providers
// ============================================================================

/// Tool for listing connected data sources (devices and providers) for the user.
pub struct ListDataSourcesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListDataSourcesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };

        tool_definition(
            "list_data_sources",
            "List connected data sources (devices and providers)",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = parse_output_format(&args);
            let tenant_id = TenantId::from(context.require_tenant()?);

            match context
                .resources
                .repos()
                .data_sources
                .list_data_sources(context.user_id, &tenant_id)
                .await
            {
                Ok(sources) => {
                    let payload = json!({
                        "count": sources.len(),
                        "sources": sources,
                    });
                    Ok(ToolResult::ok(apply_format(payload, "result", format)))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to list data sources: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all data access tools for registration
#[must_use]
pub fn create_data_tools() -> Vec<Box<dyn McpTool<dyn ToolRuntime>>> {
    vec![
        Box::new(GetActivitiesTool),
        Box::new(GetAthleteTool),
        Box::new(GetStatsTool),
        Box::new(GetSleepSessionsTool),
        Box::new(GetRecoveryMetricsTool),
        Box::new(GetHealthSnapshotsTool),
        Box::new(ListDataSourcesTool),
    ]
}
