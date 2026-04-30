// ABOUTME: Tests for ActivitySummary scalar-field enrichment and retrieval advice
// ABOUTME: Pins HR/elevation/calories presence so summary-mode coaches aren't starved
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression coverage for the activity-summary-enrichment PR.
//!
//! The bug being guarded: `/coach` → Nutrition coach → "dinner based on
//! yesterday's run" → reply that admits "no HR, no elevation, no recovery
//! data" even though Strava returned all three on the list endpoint. The
//! old `ActivitySummary` had 6 fields; sensor scalars were dropped on the
//! way to the LLM. These tests pin the expanded shape and the retrieval
//! advice so a future refactor can't quietly un-strip the fields.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_core::models::{activity::ActivityBuilder, Activity, SportType};
use pierre_mcp_server::protocols::universal::handlers::fitness_api::{
    ActivityRetrievalContext, ActivitySummary, AnalysisType,
};

fn build_sensor_rich_activity() -> Activity {
    ActivityBuilder::new(
        "strava-1",
        "Morning Run",
        SportType::Run,
        Utc::now(),
        2_394, // 39:54 — the screenshot run duration
        "strava",
    )
    .distance_meters(6_040.0)
    .elevation_gain(85.0)
    .average_heart_rate(148)
    .max_heart_rate(172)
    .calories(420)
    .average_cadence(174)
    .average_power(220)
    .suffer_score(47)
    .temperature(5.0)
    .build()
}

fn build_hrm_less_activity() -> Activity {
    // Indoor trainer session, no HRM strap, no elevation from GPS.
    ActivityBuilder::new(
        "strava-2",
        "Zwift Intervals",
        SportType::Ride,
        Utc::now(),
        3_600,
        "strava",
    )
    .distance_meters(25_000.0)
    .build()
}

#[test]
fn summary_preserves_scalar_sensor_fields_when_present() {
    let activity = build_sensor_rich_activity();
    let summary: ActivitySummary = (&activity).into();

    assert_eq!(summary.id, "strava-1");
    assert_eq!(summary.name, "Morning Run");
    assert!((summary.distance_meters - 6_040.0).abs() < f64::EPSILON);
    assert_eq!(summary.duration_seconds, 2_394);
    // The enrichment — these are the fields the old summary stripped.
    assert_eq!(summary.average_heart_rate, Some(148));
    assert_eq!(summary.max_heart_rate, Some(172));
    assert_eq!(summary.elevation_gain_meters, Some(85.0));
    assert_eq!(summary.calories, Some(420));
    assert_eq!(summary.average_cadence, Some(174));
    assert_eq!(summary.average_power, Some(220));
    assert_eq!(summary.suffer_score, Some(47));
    // Ambient temperature: ski-de-fond "10 plus froides" coach query exposed
    // that every OAuth provider was dropping the field on the floor; pin it
    // here so a future refactor can't quietly strip it again.
    assert_eq!(summary.temperature, Some(5.0));
}

#[test]
fn summary_serializes_without_null_noise_when_fields_absent() {
    let activity = build_hrm_less_activity();
    let summary: ActivitySummary = (&activity).into();
    let json = serde_json::to_value(&summary).expect("summary serializes");

    // Required scalars always present.
    assert_eq!(json["id"], "strava-2");
    assert_eq!(json["duration_seconds"], 3_600);

    // Optional scalars: `skip_serializing_if = Option::is_none` must drop
    // the keys entirely rather than render `"field": null` — keeps the
    // token budget tight and the LLM context clean for no-HRM activities.
    assert!(
        json.get("average_heart_rate").is_none(),
        "absent HR must be omitted, got: {json}"
    );
    assert!(json.get("max_heart_rate").is_none());
    assert!(json.get("elevation_gain_meters").is_none());
    assert!(json.get("calories").is_none());
    assert!(json.get("average_cadence").is_none());
    assert!(json.get("average_power").is_none());
    assert!(json.get("suffer_score").is_none());
    assert!(json.get("temperature").is_none());
}

#[test]
fn retrieval_context_advice_present_iff_summary_mode() {
    let activities = vec![build_sensor_rich_activity()];

    let summary_ctx = ActivityRetrievalContext::from_activities(
        &activities,
        AnalysisType::GeneralOverview,
        "summary",
    );
    let detailed_ctx = ActivityRetrievalContext::from_activities(
        &activities,
        AnalysisType::GeneralOverview,
        "detailed",
    );

    // Summary mode: advice fires — tells the LLM the trade-off and
    // names the envvar knob for narrowing.
    let advice = summary_ctx.advice.as_ref().expect("summary advice present");
    assert!(advice.contains("Summary mode"));
    assert!(advice.contains("ACTIVITY_DETAIL_THRESHOLD"));
    assert!(advice.contains("Splits, laps, segment efforts"));

    // Detailed mode: no trade-off to surface, no advice.
    assert!(
        detailed_ctx.advice.is_none(),
        "detailed mode must not carry advice, got: {:?}",
        detailed_ctx.advice
    );
}

#[test]
fn retrieval_context_advice_serializes_only_when_populated() {
    let activities = vec![build_sensor_rich_activity()];

    let summary_ctx = ActivityRetrievalContext::from_activities(
        &activities,
        AnalysisType::GeneralOverview,
        "summary",
    );
    let detailed_ctx = ActivityRetrievalContext::from_activities(
        &activities,
        AnalysisType::GeneralOverview,
        "detailed",
    );

    let summary_json = serde_json::to_value(&summary_ctx).expect("serializes");
    let detailed_json = serde_json::to_value(&detailed_ctx).expect("serializes");

    assert!(
        summary_json.get("advice").is_some(),
        "summary ctx must carry advice field"
    );
    // Field must be dropped entirely from the JSON when None — not
    // rendered as `"advice": null` — so the LLM doesn't read null as
    // "empty advice, ignore". `skip_serializing_if = Option::is_none`.
    assert!(
        detailed_json.get("advice").is_none(),
        "detailed ctx must omit the advice key entirely"
    );
}
