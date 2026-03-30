// ABOUTME: Builds MemberFitnessSnapshot from connected fitness providers for group members
// ABOUTME: Fetches activities per member, computes CTL/ATL/TSB via TrainingLoadCalculator
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_core::models::groups::{GroupMember, MemberFitnessSnapshot, OvertrainingRiskLevel};
use pierre_groups::service::{HIGH_FATIGUE_TSB_THRESHOLD, OVERREACHING_TSB_THRESHOLD};
use pierre_intelligence::TrainingLoadCalculator;
use pierre_providers::core::ActivityQueryParams;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerResources;
use crate::models::Activity;
use crate::protocols::universal::auth_service::AuthService;

/// Maximum number of activities to fetch per member for snapshot computation
const SNAPSHOT_ACTIVITY_LIMIT: usize = 60;

/// Days of activity history to fetch (covers current + previous week for trend)
const HISTORY_DAYS: i64 = 14;

/// Boundary between current week and previous week
const CURRENT_WEEK_DAYS: i64 = 7;

/// Build fitness snapshots for all active group members.
///
/// For each member with a connected fitness provider, fetches recent activities
/// and computes training load metrics (CTL, ATL, TSB). Members without a
/// connected provider receive a default snapshot with zero metrics.
pub async fn build_member_snapshots(
    resources: &Arc<ServerResources>,
    members: &[GroupMember],
    tenant_id: &str,
) -> Vec<MemberFitnessSnapshot> {
    let auth_service = AuthService::new(Arc::clone(resources));
    let now = Utc::now();
    let mut snapshots = Vec::with_capacity(members.len());

    for member in members {
        let name = member.display_name.as_deref().unwrap_or("Unknown");
        let snapshot = build_single_snapshot(
            &auth_service,
            resources,
            member.user_id,
            name,
            tenant_id,
            now,
        )
        .await;
        snapshots.push(snapshot);
    }

    snapshots
}

/// Build a snapshot for a single member by querying their fitness provider.
async fn build_single_snapshot(
    auth_service: &AuthService,
    resources: &Arc<ServerResources>,
    user_id: Uuid,
    display_name: &str,
    tenant_id: &str,
    now: chrono::DateTime<Utc>,
) -> MemberFitnessSnapshot {
    let activities =
        match fetch_member_activities(auth_service, resources, user_id, tenant_id, now).await {
            Some(acts) if !acts.is_empty() => acts,
            _ => return default_snapshot(user_id, display_name, now),
        };

    build_snapshot_from_activities(user_id, display_name, &activities, now)
}

/// Fetch recent activities for a member from their connected provider.
///
/// Returns `None` if the member has no connected provider or if fetching fails.
async fn fetch_member_activities(
    auth_service: &AuthService,
    resources: &Arc<ServerResources>,
    user_id: Uuid,
    tenant_id: &str,
    now: chrono::DateTime<Utc>,
) -> Option<Vec<Activity>> {
    let connections = resources
        .repos
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap_or_default();

    let connection = connections.first()?;

    let provider = auth_service
        .create_authenticated_provider(&connection.provider, user_id, Some(tenant_id))
        .await
        .map_err(|e| {
            warn!(
                user_id = %user_id,
                provider = %connection.provider,
                error = ?e.error,
                "Failed to create authenticated provider for snapshot"
            );
        })
        .ok()?;

    let after_timestamp = (now - Duration::days(HISTORY_DAYS)).timestamp();
    let params = ActivityQueryParams {
        limit: Some(SNAPSHOT_ACTIVITY_LIMIT),
        offset: None,
        before: None,
        after: Some(after_timestamp),
    };

    provider
        .get_activities_with_params(&params)
        .await
        .map_err(|e| {
            warn!(
                user_id = %user_id,
                provider = %connection.provider,
                error = %e,
                "Failed to fetch activities for snapshot"
            );
        })
        .ok()
}

/// Compute a snapshot from fetched activities without any I/O.
fn build_snapshot_from_activities(
    user_id: Uuid,
    display_name: &str,
    activities: &[Activity],
    now: chrono::DateTime<Utc>,
) -> MemberFitnessSnapshot {
    // Compute training load (CTL/ATL/TSB)
    let calculator = TrainingLoadCalculator::new();
    let training_load = calculator.calculate_training_load(
        activities, None, // FTP
        None, // LTHR
        None, // max_hr
        None, // resting_hr
        None, // weight_kg
    );

    let (ctl, atl, tsb) = match &training_load {
        Ok(tl) => (Some(tl.ctl), Some(tl.atl), Some(tl.tsb)),
        Err(e) => {
            debug!(user_id = %user_id, error = %e, "Training load calculation failed");
            (None, None, None)
        }
    };

    // Split activities into current week and previous week for trend
    let current_week_cutoff = now - Duration::days(CURRENT_WEEK_DAYS);
    let mut current_week = Vec::new();
    let mut previous_week = Vec::new();
    for a in activities {
        if a.start_date() >= current_week_cutoff {
            current_week.push(a);
        } else {
            previous_week.push(a);
        }
    }

    // Compute weekly volumes
    let weekly_volume_km = sum_distance_km(&current_week);
    let previous_week_volume_km = if previous_week.is_empty() {
        None
    } else {
        Some(sum_distance_km(&previous_week))
    };

    // Days since last activity
    let days_since_last = activities
        .iter()
        .map(Activity::start_date)
        .max()
        .map(|latest| {
            #[allow(clippy::cast_possible_truncation)]
            let days = (now - latest).num_days() as i32;
            days
        });

    // Primary sport
    let primary_sport = detect_primary_sport(activities);

    // Overtraining risk based on TSB thresholds
    let overtraining_risk = tsb.map_or(OvertrainingRiskLevel::Low, |t| {
        if t < HIGH_FATIGUE_TSB_THRESHOLD {
            OvertrainingRiskLevel::High
        } else if t < OVERREACHING_TSB_THRESHOLD {
            OvertrainingRiskLevel::Moderate
        } else {
            OvertrainingRiskLevel::Low
        }
    });

    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let weekly_activity_count = current_week.len() as i32;

    MemberFitnessSnapshot {
        user_id,
        display_name: display_name.to_owned(),
        ctl,
        atl,
        tsb,
        weekly_volume_km,
        previous_week_volume_km,
        weekly_activity_count,
        primary_sport,
        vdot: None,
        overtraining_risk,
        days_since_last_activity: days_since_last,
        computed_at: now,
    }
}

/// Create a default snapshot for members without connected providers
fn default_snapshot(
    user_id: Uuid,
    display_name: &str,
    now: chrono::DateTime<Utc>,
) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id,
        display_name: display_name.to_owned(),
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
        computed_at: now,
    }
}

/// Sum distance across activities, converting meters to kilometers
fn sum_distance_km(activities: &[&Activity]) -> f64 {
    activities
        .iter()
        .filter_map(|a| a.distance_meters())
        .sum::<f64>()
        / 1000.0
}

/// Detect the most common sport type from activities.
///
/// Uses serde JSON serialization to get the canonical `snake_case` name
/// (e.g. "run", "ride") since `SportType` does not implement `Display`.
fn detect_primary_sport(activities: &[Activity]) -> Option<String> {
    if activities.is_empty() {
        return None;
    }
    let mut counts = HashMap::new();
    for a in activities {
        let label = serde_json::to_value(a.sport_type())
            .ok()
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{:?}", a.sport_type()));
        *counts.entry(label).or_insert(0u32) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(sport, _)| sport)
}
