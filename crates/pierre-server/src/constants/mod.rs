// ABOUTME: Constants module with domain-separated organization
// ABOUTME: Replaces the 933-line dumping ground with organized domain modules
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Constants module
//!
//! This module organizes application constants by domain for better maintainability.
//! Constants are grouped into logical domains rather than being in a single large file.

use crate::config::environment::ServerConfig;
use crate::errors::{AppError, AppResult};
use std::sync::OnceLock;

/// Static server configuration loaded once at startup
static SERVER_CONFIG: OnceLock<ServerConfig> = OnceLock::new();

/// Initialize server configuration (must be called once at server startup before `env_config` functions)
///
/// # Errors
///
/// Returns error if `ServerConfig` initialization fails or if called more than once
pub fn init_server_config() -> AppResult<()> {
    let config = ServerConfig::from_env()?;

    SERVER_CONFIG
        .set(config)
        .map_err(|_| AppError::internal("Server configuration already initialized"))?;

    Ok(())
}

/// Get reference to the static server configuration
///
/// Returns `None` if called before `init_server_config()`.
/// In production, `init_server_config()` should be called during server startup,
/// so this should never return `None` during normal operation.
#[must_use]
pub fn get_server_config() -> Option<&'static ServerConfig> {
    SERVER_CONFIG.get()
}

/// Try to get reference to the static server configuration without panicking
///
/// Returns `None` if `init_server_config()` hasn't been called yet (e.g., in tests)
#[must_use]
pub fn try_get_server_config() -> Option<&'static ServerConfig> {
    SERVER_CONFIG.get()
}

// Domain-specific modules

/// Cache-related constants (TTL, sizes, etc.)
pub mod cache;
/// Error codes and error-related constants
pub mod errors;
/// OAuth provider constants and configuration
pub mod oauth;
/// Protocol-specific constants for MCP
pub mod protocol;
/// Multi-protocol constants (A2A, MCP, etc.)
pub mod protocols;
/// Tool identifiers and tool-related constants
pub mod tools;
/// Unit conversion and measurement constants
pub mod units;

// Re-export commonly used items for easier access
pub use errors::*;
pub use oauth::*;
/// Tool-related constants re-export
pub use tools::*;
// Note: protocol and protocols are kept as modules to avoid conflicts

/// OAuth provider constants
pub mod oauth_providers {
    /// Re-export all OAuth constants
    pub use super::oauth::*;
}

// ============================================================================
// Re-exports from pierre_core::constants
// ============================================================================
// pierre-core owns the canonical definitions; pierre-server consumes them
// under the same module path. Adding a new constant means editing pierre-core.
pub use pierre_core::constants::{
    api_provider_limits, api_tier_limits, cache_config, configuration_system, crypto, database,
    defaults, endpoints, error_messages, goal_management, http_status, json_fields, key_prefixes,
    limits, mcp_transport, messages, oauth_config, oauth_rate_limiting, physiology, ports, project,
    rate_limit_headers, rate_limiting_bursts, rate_limits, redis, routes, security, service_names,
    sleep_recovery, status, system_config, system_monitoring, tiers, time, time_constants,
    timeouts, user_defaults,
};

/// Network configuration constants
///
/// Re-exports the shared transport/HTTP constants from pierre-core and adds
/// the AG-UI run replay buffer size, which is server-local.
pub mod network_config {
    pub use pierre_core::constants::network_config::*;

    /// Maximum number of recent AG-UI events retained per run for replay.
    ///
    /// A late-arriving or reconnecting SSE subscriber receives the
    /// buffered events before switching to live. Typical runs emit
    /// 6–20 events (`RUN_STARTED`, a handful of `STEP_*`, `RUN_FINISHED`)
    /// plus any tool-call + text-delta bursts; 256 leaves plenty of
    /// headroom before the oldest entries start dropping.
    pub const AGUI_RUN_REPLAY_BUFFER_SIZE: usize = 256;
}

/// Self-service password reset flow configuration
pub mod password_reset {
    /// Code expires after 15 minutes (shorter than admin-issued 1-hour tokens)
    pub const CODE_TTL_MINUTES: i64 = 15;

    /// Maximum reset codes a user can request per hour
    pub const MAX_CODES_PER_HOUR: i64 = 3;

    /// Lower bound of the 6-digit code range (inclusive)
    pub const CODE_RANGE_MIN: u32 = 100_000;

    /// Upper bound of the 6-digit code range (exclusive)
    pub const CODE_RANGE_MAX: u32 = 1_000_000;

    /// Label used as `created_by` for self-service reset tokens
    pub const CREATED_BY_SELF_SERVICE: &str = "self_service";
}
