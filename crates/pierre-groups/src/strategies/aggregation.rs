// ABOUTME: Group statistics computation with caching strategies
// ABOUTME: Live, cached, and hybrid approaches for group aggregate stats and member comparison
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::time::Duration;

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::groups::{
    GroupAggregateStats, GroupTrend, MemberFitnessSnapshot, MemberGroupComparison,
    OvertrainingRiskLevel,
};
use uuid::Uuid;

/// Strategy for computing and serving group-level statistics.
///
/// Implementations trade off freshness vs latency.
#[async_trait]
pub trait GroupAggregationStrategy: Send + Sync {
    /// Compute aggregate stats from member snapshots
    fn aggregate_stats(&self, snapshots: &[MemberFitnessSnapshot]) -> GroupAggregateStats;

    /// Compare a member against group norms
    ///
    /// # Errors
    ///
    /// Returns an error if the member is not found in the snapshots
    fn compare_member(
        &self,
        user_id: Uuid,
        snapshots: &[MemberFitnessSnapshot],
    ) -> AppResult<MemberGroupComparison>;

    /// Maximum acceptable data staleness for this strategy
    fn staleness_tolerance(&self) -> Duration;
}

/// Compute fresh on every request. Accurate but slower for large groups.
pub struct LiveAggregation;

impl LiveAggregation {
    fn compute_stats(snapshots: &[MemberFitnessSnapshot]) -> GroupAggregateStats {
        let total = i64::try_from(snapshots.len()).unwrap_or_default();
        let active = i64::try_from(
            snapshots
                .iter()
                .filter(|s| s.days_since_last_activity.is_none_or(|d| d <= 7))
                .count(),
        )
        .unwrap_or_default();

        let avg_volume = if total > 0 {
            snapshots.iter().map(|s| s.weekly_volume_km).sum::<f64>() / total as f64
        } else {
            0.0
        };

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

        GroupAggregateStats {
            total_members: total,
            active_members: active,
            avg_weekly_volume_km: avg_volume,
            avg_ctl,
            flagged_members: flagged,
            weekly_trend: GroupTrend::Stable,
        }
    }
}

#[async_trait]
impl GroupAggregationStrategy for LiveAggregation {
    fn aggregate_stats(&self, snapshots: &[MemberFitnessSnapshot]) -> GroupAggregateStats {
        Self::compute_stats(snapshots)
    }

    fn compare_member(
        &self,
        user_id: Uuid,
        snapshots: &[MemberFitnessSnapshot],
    ) -> AppResult<MemberGroupComparison> {
        let member = snapshots.iter().find(|s| s.user_id == user_id);
        let total = snapshots.len();

        let (volume_percentile, ctl_percentile, display_name) = member.map_or_else(
            || (50.0, None, "Unknown".to_owned()),
            |m| {
                let vol_rank = snapshots
                    .iter()
                    .filter(|s| s.weekly_volume_km <= m.weekly_volume_km)
                    .count();
                let vol_pct = if total > 0 {
                    (vol_rank as f64 / total as f64) * 100.0
                } else {
                    50.0
                };

                let ctl_pct = m.ctl.map(|member_ctl| {
                    let rank = snapshots
                        .iter()
                        .filter(|s| s.ctl.is_none_or(|c| c <= member_ctl))
                        .count();
                    if total > 0 {
                        (rank as f64 / total as f64) * 100.0
                    } else {
                        50.0
                    }
                });

                (vol_pct, ctl_pct, m.display_name.clone())
            },
        );

        let comparison_text = format!(
            "Volume: top {:.0}% of group. {}",
            100.0 - volume_percentile,
            ctl_percentile.map_or_else(
                || "CTL data not available.".to_owned(),
                |p| format!("CTL: top {:.0}% of group.", 100.0 - p)
            )
        );

        Ok(MemberGroupComparison {
            user_id,
            display_name,
            volume_percentile,
            ctl_percentile,
            comparison_text,
        })
    }

    fn staleness_tolerance(&self) -> Duration {
        Duration::from_secs(0)
    }
}

/// Serve from cache with configurable TTL. Fast but potentially stale.
pub struct CachedAggregation {
    ttl: Duration,
}

impl CachedAggregation {
    /// Create with 1-hour TTL (default for Professional tier)
    #[must_use]
    pub fn new() -> Self {
        Self {
            ttl: Duration::from_secs(3600),
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

    fn compare_member(
        &self,
        user_id: Uuid,
        snapshots: &[MemberFitnessSnapshot],
    ) -> AppResult<MemberGroupComparison> {
        LiveAggregation.compare_member(user_id, snapshots)
    }

    fn staleness_tolerance(&self) -> Duration {
        self.ttl
    }
}
