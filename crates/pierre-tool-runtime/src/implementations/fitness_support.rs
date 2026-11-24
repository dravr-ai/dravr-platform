// ABOUTME: Shared support types and helpers for the fitness-provider API tools.
// ABOUTME: Hosts TokenEstimate / AnalysisType / ActivityRetrievalContext + cache/format helpers.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Fitness API Tool Support
//!
//! Shared types and helper functions extracted from the (now-decomposed)
//! `protocol/handlers/fitness_api.rs` so that the `McpTool::execute` bodies
//! for `get_activities` and `analyze_activity` can share them without forcing
//! a single 2k-line file. The athlete and stats reads live with their two
//! tools in `athlete_stats`, which is compiled only under `tools-data`.
//!
//! Exposed types — `TokenEstimate`, `AnalysisType`, `ActivityRetrievalContext`,
//! `PaginationInfo`, and their sub-types — are
//! consumed by the tools in
//! `crate::implementations::data` and `crate::implementations::analytics`.
//! The cache/format/response-build helpers (`try_get_cached_activities`,
//! `build_activities_success_response`, etc.) are `pub(crate)` so the same
//! tool modules can call them directly.

use crate::activity_detail_store::{served_place, stored_served_rows};
use crate::activity_query::{sport_matches, ServedList};
use crate::implementations::activities_output::ActivitiesPayload;
use crate::implementations::activity_detail_promotion::{
    effective_mode, promote_to_detail, DetailPlan, Promoted,
};
use crate::implementations::activity_list_render::format_activities_as_list;
use crate::implementations::activity_summary::{detail_json, summary_json, ActivitySummary};
use crate::implementations::data_helpers::{
    activity_coverage, pagination_has_more, AskedWindow, WindowCount,
};
use crate::implementations::field_dictionary::served_header;
use crate::implementations::session_merge_summary::FragmentDedupSummary;
use crate::protocol::types::UniversalResponse;
use crate::runtime::ToolRuntime;
use dravr_cageux::physiological_constants::api_limits::{
    CLAUDE_CONTEXT_TOKENS, CONTEXT_WARNING_THRESHOLD_PERCENT, TOKENS_PER_ACTIVITY_DETAILED,
    TOKENS_PER_ACTIVITY_SUMMARY, USABLE_CONTEXT_TOKENS,
};
use dravr_tronc::dates::Timestamp;
use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::civil_time::parse_zone;
use pierre_core::civil_time::zoned_rfc3339;
use pierre_core::config::fitness::{DETAIL_WINDOW_DAYS, EXPLICIT_DETAIL_CAP};
use pierre_core::json_value::to_value_as_written;
use pierre_core::models::coverage::{Completeness, IncompleteReason};
use pierre_core::models::{resolve_sport_type, Activity, PlaceRef, SportType, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_formatters::{format_output, OutputFormat};
use pierre_providers::ai_scope;
use pierre_providers::deduplication::FragmentReport;
use pierre_services::weather_backfill::WeatherBackfill;
use serde::Serialize;
use serde_json::{to_value, Value};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::slice;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

/// Pagination metadata for list responses
/// Enables clients to intelligently paginate through large result sets
#[derive(Debug, Clone, Default)]
pub struct PaginationInfo {
    /// The offset that was requested (0 if not specified)
    pub offset: usize,
    /// The limit that was applied to the query
    pub limit: usize,
    /// Number of items actually returned in this response
    pub returned_count: usize,
    /// True if there are likely more results available (`returned_count` == `limit`)
    pub has_more: bool,
}

/// Token usage estimation for LLM context management
/// Helps users understand how much of their context window is being used
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct TokenEstimate {
    /// Estimated tokens for this response
    pub estimated_tokens: usize,
    /// Percentage of Claude's context used (out of 200K)
    pub context_usage_percent: f64,
    /// Percentage of usable context used (out of 150K, leaving room for prompts)
    pub usable_context_percent: f64,
    /// Whether context usage is above warning threshold
    pub context_warning: bool,
    /// Human-readable context guidance
    pub guidance: String,
}

impl TokenEstimate {
    /// Create token estimate based on activity count and mode
    /// Token counts are small enough that f64 precision loss is negligible
    #[allow(clippy::cast_precision_loss)]
    fn from_activities(count: usize, mode: &str) -> Self {
        let tokens_per_activity = if mode == "summary" {
            TOKENS_PER_ACTIVITY_SUMMARY
        } else {
            TOKENS_PER_ACTIVITY_DETAILED
        };

        let estimated_tokens = count * tokens_per_activity;
        let context_usage_percent =
            (estimated_tokens as f64 / CLAUDE_CONTEXT_TOKENS as f64) * 100.0;
        let usable_context_percent =
            (estimated_tokens as f64 / USABLE_CONTEXT_TOKENS as f64) * 100.0;
        let context_warning = usable_context_percent > CONTEXT_WARNING_THRESHOLD_PERCENT as f64;

        let guidance = if context_warning {
            format!(
                "Using {usable_context_percent:.1}% of usable context. Consider filtering by sport_type or using a smaller time range."
            )
        } else if usable_context_percent > 25.0 {
            format!(
                "Using {usable_context_percent:.1}% of usable context. Plenty of room for analysis."
            )
        } else {
            format!(
                "Using {usable_context_percent:.1}% of usable context. Excellent - lots of room for detailed analysis."
            )
        };

        Self {
            estimated_tokens,
            context_usage_percent,
            usable_context_percent,
            context_warning,
            guidance,
        }
    }
}

/// Analysis type for activity retrieval - helps determine appropriate data requirements
/// Each type has different minimum data needs for meaningful analysis
#[derive(Debug, Clone, Copy, Serialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisType {
    /// General overview of recent training (default: 2 weeks minimum)
    #[default]
    GeneralOverview,
    /// Weekly training summary (minimum: 1 week)
    WeeklySummary,
    /// Trend analysis over time (minimum: 4 weeks)
    TrendAnalysis,
    /// Race preparation assessment (minimum: 8 weeks)
    RacePreparation,
    /// Recovery assessment (minimum: 1 week)
    RecoveryAssessment,
}

