// ABOUTME: Tests for group analytics: stats aggregation, health flags, and weekly reports
// ABOUTME: Validates threshold-based health detection and weekly trend computation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_core::models::groups::{
    GroupTrend, HealthFlagSeverity, MemberFitnessSnapshot, MemberFlag, OvertrainingRiskLevel,
};
use pierre_groups::service::{
    GroupService, HIGH_FATIGUE_TSB_THRESHOLD, INACTIVITY_DAYS_THRESHOLD,
    OVERREACHING_TSB_THRESHOLD, TREND_DECLINING_FRACTION, TREND_IMPROVING_FRACTION,
    VOLUME_DROP_FRACTION_THRESHOLD,
};
use pierre_groups::strategies::aggregation::{GroupAggregationStrategy, LiveAggregation};
use pierre_groups::strategies::context::{
    GroupContextStrategy, GroupOverviewContext, IndividualFocusContext,
};
use pierre_groups::strategies::summarization::{AdaptiveSummarizer, GroupSummarizationStrategy};
use pierre_groups::strategies::tier::{GroupFeatureFlags, GroupTierStrategy};
use std::sync::Arc;
use uuid::Uuid;

// ============================================================================
// Test Helpers
// ============================================================================

/// Build a snapshot with configurable fields for testing
fn make_snapshot(
    name: &str,
    tsb: Option<f64>,
    weekly_volume_km: f64,
    previous_week_volume_km: Option<f64>,
    days_since_last: Option<i32>,
    overtraining_risk: OvertrainingRiskLevel,
) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id: Uuid::new_v4(),
        display_name: name.to_owned(),
        ctl: tsb.map(|t| 60.0 + t),
        atl: tsb.map(|_| 60.0),
        tsb,
        weekly_volume_km,
        previous_week_volume_km,
        weekly_activity_count: if weekly_volume_km > 0.0 { 5 } else { 0 },
        primary_sport: Some("Run".to_owned()),
        vdot: None,
        overtraining_risk,
        days_since_last_activity: days_since_last,
        computed_at: Utc::now(),
    }
}

/// Build a minimal healthy snapshot
fn healthy_snapshot(name: &str, weekly_km: f64) -> MemberFitnessSnapshot {
    make_snapshot(
        name,
        Some(5.0),
        weekly_km,
        Some(weekly_km),
        Some(1),
        OvertrainingRiskLevel::Low,
    )
}

/// Create a GroupService with live aggregation strategy for testing
fn test_group_service() -> GroupService {
    // Use a mock repo (we only test the compute_* methods which don't need the repo)
    let repo = Arc::new(MockGroupRepo);
    let tier = Arc::new(TestTier);
    GroupService::new(repo, tier)
}

/// Minimal mock repo that panics if called (we only test pure compute methods)
struct MockGroupRepo;

