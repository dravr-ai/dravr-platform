// ABOUTME: Server-only cache helper: the env-driven cache factory
// ABOUTME: Cache types and CacheProvider trait now live in pierre_cache::*
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::time::Duration;

use pierre_cache::redis_config::RedisConnectionConfig;
use pierre_cache::{Cache, CacheConfig};
use pierre_core::errors::AppResult;

use crate::constants::get_server_config;

/// Build a [`Cache`] from environment-driven server configuration.
///
/// Supports both in-memory and Redis backends based on the `REDIS_URL` server
/// config field. Uses sensible defaults if server configuration is not yet
/// initialized (e.g. in tests that bypass `init_server_config`).
///
/// # Errors
///
/// Returns an error if cache initialization fails.
pub async fn cache_from_env() -> AppResult<Cache> {
    let config = get_server_config().map_or_else(
        || CacheConfig {
            max_entries: 1000,
            redis_url: None,
            cleanup_interval: Duration::from_mins(5),
            enable_background_cleanup: true,
            redis_connection: RedisConnectionConfig::default(),
        },
        |server_config| CacheConfig {
            max_entries: server_config.cache.max_entries,
            redis_url: server_config.cache.redis_url.clone(),
            cleanup_interval: Duration::from_secs(server_config.cache.cleanup_interval_secs),
            enable_background_cleanup: true,
            redis_connection: server_config.cache.redis_connection.clone(),
        },
    );

    Cache::new(config).await
}
