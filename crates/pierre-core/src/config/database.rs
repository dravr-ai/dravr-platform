// ABOUTME: PostgreSQL connection pool configuration types for pierre-core
// ABOUTME: Provides pool sizing, timeouts, and retry settings loaded from environment
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! PostgreSQL connection pool configuration
//!
//! Provides the `PostgresPoolConfig` struct with sensible defaults and environment
//! variable overrides for connection pool sizing, timeouts, connection recycling,
//! retry behavior, and how long a booting instance queues for a free connection.

use std::env;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::constants::database;

/// `PostgreSQL` connection pool configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresPoolConfig {
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Minimum number of connections in the pool
    pub min_connections: u32,
    /// Connection acquire timeout in seconds
    pub acquire_timeout_secs: u64,
    /// Number of connection retries on startup
    pub connection_retries: u32,
    /// Initial retry delay in milliseconds (doubles with exponential backoff)
    pub initial_retry_delay_ms: u64,
    /// Maximum retry delay in milliseconds
    pub max_retry_delay_ms: u64,
    /// Seconds a connection may sit idle before the pool closes it
    /// (`SQLX_IDLE_TIMEOUT_SECS`).
    ///
    /// This is how fast a revision that lost its traffic gives its database
    /// slots back during a deploy (carnet#1129).
    pub idle_timeout_secs: u64,
    /// Seconds a connection may live before the pool replaces it
    /// (`SQLX_MAX_LIFETIME_SECS`)
    pub max_lifetime_secs: u64,
    /// Whether the pool pings a connection before handing it out
    /// (`SQLX_TEST_BEFORE_ACQUIRE`)
    pub test_before_acquire: bool,
    /// Prepared statements cached per connection
    /// (`SQLX_STATEMENT_CACHE_CAPACITY`)
    pub statement_cache_capacity: usize,
    /// Seconds a booting instance keeps retrying its key-store reads while
    /// the database has no free connection (`POSTGRES_BOOT_WAIT_SECS`).
    ///
    /// The window starts when the pool is created. Keep it inside the startup
    /// probe's budget, or the platform kills the instance while it waits.
    pub boot_wait_secs: u64,
}

/// Defaults of [`PostgresPoolConfig::default`], which
/// [`PostgresPoolConfig::from_env`] falls back to.
mod defaults {
    /// Five minutes: a connection idle that long is closed, so a quiet
    /// instance does not hold database slots it is not using
    pub const IDLE_TIMEOUT_SECS: u64 = 300;
    /// Ten minutes: a connection is closed and replaced after that long,
    /// however busy it is
    pub const MAX_LIFETIME_SECS: u64 = 600;
    /// `SQLx`'s own default statement cache size
    pub const STATEMENT_CACHE_CAPACITY: usize = 100;
    /// One minute of queueing for a free connection at boot
    pub const BOOT_WAIT_SECS: u64 = 60;
}

impl Default for PostgresPoolConfig {
    fn default() -> Self {
        // CI environment detection at config load time
        let is_ci = env::var("CI").is_ok();
        Self {
            max_connections: if is_ci { 3 } else { 10 },
            min_connections: if is_ci { 1 } else { 2 },
            acquire_timeout_secs: if is_ci { 20 } else { 30 },
            connection_retries: database::CONNECTION_RETRIES,
            initial_retry_delay_ms: database::INITIAL_RETRY_DELAY_MS,
            max_retry_delay_ms: database::MAX_RETRY_DELAY_MS,
            idle_timeout_secs: defaults::IDLE_TIMEOUT_SECS,
            max_lifetime_secs: defaults::MAX_LIFETIME_SECS,
            test_before_acquire: true,
            statement_cache_capacity: defaults::STATEMENT_CACHE_CAPACITY,
            boot_wait_secs: defaults::BOOT_WAIT_SECS,
        }
    }
}

/// Read `name` from the environment, falling back to `default` when it is
/// unset or does not parse
fn env_or<T: FromStr>(name: &str, default: T) -> T {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl PostgresPoolConfig {
    /// Load `PostgreSQL` pool configuration from environment (or defaults)
    #[must_use]
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            max_connections: env_or("POSTGRES_MAX_CONNECTIONS", defaults.max_connections),
            min_connections: env_or("POSTGRES_MIN_CONNECTIONS", defaults.min_connections),
            acquire_timeout_secs: env_or("POSTGRES_ACQUIRE_TIMEOUT", defaults.acquire_timeout_secs),
            connection_retries: env_or("POSTGRES_CONNECTION_RETRIES", defaults.connection_retries),
            initial_retry_delay_ms: env_or(
                "POSTGRES_INITIAL_RETRY_DELAY_MS",
                defaults.initial_retry_delay_ms,
            ),
            max_retry_delay_ms: env_or("POSTGRES_MAX_RETRY_DELAY_MS", defaults.max_retry_delay_ms),
            idle_timeout_secs: env_or("SQLX_IDLE_TIMEOUT_SECS", defaults.idle_timeout_secs),
            max_lifetime_secs: env_or("SQLX_MAX_LIFETIME_SECS", defaults.max_lifetime_secs),
            test_before_acquire: env_or("SQLX_TEST_BEFORE_ACQUIRE", defaults.test_before_acquire),
            statement_cache_capacity: env_or(
                "SQLX_STATEMENT_CACHE_CAPACITY",
                defaults.statement_cache_capacity,
            ),
            boot_wait_secs: env_or("POSTGRES_BOOT_WAIT_SECS", defaults.boot_wait_secs),
        }
    }
}
