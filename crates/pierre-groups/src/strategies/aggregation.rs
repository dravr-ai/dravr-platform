// ABOUTME: Group statistics computation with caching strategies
// ABOUTME: Live and cached approaches for group aggregate stats
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::time::Duration;

use async_trait::async_trait;
use pierre_core::models::groups::{
    GroupAggregateStats, GroupTrend, MemberFitnessSnapshot, OvertrainingRiskLevel,
};

use crate::service::{TREND_DECLINING_FRACTION, TREND_IMPROVING_FRACTION};

/// Strategy for computing and serving group-level statistics.
///
/// Implementations trade off freshness vs latency.
#[async_trait]
pub trait GroupAggregationStrategy: Send + Sync {
    /// Compute aggregate stats from member snapshots
    fn aggregate_stats(&self, snapshots: &[MemberFitnessSnapshot]) -> GroupAggregateStats;

    /// Maximum acceptable data staleness for this strategy
    fn staleness_tolerance(&self) -> Duration;
}

/// Compute fresh on every request. Accurate but slower for large groups.
#[derive(Debug)]
pub struct LiveAggregation;

impl LiveAggregation {
    /// Derive the group-level weekly trend from per-member volume changes.
    ///
    /// Compares average current-week volume to average prior-week volume,
    /// over the members both weeks are read for.
    /// Returns `Improving` when the increase exceeds [`TREND_IMPROVING_FRACTION`],
    /// `Declining` when the decrease exceeds [`TREND_DECLINING_FRACTION`],
    /// and `Stable` otherwise.
    fn compute_weekly_trend(snapshots: &[MemberFitnessSnapshot]) -> GroupTrend {
        let pairs: Vec<(f64, f64)> = snapshots
            .iter()
            .filter_map(|s| s.weekly_volume_km.zip(s.previous_week_volume_km))
            .collect();

        if pairs.is_empty() {
            return GroupTrend::Stable;
        }

        let current_avg = pairs.iter().map(|(c, _)| c).sum::<f64>() / pairs.len() as f64;
        let prev_avg = pairs.iter().map(|(_, p)| p).sum::<f64>() / pairs.len() as f64;

        if prev_avg <= 0.0 {
            return if current_avg > 0.0 {
                GroupTrend::Improving
            } else {
                GroupTrend::Stable
            };
        }

        let change_fraction = (current_avg - prev_avg) / prev_avg;

        if change_fraction > TREND_IMPROVING_FRACTION {
            GroupTrend::Improving
        } else if change_fraction < -TREND_DECLINING_FRACTION {
            GroupTrend::Declining
        } else {
            GroupTrend::Stable
        }
    }

    /// The mean of this week's kilometres over the members whose week is
    /// read, or `None` when no member's week is read.
    ///
    /// A member whose stored history does not reach the week has no volume:
    /// counted as a zero it would pull the average down, and a group where
    /// nobody's week is read has no average at all, which is not a zero.
    fn mean_weekly_volume_km(snapshots: &[MemberFitnessSnapshot]) -> Option<f64> {
        let volumes: Vec<f64> = snapshots
            .iter()
            .filter_map(|s| s.weekly_volume_km)
            .collect();
        if volumes.is_empty() {
            None
        } else {
            Some(volumes.iter().sum::<f64>() / volumes.len() as f64)
        }
    }

    fn compute_stats(snapshots: &[MemberFitnessSnapshot]) -> GroupAggregateStats {
        let total = i64::try_from(snapshots.len()).unwrap_or_default();
        let active = i64::try_from(
            snapshots
                .iter()
                .filter(|s| s.days_since_last_activity.is_none_or(|d| d <= 7))
                .count(),
        )
        .unwrap_or_default();

        let avg_volume = Self::mean_weekly_volume_km(snapshots);

        let ctl_values: Vec<f64> = snapshots.iter().filter_map(|s| s.ctl).collect();
        let avg_ctl = if ctl_values.is_empty() {
            None
        } else {
            Some(ctl_values.iter().sum::<f64>() / ctl_values.len() as f64)
        };

        let flagged = i64::try_from(
            snapshots
                .iter()
                .filter(|s| matches!(s.overtraining_risk, OvertrainingRiskLevel::High))
                .count(),
        )
        .unwrap_or_default();

        let weekly_trend = Self::compute_weekly_trend(snapshots);

        GroupAggregateStats {
            total_members: total,
            active_members: active,
            avg_weekly_volume_km: avg_volume,
            avg_ctl,
            flagged_members: flagged,
            weekly_trend,
        }
    }
}

