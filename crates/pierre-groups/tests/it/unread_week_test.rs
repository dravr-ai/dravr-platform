// ABOUTME: Pins that a member whose stored history does not reach this Monday week shows no volume, never zero
// ABOUTME: The roster card, the weekly digest card and the group average all leave the unread week out

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A member's weekly figures are the Monday weeks their Home volume card
//! counts from the stored history. When that history does not reach the week
//! yet, the snapshot carries no figure: the model must read "unknown", the
//! group average must not count it as a zero-kilometre week.

use dravr_tronc::dates::Timestamp;
use pierre_core::models::groups::{MemberFitnessSnapshot, OvertrainingRiskLevel};
use pierre_groups::service::GroupService;
use pierre_groups::strategies::aggregation::{GroupAggregationStrategy, LiveAggregation};
use pierre_groups::strategies::summarization::{
    GroupSummarizationStrategy, RosterCardSummarizer, WeeklyDigestSummarizer,
};
use std::collections::HashMap;
use uuid::Uuid;

/// A member whose this-week and last-week kilometres are `km` and `prev_km`,
/// `None` for a week their stored history does not reach.
fn member(name: &str, km: Option<f64>, prev_km: Option<f64>) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id: Uuid::new_v4(),
        display_name: name.to_owned(),
        ctl: None,
        atl: None,
        tsb: None,
        form_ctl: None,
        weekly_volume_km: km,
        previous_week_volume_km: prev_km,
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
fn an_unread_week_is_named_unknown_on_both_cards() {
    let unread = member("Ada", None, None);
    let roster = RosterCardSummarizer.summarize_member(&unread).summary_text;
    assert!(
        roster.contains("this week: not read yet (unknown, not zero)") && !roster.contains("km/wk"),
        "bug: the roster card gave an unread week a figure: {roster}"
    );
    let digest = WeeklyDigestSummarizer
        .summarize_member(&unread)
        .summary_text;
    assert!(
        digest.contains("this week's volume: not read yet (unknown, not zero)")
            && !digest.contains("km total"),
        "bug: the digest card gave an unread week a figure: {digest}"
    );

    let read = RosterCardSummarizer
        .summarize_member(&member("Bo", Some(0.0), None))
        .summary_text;
    assert!(
        read.contains("1:30:00 / 0km/wk"),
        "bug: a read quiet week is not its own zero: {read}"
    );
}

#[test]
fn the_group_average_counts_only_the_weeks_read() {
    let stats = LiveAggregation.aggregate_stats(&[
        member("Ada", Some(40.0), Some(40.0)),
        member("Bo", Some(20.0), Some(20.0)),
        member("Cy", None, None),
    ]);
    assert!(
        stats
            .avg_weekly_volume_km
            .is_some_and(|km| (km - 30.0).abs() < 1e-9),
        "bug: an unread week pulled the average down as a zero: {:?}",
        stats.avg_weekly_volume_km
    );
    assert_eq!(
        stats.total_members, 3,
        "bug: the unread member is not a member"
    );
}

#[test]
fn a_group_where_no_week_is_read_has_no_average() {
    let stats =
        LiveAggregation.aggregate_stats(&[member("Ada", None, None), member("Cy", None, None)]);
    assert_eq!(
        stats.avg_weekly_volume_km, None,
        "bug: two unread weeks averaged to a figure"
    );
}

#[test]
fn the_volume_ranking_leaves_out_a_member_whose_week_is_unread() {
    let snapshots = [
        member("Ada", Some(20.0), None),
        member("Cy", None, None),
        member("Bo", Some(0.0), None),
        member("Dee", Some(40.0), None),
    ];
    let names: Vec<&str> = GroupService::members_by_volume(&snapshots)
        .into_iter()
        .map(|(s, _)| s.display_name.as_str())
        .collect();
    assert_eq!(
        names,
        ["Dee", "Ada", "Bo"],
        "bug: the ranking is not by volume, or it ranked an unread week as zero"
    );
}
