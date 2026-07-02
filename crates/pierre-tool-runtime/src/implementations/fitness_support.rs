// ABOUTME: Shared support types and helpers for the fitness-provider API tools.
// ABOUTME: Hosts ActivitySummary / TokenEstimate / AnalysisType / ActivityRetrievalContext + cache/format helpers.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Fitness API Tool Support
//!
//! Shared types and helper functions extracted from the (now-decomposed)
//! `protocol/handlers/fitness_api.rs` so that the `McpTool::execute` bodies
//! for `get_activities`, `get_athlete`, `get_stats`, and `analyze_activity`
//! can share them without forcing a single 2k-line file.
//!
//! Exposed types — `ActivitySummary`, `TokenEstimate`, `AnalysisType`,
//! `ActivityRetrievalContext`, `FragmentDedupSummary`, `PaginationInfo`, and
//! their sub-types — are consumed by the tools in
//! `crate::implementations::data` and `crate::implementations::analytics`.
//! The cache/format/response-build helpers (`try_get_cached_activities`,
//! `build_activities_success_response`, etc.) are `pub(crate)` so the same
//! tool modules can call them directly.

use crate::implementations::data::activity_coverage_note;
use crate::protocol::format::build_formatted_response;
use crate::protocol::types::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::errors::protocol::ProtocolError;
use pierre_core::models::{
    resolve_sport_type, Activity, Athlete, SportType, Stats, TenantId, ZoneDistribution,
};
use pierre_formatters::{format_output, OutputFormat};
use pierre_intelligence::physiological_constants::api_limits::{
    CLAUDE_CONTEXT_TOKENS, CONTEXT_WARNING_THRESHOLD_PERCENT, TOKENS_PER_ACTIVITY_DETAILED,
    TOKENS_PER_ACTIVITY_SUMMARY, USABLE_CONTEXT_TOKENS,
};
use pierre_providers::core::FitnessProvider;
use pierre_providers::deduplication::{detect_fragments, DedupConfig, FragmentReport};
use pierre_services::weather_backfill;
use pierre_weather::WeatherProvider;
use serde::Serialize;
use serde_json::{json, to_value, Value};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Activity summary with scalar sensor fields for efficient list queries.
///
/// Used when `mode=summary`. Carries the full set of scalar fields every
/// coach persona needs for basic reasoning (HR zones, elevation load,
/// calorie estimate, cadence, power) without the arrays (splits, laps,
/// segments, HR zones, power zones, time-series data) that only a deep
/// per-activity analysis coach needs. All sensor fields are `Option<T>`
/// and `#[serde(skip_serializing_if = "Option::is_none")]` so activities
/// recorded without an HRM or on indoor trainers render cleanly without
/// null noise.
#[derive(Debug, Clone, Serialize)]
pub struct ActivitySummary {
    /// Unique activity identifier
    pub id: String,
    /// Activity name/title
    pub name: String,
    /// Activity sport type (e.g., "run", "ride", "cross\_country\_skiing")
    pub sport_type: SportType,
    /// Start date/time in ISO 8601 format (UTC). Kept UTC so day-windowing,
    /// sorting, and fragment detection stay timezone-stable.
    pub start_date: String,
    /// Start time rendered in the user's local IANA timezone (RFC3339 with
    /// offset, e.g. `2026-05-29T08:36:07-04:00`), when the user has a timezone
    /// on file. This is the field to DISPLAY to the user — `start_date` is the
    /// raw UTC instant. `None` when the user has no timezone configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date_local: Option<String>,
    /// Distance in meters (0.0 if not available)
    pub distance_meters: f64,
    /// Duration in seconds
    pub duration_seconds: u64,
    /// Total elevation gained in meters, when the provider reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevation_gain_meters: Option<f64>,
    /// Average heart rate in BPM over the activity, when the user wore an HRM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_heart_rate: Option<u32>,
    /// Maximum heart rate in BPM over the activity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_heart_rate: Option<u32>,
    /// Provider-reported calorie estimate, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calories: Option<u32>,
    /// Average cadence (rpm for cycling, spm for running).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_cadence: Option<u32>,
    /// Average power output in watts (cycling / rowing / running power meters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_power: Option<u32>,
    /// Strava's "Suffer Score" (Relative Effort) when available. Surrogate for
    /// perceived exertion grounded in HR-in-zone time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffer_score: Option<u32>,
    /// Average ambient temperature in Celsius when the provider reports it.
    /// Outdoor activities from Strava and Garmin OAuth surface this when the
    /// recording device captured ambient temp; Coros does too if its watch
    /// reported it. Whoop / Fitbit / Terra don't expose ambient temperature
    /// on workouts (skin temp on Whoop Recovery is recorded separately, on
    /// the recovery record, not the activity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Endurance Intensity Factor — `normalized_power / ftp` (Coggan).
    /// Populated by the Endurance latest-snapshot pipeline; `None` for
    /// activities without a power stream or for users without an FTP.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity_factor: Option<f64>,
    /// Endurance Efficiency Factor — `normalized_power / average_heart_rate`.
    /// `None` when either input is missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub efficiency_factor: Option<f64>,
    /// Endurance Variability Index — `normalized_power / average_power`.
    /// `None` when the activity has no power stream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variability_index: Option<f64>,
    /// Endurance aerobic decoupling percentage. `None` when the activity
    /// has fewer than 20 paired HR+speed samples (Coggan threshold).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoupling_pct: Option<f64>,
    /// Endurance time-in-zone distribution computed against the user's
    /// configured `HrZoneSet`. `None` when no HR stream or no user zones.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_distribution: Option<ZoneDistribution>,
}

