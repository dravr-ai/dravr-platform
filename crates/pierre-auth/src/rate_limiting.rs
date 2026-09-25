// ABOUTME: Request budgets for JWT, cookie, channel-link and API-key callers, plus the OAuth2 IP limiter
// ABOUTME: One RequestBudget per authenticated request: the gate reads it, the X-RateLimit-* headers render it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Request budgets
//!
//! What an authenticated principal may still spend. A user (JWT, cookie,
//! channel link) has its tier's monthly request budget, counted from the first
//! instant of the UTC month; an API key has its own `rate_limit_requests` over a
//! sliding `rate_limit_window_seconds`. Both answer with one [`RequestBudget`],
//! which the auth middleware gates on and reports as the `X-RateLimit-*`
//! response headers. Both calculators take `now` so a caller and its tests
//! agree on the instant the window is measured from.

use chrono::{DateTime, Duration, Timelike, Utc};
use serde::Serialize;

use crate::api_keys::{ApiKey, ApiKeyTier};
use crate::config::rate_limit::RateLimitConfig;
use pierre_core::models::{ApiKeyWindowUsage, User};
use pierre_database::repositories::analytics::next_utc_month_start;

/// What a principal may still spend in its current rate-limit window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestBudget {
    /// No ceiling (an Enterprise user or key): no `X-RateLimit-*` is sent.
    Unlimited,
    /// `limit` requests per window.
    Metered {
        /// Requests the window admits
        limit: u32,
        /// Requests counted in the window before this one
        used: u32,
        /// When the window frees capacity again
        resets_at: DateTime<Utc>,
    },
}

impl RequestBudget {
    /// Whether this request is refused: the window has already admitted
    /// `limit` requests.
    #[must_use]
    pub const fn is_exceeded(&self) -> bool {
        match self {
            Self::Unlimited => false,
            Self::Metered { limit, used, .. } => *used >= *limit,
        }
    }

    /// What is left once this request is counted, never below zero; `None`
    /// for an unlimited budget.
    ///
    /// `used` is the count before this request, so a request that is
    /// admitted leaves `limit - used - 1`, and a refused one leaves 0.
    #[must_use]
    pub const fn remaining_after_this_request(&self) -> Option<u32> {
        match self {
            Self::Unlimited => None,
            Self::Metered { limit, used, .. } => Some(limit.saturating_sub(used.saturating_add(1))),
        }
    }
}

/// A user's monthly request budget, from the tier's limit and the requests
/// counted since the start of the UTC month.
///
/// A tier with no monthly limit is [`RequestBudget::Unlimited`]; every other
/// tier resets at the first instant of the next UTC month.
#[must_use]
pub fn calculate_jwt_rate_limit(
    user: &User,
    used_this_month: u32,
    now: DateTime<Utc>,
) -> RequestBudget {
    user.tier
        .monthly_limit()
        .map_or(RequestBudget::Unlimited, |limit| RequestBudget::Metered {
            limit,
            used: used_this_month,
            resets_at: next_utc_month_start(now),
        })
}

/// The first instant an API key's sliding window covers at `now`: its calls
/// after this count against its `rate_limit_requests`.
#[must_use]
pub fn api_key_window_start(api_key: &ApiKey, now: DateTime<Utc>) -> DateTime<Utc> {
    now - api_key_window(api_key)
}

/// An API key's budget over its own sliding window.
///
/// An Enterprise key is [`RequestBudget::Unlimited`]. Every other key frees
/// its first slot when the oldest call in the window leaves it, `oldest +
/// window`; an empty window names `now + window`. That instant is exact while
/// `used <= limit`; after concurrent requests overshoot the limit, the first
/// freed slot still leaves the window full, so a client retrying on it is
/// refused once more with a fresh retry window.
#[must_use]
pub fn calculate_api_key_rate_limit(
    api_key: &ApiKey,
    usage: &ApiKeyWindowUsage,
    now: DateTime<Utc>,
) -> RequestBudget {
    if api_key.tier == ApiKeyTier::Enterprise {
        return RequestBudget::Unlimited;
    }
    let window = api_key_window(api_key);
    RequestBudget::Metered {
        limit: api_key.rate_limit_requests,
        used: usage.count,
        resets_at: usage.oldest.map_or(now + window, |oldest| oldest + window),
    }
}

