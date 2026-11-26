// ABOUTME: calculate_fitness_score is the engine's score over a window, trended against the window before
// ABOUTME: No score without heart-rate zones; CTL beside it reads the full history (registre#415, carnet#1807)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `calculate_fitness_score` is dravr-cageux's fitness score (carnet#1807):
//! aerobic fitness and strength endurance read on the athlete's heart-rate
//! zones, and consistency, over the `timeframe` window, with its trend against
//! the window of equal length before it. Without zones there is no score,
//! never a guessed one.
//!
//! The training load reported beside the score keeps registre#415's fix: the
//! EMA is seeded at the mean daily TSS of the span it is given, so scoping its
//! input to the scoring window left a 42-day average at roughly three-quarters
//! of steady state. CTL reads the full fetched history whatever the window.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::cmp::Reverse;

use dravr_cageux::config::intelligence::IntelligenceConfig;
use dravr_cageux::zones::HrZones;
use dravr_tronc::dates::{Elapsed, Timestamp, Zone};
use pierre_core::models::zones::HrZonesUnavailable;
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_fitness_compute::AthleteInputs;
use pierre_tool_runtime::implementations::analytics::calculate_fitness_metrics;
use pierre_tool_runtime::implementations::analytics::output::{FitnessScoreResult, ProvidersUsed};

/// The provenance every result carries; irrelevant to what these tests assert.
fn providers() -> ProvidersUsed {
    ProvidersUsed {
        activity_provider: "test".to_owned(),
        sleep_provider: None,
    }
}

/// A run `days_ago`, covering 10 km in `duration_seconds`.
///
/// Pace is duration / distance, so a smaller duration is a faster run.
fn run(days_ago: i64, duration_seconds: u64) -> Activity {
    ActivityBuilder::new(
        format!("run-{days_ago}"),
        format!("run {days_ago}d ago"),
        SportType::Run,
        Timestamp::now() - Elapsed::days(days_ago),
        duration_seconds,
        "test".to_owned(),
    )
    .distance_meters(10_000.0)
    .average_heart_rate(150)
    .build()
}

/// The provider's own ordering: newest first.
///
/// Passing an already-sorted list would hide the defect entirely, which is why
/// the fixture is built in the order the service actually returns.
fn newest_first(mut activities: Vec<Activity>) -> Vec<Activity> {
    activities.sort_by_key(|a| Reverse(a.start_date()));
    activities
}

/// The athlete's zones: resting 50 bpm, maximum 190 bpm.
fn zones() -> HrZones {
    HrZones::for_athlete(None, 50, 190).expect("zones for 50/190")
}

/// A run `days_ago` lasting `minutes` at an average of `heart_rate` bpm.
fn session(days_ago: i64, minutes: u64, heart_rate: u32) -> Activity {
    ActivityBuilder::new(
        format!("session-{days_ago}"),
        format!("session {days_ago}d ago"),
        SportType::Run,
        Timestamp::now() - Elapsed::days(days_ago),
        minutes * 60,
        "test".to_owned(),
    )
    .distance_meters(10_000.0)
    .average_heart_rate(heart_rate)
    .build()
}

/// No resting or maximum heart rate saved: no zones, so no score, and the
/// answer says what is missing rather than guessing a number.
#[tokio::test]
async fn without_heart_rate_zones_there_is_no_score() {
    let activities = newest_first((0..10).map(|i| run(25 - i * 2, 3600)).collect());

    let result = calculate_fitness_metrics(
        &activities,
        "month",
        &AthleteInputs::default(),
        Err(HrZonesUnavailable::NoProfile),
        &IntelligenceConfig::default(),
        &Zone::utc(),
        providers(),
    )
    .await;

    let FitnessScoreResult::NoData(empty) = result else {
        panic!("bug: a score was guessed without heart-rate zones: {result:?}");
    };
    assert!(
        empty.message.contains("resting and maximum heart rate"),
        "the answer must name what is missing: {}",
        empty.message
    );
}

/// The trend compares the month scored with the month before it: an athlete
/// who trained more, and harder, this month reads as improving.
#[tokio::test]
async fn the_trend_compares_the_window_with_the_one_before() {
    // The month before: two easy sessions. This month: twelve steady ones.
    let before = (0..2).map(|i| session(40 + i * 7, 40, 125));
    let this_month = (0..12).map(|i| session(1 + i * 2, 60, 160));
    let activities = newest_first(before.chain(this_month).collect());

    let result = calculate_fitness_metrics(
        &activities,
        "month",
        &AthleteInputs::default(),
        Ok(zones()),
        &IntelligenceConfig::default(),
        &Zone::utc(),
        providers(),
    )
    .await;

    let FitnessScoreResult::Scored(detail) = result else {
        panic!("a month of sessions on known zones must score, got {result:?}");
    };
    let previous = detail
        .previous_fitness_score
        .expect("the month before held sessions, so it has a score");
    assert!(
        detail.fitness_score > previous,
        "this month ({}) must score above the month before ({previous})",
        detail.fitness_score
    );
    assert_eq!(
        detail.trend.as_deref(),
        Some("improving"),
        "bug: a month scoring above the month before does not read as improving"
    );
    assert_eq!(detail.activities_analyzed, 12, "only this month is scored");
}

