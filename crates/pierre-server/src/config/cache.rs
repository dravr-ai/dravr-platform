// ABOUTME: Cache and rate limiting configuration types
// ABOUTME: Handles Redis connections, cache TTLs, and rate limiting settings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use pierre_cache::redis_config::RedisConnectionConfig;

use crate::constants::cache;
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
    /// Cache TTL configuration
    #[serde(default)]
    pub ttl: CacheTtlConfig,
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
            ttl: CacheTtlConfig::from_env(),
        }
    }
}

/// Cache TTL configuration for different resource types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheTtlConfig {
    /// Athlete profile cache TTL in seconds (default: 24 hours)
    pub profile_secs: u64,
    /// Activity list cache TTL in seconds (default: 15 minutes)
    pub activity_list_secs: u64,
    /// Individual activity cache TTL in seconds (default: 1 hour)
    pub activity_secs: u64,
    /// Stats cache TTL in seconds (default: 6 hours)
    pub stats_secs: u64,
}

impl Default for CacheTtlConfig {
    fn default() -> Self {
        Self {
            profile_secs: cache::TTL_PROFILE_SECS,
            activity_list_secs: cache::TTL_ACTIVITY_LIST_SECS,
            activity_secs: cache::TTL_ACTIVITY_SECS,
            stats_secs: cache::TTL_STATS_SECS,
        }
    }
}

impl CacheTtlConfig {
    /// Load cache TTL configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            profile_secs: env::var("CACHE_TTL_PROFILE_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(cache::TTL_PROFILE_SECS),
            activity_list_secs: env::var("CACHE_TTL_ACTIVITY_LIST_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(cache::TTL_ACTIVITY_LIST_SECS),
            activity_secs: env::var("CACHE_TTL_ACTIVITY_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(cache::TTL_ACTIVITY_SECS),
            stats_secs: env::var("CACHE_TTL_STATS_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(cache::TTL_STATS_SECS),
        }
    }
}

/// Rate limiting configuration - re-exported from pierre-auth
pub use pierre_auth::config::RateLimitConfig;
