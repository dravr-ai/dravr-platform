// ABOUTME: Pins that the group roster's Recent: rows name the member's own weekday, not the UTC one
// ABOUTME: Regression for 2026-09-02 — the coach reassigned five activities to wrong weekdays three times
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The `Recent:` rows exist so the coach can answer sub-week questions
//! ("Saturday vs Sunday", "longest ride this week") **without inventing
//! per-day numbers** — the comment above them says exactly that.
//!
//! They rendered `act.start`, a `DateTime<Utc>`, through `%a` with no
//! conversion. For an `America/Toronto` athlete every session starting at or
//! after 20:00 local printed the *following* day and the following weekday
//! name. The one question the block was built to answer was the one it got
//! wrong.
//!
//! Production Telegram, 2026-09-02: the coach placed a Tuesday ride on Sunday,
//! then rebuilt the week twice more and moved it again each time. The athlete
//! corrected it three times — *"road 2 aus etait hier, mardi. T'es melé big"*,
//! *"date ride etait lundi. Ca va pas les dates"* — and left the conversation
//! telling the bot to go rest.

use chrono::{TimeZone, Utc};
use pierre_core::models::groups::{MemberFitnessSnapshot, OvertrainingRiskLevel, RosterActivity};
use pierre_groups::strategies::summarization::{
    GroupSummarizationStrategy, WeeklyDigestSummarizer,
};
use std::collections::HashMap;
use uuid::Uuid;

/// 2026-09-02 01:30 UTC — 2026-09-01 21:30 in America/Toronto.
///
/// A Tuesday evening ride in the athlete's own frame; a Wednesday in the
/// server's. Any surface that skips the conversion reports `mer`/`Wed` here.
fn tuesday_evening_ride() -> RosterActivity {
    RosterActivity {
        start: Utc.with_ymd_and_hms(2026, 9, 2, 1, 30, 0).unwrap(),
        sport: "Ride".to_owned(),
        distance_km: Some(161.0),
        duration_minutes: 372,
        name: "Road 2 AUS".to_owned(),
        city: None,
        start_latitude: None,
        start_longitude: None,
        elevation_gain_m: Some(2391.0),
    }
}

fn snapshot_in(timezone: Option<&str>) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id: Uuid::new_v4(),
        display_name: "Raph".to_owned(),
        ctl: None,
        atl: None,
        tsb: None,
        weekly_volume_km: 300.0,
        previous_week_volume_km: None,
        weekly_activity_count: 6,
        weekly_duration_seconds: 60_840,
        primary_sport: None,
        vdot: None,
        overtraining_risk: OvertrainingRiskLevel::Low,
        days_since_last_activity: None,
        last_activity_per_provider: HashMap::new(),
        recent_activities: vec![tuesday_evening_ride()],
        needs_reauth_providers: Vec::new(),
        served_stale: false,
        timezone: timezone.map(str::to_owned),
        computed_at: Utc::now(),
    }
}

#[test]
fn a_late_evening_ride_keeps_the_members_weekday() {
    let card = WeeklyDigestSummarizer.summarize_member(&snapshot_in(Some("America/Toronto")));
    let rendered = &card.summary_text;

    assert!(
        rendered.contains("2026-09-01 Tue"),
        "21:30 Tuesday in Toronto is a Tuesday ride; the roster must say so: {rendered}"
    );
    assert!(
        !rendered.contains("Wed"),
        "the UTC frame calls this Wednesday — that shift is the defect: {rendered}"
    );
}

#[test]
fn the_date_moves_with_the_weekday() {
    let card = WeeklyDigestSummarizer.summarize_member(&snapshot_in(Some("America/Toronto")));

    assert!(
        !card.summary_text.contains("2026-09-02"),
        "the calendar date must follow the same zone as the weekday, or the row \
         contradicts itself: {}",
        card.summary_text
    );
}

/// A member on the other side of the date line, so a single hard-coded offset
/// cannot pass this file.
#[test]
fn each_member_is_read_on_their_own_clock() {
    let card = WeeklyDigestSummarizer.summarize_member(&snapshot_in(Some("Australia/Sydney")));

    assert!(
        card.summary_text.contains("2026-09-02 Wed"),
        "01:30 UTC is Wednesday mid-morning in Sydney; the same instant is a \
         different civil day for a different member: {}",
        card.summary_text
    );
}

/// No zone on file falls back to UTC rather than guessing — and still names the
/// weekday, because the naming is the fix and the zone is a separate question.
#[test]
fn without_a_timezone_the_roster_states_the_utc_weekday() {
    let card = WeeklyDigestSummarizer.summarize_member(&snapshot_in(None));

    assert!(
        card.summary_text.contains("2026-09-02 Wed"),
        "with no zone on file the roster stays on UTC, and still names the day: {}",
        card.summary_text
    );
}
