// ABOUTME: Batch fitness snapshot fetcher for group coaching context
// ABOUTME: Fetches CTL, ATL, TSB, and weekly volume for group members in parallel
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Group fitness snapshot service
//!
//! Provides batch fetching of fitness metrics (CTL, ATL, TSB, weekly volume)
//! for all members of a coaching group. Used by the group context injection
//! system to give the AI coach data-driven group advice.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::{Arc, LazyLock, Mutex as StdMutex, PoisonError};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use futures_util::future::join_all;
use pierre_core::models::groups::{MemberFitnessSnapshot, OvertrainingRiskLevel, RosterActivity};
use pierre_core::models::{Activity, Tenant, TenantId};
use pierre_intelligence::{AlgorithmConfig, TrainingLoadCalculator};
use pierre_providers::core::ActivityQueryParams;
use pierre_runtime_context::DataContext;
use tokio::time::timeout;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::protocol::AuthService;
use crate::runtime::ToolRuntime;

/// Number of days of activity history to fetch for training load calculation.
/// CTL uses a 42-day exponential moving average, so 60 days gives adequate data.
const TRAINING_LOAD_LOOKBACK_DAYS: i64 = 60;

/// Retention + read window for the provider-agnostic activity cache. Exceeds
/// the training-load lookback so cached reads always cover CTL/ATL/TSB.
const ACTIVITY_CACHE_RETENTION_DAYS: i64 = 90;

/// Age beyond which cached activities trigger a background revalidation
/// (stale-while-revalidate). Cached data is served immediately regardless;
/// this only decides whether to ALSO kick a background refresh.
const ACTIVITY_CACHE_FRESH_SECS: i64 = 4 * 3600;

/// Upper bound on a single background revalidation. A sciotte/Garmin scrape
/// that hangs on a provider-side throttle must not hold the single-flight slot
/// (nor the per-profile Chrome `SingletonLock`) open indefinitely, blocking
/// every later revalidation for the same user. Generous relative to a healthy
/// ~2-minute scrape so it only fires on a genuine stall.
const REVALIDATION_TIMEOUT_SECS: u64 = 240;

/// Default time window (minutes) for considering two activities as duplicates
const DEFAULT_DEDUP_TIME_WINDOW_MINUTES: i64 = 15;

/// Default distance tolerance (percentage) for duplicate detection
const DEFAULT_DEDUP_DISTANCE_TOLERANCE_PCT: f64 = 10.0;

/// Minimum fraction of the shorter activity's duration that must overlap
/// with the longer one for the cross-sport dedup path to collapse them.
///
/// Wrist-based trackers (WHOOP, Apple Watch) frequently misclassify
/// activities — a long bike ride shows up as a `Run` because the wrist
/// moves enough to trigger the wrong heuristic. The GPS provider
/// (Strava) reports the same physical session as `Ride`. Same-sport
/// dedup rejects this pair because of the sport mismatch, so both rows
/// survive and double-count toward weekly volume and the `Recent:`
/// block. The cross-sport check below relaxes the sport requirement
/// when the two activities' time windows overlap substantially (≥60%
/// of the shorter session), interpreting that overlap as strong
/// evidence of the same physical workout regardless of the declared
/// sport label.
const CROSS_SPORT_OVERLAP_FRACTION: f64 = 0.6;

/// Lookback (days) for the `recent_activities` roster list rendered into
/// the group context. One week is what coaches ask about ("this week",
/// "the weekend", "yesterday"); longer windows blow the token budget.
const RECENT_ACTIVITIES_LOOKBACK_DAYS: i64 = 7;

/// Cap on the number of `recent_activities` rendered per member.
/// Protects the prompt budget against athletes who log many short
/// sessions (WHOOP can emit 5+ entries on a hard day).
const RECENT_ACTIVITIES_LIMIT: usize = 12;

/// Returns `true` when a timestamp's time-of-day component is exactly
/// `00:00:00` UTC, indicating the provider only resolved the workout day
/// (e.g. the Strava-mirror scraper). Used by the deduplicator to widen
/// the comparison window from minute-precision to calendar-day precision
/// for such pairs so they collapse against their precise-timestamped
/// counterparts on the same day.
fn is_date_only_timestamp(ts: DateTime<Utc>) -> bool {
    ts.time() == chrono::NaiveTime::MIN
}

/// Compute the wall-clock overlap in seconds between two activities, using
/// each one's start time and reported duration. Returns `None` when either
/// duration is unrepresentable as `i64` (effectively never under realistic
/// activity lengths) or when the intervals don't overlap.
fn time_overlap_seconds(a: &Activity, b: &Activity) -> Option<u64> {
    let a_dur_secs = i64::try_from(a.duration_seconds()).ok()?;
    let b_dur_secs = i64::try_from(b.duration_seconds()).ok()?;
    let a_end = a.start_date() + Duration::seconds(a_dur_secs);
    let b_end = b.start_date() + Duration::seconds(b_dur_secs);
    let overlap_start = a.start_date().max(b.start_date());
    let overlap_end = a_end.min(b_end);
    let overlap = (overlap_end - overlap_start).num_seconds();
    if overlap > 0 {
        u64::try_from(overlap).ok()
    } else {
        None
    }
}

// ══════════════════════════════════════════════════════════════
// Activity Deduplication
// ══════════════════════════════════════════════════════════════

