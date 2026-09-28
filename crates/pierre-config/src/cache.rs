// ABOUTME: Cache and rate limiting configuration types
// ABOUTME: Handles Redis connections, cache TTLs, and rate limiting settings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use pierre_cache::redis_config::RedisConnectionConfig;

use serde::{Deserialize, Serialize};
use std::env;

/// Cache configuration for Redis and in-memory caching
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheConfig {
    /// Redis URL for distributed caching (optional)
    #[serde(default)]
    pub redis_url: Option<String>,
    /// Maximum number of entries in local cache
    #[serde(default)]
    pub max_entries: usize,
    /// Cache cleanup interval in seconds
    #[serde(default)]
    pub cleanup_interval_secs: u64,
    /// Redis connection configuration
    #[serde(default)]
    pub redis_connection: RedisConnectionConfig,
}

impl CacheConfig {
    /// Load cache configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            redis_url: env::var("REDIS_URL").ok(),
            max_entries: env::var("CACHE_MAX_ENTRIES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1000),
            cleanup_interval_secs: env::var("CACHE_CLEANUP_INTERVAL_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300),
            redis_connection: RedisConnectionConfig::from_env(),
        }
    }
}

/// Rate limiting configuration - re-exported from pierre-auth
pub use pierre_auth::config::RateLimitConfig;
