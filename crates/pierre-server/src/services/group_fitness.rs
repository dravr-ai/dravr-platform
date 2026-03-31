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

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use futures_util::future::join_all;
use pierre_core::models::groups::{MemberFitnessSnapshot, OvertrainingRiskLevel};
use pierre_core::models::{Activity, TenantId};
use pierre_providers::core::ActivityQueryParams;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::intelligence::TrainingLoadCalculator;
use crate::mcp::resources::ServerResources;
use crate::protocols::universal::AuthService;

/// Number of days of activity history to fetch for training load calculation.
/// CTL uses a 42-day exponential moving average, so 60 days gives adequate data.
const TRAINING_LOAD_LOOKBACK_DAYS: i64 = 60;

/// Maximum activities to fetch per user for snapshot computation
const MAX_ACTIVITIES_PER_USER: usize = 200;

/// Fetch fitness snapshots for a batch of group members in parallel.
///
/// For each user, creates an authenticated provider, fetches recent activities,
/// and computes training load metrics. Users without connected providers or
/// activity data receive snapshots with `None` metric fields.
///
/// Errors are handled per-user: a failed provider connection or activity fetch
/// does not prevent other members' snapshots from being returned.
pub async fn fetch_member_snapshots(
    resources: &Arc<ServerResources>,
    user_ids: &[Uuid],
    tenant_id: TenantId,
) -> Vec<MemberFitnessSnapshot> {
    if user_ids.is_empty() {
        return Vec::new();
    }

    let futures: Vec<_> = user_ids
        .iter()
        .map(|&user_id| {
            let resources = Arc::clone(resources);
            let tid = tenant_id;
            async move { fetch_single_member_snapshot(&resources, user_id, tid).await }
        })
        .collect();

    join_all(futures).await
}

/// Fetch display name for a user from the global user database.
///
/// Returns the user's display name if set, email prefix if not, or "Unknown"
/// if the user cannot be fetched.
async fn fetch_user_display_name(resources: &Arc<ServerResources>, user_id: Uuid) -> String {
    match resources.repos.users.get_global(user_id).await {
        Ok(Some(user)) => user
            .display_name
            .unwrap_or_else(|| user.email.split('@').next().unwrap_or("Unknown").to_owned()),
        Ok(None) => "Unknown".to_owned(),
        Err(e) => {
            debug!(user_id = %user_id, error = %e, "Failed to fetch user for snapshot");
            "Unknown".to_owned()
        }
    }
}

/// Compute training load metrics from a list of activities.
///
/// Uses `TrainingLoadCalculator` to compute CTL, ATL, and TSB.
/// Returns `(None, None, None)` if calculation fails.
fn compute_training_metrics(activities: &[Activity]) -> (Option<f64>, Option<f64>, Option<f64>) {
    let calculator = TrainingLoadCalculator::new();
    match calculator.calculate_training_load(
        activities, None, // FTP
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

/// Compute weekly metrics from activities in the past 7 days.
///
/// Returns `(activity_count, volume_km, days_since_last)`.
fn compute_weekly_metrics(activities: &[Activity], now: DateTime<Utc>) -> (i32, f64, Option<i32>) {
    let seven_days_ago = now - Duration::days(7);
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

    let days_since_last = activities
        .iter()
        .map(Activity::start_date)
        .max()
        .map(|last| i32::try_from((now - last).num_days()).unwrap_or(i32::MAX));

    (weekly_activity_count, weekly_volume_km, days_since_last)
}

/// Get the first connected provider name for a user.
///
/// Returns the provider name, or None if no provider is connected.
async fn get_first_connected_provider(
    resources: &Arc<ServerResources>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<String> {
    let conns = resources
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .ok()?;

    conns.first().map(|c| c.provider.clone())
}

/// Fetch activities from an authenticated provider.
///
/// Returns activities, or None if provider creation or fetch fails.
async fn fetch_activities_from_provider(
    auth_service: &AuthService,
    provider_name: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<Vec<Activity>> {
    let tenant_id_str = tenant_id.to_string();
    let provider = auth_service
        .create_authenticated_provider(provider_name, user_id, Some(&tenant_id_str))
        .await
        .ok()?;

    // Fetch recent activities for training load calculation
    let now = Utc::now();
    let lookback_start = (now - Duration::days(TRAINING_LOAD_LOOKBACK_DAYS)).timestamp();
    let params = ActivityQueryParams {
        limit: Some(MAX_ACTIVITIES_PER_USER),
        offset: None,
        before: None,
        after: Some(lookback_start),
    };

    provider.get_activities_with_params(&params).await.ok()
}

/// Fetch a single member's fitness snapshot.
///
/// Attempts to find a connected provider for the user and compute training
/// load from their recent activities. Returns a snapshot with `None` metrics
/// if no provider is connected or if the fetch fails.
async fn fetch_single_member_snapshot(
    resources: &Arc<ServerResources>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> MemberFitnessSnapshot {
    let now = Utc::now();

    // Get display name from user profile
    let display_name = fetch_user_display_name(resources, user_id).await;

    // Get first connected provider name
    let Some(provider_name) = get_first_connected_provider(resources, user_id, tenant_id).await
    else {
        return empty_snapshot(user_id, display_name, now);
    };

    // Fetch activities from the provider
    let auth_service = AuthService::new(Arc::clone(resources));
    let Some(activities) =
        fetch_activities_from_provider(&auth_service, &provider_name, user_id, tenant_id).await
    else {
        warn!(
            user_id = %user_id,
            provider = %provider_name,
            "Failed to fetch activities for snapshot"
        );
        return empty_snapshot(user_id, display_name, now);
    };

    if activities.is_empty() {
        return empty_snapshot(user_id, display_name, now);
    }

    // Compute training load (CTL, ATL, TSB)
    let (ctl, atl, tsb) = compute_training_metrics(&activities);

    // Compute weekly volume and activity count (last 7 days)
    let (weekly_activity_count, weekly_volume_km, days_since_last) =
        compute_weekly_metrics(&activities, now);

    // Determine primary sport from most common activity type
    let primary_sport = determine_primary_sport(&activities);

    // Assess overtraining risk from TSB and ATL/CTL ratio
    let overtraining_risk = assess_overtraining_risk(ctl, atl, tsb);

    MemberFitnessSnapshot {
        user_id,
        display_name,
        ctl,
        atl,
        tsb,
        weekly_volume_km,
        previous_week_volume_km: None,
        weekly_activity_count,
        primary_sport,
        vdot: None,
        overtraining_risk,
        days_since_last_activity: days_since_last,
        computed_at: now,
    }
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
        primary_sport: None,
        vdot: None,
        overtraining_risk: OvertrainingRiskLevel::Low,
        days_since_last_activity: None,
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