/// Pluggable strategy for deduplicating activities from multiple providers.
///
/// When a user connects multiple providers (e.g., Strava + Garmin), the same
/// workout may appear from both sources. Implementations decide how to detect
/// and resolve these overlaps.
pub trait ActivityDeduplicator: Send + Sync {
    /// Remove duplicate activities, keeping the best version of each.
    fn deduplicate(&self, activities: Vec<Activity>) -> Vec<Activity>;
}

/// Deduplicates activities using time proximity, sport type, and distance similarity.
///
/// Configuration loaded from environment:
/// - `ACTIVITY_DEDUP_TIME_WINDOW_MINUTES` — max minutes between start times (default: 15)
/// - `ACTIVITY_DEDUP_DISTANCE_TOLERANCE_PCT` — max distance difference percentage (default: 10)
pub struct TimeWindowDeduplicator {
    time_window_minutes: i64,
    distance_tolerance_pct: f64,
}

impl TimeWindowDeduplicator {
    /// Create from environment variables with defaults.
    #[must_use]
    pub fn from_env() -> Self {
        let time_window_minutes = env::var("ACTIVITY_DEDUP_TIME_WINDOW_MINUTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_DEDUP_TIME_WINDOW_MINUTES);

        let distance_tolerance_pct = env::var("ACTIVITY_DEDUP_DISTANCE_TOLERANCE_PCT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_DEDUP_DISTANCE_TOLERANCE_PCT);

        Self {
            time_window_minutes,
            distance_tolerance_pct,
        }
    }

    /// Check if two activities from different providers are likely the same workout.
    ///
    /// Two-mode time check:
    /// - Strict `time_window_minutes` when both timestamps look precise
    ///   (anything more than ~1 minute past midnight UTC).
    /// - Calendar-date match when either side is anchored at midnight UTC.
    ///   Scrape-based mirrors (e.g. the Strava-mirror sciotte provider)
    ///   only resolve to the workout day, not its actual start time, so
    ///   minute-window comparisons against a real Strava timestamp 12h
    ///   later let cross-provider duplicates slip through and double-count.
    fn is_likely_duplicate(&self, a: &Activity, b: &Activity) -> bool {
        // Same provider can't produce cross-provider duplicates
        if a.provider() == b.provider() {
            return false;
        }

        if a.sport_type() == b.sport_type() {
            self.is_same_sport_duplicate(a, b)
        } else {
            Self::is_cross_sport_duplicate(a, b)
        }
    }

    /// Same-sport cross-provider dedup: time proximity + distance similarity.
    /// Distance check is skipped when either source lacks GPS data, in which
    /// case the date+sport match alone is sufficient.
    fn is_same_sport_duplicate(&self, a: &Activity, b: &Activity) -> bool {
        // Time proximity: prefer minute-window when both look precise, fall
        // back to calendar-date match when at least one side is date-only.
        let a_is_date_only = is_date_only_timestamp(a.start_date());
        let b_is_date_only = is_date_only_timestamp(b.start_date());
        if a_is_date_only || b_is_date_only {
            if a.start_date().date_naive() != b.start_date().date_naive() {
                return false;
            }
        } else {
            let time_diff = (a.start_date() - b.start_date()).num_minutes().abs();
            if time_diff > self.time_window_minutes {
                return false;
            }
        }

        // Distance proximity (when both have distance data)
        match (a.distance_meters(), b.distance_meters()) {
            (Some(da), Some(db)) => {
                let max = da.max(db);
                max > 0.0 && (da - db).abs() / max * 100.0 < self.distance_tolerance_pct
            }
            // Both missing distance — trust the date+sport match
            _ => true,
        }
    }

    /// Cross-sport cross-provider dedup: time-overlap fraction only.
    ///
    /// Distance and sport-label are intentionally ignored — the wrist-tracker
    /// (WHOOP, Apple Watch) doesn't have GPS for the GPS provider's sport
    /// (cycling on a watch reports as `Run`; the same workout shows up as
    /// `Ride` on Strava with full distance). When both sessions overlap in
    /// wall-clock time by at least `CROSS_SPORT_OVERLAP_FRACTION` of the
    /// shorter one's duration, they are the same physical workout from
    /// different sources. `pick_best` then keeps the GPS row.
    ///
    /// Conservative: returns false when either side is anchored at midnight
    /// UTC (date-only timestamps from scrape-mirror providers like sciotte),
    /// since the overlap math is unreliable without a real start time.
    /// Returns false when either side has zero duration.
    fn is_cross_sport_duplicate(a: &Activity, b: &Activity) -> bool {
        if is_date_only_timestamp(a.start_date()) || is_date_only_timestamp(b.start_date()) {
            return false;
        }
        let a_dur = a.duration_seconds();
        let b_dur = b.duration_seconds();
        if a_dur == 0 || b_dur == 0 {
            return false;
        }
        let Some(overlap_secs) = time_overlap_seconds(a, b) else {
            return false;
        };
        let shorter_secs = a_dur.min(b_dur);
        if shorter_secs == 0 {
            return false;
        }
        #[allow(clippy::cast_precision_loss)]
        // bounded by single-activity duration; far below f64 limits
        let overlap_fraction = overlap_secs as f64 / shorter_secs as f64;
        overlap_fraction >= CROSS_SPORT_OVERLAP_FRACTION
    }

    /// Pick the better version of two duplicate activities.
    /// Prefers the one with more data (distance present, longer duration).
    fn pick_best(a: &Activity, b: &Activity) -> usize {
        let a_has_distance = a.distance_meters().is_some_and(|d| d > 0.0);
        let b_has_distance = b.distance_meters().is_some_and(|d| d > 0.0);

        if a_has_distance && !b_has_distance {
            return 0;
        }
        if b_has_distance && !a_has_distance {
            return 1;
        }

        // Both have or both lack distance — prefer longer duration
        usize::from(a.duration_seconds() < b.duration_seconds())
    }
}

impl ActivityDeduplicator for TimeWindowDeduplicator {
    fn deduplicate(&self, mut activities: Vec<Activity>) -> Vec<Activity> {
        if activities.len() < 2 {
            return activities;
        }

        // Sort by start time for efficient comparison
        activities.sort_by_key(Activity::start_date);

        let mut keep = vec![true; activities.len()];
        for i in 0..activities.len() {
            if !keep[i] {
                continue;
            }
            for j in (i + 1)..activities.len() {
                if !keep[j] {
                    continue;
                }
                // Early exit: once activities[j] falls on a strictly later
                // calendar day than activities[i], no further pair on this
                // outer index can be a same-workout duplicate. Date-bucketed
                // rather than minute-bucketed so scrape-mirror providers
                // (sciotte: date-only timestamps) still pair against the
                // matching Strava workout 12+ hours later in the same day.
                if activities[j].start_date().date_naive() > activities[i].start_date().date_naive()
                {
                    break;
                }
                if self.is_likely_duplicate(&activities[i], &activities[j]) {
                    let loser = if Self::pick_best(&activities[i], &activities[j]) == 0 {
                        j
                    } else {
                        i
                    };
                    keep[loser] = false;
                }
            }
        }

        let total = activities.len();
        let result: Vec<Activity> = activities
            .into_iter()
            .zip(keep.iter())
            .filter(|(_, &k)| k)
            .map(|(a, _)| a)
            .collect();

        let removed = total - result.len();
        if removed > 0 {
            debug!(
                removed,
                total, "Deduplication removed cross-provider duplicates"
            );
        }

        result
    }
}

// ══════════════════════════════════════════════════════════════
// Activity Merge Strategy
// ══════════════════════════════════════════════════════════════

/// Pluggable strategy for fetching and merging activities across providers.
///
/// Implementations control whether to use one provider, all providers,
/// or a subset, and how to combine the results.
#[async_trait]
pub(crate) trait ActivityMergeStrategy: Send + Sync {
    /// Fetch and merge activities from multiple providers for a single user.
    async fn fetch_and_merge(
        &self,
        auth_service: &AuthService,
        providers: &[String],
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Vec<Activity>;
}

/// Fetches from ALL connected providers in parallel, merges, and deduplicates.
///
/// This is the default strategy: every provider's activities contribute to the
/// training load calculation, giving a complete picture of the athlete's workload.
pub(crate) struct AllProvidersMerge {
    deduplicator: Box<dyn ActivityDeduplicator>,
    activity_limit: usize,
}

impl AllProvidersMerge {
    /// Create with the default time-window deduplicator (env-configured) and a
    /// caller-supplied per-provider activity limit — typically sourced from
    /// `ServerConfig::activity_fetch_limit` so that a single env variable
    /// (`ACTIVITY_FETCH_LIMIT`) governs every activity-fetching path.
    pub(crate) fn new(activity_limit: usize) -> Self {
        Self {
            deduplicator: Box::new(TimeWindowDeduplicator::from_env()),
            activity_limit,
        }
    }
}

#[async_trait]
impl ActivityMergeStrategy for AllProvidersMerge {
    async fn fetch_and_merge(
        &self,
        auth_service: &AuthService,
        providers: &[String],
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Vec<Activity> {
        let tenant_id_str = tenant_id.to_string();
        let now = Utc::now();
        let lookback_start = (now - Duration::days(TRAINING_LOAD_LOOKBACK_DAYS)).timestamp();
        let params = ActivityQueryParams {
            limit: Some(self.activity_limit),
            offset: None,
            before: None,
            after: Some(lookback_start),
        };

        // Fetch from all providers in parallel
        let futures: Vec<_> = providers
            .iter()
            .map(|provider_name| {
                let tid = tenant_id_str.clone();
                let p = params.clone();
                let pname = provider_name.clone();
                async move {
                    let result =
                        try_fetch_from_provider(auth_service, &pname, user_id, &tid, &p).await;
                    (pname, result)
                }
            })
            .collect();

        let results = join_all(futures).await;

        // Merge all successful results
        let mut all_activities = Vec::new();
        let mut provider_count = 0u32;
        for (provider_name, maybe_activities) in results {
            if let Some(activities) = maybe_activities {
                info!(
                    user_id = %user_id,
                    provider = %provider_name,
                    count = activities.len(),
                    "Snapshot: fetched from provider"
                );
                // Write-through: persist the freshly fetched activities keyed by
                // the connection provider name so subsequent chat turns serve
                // them from cache instead of re-fetching (stale-while-revalidate).
                write_through_activity_cache(
                    auth_service,
                    user_id,
                    tenant_id,
                    &provider_name,
                    &activities,
                )
                .await;
                all_activities.extend(activities);
                provider_count += 1;
            }
        }

        if all_activities.is_empty() {
            return Vec::new();
        }

        // Deduplicate cross-provider overlaps
        let before_dedup = all_activities.len();
        let merged = self.deduplicator.deduplicate(all_activities);

        info!(
            user_id = %user_id,
            providers_used = provider_count,
            before_dedup,
            after_dedup = merged.len(),
            "Snapshot: merged activities from all providers"
        );

        merged
    }
}

/// Fetch fitness snapshots for a batch of group members in parallel.
///
/// For each user, creates an authenticated provider, fetches recent activities,
/// and computes training load metrics. Users without connected providers or
/// activity data receive snapshots with `None` metric fields.
///
/// Errors are handled per-user: a failed provider connection or activity fetch
/// does not prevent other members' snapshots from being returned.
pub async fn fetch_member_snapshots(
    runtime: &Arc<dyn ToolRuntime>,
    user_ids: &[Uuid],
    tenant_id: TenantId,
) -> Vec<MemberFitnessSnapshot> {
    if user_ids.is_empty() {
        return Vec::new();
    }

    let futures: Vec<_> = user_ids
        .iter()
        .map(|&user_id| {
            let runtime = Arc::clone(runtime);
            let tid = tenant_id;
            async move { fetch_single_member_snapshot(&runtime, user_id, tid).await }
        })
        .collect();

    join_all(futures).await
}

/// Fetch display name for a user from the global user database.
///
/// Returns the user's display name if set, email prefix if not, or "Unknown"
/// if the user cannot be fetched.
async fn fetch_user_display_name(data: &DataContext, user_id: Uuid) -> String {
    match data.repos().users.get_global(user_id).await {
        Ok(Some(user)) => user
            .display_name
            .unwrap_or_else(|| user.email.split('@').next().unwrap_or("Unknown").to_owned()),
        Ok(None) => {
            info!(
                user_id = %user_id,
                "Snapshot: user record not found, display_name falls back to 'Unknown'"
            );
            "Unknown".to_owned()
        }
        Err(e) => {
            info!(user_id = %user_id, error = %e, "Snapshot: failed to fetch user; display_name falls back to 'Unknown'");
            "Unknown".to_owned()
        }
    }
}

/// Compute training load metrics from a list of activities.
///
/// Uses `TrainingLoadCalculator` to compute CTL, ATL, and TSB.
/// Returns `(None, None, None)` if calculation fails.
fn compute_training_metrics(
    activities: &[Activity],
    algorithm_config: &AlgorithmConfig,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    // Sort oldest-first — EMA calculation requires chronological order
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);

    let calculator = TrainingLoadCalculator::from_config(algorithm_config.clone());
    log_per_activity_tss(&calculator, &sorted);

    match calculator.calculate_training_load(
        &sorted, None, // FTP
        None, // LTHR
        None, // max_hr
        None, // resting_hr
        None, // weight_kg
    ) {
        Ok(load) => (Some(load.ctl), Some(load.atl), Some(load.tsb)),
        Err(e) => {
            debug!(error = ?e, "Training load calculation failed");
            (None, None, None)
        }
    }
}

/// Emit one log line per activity showing its computed TSS plus an
/// aggregate summary. Diagnoses why an athlete with N workouts can land
/// at unexpectedly low CTL/ATL (e.g. WHOOP entries with no distance/HR
/// falling through to the duration-only fallback). Extracted from
/// `compute_training_metrics` to keep that fn under the cognitive
/// complexity budget.
fn log_per_activity_tss(calculator: &TrainingLoadCalculator, sorted: &[Activity]) {
    let mut total_tss = 0.0_f64;
    let mut tss_some = 0_usize;
    let mut tss_none = 0_usize;
    for a in sorted {
        if let Ok(v) = calculator.calculate_tss(a, None, None, None, None, None) {
            total_tss += v;
            tss_some += 1;
            info!(
                activity_id = %a.id(),
                name = %a.name(),
                sport = ?a.sport_type(),
                start = %a.start_date(),
                duration_s = a.duration_seconds(),
                distance_m = ?a.distance_meters(),
                tss = v,
                "Snapshot TSS per-activity"
            );
        } else {
            tss_none += 1;
            info!(
                activity_id = %a.id(),
                name = %a.name(),
                sport = ?a.sport_type(),
                start = %a.start_date(),
                duration_s = a.duration_seconds(),
                distance_m = ?a.distance_meters(),
                "Snapshot TSS skipped — no estimator could produce a value"
            );
        }
    }
    info!(
        total_activities = sorted.len(),
        tss_included = tss_some,
        tss_skipped = tss_none,
        total_tss,
        "Snapshot TSS aggregate"
    );
}

/// Number of days in one week, used to split current vs previous week.
const DAYS_PER_WEEK: i64 = 7;

/// Aggregated current-week metrics plus the prior week's volume trend input.
///
/// `previous_week_volume_km` is `None` when no activity falls in the
/// 7-to-14-day window — the group trend aggregation treats a missing prior
/// week as "no trend signal" rather than a zero-volume week.
pub struct WeeklyMetrics {
    /// Number of activities in the trailing 7-day window.
    pub activity_count: i32,
    /// Distance (km) summed across the trailing 7-day window.
    pub volume_km: f64,
    /// Total active duration (seconds) across the trailing 7-day window.
    pub duration_seconds: i64,
    /// Distance (km) for the 7-to-14-day window, or `None` when that window
    /// holds no activities. Feeds the group-level weekly trend.
    pub previous_week_volume_km: Option<f64>,
    /// Whole days since the most-recent activity, or `None` when there are none.
    pub days_since_last: Option<i32>,
}

/// Compute weekly metrics from activities in the past 7 days, plus the prior
/// week's volume for trend detection.
///
/// `duration_seconds` is the sum of `Activity::duration_seconds()` across the
/// week — preserved separately from `volume_km` so HR/duration-only sources
/// (WHOOP, indoor trainers) surface as training volume even with no GPS.
/// `previous_week_volume_km` covers the 7-to-14-day window and feeds the
/// group-level weekly trend; the 60-day lookback already fetched that data.
#[must_use]
pub fn compute_weekly_metrics(activities: &[Activity], now: DateTime<Utc>) -> WeeklyMetrics {
    let seven_days_ago = now - Duration::days(DAYS_PER_WEEK);
    let fourteen_days_ago = now - Duration::days(2 * DAYS_PER_WEEK);
    let weekly_activities: Vec<_> = activities
        .iter()
        .filter(|a| a.start_date() >= seven_days_ago)
        .collect();

    let weekly_activity_count = i32::try_from(weekly_activities.len()).unwrap_or(i32::MAX);

    let weekly_volume_km = weekly_activities
        .iter()
        .filter_map(|a| a.distance_meters())
        .sum::<f64>()
        / 1000.0;

    let weekly_duration_seconds: i64 = weekly_activities
        .iter()
        .map(|a| i64::try_from(a.duration_seconds()).unwrap_or(i64::MAX))
        .sum();

    let previous_week: Vec<_> = activities
        .iter()
        .filter(|a| a.start_date() >= fourteen_days_ago && a.start_date() < seven_days_ago)
        .collect();
    let previous_week_volume_km = if previous_week.is_empty() {
        None
    } else {
        Some(
            previous_week
                .iter()
                .filter_map(|a| a.distance_meters())
                .sum::<f64>()
                / 1000.0,
        )
    };

    let days_since_last = activities
        .iter()
        .map(Activity::start_date)
        .max()
        .map(|last| i32::try_from((now - last).num_days()).unwrap_or(i32::MAX));

    WeeklyMetrics {
        activity_count: weekly_activity_count,
        volume_km: weekly_volume_km,
        duration_seconds: weekly_duration_seconds,
        previous_week_volume_km,
        days_since_last,
    }
}

/// Get all connected provider names for a user.
///
/// Returns provider names in connection order, or empty vec if none connected.
async fn get_connected_providers(
    data: &DataContext,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Vec<String> {
    data.repos()
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.provider)
        .collect()
}

/// Try authenticating with one provider and fetching its activities.
///
/// Returns `Some(activities)` on success, `None` if auth or fetch fails.
async fn try_fetch_from_provider(
    auth_service: &AuthService,
    provider_name: &str,
    user_id: Uuid,
    tenant_id_str: &str,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let provider = auth_service
        .create_authenticated_provider(provider_name, user_id, Some(tenant_id_str))
        .await
        .map_err(|e| {
            info!(user_id = %user_id, provider = %provider_name, error = ?e, "Snapshot: auth failed");
        })
        .ok()?;

    let activities = provider
        .get_activities_with_params(params)
        .await
        .map_err(|e| {
            // WARN, not INFO: a failed live fetch is why the stale-while-revalidate
            // cache stops advancing. Logging it at INFO hid a Chrome profile-lock
            // collision that froze a user's activity data for a full day.
            warn!(user_id = %user_id, provider = %provider_name, error = %e, "Snapshot: fetch failed");
        })
        .ok()
        .filter(|a| !a.is_empty())?;

    debug!(user_id = %user_id, provider = %provider_name, count = activities.len(), "Snapshot: fetched");
    Some(activities)
}

/// Resolve a group member's own tenant for OAuth credential lookup.
///
/// Cross-tenant group members need their own tenant to find their provider
/// connections and OAuth tokens. Prefers the requester's tenant (`fallback`)
/// when the member is a shared participant — this is the tenant providing the
/// shared group context, and any other membership is unrelated to this call.
/// Only picks a different tenant when the member does not share the
/// requester's tenant, and even then only when exactly one alternative exists,
/// which removes the order-dependent ambiguity that previously caused
/// cross-tenant reads for members belonging to multiple tenants.
async fn resolve_member_tenant(data: &DataContext, user_id: Uuid, fallback: TenantId) -> TenantId {
    match data.repos().tenants.list_for_user(user_id).await {
        Ok(tenants) => pick_member_tenant_id(user_id, fallback, &tenants),
        Err(e) => {
            info!(
                user_id = %user_id,
                error = %e,
                fallback_tenant = %fallback,
                "Snapshot: failed to resolve member tenant — using fallback tenant for OAuth lookup"
            );
            fallback
        }
    }
}

/// Pure tenant-selection policy extracted from [`resolve_member_tenant`].
///
/// Encodes the "prefer shared tenant, then unique alternative, else fallback"
/// rule separately from the repository call so it stays below the cognitive
/// complexity threshold and is easier to reason about at a glance.
fn pick_member_tenant_id(user_id: Uuid, fallback: TenantId, tenants: &[Tenant]) -> TenantId {
    if tenants.is_empty() {
        info!(user_id = %user_id, "Snapshot: member has no tenant memberships, using fallback");
        return fallback;
    }
    if tenants.iter().any(|t| t.id == fallback) {
        return fallback;
    }
    if let [only] = tenants {
        log_member_tenant_resolution(user_id, only.id, fallback);
        return only.id;
    }
    info!(
        user_id = %user_id,
        tenant_count = tenants.len(),
        "Snapshot: member does not share requester's tenant and has multiple memberships; using fallback to avoid ambiguous resolution"
    );
    fallback
}

fn log_member_tenant_resolution(user_id: Uuid, resolved: TenantId, fallback: TenantId) {
    if resolved != fallback {
        info!(
            user_id = %user_id,
            member_tenant = %resolved,
            requester_tenant = %fallback,
            "Using member's own tenant for snapshot fetch"
        );
    }
}

/// Fetch a single member's fitness snapshot.
///
/// Resolves the member's own tenant for OAuth credential lookup, then
/// attempts to find connected providers and compute training load from
/// their recent activities. Returns a snapshot with `None` metrics
/// if no provider is connected or if the fetch fails.
async fn fetch_single_member_snapshot(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    fallback_tenant_id: TenantId,
) -> MemberFitnessSnapshot {
    let now = Utc::now();
    let data = runtime.data();
    let display_name = fetch_user_display_name(&data, user_id).await;

    let tenant_id = resolve_member_tenant(&data, user_id, fallback_tenant_id).await;

    let activities = fetch_member_activities(runtime, user_id, tenant_id).await;
    // Emit one log line per fetched activity so an operator can verify
    // what reached the training-load calc when a snapshot looks stale
    // (e.g. "Phil rode 250km yesterday but ATL=19" — either the ride
    // isn't on the provider yet, or its TSS was skipped for missing
    // fields). debug! to keep the noise off INFO in normal operation.
    for a in &activities {
        info!(
            user_id = %user_id,
            activity_id = %a.id(),
            name = %a.name(),
            sport = ?a.sport_type(),
            start = %a.start_date(),
            duration_s = a.duration_seconds(),
            distance_m = ?a.distance_meters(),
            avg_hr = ?a.average_heart_rate(),
            "Snapshot: activity included in training-load input"
        );
    }
    if activities.is_empty() {
        return empty_snapshot(user_id, display_name, now);
    }

    build_snapshot_from_activities(
        user_id,
        display_name,
        &activities,
        now,
        &runtime.cageux_config().algorithms,
    )
}

/// Resolve a member's connected providers and fetch activities using the merge strategy.
///
/// Fetches from ALL providers, merges, and deduplicates to produce a complete
/// activity list for training load computation.
async fn fetch_member_activities(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Vec<Activity> {
    let data = runtime.data();
    let providers = get_connected_providers(&data, user_id, tenant_id).await;
    if providers.is_empty() {
        info!(
            user_id = %user_id,
            tenant_id = %tenant_id,
            "Snapshot: member has no connected providers — emitting empty snapshot (weekly counts and CTL/ATL/TSB will all be zero/None)"
        );
        return Vec::new();
    }

    let now = Utc::now();
    let window_start = now - Duration::days(ACTIVITY_CACHE_RETENTION_DAYS);
    let read_limit = i64::try_from(
        runtime
            .config()
            .activity_fetch_limit
            .saturating_mul(providers.len()),
    )
    .unwrap_or(i64::MAX);

    // Stale-while-revalidate: serve cached activities immediately when present.
    let cached = data
        .repos()
        .activity_cache
        .get_cached_activities(user_id, &tenant_id, None, window_start, now, read_limit)
        .await
        .unwrap_or_else(|e| {
            info!(user_id = %user_id, error = %e, "Activity cache: read failed; falling back to live fetch");
            Vec::new()
        });

    if cached.is_empty() {
        // Cold cache: fetch live now; write-through persists for next time.
        return fetch_and_persist_live(runtime, &providers, user_id, tenant_id).await;
    }

    // Warm cache: revalidate in the background only when stale, then serve cached.
    if activity_cache_is_stale(&data, user_id, tenant_id, &providers, now).await {
        spawn_activity_revalidation(runtime, providers, user_id, tenant_id);
    }

    // Dedup on read. Write-through persists each provider's activities under its
    // own cache key, so a workout synced from two providers (e.g. Strava +
    // sciotte/Garmin) is cached twice. The live-fetch path dedups before
    // returning (see `AllProvidersMerge::fetch_and_merge`); the warm-cache path
    // must do the same or cross-provider duplicates double-count in the snapshot.
    let before_dedup = cached.len();
    let cached = TimeWindowDeduplicator::from_env().deduplicate(cached);
    info!(
        user_id = %user_id,
        before_dedup,
        after_dedup = cached.len(),
        "Snapshot: served activities from cache (stale-while-revalidate, deduplicated)"
    );
    cached
}

/// Fetch activities live from all connected providers and persist them
/// (write-through happens inside [`AllProvidersMerge::fetch_and_merge`]).
///
/// Used on a cold cache and by the background revalidation task.
async fn fetch_and_persist_live(
    runtime: &Arc<dyn ToolRuntime>,
    providers: &[String],
    user_id: Uuid,
    tenant_id: TenantId,
) -> Vec<Activity> {
    let auth_service = AuthService::new(Arc::clone(runtime));
    let strategy = AllProvidersMerge::new(runtime.config().activity_fetch_limit);
    strategy
        .fetch_and_merge(&auth_service, providers, user_id, tenant_id)
        .await
}

/// True when any connected provider's cached activities are missing or older
/// than [`ACTIVITY_CACHE_FRESH_SECS`], signalling a background revalidation.
async fn activity_cache_is_stale(
    data: &DataContext,
    user_id: Uuid,
    tenant_id: TenantId,
    providers: &[String],
    now: DateTime<Utc>,
) -> bool {
    let fresh_cutoff = now - Duration::seconds(ACTIVITY_CACHE_FRESH_SECS);
    for provider in providers {
        let last_sync = data
            .repos()
            .activity_cache
            .latest_activity_sync(user_id, &tenant_id, provider)
            .await
            .unwrap_or(None);
        // Stale when never cached or the newest cached activity predates the
        // freshness window.
        if last_sync.is_none_or(|ts| ts < fresh_cutoff) {
            return true;
        }
    }
    false
}

/// Tracks which `(user, tenant)` background revalidations are in flight so
/// concurrent stale-cache chat turns collapse onto a single refresh.
///
/// Without this, every stale-cache turn spawned its own revalidation. The
/// per-profile Chrome `SingletonLock` (see [`pierre_providers`]) serializes
/// those scrapes rather than crashing, so N rapid turns queue N ~2-minute
/// scrapes behind one lock — the later ones redundant by the time they run.
/// Stale-while-revalidate only needs one in-flight refresh per user; the rest
/// are dropped. Entries are removed when the revalidation task finishes (or
/// times out), so the slot frees for the next genuinely-stale turn.
///
/// Production code shares one registry via [`Self::global`]; tests construct
/// isolated instances with [`Self::new`].
pub struct RevalidationRegistry {
    in_flight: Arc<StdMutex<HashSet<(Uuid, TenantId)>>>,
}

impl RevalidationRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            in_flight: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    /// The process-global registry shared across every chat turn.
    pub fn global() -> &'static Self {
        static GLOBAL: LazyLock<RevalidationRegistry> = LazyLock::new(RevalidationRegistry::new);
        &GLOBAL
    }

    /// Claim the revalidation slot for a user. Returns a guard that frees the
    /// slot on drop, or `None` if a revalidation is already in flight (the
    /// caller then skips spawning a duplicate).
    pub fn try_claim(&self, key: (Uuid, TenantId)) -> Option<RevalidationGuard> {
        // Recover from poisoning: the set stays consistent even if a holder
        // panicked, and refusing all future revalidations would be worse.
        let mut set = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if set.insert(key) {
            Some(RevalidationGuard {
                in_flight: Arc::clone(&self.in_flight),
                key,
            })
        } else {
            None
        }
    }
}

impl Default for RevalidationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Frees a claimed revalidation slot when dropped, including on panic or
/// timeout, so a single failed refresh never wedges a user's slot shut.
pub struct RevalidationGuard {
    in_flight: Arc<StdMutex<HashSet<(Uuid, TenantId)>>>,
    key: (Uuid, TenantId),
}

impl Drop for RevalidationGuard {
    fn drop(&mut self) {
        let mut set = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set.remove(&self.key);
    }
}

/// Spawn a detached background task that re-fetches and persists activities so
/// the next chat turn sees fresh data. Errors are logged, never surfaced.
///
/// Deduplicated and timeout-bound: only one revalidation runs per `(user,
/// tenant)` at a time (concurrent stale-cache turns are dropped), and each is
/// capped at [`REVALIDATION_TIMEOUT_SECS`] so a hung scrape releases the slot
/// instead of blocking later refreshes behind the per-profile Chrome lock.
fn spawn_activity_revalidation(
    runtime: &Arc<dyn ToolRuntime>,
    providers: Vec<String>,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    let Some(guard) = RevalidationRegistry::global().try_claim((user_id, tenant_id)) else {
        debug!(
            user_id = %user_id,
            "Activity cache: revalidation already in flight; skipping duplicate"
        );
        return;
    };

    let runtime = Arc::clone(runtime);
    tokio::spawn(async move {
        // Hold the slot for the whole refresh; dropping `guard` frees it.
        let _guard = guard;
        info!(user_id = %user_id, "Activity cache: background revalidation started");
        match timeout(
            StdDuration::from_secs(REVALIDATION_TIMEOUT_SECS),
            fetch_and_persist_live(&runtime, &providers, user_id, tenant_id),
        )
        .await
        {
            Ok(activities) => info!(
                user_id = %user_id,
                refreshed = activities.len(),
                "Activity cache: background revalidation complete"
            ),
            Err(_elapsed) => warn!(
                user_id = %user_id,
                timeout_secs = REVALIDATION_TIMEOUT_SECS,
                "Activity cache: background revalidation timed out; releasing slot"
            ),
        }
    });
}

/// Persist a provider's freshly fetched activities into the activity cache and
/// prune rows outside the retention window. Failures are logged, not fatal —
/// the live activities still flow to the caller.
///
/// Shared with [`crate::activity_fetch`] so the coach/recommendation fetch path
/// warms the same stale-while-revalidate cache the group snapshot builder reads.
pub(crate) async fn write_through_activity_cache(
    auth_service: &AuthService,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    activities: &[Activity],
) {
    let data = auth_service.runtime().data();
    let cache = data.repos().activity_cache.clone();
    if let Err(e) = cache
        .upsert_activities(user_id, &tenant_id, provider, activities)
        .await
    {
        info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: write-through failed");
        return;
    }
    let cutoff = Utc::now() - Duration::days(ACTIVITY_CACHE_RETENTION_DAYS);
    if let Err(e) = cache
        .prune_activities_before(user_id, &tenant_id, cutoff)
        .await
    {
        info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: prune failed");
    }
}

/// Compute all fitness metrics from activities and assemble the snapshot.
fn build_snapshot_from_activities(
    user_id: Uuid,
    display_name: String,
    activities: &[Activity],
    now: DateTime<Utc>,
    algorithm_config: &AlgorithmConfig,
) -> MemberFitnessSnapshot {
    let (ctl, atl, tsb) = compute_training_metrics(activities, algorithm_config);
    let weekly = compute_weekly_metrics(activities, now);
    let primary_sport = determine_primary_sport(activities);
    let overtraining_risk = assess_overtraining_risk(ctl, atl, tsb);
    let last_activity_per_provider = compute_last_activity_per_provider(activities);
    let recent_activities = compute_recent_activities(activities, now);

    MemberFitnessSnapshot {
        user_id,
        display_name,
        ctl,
        atl,
        tsb,
        weekly_volume_km: weekly.volume_km,
        previous_week_volume_km: weekly.previous_week_volume_km,
        weekly_activity_count: weekly.activity_count,
        weekly_duration_seconds: weekly.duration_seconds,
        primary_sport,
        vdot: None,
        overtraining_risk,
        days_since_last_activity: weekly.days_since_last,
        last_activity_per_provider,
        recent_activities,
        computed_at: now,
    }
}

/// Build the compact per-activity list rendered into the group context.
///
/// Returns at most `RECENT_ACTIVITIES_LIMIT` activities from the past
/// `RECENT_ACTIVITIES_LOOKBACK_DAYS` days, newest first. The cap is
/// applied after the time filter so a single heavy week doesn't push
/// older context into the prompt.
fn compute_recent_activities(activities: &[Activity], now: DateTime<Utc>) -> Vec<RosterActivity> {
    let cutoff = now - Duration::days(RECENT_ACTIVITIES_LOOKBACK_DAYS);
    let total_input = activities.len();
    let mut filtered: Vec<&Activity> = activities
        .iter()
        .filter(|a| a.start_date() >= cutoff)
        .collect();
    let after_filter = filtered.len();
    filtered.sort_by_key(|a| Reverse(a.start_date()));
    let result: Vec<RosterActivity> = filtered
        .into_iter()
        .take(RECENT_ACTIVITIES_LIMIT)
        .map(|a| RosterActivity {
            start: a.start_date(),
            sport: format!("{:?}", a.sport_type()),
            distance_km: a.distance_meters().map(|m| m / 1000.0),
            duration_minutes: i64::try_from(a.duration_seconds() / 60).unwrap_or(i64::MAX),
            name: a.name().to_owned(),
            city: a.city().map(str::to_owned),
            start_latitude: a.start_latitude(),
            start_longitude: a.start_longitude(),
        })
        .collect();
    info!(
        total_input,
        after_filter,
        emitted = result.len(),
        cutoff = %cutoff,
        "Snapshot: recent_activities computed"
    );
    result
}

/// Group activities by provider and keep the most-recent `start_date` per provider.
///
/// Gives the LLM a per-source freshness signal so it can say "Strava
/// stale 33 days, WHOOP current today" instead of pasting a single
/// `days_since_last_activity` over a multi-provider member and inventing
/// "your Strava is not synced" defenses.
fn compute_last_activity_per_provider(activities: &[Activity]) -> HashMap<String, DateTime<Utc>> {
    let mut latest: HashMap<String, DateTime<Utc>> = HashMap::new();
    for activity in activities {
        let provider = activity.provider().to_owned();
        let start = activity.start_date();
        latest
            .entry(provider)
            .and_modify(|existing| {
                if start > *existing {
                    *existing = start;
                }
            })
            .or_insert(start);
    }
    latest
}

/// Create an empty snapshot for a user with no activity data.
fn empty_snapshot(
    user_id: Uuid,
    display_name: String,
    computed_at: DateTime<Utc>,
) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id,
        display_name,
        ctl: None,
        atl: None,
        tsb: None,
        weekly_volume_km: 0.0,
        previous_week_volume_km: None,
        weekly_activity_count: 0,
        weekly_duration_seconds: 0,
        primary_sport: None,
        vdot: None,
        overtraining_risk: OvertrainingRiskLevel::Low,
        days_since_last_activity: None,
        last_activity_per_provider: HashMap::new(),
        recent_activities: Vec::new(),
        computed_at,
    }
}

