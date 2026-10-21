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
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use dravr_meteo::{WeatherError, WeatherProvider, WeatherQuery, WeatherSample};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_services::weather_backfill::fill_activity_temperatures;
use tokio::time::sleep;

/// Passes over the backfill before the test gives up.
///
/// Since 2026-10-16, Open-Meteo's geocoding API has sometimes held a request
/// from a GitHub runner until dravr-meteo's 10s request timeout. Five of
/// seven scheduled runs failed after exactly 10s or 20s, while the same
/// lookup answers in under a second from a workstation (carnet#1121).
///
/// The backfill drops a city it cannot geocode instead of returning the
/// error, so a pass that resolves fewer cities than it was given is the
/// only signal available. The geocoder caches resolutions but not failures,
/// so a later pass asks again only for the cities still missing. A city that
/// never resolves fails every pass, and the test still fails: nothing here
/// skips (carnet#805).
const GEOCODE_ATTEMPTS: usize = 3;
const GEOCODE_BACKOFF: Duration = Duration::from_secs(15);

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

    let mut calls_before_pass = 0;
    let mut result = fill_activity_temperatures(&activities, provider.clone()).await;
    let mut attempt = 1;
    while result.len() < activities.len() && attempt < GEOCODE_ATTEMPTS {
        eprintln!(
            "geocode attempt {attempt} resolved {} of {} cities, retrying",
            result.len(),
            activities.len()
        );
        attempt += 1;
        sleep(GEOCODE_BACKOFF).await;
        calls_before_pass = calls.load(Ordering::SeqCst);
        result = fill_activity_temperatures(&activities, provider.clone()).await;
    }

    assert_eq!(
        result.len(),
        2,
        "both city-only activities should geocode and resolve"
    );
    assert!((result["a1"] - -8.0).abs() < f32::EPSILON);
    assert!((result["a2"] - -8.0).abs() < f32::EPSILON);
    // The pass that resolved both cities asked the provider once for each.
    assert_eq!(calls.load(Ordering::SeqCst) - calls_before_pass, 2);
}
