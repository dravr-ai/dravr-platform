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
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use futures_util::future::join_all;
use pierre_core::models::groups::{MemberFitnessSnapshot, OvertrainingRiskLevel, RosterActivity};
use pierre_core::models::FormBand;
use pierre_core::models::{Activity, ProviderConnection, TenantId};
use pierre_intelligence::{AlgorithmConfig, TrainingLoadCalculator};
use pierre_providers::core::ActivityQueryParams;
use pierre_runtime_context::DataContext;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::activity_dedup::{ActivityDeduplicator, TimeWindowDeduplicator};
use crate::activity_fetch::{activity_cache_retention_days, write_through_activity_cache};
use crate::group_activity_cache::fetch_member_activities;
use crate::protocol::AuthService;
use crate::runtime::ToolRuntime;

/// Lookback (days) for the `recent_activities` roster list rendered into
/// the group context. One week is what coaches ask about ("this week",
/// "the weekend", "yesterday"); longer windows blow the token budget.
const RECENT_ACTIVITIES_LOOKBACK_DAYS: i64 = 7;

/// Cap on the number of `recent_activities` rendered per member.
/// Protects the prompt budget against athletes who log many short
/// sessions (WHOOP can emit 5+ entries on a hard day).
const RECENT_ACTIVITIES_LIMIT: usize = 12;