impl AnalysisType {
    /// Minimum weeks of data needed for meaningful analysis of this type
    #[must_use]
    pub const fn minimum_weeks_required(&self) -> u32 {
        match self {
            Self::GeneralOverview => 2,
            Self::WeeklySummary | Self::RecoveryAssessment => 1,
            Self::TrendAnalysis => 4,
            Self::RacePreparation => 8,
        }
    }

    /// Default activity limit for this analysis type
    #[must_use]
    pub const fn default_limit(&self) -> u32 {
        match self {
            Self::GeneralOverview => 20,
            Self::WeeklySummary => 10,
            Self::TrendAnalysis => 50,
            Self::RacePreparation => 80,
            Self::RecoveryAssessment => 15,
        }
    }
}

/// Breakdown of activities by sport type for sufficiency assessment
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ActivityTypeBreakdown {
    /// Sport type name (e.g., "run", "ride", "swim")
    pub sport_type: String,
    /// Number of activities of this type
    pub count: usize,
    /// Percentage of total activities
    pub percentage: f64,
}

/// Data sufficiency assessment for activity retrieval
/// Guides LLMs on whether they have enough data for analysis
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct DataSufficiency {
    /// Number of full weeks covered by the returned activities
    pub weeks_covered: u32,
    /// Total number of activities returned
    pub total_activities: usize,
    /// Breakdown by activity type
    pub activity_type_breakdown: Vec<ActivityTypeBreakdown>,
    /// Whether data is sufficient for the requested analysis type
    pub has_sufficient_data: bool,
    /// Human-readable recommendation for LLM
    pub recommendation: String,
}

/// Context metadata for activity retrieval responses
/// Provides LLMs with guidance on data sufficiency and whether to fetch more
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ActivityRetrievalContext {
    /// Type of analysis being performed
    pub analysis_type: AnalysisType,
    /// Date range of returned activities
    pub date_range: DateRange,
    /// Assessment of data sufficiency
    pub data_sufficiency: DataSufficiency,
    /// Actionable advice for the LLM when the requested payload shape
    /// has trade-offs it should be aware of.
    ///
    /// Populated when the response is summary-mode: the last 14 days carry
    /// full detail and older rows only scalar fields. The sentence tells
    /// the LLM what each row carries and how to read an older row's
    /// detail. `None` when every row is detailed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advice: Option<String>,
    /// Fragment-deduplication summary when overlapping recordings of the same
    /// workout were detected. `None` when the activity slice contained no
    /// fragment groups — most queries land here. When `Some`, the LLM MUST
    /// report `session_count` to the user, not `raw_count`: the raw rows
    /// include Garmin auto-splits, dual-device captures, and Strava
    /// re-uploads that describe the same physical workout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragment_dedup: Option<FragmentDedupSummary>,
}

/// Date range for activity retrieval
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct DateRange {
    /// Earliest activity date in ISO 8601 format
    pub start: String,
    /// Latest activity date in ISO 8601 format
    pub end: String,
    /// Number of days spanned
    pub days_spanned: u32,
}

