// ABOUTME: PostgreSQL connection pool configuration types for pierre-core
// ABOUTME: Provides pool sizing, timeouts, and retry settings loaded from environment
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! PostgreSQL connection pool configuration
//!
//! Provides the `PostgresPoolConfig` struct with sensible defaults and environment
//! variable overrides for connection pool sizing, timeouts, and retry behavior.

use std::env;

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
        }
    }
}

impl PostgresPoolConfig {
    /// Load `PostgreSQL` pool configuration from environment (or defaults)
    #[must_use]
    pub fn from_env() -> Self {
        let is_ci = env::var("CI").is_ok();
        Self {
            max_connections: env::var("POSTGRES_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(if is_ci { 3 } else { 10 }),
            min_connections: env::var("POSTGRES_MIN_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(if is_ci { 1 } else { 2 }),
            acquire_timeout_secs: env::var("POSTGRES_ACQUIRE_TIMEOUT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(if is_ci { 20 } else { 30 }),
            connection_retries: env::var("POSTGRES_CONNECTION_RETRIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(database::CONNECTION_RETRIES),
            initial_retry_delay_ms: env::var("POSTGRES_INITIAL_RETRY_DELAY_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(database::INITIAL_RETRY_DELAY_MS),
            max_retry_delay_ms: env::var("POSTGRES_MAX_RETRY_DELAY_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(database::MAX_RETRY_DELAY_MS),
        }
    }
}
