// ABOUTME: An athlete getting faster must not score 0 for performance, and CTL must not shrink with the window
// ABOUTME: Pins the two halves of registre#415 — the inverted trend split and the truncated EMA input

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `calculate_fitness_score` answered an athlete with two wrong numbers at once.
//!
//! **"le volet performance est à 0."** `calculate_performance_trend` splits the
//! activity list at its midpoint and computes
//! `(first_half_pace − second_half_pace) / first_half_pace`, which is only an
//! improvement if the first half is the *older* one. The provider returns
//! newest-first — dravr-cageux documents this and rejects the ordering outright
//! — so the halves were reversed: an athlete getting faster produced a negative
//! improvement, `(neg + 10.0) * 5.0` went below zero, and `clamp` reported 0.
//! The handler sorted a *clone* for the EMA and passed the unsorted list to the
//! two components that split by index.
//!
//! **"charge chronique … 66."** The EMA is seeded at zero and walked forward
//! over whatever span it is given, so scoping its input to the 30-day scoring
//! window left a 42-day average at roughly three-quarters of steady state. CTL
//! is a point-in-time state, not a windowed aggregate; the window now scopes
//! only the two genuine aggregates.
//!
//! Both tests fail against the previous implementation — the first with a
//! performance component of exactly 0, the second with a CTL that shrinks as
//! the window narrows.

#![cfg(feature = "tools-analytics")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::cmp::Reverse;

use chrono::{Duration, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_intelligence::AlgorithmConfig;
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
        Utc::now() - Duration::days(days_ago),
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
/// the fixture is built in the order the scraper actually returns.
fn newest_first(mut activities: Vec<Activity>) -> Vec<Activity> {
    activities.sort_by_key(|a| Reverse(a.start_date()));
    activities
}

/// An athlete whose pace improves steadily must not be reported as making no
/// progress. This is the athlete-visible half of registre#415.
#[test]
fn an_improving_athlete_does_not_score_zero_for_performance() {
    // 3600s -> 3000s over 60 days: unmistakably getting faster.
    let activities = newest_first(
        (0..20)
            .map(|i| {
                let days_ago = 60 - i * 3;
                let seconds = 3000 + u64::try_from(days_ago).unwrap_or(0) * 10;
                run(days_ago, seconds)
            })
            .collect(),
    );

    let result = calculate_fitness_metrics(
        &activities,
        "quarter",
        &AlgorithmConfig::default(),
        providers(),
    );

    let FitnessScoreResult::Scored(detail) = result else {
        panic!("20 activities must produce a score, got {result:?}");
    };

    assert!(
        detail.components.performance_score > 50.0,
        "an athlete improving from 3600s to 3000s per 10km must score above the \
         neutral 50, got {} — a score of 0 is the reversed-halves bug",
        detail.components.performance_score
    );
    assert_ne!(
        detail.components.performance_score, 0.0,
        "performance scored exactly 0, which is what the inverted split produced"
    );
}

/// Narrowing the scoring window must not shrink chronic training load. CTL is a
/// current-state number computed from the full fetched history.
#[test]
fn the_scoring_window_does_not_truncate_chronic_load() {
    // 120 days of steady training — long enough for a 42-day EMA to converge.
    let activities = newest_first((0..40).map(|i| run(120 - i * 3, 3600)).collect());
    let config = AlgorithmConfig::default();

    let wide = calculate_fitness_metrics(&activities, "all_time", &config, providers());
    let narrow = calculate_fitness_metrics(&activities, "month", &config, providers());

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

    // The window does scope the aggregates, and the payload says which.
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
#[test]
fn the_ctl_sentence_comes_from_config_not_a_hardcoded_42() {
    let activities = newest_first((0..12).map(|i| run(90 - i * 5, 3600)).collect());
    let mut config = AlgorithmConfig::default();
    config.params.training_load_ctl_days = 28;

    let result = calculate_fitness_metrics(&activities, "quarter", &config, providers());
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