impl ActivityRetrievalContext {
    /// Create retrieval context from activities and analysis type without
    /// fragment-deduplication metadata. Equivalent to
    /// [`Self::from_activities_with_dedup`] with `fragment_report = None`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn from_activities(
        activities: &[Activity],
        analysis_type: AnalysisType,
        mode_used: &str,
    ) -> Self {
        Self::from_activities_with_dedup(activities, analysis_type, mode_used, None)
    }

    /// Create retrieval context with optional session-merge metadata.
    ///
    /// When `fragment_report` is `Some` AND it contains at least one merged
    /// group, the resulting context surfaces a `fragment_dedup` field whose
    /// `advice` string instructs the LLM to report `session_count` to the
    /// user (the deduplicated training-session count) rather than the raw
    /// row count. Empty reports — no groups — pass through with
    /// `fragment_dedup = None` to keep the JSON shape compact.
    ///
    /// `mode_used` lets the caller tell the LLM when the response is
    /// summary-mode (limit > threshold): the `advice` field is populated
    /// with a one-line hint about what is still available (HR, elevation,
    /// cadence, power, calories) and how to narrow for splits/laps.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn from_activities_with_dedup(
        activities: &[Activity],
        analysis_type: AnalysisType,
        mode_used: &str,
        fragment_report: Option<&FragmentReport>,
    ) -> Self {
        let date_range = Self::calculate_date_range(activities);
        let weeks_covered = date_range.days_spanned / 7;
        let activity_type_breakdown = Self::calculate_type_breakdown(activities);

        let min_weeks = analysis_type.minimum_weeks_required();
        let has_sufficient_data = weeks_covered >= min_weeks && !activities.is_empty();

        let recommendation = Self::generate_recommendation(
            activities.len(),
            weeks_covered,
            min_weeks,
            has_sufficient_data,
            analysis_type,
        );

        // Surface the summary-mode trade-off exactly once, to the LLM,
        // at the point it reads the tool result. Keeps the per-activity
        // payload compact (no repeated warnings) while still informing
        // the model what the summary rows omit and how to get it.
        let advice = (mode_used == "summary").then(|| {
            format!(
                "Summary mode: activities from the last {DETAIL_WINDOW_DAYS} days carry full \
                 detail (description, device, splits, laps; segment efforts where the \
                 connection that read them returns any); older rows carry scalar fields \
                 only (HR avg/max, elevation, cadence, power, calories, suffer score, \
                 moving time). Pass mode=detailed to read the detail of older rows (up to \
                 {EXPLICIT_DETAIL_CAP} per call). For one segment's efforts across \
                 activities, call get_segment_history."
            )
        });

        let fragment_dedup = fragment_report.and_then(FragmentDedupSummary::from_report);

        Self {
            analysis_type,
            date_range,
            data_sufficiency: DataSufficiency {
                weeks_covered,
                total_activities: activities.len(),
                activity_type_breakdown,
                has_sufficient_data,
                recommendation,
            },
            advice,
            fragment_dedup,
        }
    }

    /// Calculate the date range from activities
    fn calculate_date_range(activities: &[Activity]) -> DateRange {
        if activities.is_empty() {
            let now = Timestamp::now();
            return DateRange {
                start: now.to_rfc3339(),
                end: now.to_rfc3339(),
                days_spanned: 0,
            };
        }

        let mut earliest = activities[0].start_date();
        let mut latest = activities[0].start_date();

        for activity in activities.iter().skip(1) {
            let date = activity.start_date();
            if date < earliest {
                earliest = date;
            }
            if date > latest {
                latest = date;
            }
        }

        let duration = latest.signed_duration_since(earliest);
        let days_spanned = duration.num_days().max(0) as u32;

        DateRange {
            start: earliest.to_rfc3339(),
            end: latest.to_rfc3339(),
            days_spanned,
        }
    }

    /// Calculate breakdown of activities by sport type
    #[allow(clippy::cast_precision_loss)]
    fn calculate_type_breakdown(activities: &[Activity]) -> Vec<ActivityTypeBreakdown> {
        if activities.is_empty() {
            return Vec::new();
        }

        let mut counts: HashMap<String, usize> = HashMap::new();
        for activity in activities {
            let sport_type = match activity.sport_type() {
                SportType::Other(s) => s.clone(),
                other => format!("{other:?}").to_lowercase(),
            };
            *counts.entry(sport_type).or_insert(0) += 1;
        }

        let total = activities.len().max(1) as f64;
        let mut breakdown: Vec<_> = counts
            .into_iter()
            .map(|(sport_type, count)| ActivityTypeBreakdown {
                sport_type,
                count,
                percentage: (count as f64 / total) * 100.0,
            })
            .collect();

        // Sort by count descending
        breakdown.sort_by_key(|b| Reverse(b.count));
        breakdown
    }

    /// Generate human-readable recommendation for the LLM
    fn generate_recommendation(
        activity_count: usize,
        weeks_covered: u32,
        min_weeks: u32,
        has_sufficient_data: bool,
        analysis_type: AnalysisType,
    ) -> String {
        if activity_count == 0 {
            return "No activities found. User may not have connected their fitness provider or has no recent activities.".to_owned();
        }

        if has_sufficient_data {
            return format!(
                "Data is sufficient for {analysis_type:?}. You have {activity_count} activities spanning {weeks_covered} weeks. No additional fetches needed."
            );
        }

        // Calculate how many more weeks we need
        let weeks_needed = min_weeks.saturating_sub(weeks_covered);

        if weeks_covered == 0 {
            format!(
                "Only {activity_count} activities found but they span less than a week. For {analysis_type:?}, you need at least {min_weeks} weeks of data. Consider fetching with a wider date range."
            )
        } else {
            format!(
                "Found {activity_count} activities spanning {weeks_covered} weeks, but {analysis_type:?} requires {min_weeks} weeks minimum. Need {weeks_needed} more weeks of data. Consider expanding the date range."
            )
        }
    }
}

