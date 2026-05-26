// ABOUTME: Repository trait definitions for the weather cache persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

/// Persistent backing store row for the dravr-meteo weather cache.
///
/// Geographic + temporal bucket: lat/lng in centi-degrees (~1.1 km),
/// timestamp floored to the hour. The `provider` column scopes the
/// cache so `OpenMeteo` and `OpenWeatherMap` entries don't collide if a
/// vendor swap is in flight.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherCacheEntry {
    /// `round(latitude * 100)` — ~1.1 km bucket at the equator.
    pub lat_centi: i32,
    /// `round(longitude * 100)`.
    pub lng_centi: i32,
    /// `floor(unix_timestamp_secs / 3600)`.
    pub hour_unix: i64,
    /// Vendor name, e.g. `"openmeteo"` or `"openweathermap"`.
    pub provider: String,
    /// Ambient temperature in degrees Celsius.
    pub temperature_celsius: f32,
    /// Relative humidity, 0–100.
    pub humidity_percentage: Option<f32>,
    /// Wind speed in km/h.
    pub wind_speed_kmh: Option<f32>,
    /// Free-text condition summary (e.g. `"snow"`, `"clear sky"`).
    pub conditions: String,
}

/// Persistent storage for the dravr-meteo weather cache.
///
/// Not tenant-scoped — weather data is geographic and shared across
/// tenants by design. The `pierre-server::intelligence::weather_cache_adapter`
/// module bridges this trait to dravr-meteo's `WeatherCacheStore` trait.
#[async_trait]
pub trait WeatherCacheRepository: Send + Sync {
    /// Look up a sample by geographic + temporal bucket and provider.
    /// Returns `None` on miss.
    async fn get(
        &self,
        lat_centi: i32,
        lng_centi: i32,
        hour_unix: i64,
        provider: &str,
    ) -> AppResult<Option<WeatherCacheEntry>>;

    /// Persist (or replace) an entry. Upsert semantics — newer writes win.
    async fn put(&self, entry: WeatherCacheEntry) -> AppResult<()>;
}