impl From<&Activity> for ActivitySummary {
    fn from(activity: &Activity) -> Self {
        Self {
            id: activity.id().to_owned(),
            name: activity.name().to_owned(),
            sport_type: activity.sport_type().clone(),
            start_date: activity.start_date().to_rfc3339(),
            // Populated by prepare_activity_data when the user's timezone is
            // known; the From conversion has no timezone context.
            start_date_local: None,
            distance_meters: activity.distance_meters().unwrap_or(0.0),
            duration_seconds: activity.duration_seconds(),
            elevation_gain_meters: activity.elevation_gain(),
            average_heart_rate: activity.average_heart_rate(),
            max_heart_rate: activity.max_heart_rate(),
            calories: activity.calories(),
            average_cadence: activity.average_cadence(),
            average_power: activity.average_power(),
            suffer_score: activity.suffer_score(),
            temperature: activity.temperature(),
            // Endurance metrics are derived in the latest_snapshot pipeline,
            // not at the per-activity summary boundary. Keep them None here
            // so the JSON shape is stable for non-Section-11 callers.
            intensity_factor: None,
            efficiency_factor: None,
            variability_index: None,
            decoupling_pct: None,
            zone_distribution: None,
        }
    }
}

/// Localized short sport-type label for the activity list, keyed by BCP-47
/// locale.
///
/// The names are intentionally short coach/athlete nouns (fr "course à
/// pied"/"rando"/"ski de fond", not the English `display_name`) so a French
/// chat reads natively. Falls back to English for an unrecognized locale and
/// keeps the provider-supplied label for `SportType::Other`; the match has no
/// wildcard arm, so a new `SportType` variant fails compilation here until its
/// five translations are added. `pub` so the locale table (human-language rows
/// the compiler can't check for wrong-column pastes) is exercisable by the
/// render tests.
#[must_use]
pub fn localized_sport_name(sport: &SportType, locale: &str) -> String {
    // [fr, en, es, de, pt]
    let names: [&str; 5] = match sport {
        SportType::Other(s) => return s.clone(),
        SportType::Run => ["course à pied", "run", "carrera", "laufen", "corrida"],
        SportType::Ride => ["vélo", "bike ride", "bici", "radtour", "pedalada"],
        SportType::Swim => ["natation", "swim", "natación", "schwimmen", "natação"],
        SportType::Walk => ["marche", "walk", "caminata", "spaziergang", "caminhada"],
        SportType::Hike => ["rando", "hike", "senderismo", "wanderung", "trilha"],
        SportType::VirtualRide => [
            "home trainer",
            "indoor ride",
            "bici indoor",
            "indoor-radfahren",
            "bike indoor",
        ],
        SportType::VirtualRun => [
            "tapis de course",
            "treadmill run",
            "cinta de correr",
            "laufband",
            "esteira",
        ],
        SportType::Workout => [
            "entraînement",
            "workout",
            "entrenamiento",
            "training",
            "treino",
        ],
        SportType::Yoga => ["yoga", "yoga", "yoga", "yoga", "yoga"],
        SportType::EbikeRide => [
            "vélo électrique",
            "e-bike ride",
            "bici eléctrica",
            "e-bike-tour",
            "e-bike",
        ],
        SportType::MountainBike => [
            "VTT",
            "mountain bike",
            "btt",
            "mountainbike",
            "mountain bike",
        ],
        SportType::GravelRide => ["gravel", "gravel ride", "gravel", "gravel-tour", "gravel"],
        SportType::CrossCountrySkiing => [
            "ski de fond",
            "cross-country ski",
            "esquí de fondo",
            "langlauf",
            "esqui de fundo",
        ],
        SportType::AlpineSkiing => [
            "ski alpin",
            "alpine ski",
            "esquí alpino",
            "ski alpin",
            "esqui alpino",
        ],
        SportType::Snowboarding => [
            "snowboard",
            "snowboard",
            "snowboard",
            "snowboard",
            "snowboard",
        ],
        SportType::Snowshoe => [
            "raquette",
            "snowshoe",
            "raquetas de nieve",
            "schneeschuhwandern",
            "raquetes de neve",
        ],
        SportType::IceSkating => [
            "patin à glace",
            "ice skating",
            "patinaje sobre hielo",
            "schlittschuhlaufen",
            "patinação no gelo",
        ],
        SportType::BackcountrySkiing => [
            "ski de rando",
            "backcountry ski",
            "esquí de travesía",
            "skitour",
            "esqui de travessia",
        ],
        SportType::Kayaking => ["kayak", "kayak", "kayak", "kajak", "caiaque"],
        SportType::Canoeing => ["canoë", "canoe", "canoa", "kanu", "canoagem"],
        SportType::Rowing => ["aviron", "rowing", "remo", "rudern", "remo"],
        SportType::Paddleboarding => [
            "paddle",
            "paddleboard",
            "paddle surf",
            "stand-up-paddling",
            "stand up paddle",
        ],
        SportType::Surfing => ["surf", "surf", "surf", "surfen", "surf"],
        SportType::Kitesurfing => ["kitesurf", "kitesurf", "kitesurf", "kitesurfen", "kitesurf"],
        SportType::StrengthTraining => [
            "musculation",
            "strength training",
            "fuerza",
            "krafttraining",
            "musculação",
        ],
        SportType::Crossfit => ["CrossFit", "CrossFit", "CrossFit", "CrossFit", "CrossFit"],
        SportType::Pilates => ["Pilates", "Pilates", "Pilates", "Pilates", "Pilates"],
        SportType::RockClimbing => ["escalade", "climbing", "escalada", "klettern", "escalada"],
        SportType::TrailRunning => ["trail", "trail run", "trail", "trailrunning", "trail run"],
        SportType::Soccer => ["foot", "soccer", "fútbol", "fußball", "futebol"],
        SportType::Basketball => [
            "basket",
            "basketball",
            "baloncesto",
            "basketball",
            "basquete",
        ],
        SportType::Tennis => ["tennis", "tennis", "tenis", "tennis", "tênis"],
        SportType::Golf => ["golf", "golf", "golf", "golf", "golfe"],
        SportType::Skateboarding => ["skate", "skate", "skate", "skateboarden", "skate"],
        SportType::InlineSkating => [
            "roller",
            "inline skating",
            "patinaje en línea",
            "inlineskaten",
            "patinação inline",
        ],
    };
    let idx = match locale.get(0..2).unwrap_or("en") {
        "fr" => 0,
        "es" => 2,
        "de" => 3,
        "pt" => 4,
        _ => 1,
    };
    names[idx].to_owned()
}

