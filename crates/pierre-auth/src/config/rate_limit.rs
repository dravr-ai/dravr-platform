// ABOUTME: Rate limiting configuration for the OAuth endpoint limiter and admin-provisioned API keys
// ABOUTME: Per-minute OAuth endpoint limits, their window, trusted proxies and key defaults, from the environment
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::{oauth_rate_limiting, system_config};
use serde::{Deserialize, Serialize};
use std::env;
use tracing::warn;

use crate::client_address::{IpNetwork, TrustedProxies};

/// Rate limiting configuration: the `OAuth` endpoint limiter and the default
/// budget of an admin-provisioned API key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// OAuth authorize endpoint rate limit (requests per minute)
    pub oauth_authorize_rpm: u32,
    /// OAuth token endpoint rate limit (requests per minute)
    pub oauth_token_rpm: u32,
    /// OAuth register endpoint rate limit (requests per minute)
    pub oauth_register_rpm: u32,
    /// Rate limit window duration in seconds
    pub rate_limit_window_secs: u64,
    /// The proxies whose `X-Forwarded-For` entries the OAuth endpoint limiter
    /// reads past to find the client: the internal networks, plus any listed
    /// in `TRUSTED_PROXY_CIDRS`
    pub trusted_proxies: TrustedProxies,
    /// Admin-provisioned API key default monthly request limit
    pub admin_provisioned_api_key_monthly_limit: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            oauth_authorize_rpm: oauth_rate_limiting::AUTHORIZE_RPM,
            oauth_token_rpm: oauth_rate_limiting::TOKEN_RPM,
            oauth_register_rpm: oauth_rate_limiting::REGISTER_RPM,
            rate_limit_window_secs: oauth_rate_limiting::WINDOW_SECS,
            trusted_proxies: TrustedProxies::internal(),
            admin_provisioned_api_key_monthly_limit: system_config::STARTER_MONTHLY_LIMIT,
        }
    }
}

impl RateLimitConfig {
    /// Load rate limiting configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            oauth_authorize_rpm: env::var("OAUTH_AUTHORIZE_RATE_LIMIT_RPM")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(oauth_rate_limiting::AUTHORIZE_RPM),
            oauth_token_rpm: env::var("OAUTH_TOKEN_RATE_LIMIT_RPM")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(oauth_rate_limiting::TOKEN_RPM),
            oauth_register_rpm: env::var("OAUTH_REGISTER_RATE_LIMIT_RPM")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(oauth_rate_limiting::REGISTER_RPM),
            rate_limit_window_secs: env::var("OAUTH2_RATE_LIMIT_WINDOW_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(oauth_rate_limiting::WINDOW_SECS),
            trusted_proxies: env::var("TRUSTED_PROXY_CIDRS")
                .map_or_else(|_| TrustedProxies::internal(), |raw| trusted_proxies(&raw)),
            admin_provisioned_api_key_monthly_limit: env::var("PIERRE_ADMIN_API_KEY_MONTHLY_LIMIT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(system_config::STARTER_MONTHLY_LIMIT),
        }
    }
}

/// The internal networks plus the comma-separated networks in `raw`. An entry
/// that is not an address or `address/prefix` is left out, with a warning.
#[must_use]
pub fn trusted_proxies(raw: &str) -> TrustedProxies {
    TrustedProxies::with(
        raw.split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .filter_map(|entry| {
                entry
                    .parse::<IpNetwork>()
                    .inspect_err(|e| {
                        warn!(entry, error = %e, "TRUSTED_PROXY_CIDRS entry ignored");
                    })
                    .ok()
            }),
    )
}