/// Parameters for trying to get cached activities
pub(crate) struct CachedActivitiesParams<'a> {
    pub cache: &'a Arc<Cache>,
    /// What the hit's window is counted from: the store (carnet#1485, OQ6).
    pub runtime: &'a Arc<dyn ToolRuntime>,
    /// Each provider's AI policy: the cached slice may have been stored
    /// outside a read for a model (a login prefetch), so it is filtered on read.
    pub policies: &'a dyn ProviderTerms,
    pub cache_key: &'a CacheKey,
    pub user_uuid: Uuid,
    pub tenant_id: Option<String>,
    pub provider_name: &'a str,
    pub mode: &'a str,
    pub output_format: OutputFormat,
    pub limit: usize,
    /// What the provider was asked for when the window was stored (see
    /// `window_fetch_limit`): a stored window that filled it may continue.
    pub fetch_limit: usize,
    pub offset: usize,
    /// Analysis type for sufficiency calculation
    pub analysis_type: AnalysisType,
    /// The backfill of ambient temperature on activities whose source
    /// provider didn't surface it. `None` when the deployment's weather
    /// configuration turns it off.
    pub weather_backfill: Option<WeatherBackfill>,
    /// User's IANA timezone for rendering local start times (see
    /// [`ActivitiesResponseParams::user_timezone`]).
    pub user_timezone: Option<String>,
    /// User's BCP-47 locale, threaded to [`ActivitiesResponseParams::locale`]
    /// for localized sport-type labels. Defaults to "en".
    pub locale: String,
    /// What the hit details, from the stored details alone.
    pub detail: CachedDetail<'a>,
    /// The window the call asked for.
    pub asked: AskedWindow,
    /// The one source the call was pinned to, by its backend.
    pub pinned: Option<&'a str>,
    /// The call's `sport_type` filter, which the stored list was filtered by.
    pub sport_type: Option<&'a str>,
}

/// Where a cached list's recent window finds its stored details.
pub(crate) struct CachedDetail<'a> {
    pub repos: &'a RepositoryRegistry,
    pub tenant_id: TenantId,
    pub window_start: Timestamp,
    /// The provider key the list was read under.
    pub serving: &'a str,
    pub provider_names: &'a [&'a str],
}

/// Detail a cached list's recent window from the store only: the live read
/// that stored the list stored its details too. An explicit `mode=detailed`
/// never reaches a hit (`response_cache_eligible`). `None` when a recent row
/// has no stored detail yet: the hit is then a miss, and the live read
/// details it (D1, every activity of the last 14 days). The same read of the
/// served rows gives each activity's place.
async fn detail_cached_list(
    detail: &CachedDetail<'_>,
    policies: &dyn ProviderTerms,
    user_id: Uuid,
    activities: &[Activity],
) -> Option<(Promoted, HashMap<(String, String), PlaceRef>)> {
    let plan = DetailPlan {
        window_start: detail.window_start,
        explicit_cap: None,
        read_budget: 0,
        provider_names: detail.provider_names,
        terms: Some(policies),
    };
    let served = stored_served_rows(
        detail.repos,
        policies,
        detail.tenant_id,
        user_id,
        detail.serving,
        activities,
    )
    .await;
    let promoted = promote_to_detail(None, activities, &plan, &served.details).await;
    let unread = promoted.activities.iter().any(|a| {
        a.start_date() >= plan.window_start
            && plan.provider_names.contains(&a.provider())
            && !promoted.detailed_ids.contains(a.id())
    });
    if unread {
        info!("Cache hit holds a recent activity with no stored detail; reading live");
        return None;
    }
    Some((promoted, served.places))
}

