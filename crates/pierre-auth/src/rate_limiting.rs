// ABOUTME: Request budgets for JWT, cookie, channel-link, API-key and A2A-client callers, plus the OAuth2 IP limiter
// ABOUTME: One RequestBudget per authenticated request: the gate reads it, the X-RateLimit-* headers render it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Request budgets
//!
//! What an authenticated principal may still spend. A user (JWT, cookie,
//! channel link) has a monthly request budget, counted from the first instant
//! of the UTC month: an admin's per-user override when one is set, else its
//! tier's limit. An API key, and an A2A client acting on a client-credentials
//! token, each have their own row's `rate_limit_requests` over a sliding
//! `rate_limit_window_seconds`. Every calculator answers with one
//! [`RequestBudget`], which authentication gates on and reports as the
//! `X-RateLimit-*` response headers, and takes `now` so a caller and its tests
//! agree on the instant the window is measured from.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::api_keys::{ApiKey, ApiKeyTier};
use crate::config::rate_limit::RateLimitConfig;
use pierre_core::models::{A2AClient, JwtMonthlyUsage, User, WindowUsage};
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

/// A user's monthly request budget, from its month-to-date requests and the
/// admin's override read with them.
///
/// The limit is the override's when one is set, else the tier's
/// ([`MonthlyLimitOverride::resolve`](pierre_core::models::MonthlyLimitOverride::resolve)).
/// With no limit it is [`RequestBudget::Unlimited`]; otherwise it resets at
/// the first instant of the next UTC month.
#[must_use]
pub fn calculate_jwt_rate_limit(
    user: &User,
    usage: JwtMonthlyUsage,
    now: DateTime<Utc>,
) -> RequestBudget {
    usage
        .monthly_override
        .resolve(&user.tier)
        .map_or(RequestBudget::Unlimited, |limit| RequestBudget::Metered {
            limit,
            used: usage.used,
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
/// An Enterprise key is [`RequestBudget::Unlimited`]; every other key is
/// metered by [`sliding_window_budget`].
#[must_use]
pub fn calculate_api_key_rate_limit(
    api_key: &ApiKey,
    usage: &WindowUsage,
    now: DateTime<Utc>,
) -> RequestBudget {
    if api_key.tier == ApiKeyTier::Enterprise {
        return RequestBudget::Unlimited;
    }
    sliding_window_budget(
        api_key.rate_limit_requests,
        api_key_window(api_key),
        usage,
        now,
    )
}

/// The length of an API key's sliding window.
fn api_key_window(api_key: &ApiKey) -> Duration {
    Duration::seconds(i64::from(api_key.rate_limit_window_seconds))
}

/// The first instant an A2A client's sliding window covers at `now`: its
/// calls after this count against its `rate_limit_requests`.
#[must_use]
pub fn a2a_client_window_start(client: &A2AClient, now: DateTime<Utc>) -> DateTime<Utc> {
    now - a2a_client_window(client)
}

/// An A2A client's budget on a client-credentials token: its row's
/// `rate_limit_requests` over a sliding `rate_limit_window_seconds`, metered
/// by [`sliding_window_budget`].
///
/// Always metered: a client row carries a limit and no tier that lifts it.
#[must_use]
pub fn calculate_a2a_client_rate_limit(
    client: &A2AClient,
    usage: &WindowUsage,
    now: DateTime<Utc>,
) -> RequestBudget {
    sliding_window_budget(
        client.rate_limit_requests,
        a2a_client_window(client),
        usage,
        now,
    )
}

/// When an A2A client's sliding window frees its first slot, given the
/// calls `usage` counted inside it: the reset its budget names.
#[must_use]
pub fn a2a_client_window_resets_at(
    client: &A2AClient,
    usage: &WindowUsage,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    window_resets_at(a2a_client_window(client), usage, now)
}

/// The length of an A2A client's sliding window.
fn a2a_client_window(client: &A2AClient) -> Duration {
    Duration::seconds(i64::from(client.rate_limit_window_seconds))
}

/// `limit` calls per sliding `window`, given the calls `usage` counted inside
/// it, resetting at [`window_resets_at`].
fn sliding_window_budget(
    limit: u32,
    window: Duration,
    usage: &WindowUsage,
    now: DateTime<Utc>,
) -> RequestBudget {
    RequestBudget::Metered {
        limit,
        used: usage.count,
        resets_at: window_resets_at(window, usage, now),
    }
}

/// When a sliding `window` frees its first slot: when the oldest call in it
/// leaves, at `oldest + window`; an empty window names `now + window`.
///
/// That instant is exact while `used <= limit`; after concurrent requests
/// overshoot the limit, the first freed slot still leaves the window full, so
/// a client retrying on it is refused once more with a fresh retry window.
fn window_resets_at(window: Duration, usage: &WindowUsage, now: DateTime<Utc>) -> DateTime<Utc> {
    usage.oldest.map_or(now + window, |oldest| oldest + window)
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
    /// A signed-in account re-confirming its password (`change-password`,
    /// account deletion), metered per account rather than per address
    PasswordConfirm,
    /// A refused password sign-in (the `POST /oauth/token` password grant,
    /// `POST /oauth2/login`, the messaging link page, the device approval
    /// page), metered per client address
    PasswordLogin,
    /// A refused password sign-in, metered per account named rather than per
    /// address
    PasswordLoginAccount,
}

impl OAuth2Endpoint {
    /// The endpoint's name, as its window's cache key spells it
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authorize => "authorize",
            Self::Token => "token",
            Self::Register => "register",
            Self::PasswordConfirm => "password_confirm",
            Self::PasswordLogin => "password_login",
            Self::PasswordLoginAccount => "password_login_account",
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
    /// Password re-confirmations per account per window
    pub password_confirm_rpm: u32,
    /// Refused password sign-ins per client address per window
    pub password_login_rpm: u32,
    /// Refused password sign-ins per account named per window
    pub password_login_account_rpm: u32,
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
            password_confirm_rpm: oauth_rate_limiting::PASSWORD_CONFIRM_RPM,
            password_login_rpm: oauth_rate_limiting::PASSWORD_LOGIN_RPM,
            password_login_account_rpm: oauth_rate_limiting::PASSWORD_LOGIN_ACCOUNT_RPM,
        }
    }

    /// Create `OAuth2` rate limit configuration from `RateLimitConfig`
    #[must_use]
    pub const fn from_rate_limit_config(config: &RateLimitConfig) -> Self {
        Self {
            authorize_rpm: config.oauth_authorize_rpm,
            token_rpm: config.oauth_token_rpm,
            register_rpm: config.oauth_register_rpm,
            password_confirm_rpm: config.password_confirm_rpm,
            password_login_rpm: config.password_login_rpm,
            password_login_account_rpm: config.password_login_account_rpm,
        }
    }

    /// Create custom `OAuth2` rate limit configuration
    #[must_use]
    pub const fn custom(
        authorize_rpm: u32,
        token_rpm: u32,
        register_rpm: u32,
        password_confirm_rpm: u32,
        password_login_rpm: u32,
        password_login_account_rpm: u32,
    ) -> Self {
        Self {
            authorize_rpm,
            token_rpm,
            register_rpm,
            password_confirm_rpm,
            password_login_rpm,
            password_login_account_rpm,
        }
    }

    /// Requests `endpoint` admits per window
    #[must_use]
    pub const fn get_limit(&self, endpoint: OAuth2Endpoint) -> u32 {
        match endpoint {
            OAuth2Endpoint::Authorize => self.authorize_rpm,
            OAuth2Endpoint::Token => self.token_rpm,
            OAuth2Endpoint::Register => self.register_rpm,
            OAuth2Endpoint::PasswordConfirm => self.password_confirm_rpm,
            OAuth2Endpoint::PasswordLogin => self.password_login_rpm,
            OAuth2Endpoint::PasswordLoginAccount => self.password_login_account_rpm,
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
}
