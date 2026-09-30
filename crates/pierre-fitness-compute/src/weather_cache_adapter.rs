// ABOUTME: Bridges pierre-database WeatherCacheRepository to dravr-meteo WeatherCacheStore
// ABOUTME: Lets the dravr-meteo CachedProvider decorator use the platform's persistent cache
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Adapter wrapping an `Arc<dyn WeatherCacheRepository>` so it satisfies
//! the `dravr_meteo::WeatherCacheStore` trait expected by the
//! `CachedProvider` decorator.
//!
//! Storage stays in pierre-database (`SQLite` or `PostgreSQL`); dravr-meteo
//! stays vendor-agnostic. Cache faults are logged but never propagate —
//! a cache hiccup must not break weather lookups.

use std::sync::Arc;

use async_trait::async_trait;
use dravr_meteo::{CacheError, CacheKey, WeatherCacheStore, WeatherSample};
use pierre_database::repositories::{WeatherCacheEntry, WeatherCacheRepository};

/// Adapter that lets a database-backed `WeatherCacheRepository` satisfy
/// the dravr-meteo `WeatherCacheStore` trait.
pub struct WeatherCacheRepoAdapter {
    repo: Arc<dyn WeatherCacheRepository>,
}

impl WeatherCacheRepoAdapter {
    /// Wrap an existing repository handle.
    #[must_use]
    pub const fn new(repo: Arc<dyn WeatherCacheRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl WeatherCacheStore for WeatherCacheRepoAdapter {
    async fn get(
        &self,
        key: &CacheKey,
        provider: &str,
    ) -> Result<Option<WeatherSample>, CacheError> {
        let entry = self
            .repo
            .get(key.lat_centi, key.lng_centi, key.hour_unix, provider)
            .await
            .map_err(|e| CacheError::Store(format!("weather_cache repo get failed: {e}")))?;

        Ok(entry.map(entry_to_sample))
    }

    async fn put(
        &self,
        key: CacheKey,
        provider: &str,
        sample: WeatherSample,
    ) -> Result<(), CacheError> {
        let entry = WeatherCacheEntry {
            lat_centi: key.lat_centi,
            lng_centi: key.lng_centi,
            hour_unix: key.hour_unix,
            provider: provider.to_owned(),
            temperature_celsius: sample.temperature_celsius,
            humidity_percentage: sample.humidity_percentage,
            wind_speed_kmh: sample.wind_speed_kmh,
            conditions: sample.conditions,
        };

        self.repo
            .put(entry)
            .await
            .map_err(|e| CacheError::Store(format!("weather_cache repo put failed: {e}")))
    }
}

fn entry_to_sample(entry: WeatherCacheEntry) -> WeatherSample {
    WeatherSample {
        temperature_celsius: entry.temperature_celsius,
        humidity_percentage: entry.humidity_percentage,
        wind_speed_kmh: entry.wind_speed_kmh,
        conditions: entry.conditions,
    }
}