/// Try to get activities from cache
pub(crate) async fn try_get_cached_activities(
    params: CachedActivitiesParams<'_>,
) -> Option<UniversalResponse> {
    if let Ok(Some(cached_activities)) = params.cache.get::<Vec<Activity>>(params.cache_key).await {
        info!(
            "Cache hit for activities (count={}, mode={}, format={:?})",
            cached_activities.len(),
            params.mode,
            params.output_format
        );

        // Sort by start_date descending (newest first) for consistent ordering
        // Tallied apart: a hit that turns out to hold an unread recent row is
        // a miss, and the live read then tallies the same items itself.
        let (mut sorted_activities, withheld) = ai_scope::tallied(async {
            ai_scope::filter_activities(params.policies, cached_activities)
        })
        .await;
        sorted_activities.sort_by_key(|a| Reverse(a.start_date()));

        // The whole window was stored, before the display limit: it is
        // counted from the store as the live read's is, once the hit stands.
        let served = sorted_activities.clone();
        let fetched = served.len();
        sorted_activities.truncate(params.limit);

        let (promoted, places) = detail_cached_list(
            &params.detail,
            params.policies,
            params.user_uuid,
            &sorted_activities,
        )
        .await?;
        ai_scope::record_withheld(withheld);
        let keep = |activity: &Activity| {
            params
                .sport_type
                .is_none_or(|sport| activity_matches_sport(activity, sport))
        };
        let window = WindowCount::of_served(
            params.runtime,
            params.user_uuid,
            params.detail.tenant_id,
            ServedList {
                served: &served,
                after: params
                    .asked
                    .after
                    .and_then(|at| Timestamp::from_epoch_seconds(at).ok()),
                before: params
                    .asked
                    .before
                    .and_then(|at| Timestamp::from_epoch_seconds(at).ok()),
                pinned: params.pinned,
                keep: &keep,
            },
        )
        .await;
        let sorted_activities = promoted.activities;
        let detailed_ids = promoted.detailed_ids;
        let mode = effective_mode(params.mode, &sorted_activities, &detailed_ids);

        let pagination = PaginationInfo {
            offset: params.offset,
            limit: params.limit,
            returned_count: sorted_activities.len(),
            has_more: pagination_has_more(
                sorted_activities.len(),
                fetched,
                fetched >= params.fetch_limit,
            ),
        };

        // Backfill missing ambient temperatures from weather provider
        // (cheap on warm cache; the weather_cache table fronts the API).
        let backfill_temps = if let Some(backfill) = params.weather_backfill {
            backfill.fill(&sorted_activities).await
        } else {
            HashMap::new()
        };

        // Use the same response builder as the non-cached path to apply mode/format
        let mut response = build_activities_success_response(ActivitiesResponseParams {
            activities: &sorted_activities,
            user_uuid: params.user_uuid,
            tenant_id: params.tenant_id,
            provider_name: params.provider_name,
            mode,
            detailed_ids: &detailed_ids,
            output_format: params.output_format,
            pagination: Some(&pagination),
            analysis_type: params.analysis_type,
            backfill_temps: &backfill_temps,
            places: &places,
            user_timezone: params.user_timezone,
            locale: params.locale,
            window,
            asked: params.asked,
            // The whole window was stored; only a window that filled the
            // provider's page at fetch time may hold older rows.
            completeness: if fetched >= params.fetch_limit {
                Completeness::Incomplete(IncompleteReason::ReadCapped)
            } else {
                Completeness::Complete
            },
            // The slice was merged before it was cached; the grouping that
            // produced it is not stored with it, so no merge note is emitted.
            merge_report: None,
        });
        // Mark as cached in metadata
        if let Some(ref mut metadata) = response.metadata {
            metadata.insert("cached".to_owned(), Value::Bool(true));
        }
        return Some(response);
    }
    info!("Cache miss for activities");
    None
}

/// Cache activities after fetching from API
pub(crate) async fn cache_activities_result(
    cache: &Arc<Cache>,
    cache_key: &CacheKey,
    activities: &Vec<Activity>,
    per_page: u32,
) {
    let ttl = CacheResource::ActivityList {
        page: 1,
        per_page,
        before: None,
        after: None,
        sport_type: None,
    }
    .recommended_ttl();
    if let Err(e) = cache.set(cache_key, activities, ttl).await {
        warn!("Failed to cache activities: {}", e);
    } else {
        info!("Cached {} activities with TTL {:?}", activities.len(), ttl);
    }
}

/// The ways an LLM asks for "every sport" rather than naming one, in the five
/// locales the platform ships (en/fr/es/de/pt).
///
/// These are not sport names: matching them literally drops every activity and
/// makes the agent report "no recent data" over a full cache (the 2026-07-20
/// dev incident, where English `all` did exactly that). The agent speaks the
/// athlete's language, so it emits the wildcard in that language too.
///
/// Compared after [`normalise_sport_string`], which lowercases and strips
/// separators — so entries here are lowercase and separator-free.
const SPORT_WILDCARDS: &[&str] = &[
    // no filter at all
    "",
    // en
    "all",
    "any",
    "everything",
    // fr
    "tous",
    "toutes",
    "tout",
    // es
    "todos",
    "todas",
    "todo",
    "cualquiera",
    // de
    "alle",
    "alles",
    "jede",
    // pt
    "tudo",
    "qualquer",
];

/// Filter activities by sport type, using alias-aware enum matching.
///
/// A wildcard input ([`SPORT_WILDCARDS`]) passes the list through untouched.
///
/// The LLM (or a UI) passes a free-form string like `nordicski`, `xc_ski`,
/// `ski de fond`, or `cross_country_skiing`. The filter resolves the input
/// to a canonical [`SportType`] via
/// [`pierre_core::models::resolve_sport_type`] and compares enum-to-enum
/// against each activity's `sport_type` — so all of those inputs match
/// [`SportType::CrossCountrySkiing`] without requiring per-call alias logic
/// in the prompt or upstream callers.
///
/// When the input doesn't resolve to a known variant, falls back to a
/// normalised string compare (separator-insensitive, lowercase) against the
/// activity's serialised `sport_type` — preserves the previous behaviour for
/// unknown / custom `Other(...)` values.
#[must_use]
pub fn filter_activities_by_sport_type(
    activities: Vec<Activity>,
    sport_type_filter: Option<&str>,
) -> Vec<Activity> {
    let Some(filter) = sport_type_filter else {
        return activities;
    };
    activities
        .into_iter()
        .filter(|a| activity_matches_sport(a, filter))
        .collect()
}