/// Determine the primary sport type from a list of activities.
///
/// Returns the most frequently occurring sport type, or `None` if empty.
fn determine_primary_sport(activities: &[Activity]) -> Option<String> {
    if activities.is_empty() {
        return None;
    }

    let mut counts = HashMap::new();
    for activity in activities {
        *counts.entry(activity.sport_type().clone()).or_insert(0u32) += 1;
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(sport, _)| format!("{sport:?}"))
}

/// Assess overtraining risk based on training load metrics.
///
/// Uses TSB (Training Stress Balance) and ATL/CTL ratio to determine risk:
/// - TSB < -30 or ATL/CTL > 1.5 → High risk
/// - TSB < -10 or ATL/CTL > 1.3 → Moderate risk
/// - Otherwise → Low risk
fn assess_overtraining_risk(
    ctl: Option<f64>,
    atl: Option<f64>,
    tsb: Option<f64>,
) -> OvertrainingRiskLevel {
    // Check TSB threshold
    if let Some(tsb_val) = tsb {
        if tsb_val < -30.0 {
            return OvertrainingRiskLevel::High;
        }
        if tsb_val < -10.0 {
            return OvertrainingRiskLevel::Moderate;
        }
    }

    // Check ATL/CTL ratio
    if let (Some(atl_val), Some(ctl_val)) = (atl, ctl) {
        if ctl_val > 0.0 {
            let ratio = atl_val / ctl_val;
            if ratio > 1.5 {
                return OvertrainingRiskLevel::High;
            }
            if ratio > 1.3 {
                return OvertrainingRiskLevel::Moderate;
            }
        }
    }

    OvertrainingRiskLevel::Low
}

/// Alias for backwards compatibility with external references
pub type MemberSnapshot = MemberFitnessSnapshot;
