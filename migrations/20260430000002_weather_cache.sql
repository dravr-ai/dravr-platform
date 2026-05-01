-- ABOUTME: weather_cache table for dravr-meteo persistent backing store
-- ABOUTME: Geographic + hourly bucket; not tenant-scoped (weather is shared by location)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS weather_cache (
    lat_centi             INTEGER NOT NULL,
    lng_centi             INTEGER NOT NULL,
    hour_unix             INTEGER NOT NULL,
    provider              TEXT    NOT NULL,
    temperature_celsius   REAL    NOT NULL,
    humidity_percentage   REAL,
    wind_speed_kmh        REAL,
    conditions            TEXT    NOT NULL,
    cached_at             TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (lat_centi, lng_centi, hour_unix, provider)
);

CREATE INDEX IF NOT EXISTS idx_weather_cache_age ON weather_cache(cached_at);
