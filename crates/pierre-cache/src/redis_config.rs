// ABOUTME: Redis connection and retry configuration for cache backends
// ABOUTME: Defines timeouts, retry policies, and exponential backoff settings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::constants::redis;
use serde::{Deserialize, Serialize};
use std::env;

/// Redis connection and retry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConnectionConfig {
    /// Connection timeout in seconds
    pub connection_timeout_secs: u64,
    /// Response/command timeout in seconds
    pub response_timeout_secs: u64,
    /// Number of reconnection retries after connection drop
    pub reconnection_retries: usize,
    /// Exponential backoff base for retry delays
    pub retry_exponent_base: u64,
    /// Maximum retry delay in milliseconds
    pub max_retry_delay_ms: u64,
    /// Number of retries for initial connection at startup
    pub initial_connection_retries: u32,
    /// Initial retry delay in milliseconds (doubles with exponential backoff)
    pub initial_retry_delay_ms: u64,
}

impl Default for RedisConnectionConfig {
    fn default() -> Self {
        Self {
            connection_timeout_secs: redis::CONNECTION_TIMEOUT_SECS,
            response_timeout_secs: redis::RESPONSE_TIMEOUT_SECS,
            reconnection_retries: redis::RECONNECTION_RETRIES,
            retry_exponent_base: redis::RETRY_EXPONENT_BASE,
            max_retry_delay_ms: redis::MAX_RETRY_DELAY_MS,
            initial_connection_retries: redis::INITIAL_CONNECTION_RETRIES,
            initial_retry_delay_ms: 500, // Same as database default
        }
    }
}

impl RedisConnectionConfig {
    /// Load Redis connection configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            connection_timeout_secs: env::var("REDIS_CONNECTION_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(redis::CONNECTION_TIMEOUT_SECS),
            response_timeout_secs: env::var("REDIS_RESPONSE_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(redis::RESPONSE_TIMEOUT_SECS),
            reconnection_retries: env::var("REDIS_RECONNECTION_RETRIES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(redis::RECONNECTION_RETRIES),
            retry_exponent_base: env::var("REDIS_RETRY_EXPONENT_BASE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(redis::RETRY_EXPONENT_BASE),
            max_retry_delay_ms: env::var("REDIS_MAX_RETRY_DELAY_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(redis::MAX_RETRY_DELAY_MS),
            initial_connection_retries: env::var("REDIS_INITIAL_CONNECTION_RETRIES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(redis::INITIAL_CONNECTION_RETRIES),
            initial_retry_delay_ms: env::var("REDIS_INITIAL_RETRY_DELAY_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(500),
        }
    }
}
