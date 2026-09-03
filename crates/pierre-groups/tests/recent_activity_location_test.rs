// ABOUTME: Regression test for the RosterActivity location projection
// ABOUTME: Verifies the WeeklyDigest Recent: block surfaces city + lat/lng when present
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The group-coaching prompt's `Recent:` block must surface `city` (and
//! optionally `lat,lng`) for every `RosterActivity` that carries them, so
//! the LLM can answer "endroit entre vous deux" questions grounded in
//! provider data instead of refusing or fabricating.
//!
//! Anti-fabrication invariant: `WHOOP` / treadmill / non-GPS activities
//! must render unchanged (no fake `@ ...` segment).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration as ChronoDuration, Utc};
use pierre_core::models::groups::{MemberFitnessSnapshot, OvertrainingRiskLevel, RosterActivity};
use pierre_groups::strategies::summarization::{
    GroupSummarizationStrategy, WeeklyDigestSummarizer,
};
use std::collections::HashMap;
use uuid::Uuid;

fn snapshot_with_recent(recent: Vec<RosterActivity>) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id: Uuid::new_v4(),
        display_name: "Phil".to_owned(),
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
        recent_activities: recent,
        needs_reauth_providers: Vec::new(),
        served_stale: false,
        timezone: None,
        computed_at: Utc::now(),
    }
}

fn ride_with_location(city: Option<&str>, lat: Option<f64>, lng: Option<f64>) -> RosterActivity {
    RosterActivity {
        start: Utc::now() - ChronoDuration::days(2),
        sport: "Ride".to_owned(),
        distance_km: Some(42.0),
        duration_minutes: 90,
        name: "Saturday loop".to_owned(),
        city: city.map(str::to_owned),
        start_latitude: lat,
        start_longitude: lng,
        elevation_gain_m: None,
    }
}

#[test]
fn renderer_surfaces_city_when_strava_supplies_it() {
    let snapshot = snapshot_with_recent(vec![ride_with_location(
        Some("Saint-Bruno-de-Montarville"),
        Some(45.5331),
        Some(-73.3493),
    )]);

    let summarizer = WeeklyDigestSummarizer;
    let card = summarizer.summarize_member(&snapshot);
    let rendered = &card.summary_text;

    assert!(
        rendered.contains("@ Saint-Bruno-de-Montarville"),
        "Recent: block must include the city when present. Got:\n{rendered}"
    );
    assert!(
        rendered.contains("45.5331") && rendered.contains("-73.3493"),
        "Recent: block must include start coords for midpoint math. Got:\n{rendered}"
    );
}

#[test]
fn renderer_surfaces_coords_without_city_when_only_gps_available() {
    let snapshot = snapshot_with_recent(vec![ride_with_location(
        None,
        Some(45.5017),
        Some(-73.5673),
    )]);

    let summarizer = WeeklyDigestSummarizer;
    let card = summarizer.summarize_member(&snapshot);
    let rendered = &card.summary_text;

    assert!(
        rendered.contains("(45.5017,-73.5673)"),
        "Recent: block must render coords even without a city. Got:\n{rendered}"
    );
    assert!(
        !rendered.contains(" @ Saint-"),
        "Must not invent a city when source has none"
    );
}

#[test]
fn renderer_omits_location_for_hr_only_sources() {
    // WHOOP / treadmill: no city, no GPS. The renderer must stay
    // location-less so the LLM can't claim "Philippe was in X" when the
    // provider didn't record it.
    let snapshot = snapshot_with_recent(vec![ride_with_location(None, None, None)]);

    let summarizer = WeeklyDigestSummarizer;
    let card = summarizer.summarize_member(&snapshot);
    let rendered = &card.summary_text;

    assert!(
        !rendered.contains(" @ "),
        "HR/duration-only activities must render WITHOUT a location segment. Got:\n{rendered}"
    );
}

#[test]
fn renderer_surfaces_elevation_when_provider_supplies_it() {
    // Strava / sciotte / Coros carry total ascent. The Recent: block must
    // surface it so the LLM can answer "combien de dénivelé pour vous deux"
    // with a real shared total instead of abstaining.
    let mut ride = ride_with_location(Some("Saint-Bruno-de-Montarville"), None, None);
    ride.elevation_gain_m = Some(650.0);
    let snapshot = snapshot_with_recent(vec![ride]);

    let summarizer = WeeklyDigestSummarizer;
    let card = summarizer.summarize_member(&snapshot);
    let rendered = &card.summary_text;

    assert!(
        rendered.contains("↑650m"),
        "Recent: block must include total ascent when present. Got:\n{rendered}"
    );
}

#[test]
fn renderer_omits_elevation_for_sources_without_ascent() {
    // HR-only / treadmill sources carry no elevation. The renderer must
    // stay ascent-less so the LLM can't claim a climb the provider never
    // recorded.
    let snapshot = snapshot_with_recent(vec![ride_with_location(None, None, None)]);

    let summarizer = WeeklyDigestSummarizer;
    let card = summarizer.summarize_member(&snapshot);
    let rendered = &card.summary_text;

    assert!(
        !rendered.contains('↑'),
        "Activities without elevation must render WITHOUT an ascent segment. Got:\n{rendered}"
    );
}