/// Format activities as a numbered human-readable list for LLM output.
///
/// This helps smaller models include the list in their response without
/// transforming JSON. Activities render in the order the caller established
/// (`get_activities` sorts by the requested `sort_by` before the display
/// limit).
///
/// `backfill_temps` carries weather-backfilled temperatures keyed by activity id;
/// used when the provider didn't surface ambient temp on the row itself
/// (sciotte / Whoop / Fitbit / Terra all leave it empty).
///
/// `fragment_report` carries fragment-deduplication metadata when overlapping
/// recordings of the same workout were detected; when `Some` and at least one
/// group is present, a header note is prepended so the LLM sees the
/// session-vs-row distinction inline with the list (smaller models that skip
/// the structured `retrieval_context` JSON still get the cue from the prose).
fn format_activities_as_list(
    activities: &[Activity],
    backfill_temps: &HashMap<String, f32>,
    fragment_report: Option<&FragmentReport>,
    locale: &str,
) -> String {
    let mut lines = Vec::with_capacity(activities.len() + 6);
    lines.push("Your Activities:".to_owned());
    lines.push(String::new());
    if let Some(report) = fragment_report {
        if report.has_fragments() {
            lines.push(format!(
                "[Note] {raw} GPS recordings detected, representing ~{sessions} distinct training sessions.",
                raw = report.raw_count,
                sessions = report.session_count,
            ));
            lines.push(
                "[Note] The following appear to be fragments of the same workout (count sessions, not rows):"
                    .to_owned(),
            );
            for group in &report.groups {
                let ids = group.fragment_ids.join(", ");
                lines.push(format!(
                    "       - canonical {canon}; group: [{ids}] ({sport}, {start} → {end})",
                    canon = group.canonical_id,
                    sport = localized_sport_name(&group.sport_type, locale),
                    start = group.window_start.format("%Y-%m-%d %H:%M UTC"),
                    end = group.window_end.format("%Y-%m-%d %H:%M UTC"),
                ));
            }
            lines.push(String::new());
        }
    }

    // Render in the order the caller already established (get_activities sorts
    // by the requested `sort_by` before the display limit). Re-sorting here
    // would override "longest to shortest" / "oldest first" back to date order.
    for (i, activity) in activities.iter().enumerate() {
        let date = activity.start_date().format("%Y-%m-%d").to_string();
        // Render the sport with its localized short label (fr "trail"/"rando",
        // not the English "trail run") so the list reads natively in the user's
        // chat language; `Other` keeps its provider-supplied label, unknown
        // locales fall back to English.
        let sport = localized_sport_name(activity.sport_type(), locale);
        let distance_km = activity.distance_meters().unwrap_or(0.0) / 1000.0;
        let duration_secs = activity.duration_seconds();
        let hours = duration_secs / 3600;
        let minutes = (duration_secs % 3600) / 60;
        let seconds = duration_secs % 60;

        let duration_str = if hours > 0 {
            format!("{hours}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes}:{seconds:02}")
        };

        // Append scalar sensor fields when the provider returned them.
        // Small models that skip the JSON tool result still see the
        // enrichment inline — no more "I don't have HR" when the data
        // was on the row all along. `write!` on a String is infallible
        // so the Result is intentionally discarded — matches
        // clippy::format_push_string guidance.
        let mut extras = String::new();
        match (activity.average_heart_rate(), activity.max_heart_rate()) {
            (Some(avg), Some(max)) => {
                let _ = write!(extras, " - HR {avg}/{max}");
            }
            (Some(avg), None) => {
                let _ = write!(extras, " - HR {avg} avg");
            }
            (None, Some(max)) => {
                let _ = write!(extras, " - HR {max} max");
            }
            (None, None) => {}
        }
        if let Some(elevation) = activity.elevation_gain() {
            // Round to whole meters — the coach reasoning doesn't need
            // decimals and the Strava field comes as Option<f32> which
            // sometimes carries spurious fractional noise.
            #[allow(clippy::cast_possible_truncation)]
            let rounded = elevation.round() as i64;
            let _ = write!(extras, " - +{rounded}m");
        }
        if let Some(calories) = activity.calories() {
            let _ = write!(extras, " - {calories} kcal");
        }
        // Prefer the provider-surfaced temperature; fall back to the
        // weather-backfill side-table for activities whose provider
        // didn't capture ambient temp (sciotte / Whoop / Fitbit / Terra).
        let temp = activity
            .temperature()
            .or_else(|| backfill_temps.get(activity.id()).copied());
        if let Some(temp) = temp {
            // Round to whole degrees — sub-degree precision is meaningless to
            // the coach reasoning loop and the providers report 1-decimal at
            // best. The leading sign survives `{:.0}` for sub-zero readings.
            let _ = write!(extras, " - {temp:.0}°C");
        }

        lines.push(format!(
            "{}. [{}] {} - {} - {:.2} km - {}{}",
            i + 1,
            sport,
            activity.name(),
            date,
            distance_km,
            duration_str,
            extras
        ));
    }

    lines.join("\n")
}

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
#[derive(Debug, Clone, Serialize)]
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
            format!("Using {usable_context_percent:.1}% of usable context. Plenty of room for analysis.")
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
#[derive(Debug, Clone, Copy, Serialize, Default)]
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
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
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
    /// Populated when `limit > activity_detail_threshold`: the response
    /// is summary-mode, so fields like splits, laps, segment efforts,
    /// and full time-series are absent. The sentence tells the LLM what
    /// is still available (HR, elevation, cadence, power, calories in
    /// the expanded summary) and how to narrow if it needs
    /// per-activity depth. `None` when no trade-off applies
    /// (auto-promoted queries).
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

