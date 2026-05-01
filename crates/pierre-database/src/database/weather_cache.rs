// ABOUTME: SQLite-backed WeatherCacheRepository for dravr-meteo persistent cache
// ABOUTME: Geographic + hourly bucket; not tenant-scoped (weather is shared by location)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::{WeatherCacheEntry, WeatherCacheRepository};
use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;

#[async_trait]
impl WeatherCacheRepository for Database {
    async fn get(
        &self,
        lat_centi: i32,
        lng_centi: i32,
        hour_unix: i64,
        provider: &str,
    ) -> AppResult<Option<WeatherCacheEntry>> {
        let row = sqlx::query(
            r"
            SELECT lat_centi, lng_centi, hour_unix, provider,
                   temperature_celsius, humidity_percentage, wind_speed_kmh, conditions
            FROM weather_cache
            WHERE lat_centi = ?1 AND lng_centi = ?2 AND hour_unix = ?3 AND provider = ?4
            ",
        )
        .bind(lat_centi)
        .bind(lng_centi)
        .bind(hour_unix)
        .bind(provider)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to read weather_cache: {e}")))?;

        let Some(row) = row else { return Ok(None) };

        Ok(Some(WeatherCacheEntry {
            lat_centi: row.get("lat_centi"),
            lng_centi: row.get("lng_centi"),
            hour_unix: row.get("hour_unix"),
            provider: row.get("provider"),
            temperature_celsius: row.get("temperature_celsius"),
            humidity_percentage: row.get("humidity_percentage"),
            wind_speed_kmh: row.get("wind_speed_kmh"),
            conditions: row.get("conditions"),
        }))
    }

    async fn put(&self, entry: WeatherCacheEntry) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO weather_cache (
                lat_centi, lng_centi, hour_unix, provider,
                temperature_celsius, humidity_percentage, wind_speed_kmh, conditions, cached_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
            ON CONFLICT(lat_centi, lng_centi, hour_unix, provider) DO UPDATE SET
                temperature_celsius = excluded.temperature_celsius,
                humidity_percentage = excluded.humidity_percentage,
                wind_speed_kmh      = excluded.wind_speed_kmh,
                conditions          = excluded.conditions,
                cached_at           = excluded.cached_at
            ",
        )
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
        .map_err(|e| AppError::database(format!("Failed to upsert weather_cache: {e}")))?;

        Ok(())
    }
}