/// Number of days of activity history to fetch for training load calculation.
/// CTL uses a 42-day exponential moving average, so 60 days gives adequate data.
const TRAINING_LOAD_LOOKBACK_DAYS: i64 = 60;

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
                    activity_cache_retention_days(),
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
/// if the user cannot be fetched. `pub(crate)` so the group peer-fetch tool
/// resolves a member by the SAME display name the snapshot roster shows the LLM.
pub(crate) async fn fetch_user_display_name(data: &DataContext, user_id: Uuid) -> String {
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
            debug!(
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

/// Collect the distinct tenants in which a member actually holds a provider
/// connection, falling back to `fallback_tenant_id` only when they hold none.
///
/// A member's OAuth token + `provider_connections` row are pinned to the tenant
/// that was active when they connected the provider (their earliest-joined
/// tenant — see `oauth_flow::get_user_and_tenant`), which is NOT necessarily the
/// group-host/requester tenant. Enumerating the member's own connection tenants
/// (from a cross-tenant lookup) and fetching each under its OWN tenant makes the
/// group snapshot resolve identically to the member's 1-1 chat, instead of
/// guessing a single tenant relative to the requester and silently reading
/// nothing when the guess misses.
fn member_connection_tenants(
    user_id: Uuid,
    connections: &[ProviderConnection],
    fallback_tenant_id: TenantId,
) -> Vec<TenantId> {
    let mut tenants: Vec<TenantId> = Vec::new();
    for conn in connections {
        match TenantId::parse_str(&conn.tenant_id) {
            Ok(t) if !tenants.contains(&t) => tenants.push(t),
            Ok(_) => {}
            Err(e) => info!(
                user_id = %user_id,
                tenant = %conn.tenant_id,
                error = %e,
                "Snapshot: skipping connection with unparseable tenant id"
            ),
        }
    }
    if tenants.is_empty() {
        tenants.push(fallback_tenant_id);
    }
    tenants
}

/// Fetch + merge a member's activities across every tenant where they hold a
/// provider connection.
///
/// Each tenant is fetched via [`fetch_member_activities`] (its own
/// stale-while-revalidate cache read + write-through), then results are merged
/// and deduplicated. The same provider connected under two tenants can surface
/// a workout twice, so a final dedup runs across the merged set even though each
/// per-tenant fetch already dedups within itself.
/// The second element is true when ANY tenant's rows were served stale — one
/// unrefreshed leg is enough to make the merged training load untrustworthy.
async fn fetch_member_activities_across_tenants(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenants: &[TenantId],
) -> (Vec<Activity>, bool) {
    let mut all = Vec::new();
    let mut any_stale = false;
    for &tenant_id in tenants {
        let (activities, served_stale) = fetch_member_activities(runtime, user_id, tenant_id).await;
        all.extend(activities);
        any_stale = any_stale || served_stale;
    }
    if tenants.len() > 1 {
        return (
            TimeWindowDeduplicator::from_env().deduplicate(all),
            any_stale,
        );
    }
    (all, any_stale)
}

/// Fetch a single member's fitness snapshot.
///
/// Resolves the member's connections across all their tenants, fetches and
/// merges activities under each connection's own tenant, and computes training
/// load. Returns a snapshot with `None` metrics if no provider is connected or
/// if the fetch fails.
async fn fetch_single_member_snapshot(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    fallback_tenant_id: TenantId,
) -> MemberFitnessSnapshot {
    let now = Utc::now();
    let data = runtime.data();
    let display_name = fetch_user_display_name(&data, user_id).await;

    // Cross-tenant lookup: a member's connections live under their own tenant,
    // which may differ from the requester/group-host tenant. See
    // [`member_connection_tenants`] for why guessing a single tenant fails.
    let connections = data
        .repos()
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap_or_else(|e| {
            info!(
                user_id = %user_id,
                error = %e,
                "Snapshot: cross-tenant connection lookup failed; treating member as having no connections"
            );
            Vec::new()
        });

    let tenants = member_connection_tenants(user_id, &connections, fallback_tenant_id);
    let (activities, served_stale) =
        fetch_member_activities_across_tenants(runtime, user_id, &tenants).await;
    // Emit one log line per fetched activity so an operator can verify
    // what reached the training-load calc when a snapshot looks stale
    // (e.g. "Phil rode 250km yesterday but ATL=19" — either the ride
    // isn't on the provider yet, or its TSS was skipped for missing
    // fields). debug! to keep the noise off INFO in normal operation.
    for a in &activities {
        debug!(
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
    let mut snapshot = if activities.is_empty() {
        empty_snapshot(user_id, display_name, now)
    } else {
        build_snapshot_from_activities(
            user_id,
            display_name,
            &activities,
            now,
            &runtime.cageux_config().algorithms,
        )
    };

    // Surface any provider whose connection died non-recoverably so the group coach can
    // name it ("Phil's WHOOP needs reconnecting") instead of treating the dead source as
    // merely quiet. Drawn from the same cross-tenant connection set so a dead provider is
    // named regardless of which tenant it lives in (a tenant-scoped re-query here was the
    // bug: it read nothing when the member's connection lived in a non-host tenant).
    snapshot.needs_reauth_providers = connections
        .iter()
        .filter(|c| c.status.requires_reauth())
        .map(|c| c.provider.clone())
        .collect();
    snapshot.served_stale = served_stale;
    snapshot
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
    let overtraining_risk = assess_overtraining_risk(ctl, tsb);
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
        // Populated by the caller (fetch_single_member_snapshot), which holds the repos
        // needed to read connection status. Activity data alone can't tell needs_reauth.
        needs_reauth_providers: Vec::new(),
        // Also populated by the caller — freshness is a property of how the
        // activities were fetched, which this pure computation never sees.
        served_stale: false,
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
            elevation_gain_m: a.elevation_gain(),
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
        needs_reauth_providers: Vec::new(),
        served_stale: false,
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

/// Assess overtraining risk from the athlete's form band:
/// [`FormBand::DeepFatigue`] → High, [`FormBand::HeavyBlock`] → Moderate,
/// otherwise Low.
///
/// The ATL/CTL ratio is deliberately absent: because `tsb == ctl - atl`,
/// `DeepFatigue` (form below -30% of CTL) *is* `atl > 1.3 * ctl`, so a ratio
/// test alongside the band is the same inequality counted twice — it read as a
/// second corroborating signal while adding no information. One axis, stated
/// once.
///
/// An athlete with no chronic base bands as [`FormBand::InsufficientHistory`]
/// and is reported Low: at a near-zero CTL a single session swings the ratio
/// wildly, so there is no honest risk claim to make, and inventing one is how
/// beginners collected critical flags for an ordinary hard week.
fn assess_overtraining_risk(ctl: Option<f64>, tsb: Option<f64>) -> OvertrainingRiskLevel {
    match tsb
        .zip(ctl)
        .map_or(FormBand::InsufficientHistory, |(t, c)| {
            FormBand::from_tsb(t, c)
        }) {
        FormBand::DeepFatigue => OvertrainingRiskLevel::High,
        FormBand::HeavyBlock => OvertrainingRiskLevel::Moderate,
        _ => OvertrainingRiskLevel::Low,
    }
}