/// The whole history has no window before it, so it carries no trend.
#[tokio::test]
async fn all_time_has_no_trend() {
    let activities = newest_first((0..12).map(|i| session(1 + i * 5, 60, 160)).collect());

    let result = calculate_fitness_metrics(
        &activities,
        "all_time",
        &AthleteInputs::default(),
        Ok(zones()),
        &IntelligenceConfig::default(),
        &Zone::utc(),
        providers(),
    )
    .await;

    let FitnessScoreResult::Scored(detail) = result else {
        panic!("expected a score, got {result:?}");
    };
    assert_eq!(
        detail.window_days, None,
        "bug: all_time is scoped to a window"
    );
    assert_eq!(
        detail.previous_fitness_score, None,
        "bug: all_time has no window before it, yet carries a previous score"
    );
    assert_eq!(
        detail.trend, None,
        "bug: all_time has no window before it, yet carries a trend"
    );
}

/// Narrowing the scoring window must not shrink chronic training load. CTL is a
/// current-state number computed from the full fetched history.
#[tokio::test]
async fn the_scoring_window_does_not_truncate_chronic_load() {
    // 120 days of steady training — long enough for a 42-day EMA to converge.
    let activities = newest_first((0..40).map(|i| run(120 - i * 3, 3600)).collect());
    let config = IntelligenceConfig::default();

    // A run is scored against the athlete's own threshold pace: without one
    // (and no heart-rate thresholds or session RPE) it carries no stress.
    let athlete = AthleteInputs {
        threshold_pace_s_per_km: Some(300.0),
        ..AthleteInputs::default()
    };
    let wide = calculate_fitness_metrics(
        &activities,
        "all_time",
        &athlete,
        Ok(zones()),
        &config,
        &Zone::utc(),
        providers(),
    )
    .await;
    let narrow = calculate_fitness_metrics(
        &activities,
        "month",
        &athlete,
        Ok(zones()),
        &config,
        &Zone::utc(),
        providers(),
    )
    .await;

    let (FitnessScoreResult::Scored(wide), FitnessScoreResult::Scored(narrow)) = (wide, narrow)
    else {
        panic!("both windows must produce a score");
    };

    assert!(
        wide.metrics.ctl > 0.0,
        "the fixture must generate real training load, got ctl={}",
        wide.metrics.ctl
    );
    assert!(
        (wide.metrics.ctl - narrow.metrics.ctl).abs() < f64::EPSILON,
        "CTL must be identical regardless of the scoring window — it reads the \
         full history. all_time={} month={}; a smaller month value is the \
         truncated-EMA bug that understated the athlete's 66",
        wide.metrics.ctl,
        narrow.metrics.ctl
    );

    // The window scopes the score, and the payload says which.
    assert_eq!(narrow.window_days, Some(30));
    assert_eq!(wide.window_days, None, "all_time scopes nothing");
    assert!(
        wide.history_span_days >= 118,
        "the payload must state the span it actually analysed, got {}",
        wide.history_span_days
    );
}

/// The payload names chronic load from the configured window, so nothing has to
/// infer a period from a smoothing constant that happens to sit nearby.
#[tokio::test]
async fn the_ctl_sentence_comes_from_config_not_a_hardcoded_42() {
    let activities = newest_first((0..12).map(|i| run(90 - i * 5, 3600)).collect());
    let mut config = IntelligenceConfig::default();
    config.algorithms.params.training_load_ctl_days = 28;

    let result = calculate_fitness_metrics(
        &activities,
        "quarter",
        &AthleteInputs::default(),
        Ok(zones()),
        &config,
        &Zone::utc(),
        providers(),
    )
    .await;
    let FitnessScoreResult::Scored(detail) = result else {
        panic!("expected a score");
    };

    assert!(
        detail.interpretation.ctl.contains("28-day"),
        "a tenant configured to 28 must not be told 42, got: {}",
        detail.interpretation.ctl
    );
    assert!(
        !detail.interpretation.ctl.contains("42-day"),
        "the hardcoded 42 is what the athlete read back as their analysis window"
    );
}