#[async_trait]
impl GroupAggregationStrategy for LiveAggregation {
    fn aggregate_stats(&self, snapshots: &[MemberFitnessSnapshot]) -> GroupAggregateStats {
        Self::compute_stats(snapshots)
    }

    fn staleness_tolerance(&self) -> Duration {
        Duration::from_secs(0)
    }
}

/// Serve from cache with configurable TTL. Fast but potentially stale.
#[derive(Debug)]
pub struct CachedAggregation {
    ttl: Duration,
}

impl CachedAggregation {
    /// Create with 1-hour TTL (default for Professional tier)
    #[must_use]
    pub fn new() -> Self {
        Self {
            ttl: Duration::from_hours(1),
        }
    }

    /// Create with custom TTL
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self { ttl }
    }
}

impl Default for CachedAggregation {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GroupAggregationStrategy for CachedAggregation {
    fn aggregate_stats(&self, snapshots: &[MemberFitnessSnapshot]) -> GroupAggregateStats {
        // Cache-aside: compute fresh, caller is responsible for caching the result
        LiveAggregation::compute_stats(snapshots)
    }

    fn staleness_tolerance(&self) -> Duration {
        self.ttl
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use dravr_tronc::dates::Timestamp;
    use uuid::Uuid;

    use super::*;

    /// A member whose kilometres this week are `km`, `None` for a week their
    /// stored history does not reach.
    fn member(km: Option<f64>) -> MemberFitnessSnapshot {
        MemberFitnessSnapshot {
            user_id: Uuid::new_v4(),
            display_name: "Ada".to_owned(),
            ctl: None,
            atl: None,
            tsb: None,
            form_ctl: None,
            weekly_volume_km: km,
            previous_week_volume_km: None,
            weekly_activity_count: km.map(|_| 2),
            weekly_duration_seconds: km.map(|_| 5_400),
            primary_sport: None,
            vdot: None,
            overtraining_risk: OvertrainingRiskLevel::Low,
            days_since_last_activity: Some(1),
            last_activity_per_provider: HashMap::new(),
            recent_activities: Vec::new(),
            needs_reauth_providers: Vec::new(),
            served_stale: false,
            timezone: None,
            deep_fatigue_mornings: Vec::new(),
            computed_at: Timestamp::now(),
        }
    }

    #[test]
    fn a_group_where_no_week_is_read_has_no_average() {
        assert_eq!(
            LiveAggregation::compute_stats(&[]).avg_weekly_volume_km,
            None,
            "bug: a group without members was given an average"
        );
        let stats = LiveAggregation::compute_stats(&[member(None), member(None)]);
        assert_eq!(
            stats.avg_weekly_volume_km, None,
            "bug: two unread weeks averaged to a figure"
        );
        assert_eq!(
            stats.total_members, 2,
            "bug: a member whose week is unread is still a member"
        );
    }

    #[test]
    fn the_average_is_the_mean_of_the_weeks_that_are_read() {
        let stats = LiveAggregation::compute_stats(&[
            member(Some(40.0)),
            member(None),
            member(Some(20.0)),
            member(None),
        ]);
        let average = stats.avg_weekly_volume_km;
        assert!(
            average.is_some_and(|km| (km - 30.0).abs() < 1e-9),
            "bug: the mean is not over the two read weeks alone: {average:?}"
        );
    }

    #[test]
    fn a_read_week_without_distance_averages_to_its_own_zero() {
        let stats = LiveAggregation::compute_stats(&[member(Some(0.0))]);
        assert_eq!(
            stats.avg_weekly_volume_km,
            Some(0.0),
            "bug: a read quiet week lost its zero"
        );
    }
}
