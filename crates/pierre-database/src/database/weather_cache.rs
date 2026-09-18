// ABOUTME: SQLite-backed WeatherCacheRepository, emitted from the shared implementation
// ABOUTME: Geographic + hourly bucket; not tenant-scoped (weather is shared by location)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use crate::database::Database;
use crate::repositories::weather::{
    impl_weather_cache_repository, weather_entry_from_row, WeatherCacheEntry,
    WeatherCacheRepository, GET_WEATHER_SQL, PUT_WEATHER_SQL,
};

impl_weather_cache_repository!(Database);
