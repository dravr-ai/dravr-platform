// ABOUTME: Request budgets for JWT, cookie, channel-link and API-key callers, plus the OAuth2 IP limiter
// ABOUTME: One RequestBudget per authenticated request: the gate reads it, the X-RateLimit-* headers render it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Request budgets
//!
//! What an authenticated principal may still spend. A user (JWT, cookie,
//! channel link) has the [`UserRequestLimits`] in force for it: an admin's
//! per-user override when one is set, else its tier's monthly limit. A monthly
//! limit is counted from the first instant of the UTC month, a daily one from
//! the first instant of the UTC day. An API key has its own
//! `rate_limit_requests` over a sliding `rate_limit_window_seconds`. Both
//! answer with one [`RequestBudget`], which the auth middleware gates on and
//! reports as the `X-RateLimit-*` response headers. Both calculators take
//! `now` so a caller and its tests agree on the instant the window is measured
//! from.

use chrono::{DateTime, Duration, Timelike, Utc};
use serde::Serialize;

use crate::api_keys::{ApiKey, ApiKeyTier};
use crate::config::rate_limit::RateLimitConfig;
use pierre_core::models::{ApiKeyWindowUsage, User};
use pierre_database::repositories::analytics::{next_utc_day_start, next_utc_month_start};
use pierre_database::repositories::UserRateLimitOverride;

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

/// The request limits in force for a user. `None` on a dimension means no
/// ceiling there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserRequestLimits {
    /// Requests per UTC day
    pub daily: Option<u32>,
    /// Requests per UTC month
    pub monthly: Option<u32>,
}

impl UserRequestLimits {
    /// The limits an admin's override sets when `override_row` exists, else
    /// the tier's: its monthly limit and no daily one.
    ///
    /// The override replaces the tier outright, dimension by dimension, so a
    /// `None` on the override lifts that ceiling even where the tier has one.
    /// The admin rate-limit view reads the same resolution, so what it shows
    /// is what the gate enforces.
    #[must_use]
    pub fn resolve(user: &User, override_row: Option<&UserRateLimitOverride>) -> Self {
        override_row.map_or_else(
            || Self {
                daily: None,
                monthly: user.tier.monthly_limit(),
            },
            |row| Self {
                daily: row.daily_limit,
                monthly: row.monthly_limit,
            },
        )
    }
}

/// A user's requests counted in each limited window, before this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserRequestUsage {
    /// Since the first instant of the UTC day
    pub today: u32,
    /// Since the first instant of the UTC month
    pub this_month: u32,
}

/// A user's request budget under `limits`, from the requests counted in each
/// window.
///
/// With no limit on either dimension it is [`RequestBudget::Unlimited`]. With
/// one, it is that window's budget: a daily limit resets at the first instant
/// of the next UTC day, a monthly one at the first instant of the next UTC
/// month. With both, it is the window that decides the request — the one
/// already spent (the later-resetting one when both are), else the one with
/// fewer requests left — so the gate and the `X-RateLimit-*` headers name the
/// same window.
#[must_use]
pub fn calculate_jwt_rate_limit(
    limits: UserRequestLimits,
    usage: UserRequestUsage,
    now: DateTime<Utc>,
) -> RequestBudget {
    let daily = limits.daily.map(|limit| RequestBudget::Metered {
        limit,
        used: usage.today,
        resets_at: next_utc_day_start(now),
    });
    let monthly = limits.monthly.map(|limit| RequestBudget::Metered {
        limit,
        used: usage.this_month,
        resets_at: next_utc_month_start(now),
    });
    match (daily, monthly) {
        (Some(daily), Some(monthly)) => binding_budget(daily, monthly),
        (Some(only), None) | (None, Some(only)) => only,
        (None, None) => RequestBudget::Unlimited,
    }
}

/// Of two metered budgets, the one that decides the request.
fn binding_budget(daily: RequestBudget, monthly: RequestBudget) -> RequestBudget {
    match (daily.is_exceeded(), monthly.is_exceeded()) {
        // The monthly window resets last, so it is the one a client must
        // wait out.
        (_, true) => monthly,
        (true, false) => daily,
        (false, false) => {
            if daily.remaining_after_this_request() < monthly.remaining_after_this_request() {
                daily
            } else {
                monthly
            }
        }
    }
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

/// An `OAuth2` endpoint the per-address limiter meters. Each has its own limit
/// and, per client address, its own window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OAuth2Endpoint {
    /// `GET /oauth2/authorize`
    Authorize,
    /// `POST /oauth2/token`
    Token,
    /// `POST /oauth2/register`
    Register,
}

impl OAuth2Endpoint {
    /// The endpoint's name, as its window's cache key spells it
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authorize => "authorize",
            Self::Token => "token",
            Self::Register => "register",
        }
    }
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

    /// Requests `endpoint` admits per window
    #[must_use]
    pub const fn get_limit(&self, endpoint: OAuth2Endpoint) -> u32 {
        match endpoint {
            OAuth2Endpoint::Authorize => self.authorize_rpm,
            OAuth2Endpoint::Token => self.token_rpm,
            OAuth2Endpoint::Register => self.register_rpm,
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
