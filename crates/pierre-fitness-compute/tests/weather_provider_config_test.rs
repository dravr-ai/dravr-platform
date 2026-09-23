// ABOUTME: Pins which WEATHER_PROVIDER / OPENWEATHER_API_KEY combinations build a provider
// ABOUTME: A config that cannot work is refused instead of built with an empty key or a silent fallback
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `build_provider` used to accept every configuration: `openweathermap`
//! without a key built a provider holding `""` that failed every lookup at the
//! vendor, and an unknown vendor name fell through to Open-Meteo without a
//! word. carnet#531.
//!
//! One test function on purpose: every case mutates the same two environment
//! variables, and test functions in a binary run in parallel.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::sync::Arc;

use pierre_fitness_compute::weather::{build_provider, WeatherConfigError};
use pierre_weather::{InMemoryStore, WeatherCacheStore};

fn cache() -> Arc<dyn WeatherCacheStore> {
    Arc::new(InMemoryStore::new())
}

#[test]
fn only_a_workable_weather_configuration_builds_a_provider() {
    env::remove_var("WEATHER_PROVIDER");
    env::remove_var("OPENWEATHER_API_KEY");
    let provider = build_provider(cache()).expect("the default builds");
    assert_eq!(provider.name(), "openmeteo");

    env::set_var("WEATHER_PROVIDER", "openweathermap");
    assert_eq!(
        build_provider(cache()).err(),
        Some(WeatherConfigError::MissingOpenWeatherKey),
        "openweathermap without a key must be refused, not built with an empty key"
    );
    env::set_var("OPENWEATHER_API_KEY", "");
    assert_eq!(
        build_provider(cache()).err(),
        Some(WeatherConfigError::MissingOpenWeatherKey)
    );

    env::set_var("OPENWEATHER_API_KEY", "owm-test-key");
    assert_eq!(
        build_provider(cache()).expect("keyed builds").name(),
        "openweathermap"
    );

    env::set_var("WEATHER_PROVIDER", "open-meteo");
    assert_eq!(
        build_provider(cache()).err(),
        Some(WeatherConfigError::UnknownProvider("open-meteo".to_owned())),
        "a typo must be refused, not quietly served by Open-Meteo"
    );

    env::remove_var("WEATHER_PROVIDER");
    env::remove_var("OPENWEATHER_API_KEY");
}
