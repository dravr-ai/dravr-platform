// ABOUTME: Rate limiting configuration for tier-based request throttling
// ABOUTME: Configurable limits per API key tier with environment variable overrides
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::{oauth_rate_limiting, rate_limiting_bursts, system_config};
use serde::{Deserialize, Serialize};
use std::env;

/// Rate limiting configuration for tier-based request throttling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Free tier burst limit
    pub free_tier_burst: u32,
    /// Professional tier burst limit
    pub professional_burst: u32,
    /// Enterprise tier burst limit
    pub enterprise_burst: u32,
    /// OAuth authorize endpoint rate limit (requests per minute)
    pub oauth_authorize_rpm: u32,
    /// OAuth token endpoint rate limit (requests per minute)
    pub oauth_token_rpm: u32,
    /// OAuth register endpoint rate limit (requests per minute)
    pub oauth_register_rpm: u32,
    /// Rate limit window duration in seconds
    pub rate_limit_window_secs: u64,
    /// Rate limiter cleanup threshold
    pub cleanup_threshold: usize,
    /// Stale entry timeout in seconds
    pub stale_entry_timeout_secs: u64,
    /// Admin-provisioned API key default monthly request limit
    pub admin_provisioned_api_key_monthly_limit: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            free_tier_burst: rate_limiting_bursts::FREE_TIER_BURST,
            professional_burst: rate_limiting_bursts::PROFESSIONAL_BURST,
            enterprise_burst: rate_limiting_bursts::ENTERPRISE_BURST,
            oauth_authorize_rpm: oauth_rate_limiting::AUTHORIZE_RPM,
            oauth_token_rpm: oauth_rate_limiting::TOKEN_RPM,
            oauth_register_rpm: oauth_rate_limiting::REGISTER_RPM,
            rate_limit_window_secs: oauth_rate_limiting::WINDOW_SECS,
            cleanup_threshold: oauth_rate_limiting::CLEANUP_THRESHOLD,
            stale_entry_timeout_secs: oauth_rate_limiting::STALE_ENTRY_TIMEOUT_SECS,
            admin_provisioned_api_key_monthly_limit: system_config::STARTER_MONTHLY_LIMIT,
        }
    }
}

impl RateLimitConfig {
    /// Load rate limiting configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            free_tier_burst: env::var("RATE_LIMIT_FREE_TIER_BURST")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(rate_limiting_bursts::FREE_TIER_BURST),
            professional_burst: env::var("RATE_LIMIT_PROFESSIONAL_BURST")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(rate_limiting_bursts::PROFESSIONAL_BURST),
            enterprise_burst: env::var("RATE_LIMIT_ENTERPRISE_BURST")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(rate_limiting_bursts::ENTERPRISE_BURST),
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
            cleanup_threshold: env::var("RATE_LIMITER_CLEANUP_THRESHOLD")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(oauth_rate_limiting::CLEANUP_THRESHOLD),
            stale_entry_timeout_secs: env::var("RATE_LIMITER_STALE_ENTRY_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(oauth_rate_limiting::STALE_ENTRY_TIMEOUT_SECS),
            admin_provisioned_api_key_monthly_limit: env::var("PIERRE_ADMIN_API_KEY_MONTHLY_LIMIT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(system_config::STARTER_MONTHLY_LIMIT),
        }
    }
}