#[async_trait::async_trait]
impl pierre_database::repositories::CoachingGroupRepository for MockGroupRepo {
    async fn create_group(
        &self,
        _: pierre_core::models::TenantId,
        _: &pierre_core::models::groups::CoachingGroup,
    ) -> pierre_core::errors::AppResult<pierre_core::models::groups::CoachingGroup> {
        unimplemented!("mock")
    }
    async fn get_group(
        &self,
        _: &str,
        _: pierre_core::models::TenantId,
    ) -> pierre_core::errors::AppResult<Option<pierre_core::models::groups::CoachingGroup>> {
        unimplemented!("mock")
    }
    async fn list_groups_for_user(
        &self,
        _: Uuid,
    ) -> pierre_core::errors::AppResult<Vec<pierre_core::models::groups::GroupSummary>> {
        unimplemented!("mock")
    }
    async fn list_groups_for_coach(
        &self,
        _: &str,
        _: pierre_core::models::TenantId,
    ) -> pierre_core::errors::AppResult<Vec<pierre_core::models::groups::CoachingGroup>> {
        unimplemented!("mock")
    }
    async fn update_group(
        &self,
        _: &str,
        _: pierre_core::models::TenantId,
        _: &pierre_core::models::groups::UpdateGroupRequest,
    ) -> pierre_core::errors::AppResult<Option<pierre_core::models::groups::CoachingGroup>> {
        unimplemented!("mock")
    }
    async fn delete_group(
        &self,
        _: &str,
        _: pierre_core::models::TenantId,
    ) -> pierre_core::errors::AppResult<bool> {
        unimplemented!("mock")
    }
    async fn add_member(
        &self,
        _: &pierre_core::models::groups::GroupMember,
    ) -> pierre_core::errors::AppResult<pierre_core::models::groups::GroupMember> {
        unimplemented!("mock")
    }
    async fn remove_member(&self, _: &str, _: Uuid) -> pierre_core::errors::AppResult<bool> {
        unimplemented!("mock")
    }
    async fn get_member(
        &self,
        _: &str,
        _: Uuid,
    ) -> pierre_core::errors::AppResult<Option<pierre_core::models::groups::GroupMember>> {
        unimplemented!("mock")
    }
    async fn list_members(
        &self,
        _: &str,
    ) -> pierre_core::errors::AppResult<Vec<pierre_core::models::groups::GroupMember>> {
        unimplemented!("mock")
    }
    async fn update_member_role(
        &self,
        _: &str,
        _: Uuid,
        _: pierre_core::models::groups::GroupRole,
    ) -> pierre_core::errors::AppResult<bool> {
        unimplemented!("mock")
    }
    async fn update_peer_sharing_consent(
        &self,
        _: &str,
        _: Uuid,
        _: bool,
    ) -> pierre_core::errors::AppResult<bool> {
        unimplemented!("mock")
    }
    async fn count_members(&self, _: &str) -> pierre_core::errors::AppResult<i64> {
        unimplemented!("mock")
    }
    async fn create_invite(
        &self,
        _: &pierre_core::models::groups::GroupInvite,
    ) -> pierre_core::errors::AppResult<pierre_core::models::groups::GroupInvite> {
        unimplemented!("mock")
    }
    async fn get_invite_by_code(
        &self,
        _: &str,
    ) -> pierre_core::errors::AppResult<Option<pierre_core::models::groups::GroupInvite>> {
        unimplemented!("mock")
    }
    async fn increment_invite_use_count(&self, _: &str) -> pierre_core::errors::AppResult<bool> {
        unimplemented!("mock")
    }
    async fn deactivate_invite(&self, _: &str) -> pierre_core::errors::AppResult<bool> {
        unimplemented!("mock")
    }
    async fn list_invites(
        &self,
        _: &str,
    ) -> pierre_core::errors::AppResult<Vec<pierre_core::models::groups::GroupInvite>> {
        unimplemented!("mock")
    }
    async fn find_groups_for_user_and_coach(
        &self,
        _: Uuid,
        _: &str,
    ) -> pierre_core::errors::AppResult<Vec<pierre_core::models::groups::CoachingGroup>> {
        unimplemented!("mock")
    }
    async fn count_groups_for_owner(
        &self,
        _: Uuid,
        _: pierre_core::models::TenantId,
    ) -> pierre_core::errors::AppResult<i64> {
        unimplemented!("mock")
    }
}

/// Test tier strategy that uses LiveAggregation
struct TestTier;