/// LLM-facing summary of fragment-group detection over an activity slice.
///
/// Counterpart of [`pierre_providers::deduplication::FragmentReport`] — the
/// provider-side type carries `chrono::DateTime` values and the full sport
/// enum, this serializes them to strings so the JSON shape stays portable
/// across MCP / A2A / REST consumers and stable across cageux upgrades.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentDedupSummary {
    /// Total activities the detector saw (one per row returned by the provider).
    pub raw_count: usize,
    /// Distinct training sessions after collapsing fragment groups.
    pub session_count: usize,
    /// One entry per multi-row group; absent when no fragments were detected
    /// even though `fragment_dedup` itself is present.
    pub groups: Vec<FragmentGroupSummary>,
    /// Pre-formatted human-readable advice line the LLM should echo to the
    /// user when reporting counts. Example: "20 GPS recordings represent 12
    /// distinct sessions; the rest are likely re-uploads or auto-splits".
    pub advice: String,
}

/// A single overlapping-recording group surfaced to the LLM.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentGroupSummary {
    /// Activity id selected as the canonical session for this group.
    pub canonical_id: String,
    /// All member ids, canonical included — preserved for callers that want
    /// to render or audit the grouping decision.
    pub fragment_ids: Vec<String>,
    /// Sport type shared by every member (RFC-safe rendering of the enum).
    pub sport_type: String,
    /// Earliest start time across the group, ISO 8601.
    pub window_start: String,
    /// Latest end time across the group, ISO 8601.
    pub window_end: String,
}

impl FragmentDedupSummary {
    /// Build an LLM-facing summary from a provider-side [`FragmentReport`].
    /// Returns `None` when the report contains no fragment groups — the
    /// caller skips serializing `fragment_dedup` in that case so the JSON
    /// shape stays compact for the 95% of queries that have no fragments.
    fn from_report(report: &FragmentReport) -> Option<Self> {
        if !report.has_fragments() {
            return None;
        }
        let groups = report
            .groups
            .iter()
            .map(|g| FragmentGroupSummary {
                canonical_id: g.canonical_id.clone(),
                fragment_ids: g.fragment_ids.clone(),
                sport_type: format!("{:?}", g.sport_type),
                window_start: g.window_start.to_rfc3339(),
                window_end: g.window_end.to_rfc3339(),
            })
            .collect();
        let advice = format!(
            "Fragment deduplication: {raw} GPS recordings collapse into {sessions} distinct \
             training sessions after grouping overlapping captures (Garmin auto-splits, \
             dual-device recordings, or Strava re-uploads). When reporting counts to the user, \
             cite session_count ({sessions}), not raw_count ({raw}). The fragment_groups list \
             names which rows belong together.",
            raw = report.raw_count,
            sessions = report.session_count,
        );
        Some(Self {
            raw_count: report.raw_count,
            session_count: report.session_count,
            groups,
            advice,
        })
    }
}

