// ABOUTME: Tests for the weather_backfill orchestrator
// ABOUTME: Verifies skip-when-set, no-coords skip, parallel fan-out, and partial-fill tolerance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use dravr_meteo::{WeatherError, WeatherProvider, WeatherQuery, WeatherSample};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_mcp_server::services::weather_backfill::fill_activity_temperatures;

struct ConstantProvider {
    temp: f32,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl WeatherProvider for ConstantProvider {
    async fn weather_at(&self, _query: WeatherQuery) -> Result<WeatherSample, WeatherError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(WeatherSample {
            temperature_celsius: self.temp,
            humidity_percentage: None,
            wind_speed_kmh: None,
            conditions: "clear".into(),
        })
    }

    fn name(&self) -> &'static str {
        "constant"
    }
}

struct FailingProvider;

#[async_trait]
impl WeatherProvider for FailingProvider {
    async fn weather_at(&self, _query: WeatherQuery) -> Result<WeatherSample, WeatherError> {
        Err(WeatherError::DataUnavailable)
    }

    fn name(&self) -> &'static str {
        "failing"
    }
}

fn build_activity_with_gps(id: &str) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        "Test Activity".to_owned(),
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 4, 30, 8, 0, 0).unwrap(),
        3600,
        "test",
    )
    .start_latitude(45.5017)
    .start_longitude(-73.5673)
    .build()
}

fn build_activity_with_temp(id: &str, temp: f32) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        "Test Activity".to_owned(),
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 4, 30, 8, 0, 0).unwrap(),
        3600,
        "test",
    )
    .start_latitude(45.5017)
    .start_longitude(-73.5673)
    .temperature(temp)
    .build()
}

fn build_activity_no_gps(id: &str) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        "Indoor Activity".to_owned(),
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 4, 30, 8, 0, 0).unwrap(),
        3600,
        "test",
    )
    .build()
}

fn build_activity_city_only(id: &str, city: &str, region: &str) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        "City-only Activity".to_owned(),
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 4, 30, 8, 0, 0).unwrap(),
        3600,
        "test",
    )
    .city(city.to_owned())
    .region(region.to_owned())
    .build()
}

#[tokio::test]
async fn fills_temperature_for_activities_missing_temp_with_gps() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn WeatherProvider> = Arc::new(ConstantProvider {
        temp: -3.5,
        calls: calls.clone(),
    });

    let activities = vec![
        build_activity_with_gps("a1"),
        build_activity_with_gps("a2"),
        build_activity_with_gps("a3"),
    ];

    let result = fill_activity_temperatures(&activities, provider).await;

    assert_eq!(result.len(), 3);
    assert!((result["a1"] - -3.5).abs() < f32::EPSILON);
    assert!((result["a2"] - -3.5).abs() < f32::EPSILON);
    assert!((result["a3"] - -3.5).abs() < f32::EPSILON);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn skips_activities_with_existing_temperature() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn WeatherProvider> = Arc::new(ConstantProvider {
        temp: 5.0,
        calls: calls.clone(),
    });

    let activities = vec![
        build_activity_with_temp("warm", 22.0),
        build_activity_with_gps("cold"),
    ];

    let result = fill_activity_temperatures(&activities, provider).await;

    assert_eq!(result.len(), 1);
    assert!(result.contains_key("cold"));
    assert!(!result.contains_key("warm"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn skips_activities_without_gps() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn WeatherProvider> = Arc::new(ConstantProvider {
        temp: 10.0,
        calls: calls.clone(),
    });

    let activities = vec![
        build_activity_no_gps("indoor1"),
        build_activity_no_gps("indoor2"),
    ];

    let result = fill_activity_temperatures(&activities, provider).await;

    assert!(result.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// Live geocoding integration test against Open-Meteo's free public API.
/// Gated behind `RUN_NETWORK_TESTS=1` so CI's offline matrix doesn't
/// flake when the geocoding endpoint is rate-limited or unreachable.
/// Asserts that an activity carrying only `city` + `region` (the shape
/// produced by sciotte's dashboard-feed enrichment) does end up in the
/// backfill output once Open-Meteo resolves the coords.
#[tokio::test]
async fn fills_temperature_via_city_geocoding_when_gps_absent() {
    if env::var("RUN_NETWORK_TESTS").is_err() {
        eprintln!("skipping: set RUN_NETWORK_TESTS=1 to enable Open-Meteo geocoding test");
        return;
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn WeatherProvider> = Arc::new(ConstantProvider {
        temp: -8.0,
        calls: calls.clone(),
    });

    let activities = vec![
        build_activity_city_only("a1", "Prévost", "Quebec"),
        build_activity_city_only("a2", "Montreal", "Quebec"),
    ];

    let result = fill_activity_temperatures(&activities, provider).await;

    assert_eq!(
        result.len(),
        2,
        "both city-only activities should geocode and resolve"
    );
    assert!((result["a1"] - -8.0).abs() < f32::EPSILON);
    assert!((result["a2"] - -8.0).abs() < f32::EPSILON);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn tolerates_provider_failures_returning_partial_fill() {
    let provider: Arc<dyn WeatherProvider> = Arc::new(FailingProvider);

    let activities = vec![build_activity_with_gps("a1"), build_activity_with_gps("a2")];

    let result = fill_activity_temperatures(&activities, provider).await;

    assert!(result.is_empty());
}

#[tokio::test]
async fn returns_empty_when_no_candidates() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn WeatherProvider> = Arc::new(ConstantProvider {
        temp: 0.0,
        calls: calls.clone(),
    });

    let activities: Vec<Activity> = vec![];

    let result = fill_activity_temperatures(&activities, provider).await;

    assert!(result.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
