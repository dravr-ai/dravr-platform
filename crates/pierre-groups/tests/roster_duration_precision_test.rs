// ABOUTME: Pins that the group roster renders durations exact to the second, never rounded
// ABOUTME: A 53-minute run shown as 0.9h read back as 54 minutes and a wrong pace (carnet#1015)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The roster card is the coach's context for every member of a room. The
//! model quotes its durations and derives pace from them, so a rounded
//! duration becomes a wrong figure in a reply: challenged on Philippe's
//! 53-minute 6.1 km run, the coach re-read «0.9h» and «corrected» a right
//! answer to 54 minutes and 8 min 50/km.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration as ChronoDuration, Utc};
use pierre_core::models::groups::{MemberFitnessSnapshot, OvertrainingRiskLevel, RosterActivity};
use pierre_groups::strategies::summarization::{
    GroupSummarizationStrategy, RosterCardSummarizer, WeeklyDigestSummarizer,
};
use std::collections::HashMap;
use uuid::Uuid;

/// Philippe's run from the live incident corpus: 3180 s over 6.1 km.
const PEER_RUN_SECONDS: u64 = 3_180;

fn snapshot(weekly_duration_seconds: u64, recent: Vec<RosterActivity>) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id: Uuid::new_v4(),
        display_name: "Philippe Tremblay".to_owned(),
        ctl: None,
        atl: None,
        tsb: None,
        form_ctl: None,
        weekly_volume_km: 6.1,
        previous_week_volume_km: None,
        weekly_activity_count: 1,
        weekly_duration_seconds,
        primary_sport: None,
        vdot: None,
        overtraining_risk: OvertrainingRiskLevel::Low,
        days_since_last_activity: Some(1),
        last_activity_per_provider: HashMap::new(),
        recent_activities: recent,
        needs_reauth_providers: Vec::new(),
        served_stale: false,
        timezone: None,
        computed_at: Utc::now(),
    }
}

fn run_of(duration_seconds: u64) -> RosterActivity {
    RosterActivity {
        start: Utc::now() - ChronoDuration::days(1),
        sport: "Run".to_owned(),
        distance_km: Some(6.1),
        duration_seconds,
        name: "Course".to_owned(),
        city: None,
        start_latitude: None,
        start_longitude: None,
        elevation_gain_m: None,
    }
}

#[test]
fn a_recent_run_renders_its_minutes_not_a_rounded_hour() {
    let rendered = WeeklyDigestSummarizer
        .summarize_member(&snapshot(3_180, vec![run_of(PEER_RUN_SECONDS)]))
        .summary_text;

    let row = rendered
        .lines()
        .find(|line| line.contains(" Run "))
        .unwrap_or_else(|| panic!("no Recent: row for the run:\n{rendered}"));
    assert!(
        row.contains(" Run 0:53:00 6.1km"),
        "the row must carry the run's exact 0:53:00: {row:?}"
    );
    assert!(
        !rendered.contains("0.9h"),
        "a one-decimal hour rounds 53 minutes to 54:\n{rendered}"
    );
    assert!(
        rendered.contains("1 activities, 0:53:00 active, 6.1km total"),
        "the weekly line must carry the exact total:\n{rendered}"
    );
}

#[test]
fn seconds_past_the_minute_are_kept() {
    // 53:59 truncated to whole minutes reads as 53 — 59 s, about 10 s/km on
    // 6.1 km, the same size of error as the rounded hour.
    let rendered = WeeklyDigestSummarizer
        .summarize_member(&snapshot(3_239, vec![run_of(3_239)]))
        .summary_text;
    assert!(
        rendered.contains(" Run 0:53:59 6.1km") && rendered.contains(" 0:53:59 active"),
        "the row and the week must keep the 59 seconds:\n{rendered}"
    );
}

#[test]
fn the_roster_line_renders_the_weekly_total_exact() {
    let rendered = RosterCardSummarizer
        .summarize_member(&snapshot(70 * 60, vec![run_of(PEER_RUN_SECONDS)]))
        .summary_text;
    assert!(
        rendered.contains(" | 1:10:00 / 6km/wk"),
        "70 minutes is 1:10:00, not 1.2h:\n{rendered}"
    );
}

#[test]
fn a_short_or_empty_week_still_renders_its_minutes() {
    let short = WeeklyDigestSummarizer
        .summarize_member(&snapshot(6 * 60, Vec::new()))
        .summary_text;
    assert!(
        short.contains(" 0:06:00 active"),
        "a six-minute week must not vanish into zero:\n{short}"
    );
    let empty = WeeklyDigestSummarizer
        .summarize_member(&snapshot(0, Vec::new()))
        .summary_text;
    assert!(
        empty.contains(" 0:00:00 active"),
        "an empty week keeps the line's shape:\n{empty}"
    );
}
