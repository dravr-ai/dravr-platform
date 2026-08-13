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

/// Email-address verification issued at registration.
///
/// Reuses the reset flow's `<selector>.<verifier>` token shape (see
/// [`password_reset::SELECTOR_LEN`] / [`password_reset::VERIFIER_LEN`]) against a
/// separate token space, so only the lifetime and throttle differ here.
///
/// Both knobs are operator-tunable at runtime through `system_settings` with an
/// environment override, the same three-tier precedence `AUTO_APPROVE_USERS`
/// uses (env → stored row → the defaults below). They deliberately do **not**
/// live in the runtime configuration catalog: that surface is per-user and
/// per-tenant overridable, and a user who can lengthen their own verification
/// window or lift their own send throttle is a privilege-escalation path, not a
/// preference.
///
/// The bounds are enforced on read, so a malformed or hostile stored row
/// degrades to something sane instead of disabling the gate.
pub mod email_verification {
    /// Default lifetime of a verification link: 24 hours.
    ///
    /// Much longer than a reset code, on purpose. A reset is something the user
    /// is actively waiting on; a confirmation email is routinely opened the next
    /// morning on a different device. Single-use plus a ~190-bit verifier is what
    /// carries the security here, not a short clock.
    pub const DEFAULT_LINK_TTL_MINUTES: i64 = 24 * 60;

    /// Floor for a configured TTL. Below ~5 minutes the link is effectively dead
    /// on arrival for anyone whose mail provider greylists.
    pub const MIN_LINK_TTL_MINUTES: i64 = 5;

    /// Ceiling for a configured TTL (30 days). Past this the "single-use link"
    /// stops being meaningfully time-bounded.
    pub const MAX_LINK_TTL_MINUTES: i64 = 30 * 24 * 60;

    /// Default cap on verification emails a user can trigger per hour.
    ///
    /// Higher than the reset cap because legitimate users genuinely retry this
    /// one — wrong address at signup, mail in spam — and the endpoint is
    /// anti-enumeration, so hammering it teaches an attacker nothing.
    pub const DEFAULT_MAX_SENDS_PER_HOUR: i64 = 5;

    /// Floor for the send cap. Zero would lock every user out of their own
    /// account permanently, so one resend is the minimum a configuration can express.
    pub const MIN_MAX_SENDS_PER_HOUR: i64 = 1;

    /// Ceiling for the send cap — past this the throttle is not a throttle.
    pub const MAX_MAX_SENDS_PER_HOUR: i64 = 100;
}