impl GroupTierStrategy for TestTier {
    fn max_groups(&self) -> Option<usize> {
        None
    }
    fn max_members_per_group(&self) -> usize {
        50
    }
    fn allowed_features(&self) -> GroupFeatureFlags {
        GroupFeatureFlags {
            basic_roster: true,
            stats_dashboard: true,
            stats_charts: true,
            peer_sharing: true,
            weekly_digest: true,
            messaging_management: true,
            advanced_invites: true,
        }
    }
    fn aggregation_strategy(&self) -> Arc<dyn GroupAggregationStrategy> {
        Arc::new(LiveAggregation)
    }
    fn summarization_strategy(&self) -> Arc<dyn GroupSummarizationStrategy> {
        Arc::new(AdaptiveSummarizer::new())
    }
    fn admin_context_strategy(&self) -> Arc<dyn GroupContextStrategy> {
        Arc::new(GroupOverviewContext)
    }
    fn member_context_strategy(&self) -> Arc<dyn GroupContextStrategy> {
        Arc::new(IndividualFocusContext)
    }
}

// ============================================================================
// Health Flag Tests
// ============================================================================

#[test]
fn test_health_flags_tsb_overreaching_threshold() {
    let service = test_group_service();

    // TSB just below -20 should trigger overreaching
    let snapshots = vec![make_snapshot(
        "Alice",
        Some(OVERREACHING_TSB_THRESHOLD - 0.1),
        40.0,
        Some(40.0),
        Some(1),
        OvertrainingRiskLevel::Moderate,
    )];

    let flags = service.compute_health_flags(&snapshots);
    assert_eq!(flags.len(), 1, "TSB below -20 should produce one flag");
    assert_eq!(flags[0].flag_type, MemberFlag::Overreaching);
    assert_eq!(flags[0].severity, HealthFlagSeverity::Warning);
    assert!(flags[0].detail.contains("recommend recovery"));
}

#[test]
fn test_health_flags_tsb_high_fatigue_threshold() {
    let service = test_group_service();

    // TSB below -30 should trigger critical injury risk
    let snapshots = vec![make_snapshot(
        "Bob",
        Some(HIGH_FATIGUE_TSB_THRESHOLD - 1.0),
        30.0,
        Some(40.0),
        Some(1),
        OvertrainingRiskLevel::High,
    )];

    let flags = service.compute_health_flags(&snapshots);
    let injury_flags: Vec<_> = flags
        .iter()
        .filter(|f| f.flag_type == MemberFlag::InjuryRisk)
        .collect();
    assert_eq!(
        injury_flags.len(),
        1,
        "TSB below -30 should produce an injury risk flag"
    );
    assert_eq!(injury_flags[0].severity, HealthFlagSeverity::Critical);
    assert!(injury_flags[0].detail.contains("high injury risk"));
}

#[test]
fn test_health_flags_tsb_between_thresholds() {
    let service = test_group_service();

    // TSB between -30 and -20 should be overreaching (warning), not injury risk
    let tsb = (OVERREACHING_TSB_THRESHOLD + HIGH_FATIGUE_TSB_THRESHOLD) / 2.0;
    let snapshots = vec![make_snapshot(
        "Charlie",
        Some(tsb),
        40.0,
        Some(40.0),
        Some(1),
        OvertrainingRiskLevel::Moderate,
    )];

    let flags = service.compute_health_flags(&snapshots);
    let overreaching: Vec<_> = flags
        .iter()
        .filter(|f| f.flag_type == MemberFlag::Overreaching)
        .collect();
    let injury: Vec<_> = flags
        .iter()
        .filter(|f| f.flag_type == MemberFlag::InjuryRisk)
        .collect();
    assert_eq!(overreaching.len(), 1, "Should produce overreaching flag");
    assert!(injury.is_empty(), "Should NOT produce injury risk flag");
}

#[test]
fn test_health_flags_inactivity_threshold() {
    let service = test_group_service();

    // Exactly at the threshold should trigger
    let snapshots = vec![make_snapshot(
        "Dana",
        Some(5.0),
        0.0,
        None,
        Some(INACTIVITY_DAYS_THRESHOLD),
        OvertrainingRiskLevel::Low,
    )];

    let flags = service.compute_health_flags(&snapshots);
    let inactive: Vec<_> = flags
        .iter()
        .filter(|f| f.flag_type == MemberFlag::Inactive)
        .collect();
    assert_eq!(inactive.len(), 1, "Should flag inactive at threshold");
    assert!(inactive[0]
        .detail
        .contains(&INACTIVITY_DAYS_THRESHOLD.to_string()));
}

