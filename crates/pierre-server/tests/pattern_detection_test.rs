// ABOUTME: Unit tests for pattern_detection module
// ABOUTME: Tests pattern detection functionality with comprehensive test coverage
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, TimeZone, Utc};
use chrono_tz::UTC;
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_intelligence::{PatternDetector, RiskLevel};

fn create_test_activity(days_ago: i64, distance_km: f64, avg_hr: Option<u32>) -> Activity {
    let date = Utc::now() - Duration::days(days_ago);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let duration = (distance_km * 300.0) as u64; // ~5 min/km pace

    let mut builder = ActivityBuilder::new(
        format!("test_{}", date.timestamp()),
        "Test Activity",
        SportType::Run,
        date,
        duration,
        "test",
    )
    .distance_meters(distance_km * 1000.0);

    if let Some(hr) = avg_hr {
        builder = builder.average_heart_rate(hr);
    }

    builder.build()
}

#[test]
fn test_weekly_schedule_detection() {
    // Create consistent Monday/Wednesday/Friday pattern
    let mut activities = Vec::new();
    for week in 0..4 {
        activities.push(create_test_activity(week * 7, 10.0, Some(140))); // Monday
        activities.push(create_test_activity(week * 7 + 2, 8.0, Some(135))); // Wednesday
        activities.push(create_test_activity(week * 7 + 4, 12.0, Some(145)));
        // Friday
    }

    // UTC explicitly: this fixture builds its days in UTC, so the athlete's
    // clock and the server's are the same one here. Naming it is the point —
    // the zone used to be implicit, which is how the histograms ended up on the
    // wrong day for anyone training in the evening (registre#252).
    let pattern = PatternDetector::detect_weekly_schedule(&activities, UTC);

    // With 3 equally distributed days, consistency score ~= 33 (1/3² + 1/3² + 1/3²) * 100
    assert!(
        pattern.consistency_score > 30.0,
        "Expected consistency > 30, got {}",
        pattern.consistency_score
    );
    assert!(pattern.avg_activities_per_week > 2.5);
}

#[test]
fn test_hard_easy_pattern_detection() {
    // Create alternating hard/easy pattern
    let mut activities = Vec::new();
    for i in 0..10 {
        let is_hard = i % 2 == 0;
        let hr = if is_hard { 170 } else { 130 };
        activities.push(create_test_activity(i, 10.0, Some(hr)));
    }

    let pattern = PatternDetector::detect_hard_easy_pattern(&activities);

    assert!(pattern.pattern_detected);
    assert!(pattern.hard_percentage > 40.0 && pattern.hard_percentage < 60.0);
}

#[test]
fn test_hr_drift_detection() {
    // Create activities with increasing HR (fatigue signal)
    let mut activities = Vec::new();
    for i in 0..12 {
        let hr = 140 + (i * 2); // HR increases over time
        #[allow(clippy::cast_sign_loss)]
        activities.push(create_test_activity(i64::from(i), 10.0, Some(hr as u32)));
    }

    let signals = PatternDetector::detect_overtraining_signals(&activities);

    assert!(signals.hr_drift_detected);
    assert!(signals.risk_level != RiskLevel::Low);
}

#[test]
fn test_volume_spike_detection() {
    // Week 0: 30km (3 weeks ago, days 18-20)
    // Note: create_test_activity uses days_ago, so larger values = older dates
    let mut activities = vec![
        create_test_activity(20, 10.0, Some(140)),
        create_test_activity(19, 10.0, Some(140)),
        create_test_activity(18, 10.0, Some(140)),
        // Week 2: 60km (this week, days 2-7 - creating 100% spike)
        create_test_activity(7, 10.0, Some(140)),
        create_test_activity(6, 10.0, Some(140)),
        create_test_activity(5, 10.0, Some(140)),
        create_test_activity(4, 10.0, Some(140)),
        create_test_activity(3, 10.0, Some(140)),
        create_test_activity(2, 10.0, Some(140)),
    ];

    // Sort by date to ensure chronological order (oldest first)
    activities.sort_by_key(Activity::start_date);

    let pattern = PatternDetector::detect_volume_progression(&activities);

    // Debug output if test fails
    if !pattern.volume_spikes_detected {
        eprintln!("Weekly volumes: {:?}", pattern.weekly_volumes);
        eprintln!("Week numbers: {:?}", pattern.week_numbers);
        eprintln!("Spike weeks: {:?}", pattern.spike_weeks);
    }

    assert!(
        pattern.volume_spikes_detected,
        "Expected to detect 100% volume spike (30km to 60km)"
    );
    assert!(
        !pattern.spike_weeks.is_empty(),
        "Expected spike weeks to be non-empty"
    );
}

/// The zone actually reaches the histograms.
///
/// Every other call site in this file passes `UTC`, which is the same frame the
/// fixture builds its days in — so reverting the conversion inside
/// `detect_weekly_schedule` would leave the whole suite green and carnet#252
/// would be unmeasured (registre#258).
///
/// Six evening sessions at 21:30 America/Toronto, which is 01:30 UTC the
/// following day: the local reading and the server reading disagree on both the
/// weekday and the hour, so this fails the moment the conversion stops
/// happening.
#[test]
fn the_athletes_zone_reaches_both_histograms() {
    use chrono_tz::America::Toronto;

    let activities: Vec<Activity> = (0..6)
        .map(|w| {
            let start = Utc
                .with_ymd_and_hms(2026, 9, 2, 1, 30, 0)
                .single()
                .expect("valid instant")
                - Duration::weeks(w);
            ActivityBuilder::new(
                format!("evening-{w}"),
                "Evening ride",
                SportType::Ride,
                start,
                7_200,
                "strava",
            )
            .build()
        })
        .collect();

    let local = PatternDetector::detect_weekly_schedule(&activities, Toronto);
    let server = PatternDetector::detect_weekly_schedule(&activities, UTC);

    assert_eq!(
        local.most_common_days.first(),
        Some(&chrono::Weekday::Tue),
        "21:30 Tuesday in Toronto is Tuesday training: {:?}",
        local.most_common_days
    );
    assert_eq!(
        local.most_common_times.first(),
        Some(&21),
        "and 21:00 is the hour they ride: {:?}",
        local.most_common_times
    );
    assert_ne!(
        local.most_common_days, server.most_common_days,
        "the two frames must disagree here, or this test cannot detect the \
         conversion being removed"
    );
    assert_ne!(
        local.most_common_times, server.most_common_times,
        "same for the hour histogram, which was shifted by the whole offset"
    );
}
