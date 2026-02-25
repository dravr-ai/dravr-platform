// ABOUTME: Configurable timeout utilities for route handlers to prevent hanging operations
// ABOUTME: Provides timeout duration accessors for geocoding and MCP sampling operations

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::environment::RouteTimeoutConfig;
use std::sync::OnceLock;
use std::time::Duration;

/// Global route timeout configuration
static ROUTE_TIMEOUT_CONFIG: OnceLock<RouteTimeoutConfig> = OnceLock::new();

/// Initialize route timeout configuration
///
/// Must be called once at server startup before any route handlers use timeouts.
///
/// # Panics
/// Panics if called more than once (configuration cannot be changed after initialization)
pub fn initialize_route_timeouts(config: RouteTimeoutConfig) {
    assert!(
        ROUTE_TIMEOUT_CONFIG.set(config).is_ok(),
        "Route timeout configuration already initialized"
    );
}

/// Get the current route timeout configuration with fallback to defaults
///
/// Returns defaults if route timeout configuration was not initialized at server startup
fn get_config() -> &'static RouteTimeoutConfig {
    static DEFAULT_CONFIG: OnceLock<RouteTimeoutConfig> = OnceLock::new();
    ROUTE_TIMEOUT_CONFIG
        .get()
        .unwrap_or_else(|| DEFAULT_CONFIG.get_or_init(RouteTimeoutConfig::default))
}

/// Get MCP sampling timeout duration for manual timeout handling
///
/// # Returns
/// Duration for MCP sampling operations
#[must_use]
pub fn mcp_sampling_timeout_duration() -> Duration {
    Duration::from_secs(get_config().mcp_sampling_timeout_secs)
}

/// Get geocoding timeout duration for manual timeout handling
///
/// # Returns
/// Duration for geocoding operations
#[must_use]
pub fn geocoding_timeout_duration() -> Duration {
    Duration::from_secs(get_config().geocoding_timeout_secs)
}
