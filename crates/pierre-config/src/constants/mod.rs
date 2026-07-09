// ABOUTME: Constants module with domain-separated organization
// ABOUTME: Replaces the 933-line dumping ground with organized domain modules
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Constants module
//!
//! This module organizes application constants by domain for better maintainability.
//! Constants are grouped into logical domains rather than being in a single large file.

use crate::environment::ServerConfig;
use pierre_core::errors::{AppError, AppResult};
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

/// Server-side runtime-configurable MCP protocol values (server name, version).
///
/// Pure compile-time protocol constants live in `pierre_core::constants::protocol`
/// and are reachable via the parent re-export below.
pub mod protocol;

/// Usage-quota fallback defaults.
///
/// These mirror the canonical `ParameterDefinition` defaults in
/// [`crate::admin_definitions`]. Route handlers fall back to them when
/// the admin config service is unavailable, so enforcement and display
/// stay consistent with the registered defaults instead of diverging
/// per call site.
pub mod usage_quotas {
    /// Maximum concurrent active conversations per user. Mirrors the
    /// `usage_quotas.max_active_conversations` parameter default.
    pub const DEFAULT_MAX_ACTIVE_CONVERSATIONS: i64 = 10;
}

// ============================================================================
// Re-exports from pierre_core::constants
// ============================================================================
// pierre-core owns the canonical definitions; pierre-server consumes them
// under the same module path. Adding a new constant means editing pierre-core.
pub use pierre_core::constants::errors::*;
pub use pierre_core::constants::oauth::*;
/// Tool-related constants re-export
pub use pierre_core::constants::tools::*;
pub use pierre_core::constants::{
    api_provider_limits, api_tier_limits, cache, cache_config, configuration_system, crypto,
    database, defaults, endpoints, error_messages, errors, goal_management, http_status,
    json_fields, key_prefixes, limits, mcp_transport, messages, oauth, oauth_config,
    oauth_rate_limiting, physiology, ports, project, protocols, rate_limit_headers,
    rate_limiting_bursts, rate_limits, redis, routes, security, service_names, sleep_recovery,
    status, system_config, system_monitoring, tiers, time, time_constants, timeouts, tools, units,
    user_defaults,
};
// Note: `protocol` (local server-runtime helpers) and `protocols` (compile-time
// re-export from pierre-core) are kept as modules to avoid identifier conflicts.

/// OAuth provider constants
pub mod oauth_providers {
    /// Re-export all OAuth constants
    pub use pierre_core::constants::oauth::*;
}

/// Network configuration constants
///
/// Re-exports the shared transport/HTTP constants from pierre-core.
/// AG-UI-specific constants live in the `pierre-agui` crate.
pub mod network_config {
    pub use pierre_core::constants::network_config::*;
}

/// Self-service password reset flow configuration
pub mod password_reset {
    /// Code expires after 15 minutes (shorter than admin-issued 1-hour tokens)
    pub const CODE_TTL_MINUTES: i64 = 15;

    /// Maximum reset codes a user can request per hour
    pub const MAX_CODES_PER_HOUR: i64 = 3;

    /// Length of the plaintext `selector` (lookup half of the reset token).
    /// High enough to be unguessable as an index; stored in plaintext.
    pub const SELECTOR_LEN: usize = 16;

    /// Length of the `verifier` (secret half of the reset token).
    ///
    /// Only its SHA-256 hash is stored; ~190 bits of entropy makes the token unguessable,
    /// retiring the old 6-digit code that was brute-forceable (T3MP3ST F1 / CWE-307).
    pub const VERIFIER_LEN: usize = 32;

    /// Delimiter joining `<selector>.<verifier>` in the delivered reset token.
    pub const TOKEN_DELIMITER: char = '.';

    /// Label used as `created_by` for self-service reset tokens
    pub const CREATED_BY_SELF_SERVICE: &str = "self_service";
}