/// Date range for activity retrieval
#[derive(Debug, Clone, Serialize)]
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

    /// Create retrieval context with optional fragment-deduplication metadata.
    ///
    /// When `fragment_report` is `Some` AND it contains at least one fragment
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
        // the model that splits/laps/segments were omitted and that
        // narrower queries (<= ACTIVITY_DETAIL_THRESHOLD) would fetch them.
        let advice = (mode_used == "summary").then(|| {
            "Summary mode: each activity carries scalar fields only \
             (HR avg/max, elevation, cadence, power, calories, suffer score). \
             Splits, laps, segment efforts, and time-series are omitted. \
             Request limit <= ACTIVITY_DETAIL_THRESHOLD to auto-promote a \
             narrower query to full detail."
                .to_owned()
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
            let now = chrono::Utc::now();
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
    pub cache_key: &'a CacheKey,
    pub user_uuid: uuid::Uuid,
    pub tenant_id: Option<String>,
    pub provider_name: &'a str,
    pub mode: &'a str,
    pub output_format: OutputFormat,
    pub limit: usize,
    pub offset: usize,
    /// Analysis type for sufficiency calculation
    pub analysis_type: AnalysisType,
    /// Optional weather provider used to backfill ambient temperature on
    /// activities whose source provider didn't surface it. `None` when
    /// `WEATHER_BACKFILL_ENABLED=false`.
    pub weather_provider: Option<Arc<dyn WeatherProvider>>,
    /// User's IANA timezone for rendering local start times (see
    /// [`ActivitiesResponseParams::user_timezone`]).
    pub user_timezone: Option<String>,
    /// User's BCP-47 locale, threaded to [`ActivitiesResponseParams::locale`]
    /// for localized sport-type labels. Defaults to "en".
    pub locale: String,
}

/// Create metadata for activity analysis responses
pub(crate) fn create_activity_metadata(
    activity_id: &str,
    user_uuid: Uuid,
    tenant_id: Option<&String>,
) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    map.insert(
        "activity_id".to_owned(),
        Value::String(activity_id.to_owned()),
    );
    map.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
    map.insert(
        "tenant_id".to_owned(),
        tenant_id.map_or(Value::Null, |id| {
            Value::String(id.clone()) // Safe: String ownership for JSON value
        }),
    );
    map
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
        let mut sorted_activities = cached_activities;
        sorted_activities.sort_by_key(|a| Reverse(a.start_date()));

        // Create pagination info from cached results
        let pagination = PaginationInfo {
            offset: params.offset,
            limit: params.limit,
            returned_count: sorted_activities.len(),
            has_more: sorted_activities.len() == params.limit,
        };

        // Backfill missing ambient temperatures from weather provider
        // (cheap on warm cache; the weather_cache table fronts the API).
        let backfill_temps = if let Some(provider) = params.weather_provider {
            weather_backfill::fill_activity_temperatures(&sorted_activities, provider).await
        } else {
            HashMap::new()
        };

        // Use the same response builder as the non-cached path to apply mode/format
        let mut response = build_activities_success_response(ActivitiesResponseParams {
            activities: &sorted_activities,
            user_uuid: params.user_uuid,
            tenant_id: params.tenant_id,
            provider_name: params.provider_name,
            mode: params.mode,
            output_format: params.output_format,
            pagination: Some(&pagination),
            analysis_type: params.analysis_type,
            backfill_temps: &backfill_temps,
            user_timezone: params.user_timezone,
            locale: params.locale,
            // The cached snapshot path serves its stored slice with no distinct
            // pre-truncation window total, so it emits no coverage sidecar.
            window_total: None,
            window_span: None,
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

/// Filter activities by sport type, using alias-aware enum matching.
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
pub(crate) fn filter_activities_by_sport_type(
    activities: Vec<Activity>,
    sport_type_filter: Option<&str>,
) -> Vec<Activity> {
    let Some(filter) = sport_type_filter else {
        return activities;
    };
    let canonical_filter = resolve_sport_type(filter);
    let normalised_filter = normalise_sport_string(filter);
    activities
        .into_iter()
        .filter(|a| {
            sport_type_matches(
                a.sport_type(),
                canonical_filter.as_ref(),
                &normalised_filter,
            )
        })
        .collect()
}

/// Lower-case and strip separator characters so that
/// `"Cross-Country Skiing"`, `"cross_country_skiing"`, and
/// `"crosscountryskiing"` collapse to the same comparison key.
fn normalise_sport_string(s: &str) -> String {
    s.trim().to_lowercase().replace(['_', '-', ' ', '\''], "")
}

/// Compare an activity's [`SportType`] against the canonical-or-fallback
/// filter pair produced by [`filter_activities_by_sport_type`].
fn sport_type_matches(
    activity_sport: &SportType,
    canonical_filter: Option<&SportType>,
    normalised_filter: &str,
) -> bool {
    if let Some(canonical) = canonical_filter {
        // A generic run/course filter covers the whole on-foot running family —
        // road Run, TrailRunning, treadmill VirtualRun — because the same effort
        // is tagged inconsistently on the provider (an athlete who runs almost
        // only on trails still has many logged as a plain "Run"). A specific
        // TrailRunning / VirtualRun filter stays exact, so "trail runs" means
        // trails only.
        if *canonical == SportType::Run {
            return matches!(
                activity_sport,
                SportType::Run | SportType::TrailRunning | SportType::VirtualRun
            );
        }
        return activity_sport == canonical;
    }
    let activity_str = to_value(activity_sport).map_or_else(
        |_| String::new(),
        |v| {
            v.as_str().map_or_else(
                || {
                    v.as_object()
                        .and_then(|obj| obj.get("other"))
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                        .unwrap_or_default()
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
/// payload mirrors the inline activity-list rendering.
fn prepare_activity_data(
    activities: &[Activity],
    mode: &str,
    backfill_temps: &HashMap<String, f32>,
    user_timezone: Option<&str>,
) -> Result<(Value, &'static str), String> {
    if mode == "summary" {
        // Parse the user's IANA timezone once; activities carry UTC start_date,
        // so we localize for display without touching the timezone-stable UTC.
        let tz = user_timezone.and_then(|s| s.parse::<chrono_tz::Tz>().ok());
        let summaries: Vec<ActivitySummary> = activities
            .iter()
            .map(|a| {
                let mut summary = ActivitySummary::from(a);
                if summary.temperature.is_none() {
                    summary.temperature = backfill_temps.get(a.id()).copied();
                }
                if let Some(tz) = tz {
                    summary.start_date_local = Some(a.start_date().with_timezone(&tz).to_rfc3339());
                }
                summary
            })
            .collect();
        to_value(&summaries)
            .map(|v| (v, "summary"))
            .map_err(|e| format!("Failed to serialize activity summaries: {e}"))
    } else {
        // Detail mode serializes the raw `Activity` (UTC `start_date`). Mirror
        // the summary-mode localization by injecting `start_date_local` per
        // element when the user has a timezone on file, so deep-dive responses
        // display local start times just like list responses.
        let mut value =
            to_value(activities).map_err(|e| format!("Failed to serialize activities: {e}"))?;
        if let Some(tz) = user_timezone.and_then(|s| s.parse::<chrono_tz::Tz>().ok()) {
            if let Some(arr) = value.as_array_mut() {
                for (obj, activity) in arr.iter_mut().zip(activities.iter()) {
                    if let Some(map) = obj.as_object_mut() {
                        map.insert(
                            "start_date_local".to_owned(),
                            Value::String(activity.start_date().with_timezone(&tz).to_rfc3339()),
                        );
                    }
                }
            }
        }
        Ok((value, "detailed"))
    }
}

/// Add common fields (pagination, token estimate, time window flag, retrieval context) to activity response JSON
fn add_common_response_fields(
    json_val: &mut Value,
    pagination: Option<&PaginationInfo>,
    token_estimate: &TokenEstimate,
    retrieval_context: &ActivityRetrievalContext,
) {
    if let Some(page_info) = pagination {
        json_val["offset"] = json!(page_info.offset);
        json_val["limit"] = json!(page_info.limit);
        json_val["has_more"] = json!(page_info.has_more);
    }
    json_val["token_estimate"] = json!(token_estimate);
    json_val["retrieval_context"] = json!(retrieval_context);
}

/// Parameters for building an activities success response
pub(crate) struct ActivitiesResponseParams<'a> {
    pub activities: &'a [Activity],
    pub user_uuid: Uuid,
    pub tenant_id: Option<String>,
    pub provider_name: &'a str,
    pub mode: &'a str,
    pub output_format: OutputFormat,
    pub pagination: Option<&'a PaginationInfo>,
    /// Analysis type for sufficiency calculation (defaults to `GeneralOverview`)
    pub analysis_type: AnalysisType,
    /// Weather-backfilled ambient temperatures keyed by activity id.
    /// Empty when backfill is disabled or no candidates qualified
    /// (no GPS, no start time, or temp already present on the row).
    pub backfill_temps: &'a HashMap<String, f32>,
    /// User's IANA timezone (e.g. `"America/Toronto"`), when on file. Used to
    /// render `ActivitySummary::start_date_local` so the LLM displays start
    /// times in the user's local time rather than raw UTC. `None` → UTC only.
    pub user_timezone: Option<String>,
    /// User's BCP-47 locale ("fr"/"en"/"es"/"de"/"pt") selecting the localized
    /// sport-type labels in the rendered activity list. Defaults to "en".
    pub locale: String,
    /// Total activities in the served window BEFORE the display-limit truncation,
    /// when known (the live `get_activities` path). When it exceeds the returned
    /// count, the response gains a `coverage` sidecar so the LLM frames the true
    /// total + span instead of the oldest shown row. `None` on paths with no
    /// distinct window total (e.g. the cached snapshot) — no `coverage` emitted.
    pub window_total: Option<usize>,
    /// Oldest/newest activity date (`"YYYY-MM-DD"`) across that full pre-truncation
    /// window, paired with `window_total`. `None` leaves the `coverage` note
    /// count-only.
    pub window_span: Option<(String, String)>,
}

/// Build success response for activities with mode and format support
/// `mode="summary"` returns minimal fields (id, name, `sport_type`, `start_date`, distance, duration)
/// `mode="detailed"` returns full activity data (default for backwards compatibility when not specified)
/// `format="json"` (default) or `format="toon"` for token-efficient LLM output
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
        output_format,
        pagination,
        analysis_type,
        backfill_temps,
        user_timezone,
        locale,
        window_total,
        window_span,
    } = params;

    // Prepare the data based on mode
    let (data_value, mode_used) =
        match prepare_activity_data(activities, mode, backfill_temps, user_timezone.as_deref()) {
            Ok(result) => result,
            Err(error) => {
                return UniversalResponse {
                    success: false,
                    result: None,
                    error: Some(error),
                    metadata: None,
                }
            }
        };

    // Detect fragment groups across the activity slice so the LLM (and any
    // downstream telemetry) can distinguish raw GPS recordings from distinct
    // training sessions. Computed once here, threaded into both the prose
    // list and the structured retrieval context — the LLM sees the same
    // signal whether it reads the inline note or the JSON sidecar.
    let fragment_report = detect_fragments(activities, &DedupConfig::default());

    // Create pre-formatted activity list for LLM output (helps models include the list)
    let activity_list =
        format_activities_as_list(activities, backfill_temps, Some(&fragment_report), &locale);

    // Calculate token estimate for context management
    let token_estimate = TokenEstimate::from_activities(activities.len(), mode_used);

    // Calculate retrieval context for LLM data sufficiency guidance
    let retrieval_context = ActivityRetrievalContext::from_activities_with_dedup(
        activities,
        analysis_type,
        mode_used,
        Some(&fragment_report),
    );

    // Format the activities data according to the requested format
    let (mut result_json, format_used) = match output_format {
        OutputFormat::Toon => match format_output(&data_value, OutputFormat::Toon) {
            Ok(formatted) => {
                let mut json_val = json!({
                    "activity_list": activity_list,
                    "activities_toon": formatted.data,
                    "provider": provider_name,
                    "count": activities.len(),
                    "mode": mode_used,
                    "format": "toon"
                });
                add_common_response_fields(
                    &mut json_val,
                    pagination,
                    &token_estimate,
                    &retrieval_context,
                );
                (json_val, "toon")
            }
            Err(e) => {
                warn!("TOON serialization failed, falling back to JSON: {e}");
                let mut json_val = json!({
                    "activity_list": activity_list,
                    "activities": data_value,
                    "provider": provider_name,
                    "count": activities.len(),
                    "mode": mode_used,
                    "format": "json",
                    "format_fallback": true,
                    "format_error": e.to_string()
                });
                add_common_response_fields(
                    &mut json_val,
                    pagination,
                    &token_estimate,
                    &retrieval_context,
                );
                (json_val, "json")
            }
        },
        OutputFormat::Json => {
            let mut json_val = json!({
                "activity_list": activity_list,
                "activities": data_value,
                "provider": provider_name,
                "count": activities.len(),
                "mode": mode_used,
                "format": "json"
            });
            add_common_response_fields(
                &mut json_val,
                pagination,
                &token_estimate,
                &retrieval_context,
            );
            (json_val, "json")
        }
    };

    // Surface honest window coverage when the served slice was truncated, so the
    // LLM frames "N total in this window, showing the most recent M" instead of
    // anchoring on the oldest shown activity. LLM-facing (tool result, not the
    // input schema) → no SDK/contremaitre/locale drift.
    if let Some(coverage) =
        activity_coverage_note(window_total, activities.len(), window_span.as_ref())
    {
        if let Some(obj) = result_json.as_object_mut() {
            obj.insert("coverage".to_owned(), coverage);
        }
    }

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
        result: Some(result_json),
        error: None,
        metadata: Some(metadata),
    }
}

/// Try to get athlete from cache
pub(crate) async fn try_get_cached_athlete(
    cache: &Arc<Cache>,
    cache_key: &CacheKey,
    user_uuid: Uuid,
    tenant_id: Option<&String>,
    output_format: OutputFormat,
) -> Result<Option<UniversalResponse>, ProtocolError> {
    if let Ok(Some(cached_athlete)) = cache.get::<Athlete>(cache_key).await {
        info!("Cache hit for athlete profile");
        let mut metadata = HashMap::new();
        metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
        metadata.insert(
            "tenant_id".to_owned(),
            tenant_id.map_or(Value::Null, |id| Value::String(id.clone())),
        );
        metadata.insert("cached".to_owned(), Value::Bool(true));

        return Ok(Some(build_formatted_response(
            &cached_athlete,
            "athlete",
            output_format,
            metadata,
        )?));
    }
    info!("Cache miss for athlete profile");
    Ok(None)
}

/// Cache athlete profile after fetching from API
async fn cache_athlete_result(cache: &Arc<Cache>, cache_key: &CacheKey, athlete: &Athlete) {
    let ttl = CacheResource::AthleteProfile.recommended_ttl();
    if let Err(e) = cache.set(cache_key, athlete, ttl).await {
        warn!("Failed to cache athlete profile: {}", e);
    } else {
        info!("Cached athlete profile with TTL {:?}", ttl);
    }
}

/// Fetch athlete from API and cache result
pub(crate) async fn fetch_and_cache_athlete(
    provider: &dyn FitnessProvider,
    cache: &Arc<Cache>,
    cache_key: &CacheKey,
    user_uuid: Uuid,
    tenant_id: Option<String>,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    match provider.get_athlete().await {
        Ok(athlete) => {
            cache_athlete_result(cache, cache_key, &athlete).await;

            let mut metadata = HashMap::new();
            metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
            metadata.insert(
                "tenant_id".to_owned(),
                tenant_id.map_or(Value::Null, Value::String),
            );
            metadata.insert("cached".to_owned(), Value::Bool(false));

            build_formatted_response(&athlete, "athlete", output_format, metadata)
        }
        Err(e) => Ok(UniversalResponse {
            success: false,
            result: None,
            error: Some(format!("Failed to fetch athlete profile: {e}")),
            metadata: None,
        }),
    }
}

/// Process activity analysis when activity is found
///
/// Dispatches into the `get_activity_intelligence` tool through the shared
/// registry (`UniversalToolExecutor::execute_tool`) rather than calling the
/// analytics handler function directly. This keeps this handler in
/// pierre-tool-runtime while the analytics handler can live anywhere as long
/// as its `McpTool` impl is registered.
pub(crate) async fn process_activity_analysis(
    executor: &UniversalToolExecutor,
    mut request: UniversalRequest,
    activity_id: &str,
    user_uuid: Uuid,
) -> Result<UniversalResponse, ProtocolError> {
    "get_activity_intelligence".clone_into(&mut request.tool_name);
    let analysis_response = executor.execute_tool(request).await?;
    let analysis = analysis_response.result.unwrap_or_else(|| json!({}));

    Ok(UniversalResponse {
        success: true,
        result: Some(to_value(analysis).map_err(|e| {
            ProtocolError::SerializationError(format!("Failed to serialize analysis: {e}"))
        })?),
        error: None,
        metadata: Some(create_activity_metadata(
            activity_id,
            user_uuid,
            analysis_response
                .metadata
                .as_ref()
                .and_then(|m| m.get("tenant_id").and_then(Value::as_str).map(String::from))
                .as_ref(),
        )),
    })
}

/// Try to get athlete ID from cached athlete profile
pub(crate) async fn try_get_athlete_id_from_cache(
    cache: &Arc<Cache>,
    athlete_cache_key: &CacheKey,
) -> Option<u64> {
    if let Ok(Some(athlete)) = cache.get::<Athlete>(athlete_cache_key).await {
        return athlete
            .id
            .parse::<u64>()
            .inspect_err(|e| {
                debug!(
                    athlete_id_str = %athlete.id,
                    error = %e,
                    "Failed to parse athlete ID from cache as u64"
                );
            })
            .ok();
    }
    None
}

/// Try to get stats from cache
pub(crate) async fn try_get_cached_stats(
    cache: &Arc<Cache>,
    stats_cache_key: &CacheKey,
    user_uuid: Uuid,
    tenant_id: Option<&String>,
    output_format: OutputFormat,
) -> Result<Option<UniversalResponse>, ProtocolError> {
    if let Ok(Some(cached_stats)) = cache.get::<Stats>(stats_cache_key).await {
        info!("Cache hit for stats");
        let mut metadata = HashMap::new();
        metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
        metadata.insert(
            "tenant_id".to_owned(),
            tenant_id.map_or(Value::Null, |id| Value::String(id.clone())),
        );
        metadata.insert("cached".to_owned(), Value::Bool(true));

        return Ok(Some(build_formatted_response(
            &cached_stats,
            "stats",
            output_format,
            metadata,
        )?));
    }
    info!("Cache miss for stats");
    Ok(None)
}

/// Create metadata for stats responses
fn create_stats_metadata(
    user_uuid: Uuid,
    tenant_id: TenantId,
    cached: bool,
) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    map.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
    map.insert("tenant_id".to_owned(), Value::String(tenant_id.to_string()));
    map.insert("cached".to_owned(), Value::Bool(cached));
    map
}

/// Cache a single item with TTL, logging errors
async fn cache_item<T: serde::Serialize + Send + Sync>(
    cache: &Arc<Cache>,
    key: &CacheKey,
    item: &T,
    ttl: Duration,
    item_name: &str,
) {
    if let Err(e) = cache.set(key, item, ttl).await {
        warn!("Failed to cache {}: {}", item_name, e);
    }
}

/// Cache athlete and stats data
async fn cache_athlete_and_stats(
    cache: &Arc<Cache>,
    athlete_cache_key: &CacheKey,
    athlete: &Athlete,
    stats: &Stats,
    tenant_id: TenantId,
    user_uuid: Uuid,
    provider_name: &str,
) {
    let Some(athlete_id) = athlete
        .id
        .parse::<u64>()
        .inspect_err(|e| debug!("Failed to parse athlete ID: {e}"))
        .ok()
    else {
        return;
    };

    // Cache athlete
    let athlete_ttl = CacheResource::AthleteProfile.recommended_ttl();
    cache_item(cache, athlete_cache_key, athlete, athlete_ttl, "athlete").await;

    // Cache stats
    let stats_cache_key = CacheKey::new(
        tenant_id,
        user_uuid,
        provider_name.to_owned(),
        CacheResource::Stats { athlete_id },
    );
    let stats_ttl = CacheResource::Stats { athlete_id }.recommended_ttl();
    cache_item(cache, &stats_cache_key, stats, stats_ttl, "stats").await;
    info!("Cached stats with TTL {:?}", stats_ttl);
}

/// Fetch stats from API and cache both athlete and stats
pub(crate) async fn fetch_and_cache_stats(
    provider: &dyn FitnessProvider,
    cache: &Arc<Cache>,
    athlete_cache_key: &CacheKey,
    tenant_id: TenantId,
    user_uuid: Uuid,
    provider_name: &str,
    output_format: OutputFormat,
) -> Result<UniversalResponse, ProtocolError> {
    let stats = match provider.get_stats().await {
        Ok(stats) => stats,
        Err(e) => {
            return Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to fetch stats: {e}")),
                metadata: None,
            });
        }
    };

    // Get athlete to extract athlete_id for caching
    if let Ok(athlete) = provider.get_athlete().await {
        cache_athlete_and_stats(
            cache,
            athlete_cache_key,
            &athlete,
            &stats,
            tenant_id,
            user_uuid,
            provider_name,
        )
        .await;
    }

    let metadata = create_stats_metadata(user_uuid, tenant_id, false);
    build_formatted_response(&stats, "stats", output_format, metadata)
}
