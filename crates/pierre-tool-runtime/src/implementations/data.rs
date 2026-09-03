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

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::models::TenantId;
use serde_json::{json, Value};
use tracing::{debug, field, info, warn, Span};

use pierre_cache::{CacheKey, CacheResource};
use uuid::Uuid;

use crate::activity_backfill::{
    backfill_inline_and_serve, is_historical_backfill_window, spawn_activity_backfill,
    ActivityBackfillJob, InlineHistoricalServe,
};
use crate::activity_fetch::{
    activity_date_span, historical_depth_covered, maybe_merge_other_connections,
    read_cached_window, serve_historical_window, serve_without_primary, sort_activities,
    touch_connection_used, write_through_served_window,
};
use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    capabilities_to_tronc, object_schema, task_capable, tool_definition, tool_result_to_response,
};
use crate::implementations::athlete_stats::{GetAthleteTool, GetStatsTool};
use crate::implementations::data_helpers::{
    backfill_placeholder_message, historical_backfill_fetch_limit, historical_window_read_limit,
    provider_reconnect_note, read_only_annotations, response_cache_eligible,
    HISTORICAL_COVERAGE_BOUND_SECS,
};
use crate::implementations::fitness_support::{
    build_activities_success_response, cache_activities_result, filter_activities_by_sport_type,
    try_get_cached_activities, ActivitiesResponseParams, AnalysisType, CachedActivitiesParams,
    PaginationInfo,
};
use crate::implementations::handler_bridge;
use crate::implementations::stored_data::{
    GetHealthSnapshotsTool, GetRecoveryMetricsTool, GetSleepSessionsTool, ListDataSourcesTool,
};
use crate::protocol::provider_helpers::resolve_provider_for_tool;
use crate::protocol::{auth_required_provider, UniversalExecutor};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::config::fitness::{activity_detail_threshold, EXPENSIVE_DETAIL_PROMOTION_BUDGET};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::connection_needs_reauth;
use pierre_fitness_compute::weather::build_provider as build_weather_provider;
use pierre_fitness_compute::weather_cache_adapter::WeatherCacheRepoAdapter;
use pierre_formatters::OutputFormat;
use pierre_intelligence::physiological_constants::api_limits::{
    safe_limit_json_detailed, safe_limit_json_summary, safe_limit_toon_detailed,
    safe_limit_toon_summary, DEFAULT_ACTIVITY_LIMIT_U32, MAX_ACTIVITY_LIMIT,
};
use pierre_mcp_schema::PropertySchema;
use pierre_providers::backend_resolver;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::spi::ProviderCapabilities;
use pierre_services::weather_backfill;
use pierre_tools_core::ToolResult;
use pierre_weather::WeatherProvider;

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
                    "Unix timestamp (epoch seconds) lower bound — return only activities at or after this time. REQUIRED for any year- or date-scoped question (e.g. 'my 2022 races', 'runs last spring'): set `after` to the start of the range and `before` to the end. Do NOT try to reach older activities by raising `limit` — the feed is newest-first and will not page back far; date filters are the only way to query history. Deep historical windows are served from cache or fetched in the background: a first request may return a 'fetching, ask again shortly' status, and a client that declared the io.modelcontextprotocol/tasks extension receives a task handle to poll instead.".to_owned(),
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

        properties.insert(
            "sort_by".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Order of the returned list, applied BEFORE `limit` so the truncated set keeps the right activities. One of: 'date_desc' (default, newest first), 'date_asc' (oldest first), 'distance_desc' (longest first), 'distance_asc' (shortest first), 'duration_desc' (longest time first), 'duration_asc' (shortest time first). Map the user's wording: 'de la plus longue à la plus courte' / 'longest to shortest' => distance_desc; 'oldest first' / 'du début' => date_asc.".to_owned(),
                ),
                ..Default::default()
            },
        );

        // All parameters are optional.
        let schema = object_schema(properties, None);

        task_capable(tool_definition(
            "get_activities",
            "Retrieve user's fitness activities from connected providers. For a specific year or date range (e.g. '2022 races'), pass `after`/`before` epoch-second bounds — do NOT page recent activities via `limit` to reach old data. Use `sort_by` to honor an explicit ordering request (e.g. longest-to-shortest). Supports sport-type filtering and pagination.",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_READ)
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

            // Canonicalize to the backend that actually serves this user so an LLM
            // echoing "garmin" as an explicit arg and the stored connection
            // "sciotte_garmin" collapse to ONE provider key. They name the same
            // provider, but keying the durable cache, the backfill-coverage gate,
            // and the completion push on the raw name split them into parallel keys
            // that never saw each other — every historical re-ask re-backfilled and
            // never served or pushed. Strava was immune only because its name
            // equals its backend; this gives every mirror provider the same
            // single-key pipeline. `display_provider` recovers the user-facing name
            // ("garmin") for any copy the user or LLM sees.
            let provider_name = backend_resolver::resolve_backend(
                &context.resources.repos().auth_repos(),
                context.user_id,
                context.tenant_id.map(TenantId::from_uuid),
                &provider_name,
            )
            .await;
            let display_provider = backend_resolver::user_facing_name(&provider_name).to_owned();

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

            // Optional ordering of the returned list, applied BEFORE the display
            // limit so the truncated set reflects the user's "longest/oldest/…"
            // ask. Defaults to newest-first (`date_desc`), the historical
            // behaviour. The same order flows into both the structured array and
            // the rendered `activity_list` prose (see `sort_activities`).
            let sort_by = args
                .get("sort_by")
                .and_then(Value::as_str)
                .map_or_else(|| "date_desc".to_owned(), str::to_owned);

            let query_params = ActivityQueryParams {
                limit: Some(limit),
                offset,
                before,
                after,
            };

            // Tenant ID strings for cache keys and downstream metadata.
            let tenant_uuid = context.tenant_id.unwrap_or_else(Uuid::nil);
            let tenant_id = TenantId::from_uuid(tenant_uuid);
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
            // ...bounded by what a round-trip costs on THIS provider. On an
            // HTTP API a detail fetch is milliseconds; on a headless-browser
            // provider it is a full page navigation. A live 2026-08-12
            // Telegram turn returned its list in 37s and then spent 3m41s
            // scraping 30 detail pages one at a time — 4m37s for an answer
            // the list already supported.
            //
            // Detail is NOT dropped on expensive providers: it carries HR
            // streams, laps and the real UTC start time the date-only list
            // page lacks. It is rationed instead. The list is newest-first, so
            // the budget spends itself on the activities a coach reasons
            // about ("hier", "ma dernière sortie") and leaves the tail as
            // summaries, which already carry HR, elevation, cadence and power.
            let detail_is_cheap = context
                .resources
                .provider_registry()
                .get_descriptor(&provider_name)
                .is_some_and(|d| {
                    d.capabilities()
                        .contains(ProviderCapabilities::CHEAP_ACTIVITY_DETAIL)
                });
            let detail_threshold = activity_detail_threshold();
            let auto_promote_to_detail =
                !mode_explicit && detail_threshold > 0 && limit <= detail_threshold;
            let detail_budget = if detail_is_cheap {
                usize::MAX
            } else {
                EXPENSIVE_DETAIL_PROMOTION_BUDGET
            };

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
            let user_global = context
                .resources
                .repos()
                .users
                .get_global(context.user_id)
                .await
                .ok()
                .flatten();
            let user_timezone = user_global.as_ref().and_then(|u| u.timezone.clone());
            // User's BCP-47 locale ("fr"/"en"/"es"/"de"/"pt") drives the localized
            // sport-type tags in the rendered activity list; default to English
            // when the lookup fails or the field is empty.
            let user_locale = user_global
                .as_ref()
                .map(|u| u.locale.clone())
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "en".to_owned());

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
            if response_cache_eligible(
                auto_promote_to_detail,
                is_historical,
                sort_by != "date_desc",
            ) {
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
                    locale: user_locale.clone(),
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
            // `provider_name` is already canonicalized to the backend above, so
            // route on it directly: a deep window on a scrape-backed mirror
            // (sciotte / sciotte_garmin) goes to background backfill; a fast OAuth
            // API provider fetches inline.
            let route_to_backfill =
                is_historical && backend_resolver::is_mirror_backend(&provider_name);
            // Set to the BACKEND KEY of the elected provider when it is auth-dead
            // and a sibling connection answered in its place: the served window is
            // real, and the reconnect prompt for this provider rides along as a
            // caveat on it. The backend rather than the display name because the
            // chat pipeline mints the reconnect URL from it, and the two mint
            // routes are chosen by backend. `served_by` then names the connections
            // the rows actually came from.
            let mut dead_primary: Option<String> = None;
            // Set when the historical branch answered out of the durable cache, so
            // the write-through below can tell rows we just read from rows a
            // provider just produced. Writing the former back re-stamps their
            // `synced_at`, and `latest_activity_sync` reads that as a fresh sync.
            let mut served_from_cache = false;
            let mut served_by: Vec<String> = Vec::new();
            // An explicit `provider` argument pins the ask to ONE source — the
            // same rule `maybe_merge_other_connections` follows. A pinned
            // provider that cannot authenticate stays the whole answer:
            // substituting another connection would answer a question the
            // athlete did not ask.
            let provider_pinned = args
                .get("provider")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty());
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
                    // returned (the "2022 runs stuck at 46" bug). The year clip
                    // serves the coverage decision; it must not clip the SERVED
                    // list — `serve_historical_window` appends the open-`before`
                    // head slice above the clip and tops up a stale head.
                    let served = serve_historical_window(
                        &context.resources,
                        &provider_name,
                        context.user_id,
                        tenant_id,
                        before,
                        coverage_params.before,
                        served,
                    )
                    .await;
                    served_from_cache = true;
                    (served, None)
                } else {
                    // Synchronous reconnect (priority): if this provider's
                    // connection is flagged needs_reauth/revoked, a background
                    // backfill would just fail on the dead session and the user
                    // would loop forever on "fetching, ask again shortly". A DB
                    // read error here falls through to the normal backfill path.
                    let connections = context
                        .resources
                        .repos()
                        .provider_connections
                        .get_for_user(context.user_id, Some(tenant_id))
                        .await
                        .unwrap_or_default();
                    if connection_needs_reauth(&connections, &provider_name) {
                        // Serve what the athlete still has: a sibling connection
                        // holding a live token answers the window, and the dead
                        // provider becomes a caveat on that answer instead of
                        // replacing it. Only when nothing else can serve does the
                        // turn become the reconnect message — `tool_result_to_response`
                        // turns this `Err` into the `auth_required_provider` metadata
                        // the tool loop scans for, and `auth_recovery` hands back the
                        // short reconnect link this turn. The link is re-sent on EVERY
                        // such ask until a real reconnect clears the flag via
                        // `mark_active`.
                        let fallback = if provider_pinned {
                            None
                        } else {
                            serve_without_primary(
                                &context,
                                &provider_name,
                                is_historical,
                                &query_params,
                            )
                            .await
                        };
                        let Some(fallback) = fallback else {
                            return Err(AppError::provider_auth_required(provider_name.clone()));
                        };
                        dead_primary = Some(provider_name.clone());
                        served_by = fallback.served_by;
                        (fallback.activities, None)
                    } else if ctx.supports_tasks() {
                        // A client that declared the tasks extension: the
                        // dispatcher has already moved this execution off the
                        // request path behind a durable task handle, so the
                        // backfill runs to completion right here and the handle
                        // resolves to the real window. The "ask me again"
                        // placeholder below serves only envelopes with no way
                        // to express deferred work.
                        let mut backfill_params = query_params.clone();
                        backfill_params.limit = Some(historical_backfill_fetch_limit());
                        let job = ActivityBackfillJob {
                            resources: context.resources.clone(),
                            user_id: context.user_id,
                            tenant_id,
                            tenant_id_str: tenant_id_str.clone(),
                            provider_name: provider_name.clone(),
                            query_params: backfill_params,
                            pierre_conversation_id: context.conversation_id.clone(),
                        };
                        match backfill_inline_and_serve(job, &coverage_params, before).await {
                            InlineHistoricalServe::AuthRequired => {
                                return Err(AppError::provider_auth_required(
                                    provider_name.clone(),
                                ));
                            }
                            InlineHistoricalServe::Failed => {
                                return Ok(ToolResult::error(json!({
                                    "error": format!(
                                        "Fetching older {display_provider} history failed; \
                                         try again shortly."
                                    ),
                                })));
                            }
                            InlineHistoricalServe::Served(served) => {
                                served_from_cache = true;
                                (served, None)
                            }
                        }
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
                        // `started == false` means a backfill for this window is
                        // ALREADY in flight — the athlete has asked before. Saying
                        // "I'm pulling it now, ask again shortly" a second time
                        // reads as no progress, and it is what the live corpus
                        // caught: group_chart_ask turn 1 got that line back while
                        // the run's own log shows the backfill had already written
                        // the history. Telling the model which case it is lets it
                        // say something true either way.
                        // Whether anything will carry the finished window back
                        // on its own. A chat turn (conversation id present) with
                        // a notifier wired gets it delivered into the same
                        // conversation — on a messaging channel through that
                        // channel's adapter, in the web and mobile apps as a
                        // persisted turn. A direct MCP or A2A caller has neither,
                        // so for that one caller re-asking really is the only
                        // path and the copy must keep saying so.
                        let followed_up = context.conversation_id.is_some()
                            && context.resources.backfill_notifier().is_some();
                        let message =
                            backfill_placeholder_message(&display_provider, started, followed_up);
                        return Ok(ToolResult::ok(json!({
                            "status": "backfilling",
                            "provider": display_provider,
                            "backfill_started": started,
                            "message": message,
                        })));
                    }
                }
            } else {
                // Recent window, or a deep window on a fast OAuth API provider —
                // authenticate and fetch from the provider inline.
                let executor = UniversalExecutor::new(context.resources.clone());
                let authenticated = executor
                    .auth_service
                    .create_authenticated_provider(
                        &provider_name,
                        context.user_id,
                        tenant_id_str.as_deref(),
                    )
                    .await;

                match authenticated {
                    Ok(provider) => {
                        match provider.get_activities_with_params(&query_params).await {
                            Ok(activities) => (activities, Some(provider)),
                            Err(e) => {
                                return Ok(ToolResult::error(json!({
                                    "error": format!("Failed to fetch activities: {e}"),
                                })));
                            }
                        }
                    }
                    Err(response) => {
                        // An auth-shaped creation failure surfaces as the typed
                        // provider_auth_required error: the executor re-raises
                        // it and the chat pipeline answers with the localized
                        // reconnect link. A plain error payload here strands
                        // the athlete with a generic failure the model can
                        // only apologise about (live incident 2026-08-11).
                        let Some(dead) = auth_required_provider(&response) else {
                            let fallback_error = response.error.clone().unwrap_or_else(|| {
                                "get_activities authentication failed".to_owned()
                            });
                            let error_payload = response.result.unwrap_or_else(|| {
                                json!({
                                    "error": fallback_error,
                                })
                            });
                            return Ok(ToolResult::error(error_payload));
                        };
                        // The dead connection is one of several: an athlete whose
                        // watch token expired still has years of GPS history behind
                        // a healthy connection. Answer from the connections that
                        // still work and carry the reconnect prompt as a caveat;
                        // only a total blackout becomes the reconnect message.
                        let fallback = if provider_pinned {
                            None
                        } else {
                            serve_without_primary(
                                &context,
                                &provider_name,
                                is_historical,
                                &query_params,
                            )
                            .await
                        };
                        let Some(fallback) = fallback else {
                            return Err(AppError::provider_auth_required(dead));
                        };
                        dead_primary = Some(dead);
                        served_by = fallback.served_by;
                        (fallback.activities, None)
                    }
                }
            };

            // Only rows a PROVIDER produced are written through — never rows this
            // table produced, and never a sibling's rows filed under a dead primary.
            // `write_through_served_window` carries both reasons.
            if dead_primary.is_none() && !served_from_cache && context.tenant_id.is_some() {
                write_through_served_window(
                    &context.resources,
                    context.user_id,
                    &tenant_id,
                    &provider_name,
                    &activities,
                )
                .await;
            }

            // Record the serve against the connection that actually produced it, so
            // `resolve_most_recent` elects the backend the athlete trains on rather
            // than whichever connection was added last. A dead primary that a
            // sibling answered for is not a serve.
            if dead_primary.is_none() && context.tenant_id.is_some() {
                touch_connection_used(
                    &context.resources,
                    context.user_id,
                    tenant_id,
                    &provider_name,
                )
                .await;
            }

            // Fold in the athlete's other connections and dedup (no-op for an
            // explicit provider arg or the coverage-gated historical branch) —
            // see `maybe_merge_other_connections` for the 2026-08-22 incident
            // this exists for. Already done when a dead primary sent the window
            // through `serve_without_primary`, which merges the same set.
            let activities = if dead_primary.is_some() {
                activities
            } else {
                maybe_merge_other_connections(
                    &context,
                    &args,
                    is_historical,
                    &provider_name,
                    &query_params,
                    activities,
                )
                .await
            };

            // Apply sport_type filter server-side before any further work.
            let mut filtered_activities =
                filter_activities_by_sport_type(activities, sport_type_filter.as_deref());

            // Order by the requested key (default newest-first) BEFORE the
            // display limit, so a "longest to shortest" ask keeps the longest
            // activities instead of the most recent. The same order is honored
            // by the rendered activity_list prose downstream.
            sort_activities(&mut filtered_activities, &sort_by);

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
            // Capture the served window's TRUE size + date span BEFORE the
            // display-limit truncation, so the response can frame coverage
            // honestly ("552 in this window, showing the most recent 200")
            // rather than letting the LLM anchor on the oldest activity in the
            // truncated slice. No-op on the live-fetch path (already <= limit).
            let window_total = filtered_activities.len();
            let window_span = activity_date_span(&filtered_activities, user_timezone.as_deref());
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
                for (rank, activity) in filtered_activities.iter().enumerate() {
                    if rank >= detail_budget {
                        // Past the budget: keep the summary. Rationing, not
                        // truncation — every activity is still returned.
                        detailed.push(activity.clone());
                        continue;
                    }
                    // A merged-in row from another provider cannot be detailed
                    // through the primary provider's client — its id would 404
                    // (or worse, collide). Keep its summary.
                    if activity.provider() != provider_name
                        && activity.provider() != display_provider
                    {
                        detailed.push(activity.clone());
                        continue;
                    }
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
            // it would only accrete never-read historical entries). A window a
            // sibling served for a dead primary is skipped too: the key is the dead
            // provider's, and a later hit would replay the answer without the
            // reconnect caveat that makes it honest.
            if dead_primary.is_none()
                && response_cache_eligible(
                    auto_promote_to_detail,
                    is_historical,
                    sort_by != "date_desc",
                )
            {
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

            // Name the connections the rows came from. Normally that is the
            // elected provider; when it is auth-dead and its siblings answered,
            // it is those siblings — attributing their sessions to the provider
            // that could not answer is what would let the coach describe a
            // Strava ride as a WHOOP record.
            let source_label = if served_by.is_empty() {
                display_provider.clone()
            } else {
                served_by.join(", ")
            };

            let mut response = build_activities_success_response(ActivitiesResponseParams {
                activities: &filtered_activities,
                user_uuid: context.user_id,
                tenant_id: tenant_id_str,
                // User-facing copy: the connections that actually produced the
                // rows, already named the way an athlete names them ("garmin",
                // never the internal "sciotte_garmin" the cache keys on).
                provider_name: &source_label,
                mode: effective_mode,
                output_format,
                pagination: Some(&pagination),
                analysis_type,
                backfill_temps: &backfill_temps,
                user_timezone,
                locale: user_locale,
                window_total: Some(window_total),
                window_span,
            });

            // The reconnect prompt ACCOMPANIES the answer instead of replacing it.
            // It rides in the result payload the model reads, never in the metadata
            // the tool loop scans for `auth_required_provider` — that key aborts the
            // turn into the deterministic reconnect reply, which is exactly the
            // blanking this path exists to avoid.
            if let Some(dead) = dead_primary.as_ref() {
                if let Some(obj) = response.result.as_mut().and_then(Value::as_object_mut) {
                    obj.insert(
                        "reconnect_required".to_owned(),
                        provider_reconnect_note(backend_resolver::user_facing_name(dead), dead),
                    );
                }
            }

            handler_bridge::map_universal_response("get_activities", Ok(response))
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
pub fn create_data_tools() -> Vec<Box<dyn RuntimeTool>> {
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

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(GetActivitiesTool => UNTRUSTED_OUTPUT);