#[test]
fn test_health_flags_inactivity_below_threshold() {
    let service = test_group_service();

    // Just below threshold should NOT trigger
    let snapshots = vec![make_snapshot(
        "Eve",
        Some(5.0),
        20.0,
        Some(20.0),
        Some(INACTIVITY_DAYS_THRESHOLD - 1),
        OvertrainingRiskLevel::Low,
    )];

    let flags = service.compute_health_flags(&snapshots);
    let inactive: Vec<_> = flags
        .iter()
        .filter(|f| f.flag_type == MemberFlag::Inactive)
        .collect();
    assert!(
        inactive.is_empty(),
        "Should NOT flag inactive below threshold"
    );
}

#[test]
fn test_health_flags_volume_drop() {
    let service = test_group_service();

    // Group of 3: two at ~40km, one at ~15km (well below 30% of average)
    let snapshots = vec![
        healthy_snapshot("Alice", 40.0),
        healthy_snapshot("Bob", 42.0),
        make_snapshot(
            "Charlie",
            Some(5.0),
            15.0,
            Some(40.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
    ];

    let flags = service.compute_health_flags(&snapshots);
    // Charlie's volume (15km) is well below the group average (~32km), exceeding the 30% threshold
    let volume_flags: Vec<_> = flags
        .iter()
        .filter(|f| f.flag_type == MemberFlag::VolumeDrop)
        .collect();
    assert!(
        !volume_flags.is_empty(),
        "Should detect volume drop for the low-volume member"
    );
    assert!(volume_flags[0].detail.contains("below group average"));
}

#[test]
fn test_health_flags_empty_group() {
    let service = test_group_service();
    let flags = service.compute_health_flags(&[]);
    assert!(flags.is_empty(), "Empty group should produce no flags");
}

#[test]
fn test_health_flags_no_tsb_data_with_high_risk() {
    let service = test_group_service();

    // Member with no TSB data but high overtraining risk from other signals
    let snapshots = vec![make_snapshot(
        "Frank",
        None,
        30.0,
        Some(30.0),
        Some(1),
        OvertrainingRiskLevel::High,
    )];

    let flags = service.compute_health_flags(&snapshots);
    let overreaching: Vec<_> = flags
        .iter()
        .filter(|f| f.flag_type == MemberFlag::Overreaching)
        .collect();
    assert_eq!(
        overreaching.len(),
        1,
        "Should fall back to risk level when TSB is absent"
    );
}

// ============================================================================
// Stats Aggregation Tests
// ============================================================================

#[test]
fn test_stats_aggregation_basic() {
    let service = test_group_service();

    let snapshots = vec![
        healthy_snapshot("Alice", 50.0),
        healthy_snapshot("Bob", 30.0),
    ];

    let stats = service.compute_aggregate_stats(&snapshots);
    assert_eq!(stats.total_members, 2);
    assert_eq!(stats.active_members, 2);
    assert!((stats.avg_weekly_volume_km - 40.0).abs() < 0.1);
    assert_eq!(stats.flagged_members, 0);
}

#[test]
fn test_stats_aggregation_with_inactive() {
    let service = test_group_service();

    let snapshots = vec![
        healthy_snapshot("Alice", 50.0),
        make_snapshot("Bob", None, 0.0, None, Some(10), OvertrainingRiskLevel::Low),
    ];

    let stats = service.compute_aggregate_stats(&snapshots);
    assert_eq!(stats.total_members, 2);
    // Bob has 10 days since last activity (>7 = inactive for flag, but active_members
    // uses is_none_or(|d| d <= 7))
    assert_eq!(stats.active_members, 1);
}

#[test]
fn test_stats_aggregation_empty() {
    let service = test_group_service();
    let stats = service.compute_aggregate_stats(&[]);
    assert_eq!(stats.total_members, 0);
    assert_eq!(stats.active_members, 0);
    assert_eq!(stats.avg_weekly_volume_km, 0.0);
    assert!(stats.avg_ctl.is_none());
    assert_eq!(stats.flagged_members, 0);
    assert_eq!(stats.weekly_trend, GroupTrend::Stable);
}

// ============================================================================
// Weekly Trend Tests
// ============================================================================

#[test]
fn test_trend_improving() {
    let service = test_group_service();

    // Current week volume significantly higher than previous
    let snapshots = vec![
        make_snapshot(
            "Alice",
            Some(5.0),
            50.0,
            Some(40.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
        make_snapshot(
            "Bob",
            Some(5.0),
            60.0,
            Some(45.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
    ];

    let stats = service.compute_aggregate_stats(&snapshots);
    assert_eq!(
        stats.weekly_trend,
        GroupTrend::Improving,
        "25%+ volume increase should be 'improving'"
    );
}

#[test]
fn test_trend_declining() {
    let service = test_group_service();

    // Current week volume significantly lower than previous
    let snapshots = vec![
        make_snapshot(
            "Alice",
            Some(5.0),
            30.0,
            Some(50.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
        make_snapshot(
            "Bob",
            Some(5.0),
            25.0,
            Some(45.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
    ];

    let stats = service.compute_aggregate_stats(&snapshots);
    assert_eq!(
        stats.weekly_trend,
        GroupTrend::Declining,
        "40%+ volume decrease should be 'declining'"
    );
}

#[test]
fn test_trend_stable() {
    let service = test_group_service();

    // Very similar volumes between weeks
    let snapshots = vec![
        make_snapshot(
            "Alice",
            Some(5.0),
            40.0,
            Some(41.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
        make_snapshot(
            "Bob",
            Some(5.0),
            38.0,
            Some(39.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
    ];

    let stats = service.compute_aggregate_stats(&snapshots);
    assert_eq!(
        stats.weekly_trend,
        GroupTrend::Stable,
        "~2% change should be 'stable'"
    );
}

#[test]
fn test_trend_no_previous_data() {
    let service = test_group_service();

    // No previous week data available
    let snapshots = vec![make_snapshot(
        "Alice",
        Some(5.0),
        40.0,
        None,
        Some(1),
        OvertrainingRiskLevel::Low,
    )];

    let stats = service.compute_aggregate_stats(&snapshots);
    assert_eq!(
        stats.weekly_trend,
        GroupTrend::Stable,
        "Missing previous data should default to 'stable'"
    );
}

#[test]
fn test_trend_boundary_improving() {
    let service = test_group_service();

    // Exactly at the improving threshold boundary
    let prev = 100.0;
    let current = prev * (1.0 + TREND_IMPROVING_FRACTION + 0.001);
    let snapshots = vec![make_snapshot(
        "Alice",
        Some(5.0),
        current,
        Some(prev),
        Some(1),
        OvertrainingRiskLevel::Low,
    )];

    let stats = service.compute_aggregate_stats(&snapshots);
    assert_eq!(stats.weekly_trend, GroupTrend::Improving);
}

#[test]
fn test_trend_boundary_declining() {
    let service = test_group_service();

    // Exactly at the declining threshold boundary
    let prev = 100.0;
    let current = prev * (1.0 - TREND_DECLINING_FRACTION - 0.001);
    let snapshots = vec![make_snapshot(
        "Alice",
        Some(5.0),
        current,
        Some(prev),
        Some(1),
        OvertrainingRiskLevel::Low,
    )];

    let stats = service.compute_aggregate_stats(&snapshots);
    assert_eq!(stats.weekly_trend, GroupTrend::Declining);
}

// ============================================================================
// Weekly Report Tests
// ============================================================================

#[test]
fn test_weekly_report_summary() {
    let service = test_group_service();

    let snapshots = vec![
        healthy_snapshot("Alice", 50.0),
        healthy_snapshot("Bob", 30.0),
    ];

    let report = service.compute_weekly_report(&snapshots, "Team Alpha");
    assert!(report.summary.contains("Team Alpha"));
    assert!(report.summary.contains("2/2 active members"));
    assert!(report.summary.contains("40.0km"));
}

#[test]
fn test_weekly_report_highlights_fresh_form() {
    let service = test_group_service();

    // Alice has positive TSB (fresh form)
    let snapshots = vec![
        make_snapshot(
            "Alice",
            Some(15.0),
            40.0,
            Some(40.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
        make_snapshot(
            "Bob",
            Some(-5.0),
            40.0,
            Some(40.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
    ];

    let report = service.compute_weekly_report(&snapshots, "Team Beta");
    assert!(
        report.highlights.iter().any(|h| h.contains("Alice")),
        "Alice should appear in highlights"
    );
    assert!(
        !report.highlights.iter().any(|h| h.contains("Bob")),
        "Bob should NOT appear in highlights (negative TSB)"
    );
}

#[test]
fn test_weekly_report_concerns_from_flags() {
    let service = test_group_service();

    let snapshots = vec![make_snapshot(
        "Carol",
        Some(-25.0),
        30.0,
        Some(50.0),
        Some(1),
        OvertrainingRiskLevel::High,
    )];

    let report = service.compute_weekly_report(&snapshots, "Team Gamma");
    assert!(
        report.concerns.iter().any(|c| c.contains("Carol")),
        "Flagged member should appear in concerns"
    );
    assert!(!report.recommendations.is_empty());
}

#[test]
fn test_weekly_report_empty_group() {
    let service = test_group_service();

    let report = service.compute_weekly_report(&[], "Empty Group");
    assert!(report.summary.contains("Empty Group"));
    assert!(report.summary.contains("0/0 active"));
    assert!(report.highlights.is_empty());
    assert!(report.concerns.is_empty());
}

#[test]
fn test_weekly_report_trend_recommendations() {
    let service = test_group_service();

    // Declining trend should produce a recommendation
    let snapshots = vec![
        make_snapshot(
            "Alice",
            Some(5.0),
            20.0,
            Some(50.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
        make_snapshot(
            "Bob",
            Some(5.0),
            15.0,
            Some(45.0),
            Some(1),
            OvertrainingRiskLevel::Low,
        ),
    ];

    let report = service.compute_weekly_report(&snapshots, "Declining Team");
    assert!(report.summary.contains("declining"));
    assert!(
        report
            .recommendations
            .iter()
            .any(|r| r.contains("declining")),
        "Declining trend should produce a recommendation"
    );
}

// ============================================================================
// Threshold Constants Sanity Checks
// ============================================================================

#[test]
fn test_threshold_constants_are_reasonable() {
    assert!(
        OVERREACHING_TSB_THRESHOLD < 0.0,
        "Overreaching threshold should be negative"
    );
    assert!(
        HIGH_FATIGUE_TSB_THRESHOLD < OVERREACHING_TSB_THRESHOLD,
        "High fatigue threshold should be more negative than overreaching"
    );
    assert_eq!(OVERREACHING_TSB_THRESHOLD, -20.0);
    assert_eq!(HIGH_FATIGUE_TSB_THRESHOLD, -30.0);
    assert_eq!(INACTIVITY_DAYS_THRESHOLD, 7);
    assert!((VOLUME_DROP_FRACTION_THRESHOLD - 0.30).abs() < f64::EPSILON);
    assert!((TREND_IMPROVING_FRACTION - 0.05).abs() < f64::EPSILON);
    assert!((TREND_DECLINING_FRACTION - 0.05).abs() < f64::EPSILON);
}
