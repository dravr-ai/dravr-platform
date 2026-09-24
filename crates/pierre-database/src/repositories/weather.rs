// ABOUTME: Repository trait definitions for the weather cache persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};

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

/// The eight columns a cache read returns, in the order
/// [`weather_entry_from_row`] reads them.
macro_rules! weather_columns {
    () => {
        "lat_centi, lng_centi, hour_unix, provider, \
         temperature_celsius, humidity_percentage, wind_speed_kmh, conditions"
    };
}

/// One hourly bucket for one location and provider.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, and every bind here is a plain `i32`/`i64`/`f32`/`&str`, so one
/// statement serves both backends and cannot drift between them.
pub(crate) const GET_WEATHER_SQL: &str = concat!(
    "SELECT ",
    weather_columns!(),
    " FROM weather_cache \
     WHERE lat_centi = $1 AND lng_centi = $2 AND hour_unix = $3 AND provider = $4"
);

/// Record a bucket, refreshing one already held for the same key.
///
/// `CURRENT_TIMESTAMP` rather than `datetime('now')` or `NOW()`: it is the
/// standard spelling both engines answer, so the statement needs no
/// per-backend fragment.
pub(crate) const PUT_WEATHER_SQL: &str = concat!(
    "INSERT INTO weather_cache (",
    weather_columns!(),
    ", cached_at) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, CURRENT_TIMESTAMP) \
     ON CONFLICT (lat_centi, lng_centi, hour_unix, provider) DO UPDATE SET \
        temperature_celsius = excluded.temperature_celsius, \
        humidity_percentage = excluded.humidity_percentage, \
        wind_speed_kmh      = excluded.wind_speed_kmh, \
        conditions          = excluded.conditions, \
        cached_at           = excluded.cached_at"
);

/// Extract a [`WeatherCacheEntry`] from a row of either backend via `try_get`
/// only — `Row::get` is `try_get().unwrap()` and panics the read path on a
/// width or NULL surprise.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn weather_entry_from_row<R>(row: &R) -> AppResult<WeatherCacheEntry>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f32>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str, e: sqlx::Error| AppError::database(format!("weather_cache {name}: {e}"));
    Ok(WeatherCacheEntry {
        lat_centi: row.try_get("lat_centi").map_err(|e| col("lat_centi", e))?,
        lng_centi: row.try_get("lng_centi").map_err(|e| col("lng_centi", e))?,
        hour_unix: row.try_get("hour_unix").map_err(|e| col("hour_unix", e))?,
        provider: row.try_get("provider").map_err(|e| col("provider", e))?,
        temperature_celsius: row
            .try_get("temperature_celsius")
            .map_err(|e| col("temperature_celsius", e))?,
        humidity_percentage: row
            .try_get("humidity_percentage")
            .map_err(|e| col("humidity_percentage", e))?,
        wind_speed_kmh: row
            .try_get("wind_speed_kmh")
            .map_err(|e| col("wind_speed_kmh", e))?,
        conditions: row
            .try_get("conditions")
            .map_err(|e| col("conditions", e))?,
    })
}

/// Emit the whole [`WeatherCacheRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_weather_cache_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl WeatherCacheRepository for $ty {
            async fn get(
                &self,
                lat_centi: i32,
                lng_centi: i32,
                hour_unix: i64,
                provider: &str,
            ) -> AppResult<Option<WeatherCacheEntry>> {
                let row = sqlx::query(GET_WEATHER_SQL)
                    .bind(lat_centi)
                    .bind(lng_centi)
                    .bind(hour_unix)
                    .bind(provider)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read weather_cache: {e}"))
                    })?;
                row.as_ref().map(weather_entry_from_row).transpose()
            }

            async fn put(&self, entry: WeatherCacheEntry) -> AppResult<()> {
                sqlx::query(PUT_WEATHER_SQL)
                    .bind(entry.lat_centi)
                    .bind(entry.lng_centi)
                    .bind(entry.hour_unix)
                    .bind(&entry.provider)
                    .bind(entry.temperature_celsius)
                    .bind(entry.humidity_percentage)
                    .bind(entry.wind_speed_kmh)
                    .bind(&entry.conditions)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert weather_cache: {e}"))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_weather_cache_repository;
