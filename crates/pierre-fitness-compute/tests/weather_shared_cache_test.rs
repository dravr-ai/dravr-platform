// ABOUTME: Pins that build_provider's cache is the shared Arc it is handed, read before the vendor
// ABOUTME: A sample already stored for the place and hour comes back without a network call
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `build_provider` takes the shared cache as `Arc<dyn WeatherCacheStore>` and
//! hands it to `CachedProvider` directly, through dravr-meteo's
//! `WeatherCacheStore for Arc<T>` (v0.3.0), where it used to wrap it in a local
//! newtype. carnet#533. The sample stored here names conditions no vendor
//! reports, so a lookup that went past the cache could not produce it.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(missing_docs)]

use std::env;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use pierre_fitness_compute::weather::build_provider;
use pierre_weather::{CacheKey, InMemoryStore, WeatherCacheStore, WeatherQuery, WeatherSample};

#[tokio::test]
async fn a_stored_sample_is_answered_from_the_shared_cache() {
    env::remove_var("WEATHER_PROVIDER");
    let store = Arc::new(InMemoryStore::new());
    let shared: Arc<dyn WeatherCacheStore> = store.clone();
    let provider = build_provider(shared).expect("the default builds");

    let query = WeatherQuery {
        latitude: 45.5017,
        longitude: -73.5673,
        timestamp: Utc.with_ymd_and_hms(2026, 9, 20, 14, 0, 0).unwrap(),
    };
    let stored = WeatherSample {
        temperature_celsius: 12.8,
        humidity_percentage: Some(61.0),
        wind_speed_kmh: Some(14.5),
        conditions: "cache-only-fixture".to_owned(),
    };
    store
        .put(CacheKey::from_query(&query), provider.name(), stored)
        .await
        .expect("put");

    let answered = provider.weather_at(query).await.expect("cache hit");
    assert_eq!(answered.conditions, "cache-only-fixture");
    assert!((answered.temperature_celsius - 12.8).abs() < f32::EPSILON);
}