/// Whether `activity` is of the sport `filter` names.
///
/// As [`filter_activities_by_sport_type`] filters: any activity for a wildcard,
/// the one alias-aware matcher ([`sport_matches`]) for a sport it resolves,
/// else a normalised compare against the activity's own spelling.
#[must_use]
pub(crate) fn activity_matches_sport(activity: &Activity, filter: &str) -> bool {
    let normalised_filter = normalise_sport_string(filter);
    if SPORT_WILDCARDS.contains(&normalised_filter.as_str()) {
        return true;
    }
    resolve_sport_type(filter).map_or_else(
        || sport_spelled(activity.sport_type(), &normalised_filter),
        |canonical| sport_matches(activity, &[canonical], &[]),
    )
}

/// Lower-case and strip separator characters so that
/// `"Cross-Country Skiing"`, `"cross_country_skiing"`, and
/// `"crosscountryskiing"` collapse to the same comparison key.
fn normalise_sport_string(s: &str) -> String {
    s.trim().to_lowercase().replace(['_', '-', ' ', '\''], "")
}

/// Whether an activity's [`SportType`], serialised, normalises to
/// `normalised_filter`: the fallback for a filter no alias resolves, which
/// keeps unknown and custom `Other(...)` values matchable.
fn sport_spelled(activity_sport: &SportType, normalised_filter: &str) -> bool {
    let activity_str = to_value(activity_sport).map_or_else(
        |_| String::new(),
        |v| {
            v.as_str().map_or_else(
                || {
                    v.as_object()
                        .and_then(|obj| obj.get("other"))
                        .and_then(|v| v.as_str())
                        .map_or_default(str::to_owned)
                },
                str::to_owned,
            )
        },
    );
    normalise_sport_string(&activity_str) == normalised_filter
}

/// Build metadata for activities response
fn build_activities_metadata(
    count: usize,
    user_uuid: Uuid,
    tenant_id: Option<String>,
    mode_used: &str,
    format_used: &str,
    pagination: Option<&PaginationInfo>,
) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    map.insert("returned_count".to_owned(), Value::Number(count.into()));
    map.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
    map.insert(
        "tenant_id".to_owned(),
        tenant_id.map_or(Value::Null, Value::String),
    );
    map.insert("cached".to_owned(), Value::Bool(false));
    map.insert("mode".to_owned(), Value::String(mode_used.to_owned()));
    map.insert("format".to_owned(), Value::String(format_used.to_owned()));
    // Add pagination metadata when available
    if let Some(page_info) = pagination {
        map.insert("offset".to_owned(), Value::Number(page_info.offset.into()));
        map.insert("limit".to_owned(), Value::Number(page_info.limit.into()));
        map.insert("has_more".to_owned(), Value::Bool(page_info.has_more));
    }
    map
}

/// Prepare activity data for response based on mode (summary or detailed)
/// Returns the JSON value and mode string, or error message string if serialization fails
///
/// `backfill_temps` is consulted when the provider didn't surface ambient
/// temperature; values fill `ActivitySummary::temperature` so the JSON
/// payload mirrors the inline activity-list rendering. Every row of every
/// shape whose activity has a place in `places` carries it as `place`
/// ([`served_place`]).
fn prepare_activity_data(
    activities: &[Activity],
    mode: &str,
    detailed_ids: &HashSet<String>,
    backfill_temps: &HashMap<String, f32>,
    places: &HashMap<(String, String), PlaceRef>,
    user_timezone: Option<&str>,
) -> Result<(Value, &'static str), String> {
    let (mut value, mode_used) = activity_rows(
        activities,
        mode,
        detailed_ids,
        backfill_temps,
        user_timezone,
    )?;
    if let Some(rows) = value.as_array_mut() {
        for (row, activity) in rows.iter_mut().zip(activities) {
            let key = (activity.provider().to_owned(), activity.id().to_owned());
            if let (Some(place), Some(map)) = (places.get(&key), row.as_object_mut()) {
                map.insert("place".to_owned(), served_place(place));
            }
        }
    }
    Ok((value, mode_used))
}

/// The rows [`prepare_activity_data`] serves, before their places.
fn activity_rows(
    activities: &[Activity],
    mode: &str,
    detailed_ids: &HashSet<String>,
    backfill_temps: &HashMap<String, f32>,
    user_timezone: Option<&str>,
) -> Result<(Value, &'static str), String> {
    let tz = parse_zone(user_timezone);
    if mode == "summary" {
        // Activities carry UTC start_date; localize for display without
        // touching the timezone-stable UTC. A detailed row renders in full,
        // every other row as its summary.
        let rows = activities
            .iter()
            .map(|a| {
                let mut row = if detailed_ids.contains(a.id()) {
                    detail_json(slice::from_ref(a))
                        .map_err(|e| format!("Failed to serialize activities: {e}"))?
                        .as_array_mut()
                        .and_then(Vec::pop)
                        .unwrap_or(Value::Null)
                } else {
                    let mut summary = ActivitySummary::from(a);
                    if summary.temperature.is_none() {
                        summary.temperature = backfill_temps.get(a.id()).copied();
                    }
                    summary_json(slice::from_ref(&summary))
                        .map_err(|e| format!("Failed to serialize activity summaries: {e}"))?
                        .as_array_mut()
                        .and_then(Vec::pop)
                        .unwrap_or(Value::Null)
                };
                if let (Some(tz), Some(map)) = (tz.as_ref(), row.as_object_mut()) {
                    map.insert(
                        "start_date_local".to_owned(),
                        Value::String(zoned_rfc3339(a.start_date(), tz)),
                    );
                }
                Ok(row)
            })
            .collect::<Result<Vec<Value>, String>>()?;
        Ok((Value::Array(rows), "summary"))
    } else {
        // Detail mode serializes the `Activity` (UTC `start_date`, athlete text
        // fenced by `detail_json`), with `start_date_local` per element when
        // the user has a timezone on file.
        let mut value =
            detail_json(activities).map_err(|e| format!("Failed to serialize activities: {e}"))?;
        if let (Some(tz), Some(arr)) = (tz, value.as_array_mut()) {
            for (obj, activity) in arr.iter_mut().zip(activities.iter()) {
                if let Some(map) = obj.as_object_mut() {
                    map.insert(
                        "start_date_local".to_owned(),
                        Value::String(zoned_rfc3339(activity.start_date(), &tz)),
                    );
                }
            }
        }
        Ok((value, "detailed"))
    }
}

