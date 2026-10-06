// ABOUTME: Live weather backfill against Open-Meteo's public geocoding API
// ABOUTME: Built only with the live-e2e feature; fails, never skips, when the API is unreachable
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! ```bash
//! cargo test --features live-e2e --test weather_geocoding_live_test
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use dravr_meteo::{WeatherError, WeatherProvider, WeatherQuery, WeatherSample};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_services::weather_backfill::fill_activity_temperatures;

/// Answers every weather lookup with one temperature, so the assertion is
/// about geocoding reaching the provider, not about the forecast.
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

/// Live geocoding integration test against Open-Meteo's free public API.
/// Built only with the `live-e2e` feature so the offline matrix never
/// depends on the endpoint; built, an unreachable endpoint fails it.
/// Asserts that an activity carrying only `city` + `region` (the shape
/// produced by sciotte's dashboard-feed enrichment) does end up in the
/// backfill output once Open-Meteo resolves the coords.
#[tokio::test]
async fn fills_temperature_via_city_geocoding_when_gps_absent() {
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
