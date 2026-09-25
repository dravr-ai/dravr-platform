// ABOUTME: Rate limiting middleware for HTTP requests
// ABOUTME: Enforces request rate limits and prevents API abuse
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Rate Limiting Middleware with HTTP Headers
//!
//! This module provides utilities for adding standard HTTP rate limiting headers
//! to responses and creating proper 429 status codes when limits are exceeded.

use http::{HeaderMap, HeaderValue};
use pierre_auth::rate_limiting::UnifiedRateLimitInfo;
use pierre_core::errors::{AppError, ErrorCode};

/// HTTP header names for rate limiting
pub mod headers {
    /// HTTP header name for maximum requests allowed in the current window
    pub const X_RATE_LIMIT_LIMIT: &str = "X-RateLimit-Limit";
    /// HTTP header name for remaining requests in the current window
    pub const X_RATE_LIMIT_REMAINING: &str = "X-RateLimit-Remaining";
    /// HTTP header name for Unix timestamp when rate limit resets
    pub const X_RATE_LIMIT_RESET: &str = "X-RateLimit-Reset";
    /// HTTP header name for rate limit window duration in seconds
    pub const X_RATE_LIMIT_WINDOW: &str = "X-RateLimit-Window";
    /// HTTP header name for rate limit tier information
    pub const X_RATE_LIMIT_TIER: &str = "X-RateLimit-Tier";
    /// HTTP header name for authentication method used
    pub const X_RATE_LIMIT_AUTH_METHOD: &str = "X-RateLimit-AuthMethod";
    /// HTTP header name for retry-after duration in seconds
    pub const RETRY_AFTER: &str = "Retry-After";
}

/// Create a `HeaderMap` with rate limit headers
#[must_use]
pub fn create_rate_limit_headers(rate_limit_info: &UnifiedRateLimitInfo) -> HeaderMap {
    let mut headers = HeaderMap::new();

    // Add rate limit headers if we have the information
    if let Some(limit) = rate_limit_info.limit {
        headers.insert(headers::X_RATE_LIMIT_LIMIT, HeaderValue::from(limit));
    }

    if let Some(remaining) = rate_limit_info.remaining {
        headers.insert(
            headers::X_RATE_LIMIT_REMAINING,
            HeaderValue::from(remaining),
        );
    }

    if let Some(reset_at) = rate_limit_info.reset_at {
        // Add reset timestamp as Unix epoch
        headers.insert(
            headers::X_RATE_LIMIT_RESET,
            HeaderValue::from(reset_at.timestamp()),
        );

        // Add Retry-After header (seconds until reset)
        let retry_after = (reset_at - chrono::Utc::now()).num_seconds().max(0);
        headers.insert(headers::RETRY_AFTER, HeaderValue::from(retry_after));
    }

    // Add tier and authentication method information
    if let Ok(header_value) = HeaderValue::from_str(&rate_limit_info.tier) {
        headers.insert(headers::X_RATE_LIMIT_TIER, header_value);
    }

    if let Ok(header_value) = HeaderValue::from_str(&rate_limit_info.auth_method) {
        headers.insert(headers::X_RATE_LIMIT_AUTH_METHOD, header_value);
    }

    // Add rate limit window (always 30 days for monthly limits)
    headers.insert(
        headers::X_RATE_LIMIT_WINDOW,
        HeaderValue::from_static("2592000"), // 30 days in seconds
    );

    headers
}

/// Create a rate limit exceeded error response with proper headers
#[must_use]
pub fn create_rate_limit_error(rate_limit_info: &UnifiedRateLimitInfo) -> AppError {
    let limit = rate_limit_info.limit.unwrap_or(0);

    AppError::new(
        ErrorCode::RateLimitExceeded,
        format!(
            "Rate limit exceeded. You have reached your limit of {} requests for the {} tier",
            limit, rate_limit_info.tier
        ),
    )
}

/// Helper function to check rate limits and return appropriate response
///
/// # Errors
///
/// Returns an error if the rate limit has been exceeded
pub fn check_rate_limit_and_respond(
    rate_limit_info: &UnifiedRateLimitInfo,
) -> Result<(), AppError> {
    (!rate_limit_info.is_rate_limited).ok_or_else(|| create_rate_limit_error(rate_limit_info))
}