/// The length of an API key's sliding window.
fn api_key_window(api_key: &ApiKey) -> Duration {
    Duration::seconds(i64::from(api_key.rate_limit_window_seconds))
}

/// `OAuth2`-specific rate limit configuration
#[derive(Debug, Clone)]
pub struct OAuth2RateLimitConfig {
    /// Requests per minute for authorization endpoint
    pub authorize_rpm: u32,
    /// Requests per minute for token endpoint
    pub token_rpm: u32,
    /// Requests per minute for registration endpoint
    pub register_rpm: u32,
}

impl OAuth2RateLimitConfig {
    /// Create new `OAuth2` rate limit configuration with defaults
    #[must_use]
    pub const fn new() -> Self {
        use pierre_core::constants::oauth_rate_limiting;
        Self {
            authorize_rpm: oauth_rate_limiting::AUTHORIZE_RPM, // 1 per second
            token_rpm: oauth_rate_limiting::TOKEN_RPM,         // 1 per 2 seconds
            register_rpm: oauth_rate_limiting::REGISTER_RPM,   // 1 per 6 seconds
        }
    }

    /// Create `OAuth2` rate limit configuration from `RateLimitConfig`
    #[must_use]
    pub const fn from_rate_limit_config(config: &RateLimitConfig) -> Self {
        Self {
            authorize_rpm: config.oauth_authorize_rpm,
            token_rpm: config.oauth_token_rpm,
            register_rpm: config.oauth_register_rpm,
        }
    }

    /// Create custom `OAuth2` rate limit configuration
    #[must_use]
    pub const fn custom(authorize_rpm: u32, token_rpm: u32, register_rpm: u32) -> Self {
        Self {
            authorize_rpm,
            token_rpm,
            register_rpm,
        }
    }

    /// Get rate limit for specific `OAuth2` endpoint
    #[must_use]
    pub fn get_limit(&self, endpoint: &str) -> u32 {
        match endpoint {
            "authorize" => self.authorize_rpm,
            "token" => self.token_rpm,
            "register" => self.register_rpm,
            _ => 60,
        }
    }
}

impl Default for OAuth2RateLimitConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// `OAuth2` rate limit status including retry information
#[derive(Debug, Clone, Serialize)]
pub struct OAuth2RateLimitStatus {
    /// Whether the request is rate limited
    pub is_limited: bool,
    /// Maximum requests allowed per minute
    pub limit: u32,
    /// Remaining requests in the current minute
    pub remaining: u32,
    /// When the rate limit resets (Unix timestamp)
    pub reset_at: i64,
    /// Seconds until rate limit resets (for Retry-After header)
    pub retry_after_seconds: Option<u32>,
}

impl OAuth2RateLimitStatus {
    /// Calculate retry-after seconds from reset timestamp.
    ///
    /// Floored at one second: `reset_at` is truncated to whole seconds, so in
    /// the window's last partial second the difference reads 0, and a refusal
    /// still in force must never advertise "retry now".
    #[must_use]
    pub fn with_retry_after(mut self) -> Self {
        if self.is_limited {
            let now = Utc::now().timestamp();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // Safe: retry_after is always positive (max(1)) and bounded by the limiter's window
            let retry_after = ((self.reset_at - now).max(1)) as u32;
            self.retry_after_seconds = Some(retry_after);
        }
        self
    }

    /// Get next reset time (start of next minute)
    #[must_use]
    pub fn calculate_reset() -> DateTime<Utc> {
        let now = Utc::now();
        now.with_second(0)
            .and_then(|dt| dt.with_nanosecond(0))
            .unwrap_or(now)
            + chrono::Duration::minutes(1)
    }
}