/// Parameters for building an activities success response
pub(crate) struct ActivitiesResponseParams<'a> {
    pub activities: &'a [Activity],
    pub user_uuid: Uuid,
    pub tenant_id: Option<String>,
    pub provider_name: &'a str,
    pub mode: &'a str,
    /// The activities that carry their detail; in `summary` mode they render
    /// in full and the rest as summaries.
    pub detailed_ids: &'a HashSet<String>,
    pub output_format: OutputFormat,
    pub pagination: Option<&'a PaginationInfo>,
    /// Analysis type for sufficiency calculation (defaults to `GeneralOverview`)
    pub analysis_type: AnalysisType,
    /// Weather-backfilled ambient temperatures keyed by activity id.
    /// Empty when backfill is disabled or no candidates qualified
    /// (no GPS, no start time, or temp already present on the row).
    pub backfill_temps: &'a HashMap<String, f32>,
    /// Each served activity's place, keyed by its provider and id
    /// (`activity_detail_store::stored_served_rows`).
    pub places: &'a HashMap<(String, String), PlaceRef>,
    /// User's IANA timezone (e.g. `"America/Toronto"`), when on file. Used to
    /// render `ActivitySummary::start_date_local` so the LLM displays start
    /// times in the user's local time rather than raw UTC. `None` → UTC only.
    pub user_timezone: Option<String>,
    /// User's BCP-47 locale ("fr"/"en"/"es"/"de"/"pt") selecting the localized
    /// sport-type labels in the rendered activity list. Defaults to "en".
    pub locale: String,
    /// The store's count of the served window, before the display-limit
    /// truncation (carnet#1485, OQ6): its total, span and totals ride on the
    /// `coverage` sidecar, so the LLM frames the true total, span and totals
    /// instead of the shown slice, and the served rows the store does not
    /// hold are named. A default count (nothing counted) emits no `coverage`
    /// for a complete window.
    pub window: WindowCount,
    /// The window the call asked for.
    pub asked: AskedWindow,
    /// Whether the served window is the whole truth for the ask.
    pub completeness: Completeness,
    /// What the session merge collapsed to produce `activities`: every group of
    /// recordings folded into one session and the fields each session took from
    /// another recording. `None` when no merge report travels with the slice.
    pub merge_report: Option<&'a FragmentReport>,
}

