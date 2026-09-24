// ABOUTME: Tests for the data refresh models
// ABOUTME: Freshness from age, refresh config defaults, provider eligibility and chat refresh

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use std::time::Duration;

use chrono::Utc;
use pierre_core::models::refresh::*;

#[test]
fn freshness_from_age() {
    assert_eq!(
        DataFreshness::from_age(Duration::from_secs(0)),
        DataFreshness::Fresh
    );
    assert_eq!(
        DataFreshness::from_age(Duration::from_mins(30)),
        DataFreshness::Fresh
    );
    assert_eq!(
        DataFreshness::from_age(Duration::from_hours(1)),
        DataFreshness::Recent
    );
    assert_eq!(
        DataFreshness::from_age(Duration::from_hours(2)),
        DataFreshness::Recent
    );
    assert_eq!(
        DataFreshness::from_age(Duration::from_hours(4)),
        DataFreshness::Stale
    );
    assert_eq!(
        DataFreshness::from_age(Duration::from_hours(24)),
        DataFreshness::Outdated
    );
    assert_eq!(
        DataFreshness::from_age(Duration::from_hours(24 * 7)),
        DataFreshness::Expired
    );
}

#[test]
fn freshness_from_none_is_expired() {
    assert_eq!(DataFreshness::from_last_sync(None), DataFreshness::Expired);
}

#[test]
fn freshness_from_recent_timestamp() {
    let two_hours_ago = Utc::now() - chrono::Duration::hours(2);
    assert_eq!(
        DataFreshness::from_last_sync(Some(two_hours_ago)),
        DataFreshness::Recent
    );
}

#[test]
fn config_defaults_are_sensible() {
    let cfg = RefreshConfig::default();
    assert!(cfg.on_chat_enabled);
    assert_eq!(cfg.on_chat_max_age_secs, 14_400);
    assert!(
        cfg.wait_for_refresh,
        "default must be blocking so coaches don't read stale activity caches"
    );
    assert_eq!(cfg.wait_for_refresh_timeout_secs, 15);
    assert!(cfg.inject_agent_hint);
    assert!(cfg.providers.is_empty());
}

#[test]
fn provider_eligibility() {
    let cfg = RefreshConfig::default();
    assert!(cfg.is_provider_eligible("strava")); // empty list = all eligible

    let cfg = RefreshConfig {
        providers: vec!["strava".to_owned(), "whoop".to_owned()],
        ..Default::default()
    };
    assert!(cfg.is_provider_eligible("strava"));
    assert!(!cfg.is_provider_eligible("garmin"));
}

#[test]
fn should_refresh_on_chat() {
    let cfg = RefreshConfig::default();
    assert!(!cfg.should_refresh_on_chat(Duration::from_hours(1))); // 1h < 4h threshold
    assert!(cfg.should_refresh_on_chat(Duration::from_secs(14_401))); // > 4h

    let disabled = RefreshConfig {
        on_chat_enabled: false,
        ..Default::default()
    };
    assert!(!disabled.should_refresh_on_chat(Duration::from_secs(999_999)));
}

#[test]
fn scheduled_refresh_config_defaults() {
    let cfg = ScheduledRefreshConfig::default();
    assert!(cfg.enabled);
    assert_eq!(cfg.poll_interval_secs, 3_600);
    assert_eq!(cfg.max_data_age_secs, 86_400);
    assert_eq!(cfg.max_concurrent_syncs, 10);
}

#[test]
fn smart_schedule_weights_defaults() {
    let w = SmartScheduleWeights::default();
    assert_eq!(w.daily_trainer_threshold, 5);
    assert_eq!(w.daily_trainer_interval_secs, 3_600);
    assert_eq!(w.casual_interval_secs, 14_400);
    assert_eq!(w.default_interval_secs, 7_200);
}