/// Build success response for activities with mode and format support
/// `mode="summary"` returns minimal fields (id, name, `sport_type`, `start_date`, distance, duration)
/// `mode="detailed"` returns full activity data (default for backwards compatibility when not specified)
/// `format="json"` (default) or `format="toon"`, an opt-in for external MCP clients
/// `pagination` enables clients to paginate through large result sets
pub(crate) fn build_activities_success_response(
    params: ActivitiesResponseParams<'_>,
) -> UniversalResponse {
    let ActivitiesResponseParams {
        activities,
        user_uuid,
        tenant_id,
        provider_name,
        mode,
        detailed_ids,
        output_format,
        pagination,
        analysis_type,
        backfill_temps,
        places,
        user_timezone,
        locale,
        window,
        asked,
        completeness,
        merge_report,
    } = params;

    // Prepare the data based on mode
    let (data_value, mode_used) = match prepare_activity_data(
        activities,
        mode,
        detailed_ids,
        backfill_temps,
        places,
        user_timezone.as_deref(),
    ) {
        Ok(result) => result,
        Err(error) => {
            return UniversalResponse {
                success: false,
                result: None,
                error: Some(error),
                metadata: None,
            };
        }
    };

    let activities_len = activities.len();

    // The merge ran over the whole fetched window; describe only the sessions
    // this response returns.
    let page_merge = merge_report.map(|report| report.scoped_to(activities));

    // Create pre-formatted activity list for LLM output (helps models include the list)
    let activity_list = format_activities_as_list(
        activities,
        backfill_temps,
        page_merge.as_ref(),
        &locale,
        user_timezone.as_deref(),
        Timestamp::now(),
    );

    // Calculate retrieval context for LLM data sufficiency guidance
    let retrieval_context = ActivityRetrievalContext::from_activities_with_dedup(
        activities,
        analysis_type,
        mode_used,
        page_merge.as_ref(),
    );

    // What the connections behind these rows never supply, so the model reads
    // an absent field as unknown rather than zero.
    let field_support = served_header(activities);

    // One payload, three ways of filling it. The TOON key stays
    // `activities_toon` rather than the fixed `toon` the other typed tools
    // use: `prefetch.rs` reads `activity_list` and `count` at the top level
    // (that one key instead of the whole reply is what keeps ~3k tokens per
    // grounded turn out of the prompt) and the SDK models the siblings, so
    // nesting the payload would break both.
    let base = |activities, activities_toon, format: &str| ActivitiesPayload {
        activity_list: activity_list.clone(),
        activities,
        activities_toon,
        provider: provider_name.to_owned(),
        count: activities_len,
        mode: mode_used.to_owned(),
        format: format.to_owned(),
        format_fallback: None,
        format_error: None,
        offset: pagination.map(|p| p.offset),
        limit: pagination.map(|p| p.limit),
        has_more: pagination.map(|p| p.has_more),
        // Calculate token estimate for context management
        token_estimate: TokenEstimate::from_activities(activities_len, mode_used),
        retrieval_context: retrieval_context.clone(),
        field_support: field_support.clone(),
        coverage: None,
        reconnect_required: None,
        provider_unavailable: None,
    };

    let (mut payload, format_used) = match output_format {
        OutputFormat::Toon => match format_output(&data_value, OutputFormat::Toon) {
            Ok(formatted) => (base(None, Some(formatted.data), "toon"), "toon"),
            Err(e) => {
                warn!("TOON serialization failed, falling back to JSON: {e}");
                let mut fallen = base(Some(data_value), None, "json");
                fallen.format_fallback = Some(true);
                fallen.format_error = Some(e.to_string());
                (fallen, "json")
            }
        },
        OutputFormat::Json => (base(Some(data_value), None, "json"), "json"),
    };

    // Surface honest window coverage when the served slice was truncated, the
    // window was asked by date, or it is incomplete, so the LLM frames "N total
    // in this window, showing the most recent M" instead of anchoring on the
    // oldest shown activity. LLM-facing (tool result, not the input schema) →
    // no contremaitre/locale drift.
    payload.coverage = activity_coverage(window, activities_len, asked, completeness);

    let metadata = build_activities_metadata(
        activities.len(),
        user_uuid,
        tenant_id,
        mode_used,
        format_used,
        pagination,
    );
    UniversalResponse {
        success: true,
        result: to_value_as_written(&payload).ok(),
        error: None,
        metadata: Some(metadata),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use dravr_tronc::dates::Timestamp;
    use pierre_core::models::{Activity, ActivityBuilder, PlaceRef, SportType};
    use serde_json::{json, Value};

    use super::prepare_activity_data;

    fn ride(id: &str) -> Activity {
        ActivityBuilder::new(
            id,
            "Ride",
            SportType::Ride,
            Timestamp::now(),
            3_600,
            "strava",
        )
        .build()
    }

    fn placed(activities: &[Activity]) -> HashMap<(String, String), PlaceRef> {
        activities
            .iter()
            .map(|a| {
                (
                    (a.provider().to_owned(), a.id().to_owned()),
                    PlaceRef {
                        id: format!("place-of-{}", a.id()),
                        name: Some("Gai-Luron".to_owned()),
                        athlete_named: true,
                    },
                )
            })
            .collect()
    }

    fn places_of(rows: &Value) -> Vec<Value> {
        rows.as_array()
            .expect("rows are a list")
            .iter()
            .map(|row| row["place"].clone())
            .collect()
    }

    /// Every row shape carries its place: a summary row and a detailed row
    /// in one summary list, and every row of a detail-mode list.
    #[test]
    fn every_row_shape_carries_its_place() {
        let activities = [ride("summary-row"), ride("detailed-row")];
        let places = placed(&activities);
        let detailed = HashSet::from(["detailed-row".to_owned()]);
        let expected = vec![
            json!({ "id": "place-of-summary-row", "name": "Gai-Luron" }),
            json!({ "id": "place-of-detailed-row", "name": "Gai-Luron" }),
        ];

        for mode in ["summary", "detailed"] {
            let (rows, _) =
                prepare_activity_data(&activities, mode, &detailed, &HashMap::new(), &places, None)
                    .expect("the rows serialize");
            assert_eq!(
                places_of(&rows),
                expected,
                "bug: a {mode} list lost a row's place: {rows}"
            );
        }
    }

    /// A row whose activity has no place carries no `place` key, never a
    /// null one.
    #[test]
    fn a_row_with_no_place_carries_none() {
        let activities = [ride("unplaced")];
        let (rows, _) = prepare_activity_data(
            &activities,
            "summary",
            &HashSet::new(),
            &HashMap::new(),
            &HashMap::new(),
            None,
        )
        .expect("the rows serialize");
        assert!(
            rows[0].get("place").is_none(),
            "bug: a row with no place was given a place key: {rows}"
        );
    }
}
