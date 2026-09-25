// ABOUTME: Integration tests for rate limiting middleware functionality
// ABOUTME: Tests rate limit error creation and checking mechanisms
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, TimeZone, Utc};
use http::HeaderMap;
use pierre_auth::rate_limiting::UnifiedRateLimitInfo;
use pierre_core::errors::ErrorCode;
use pierre_middleware::rate_limiting::{
    check_rate_limit_and_respond, create_rate_limit_error, create_rate_limit_headers, headers,
};

fn header<'a>(map: &'a HeaderMap, name: &str) -> Option<&'a str> {
    map.get(name).and_then(|v| v.to_str().ok())
}

#[test]
fn test_rate_limit_error_creation() {
    let rate_limit_info = UnifiedRateLimitInfo {
        is_rate_limited: true,
        limit: Some(1000),
        remaining: Some(0),
        reset_at: Some(Utc::now() + chrono::Duration::hours(1)),
        tier: "professional".into(),
        auth_method: "api_key".into(),
    };

    let error = create_rate_limit_error(&rate_limit_info);
    assert_eq!(error.code, ErrorCode::RateLimitExceeded);
    assert_eq!(error.http_status(), 429);

    // Check basic error properties
    assert_eq!(error.code, ErrorCode::RateLimitExceeded);
    assert!(error.message.contains("1000"));
    assert!(error.message.contains("professional"));
}

#[test]
fn test_rate_limit_check() {
    // Test when not rate limited
    let info = UnifiedRateLimitInfo {
        is_rate_limited: false,
        limit: Some(1000),
        remaining: Some(500),
        reset_at: None,
        tier: "starter".into(),
        auth_method: "jwt".into(),
    };

    assert!(check_rate_limit_and_respond(&info).is_ok());

    // Test when rate limited
    let info = UnifiedRateLimitInfo {
        is_rate_limited: true,
        limit: Some(1000),
        remaining: Some(0),
        reset_at: Some(Utc::now()),
        tier: "starter".into(),
        auth_method: "jwt".into(),
    };

    assert!(check_rate_limit_and_respond(&info).is_err());
}

#[test]
fn test_rate_limit_headers_carry_exact_integer_values() {
    let reset_at = Utc::now() + Duration::hours(1);
    let info = UnifiedRateLimitInfo {
        is_rate_limited: false,
        limit: Some(1000),
        remaining: Some(250),
        reset_at: Some(reset_at),
        tier: "professional".into(),
        auth_method: "api_key".into(),
    };

    let map = create_rate_limit_headers(&info);

    assert_eq!(header(&map, headers::X_RATE_LIMIT_LIMIT), Some("1000"));
    assert_eq!(header(&map, headers::X_RATE_LIMIT_REMAINING), Some("250"));
    let expected_reset = reset_at.timestamp().to_string();
    assert_eq!(
        header(&map, headers::X_RATE_LIMIT_RESET),
        Some(expected_reset.as_str())
    );
    let retry_after: i64 = header(&map, headers::RETRY_AFTER)
        .expect("Retry-After is set when a reset time is known")
        .parse()
        .expect("Retry-After is an integer number of seconds");
    assert!(
        (3598..=3600).contains(&retry_after),
        "Retry-After counts the seconds to a reset one hour out, got {retry_after}"
    );
    assert_eq!(header(&map, headers::X_RATE_LIMIT_WINDOW), Some("2592000"));
    assert_eq!(
        header(&map, headers::X_RATE_LIMIT_TIER),
        Some("professional")
    );
    assert_eq!(
        header(&map, headers::X_RATE_LIMIT_AUTH_METHOD),
        Some("api_key")
    );
}

#[test]
fn test_rate_limit_headers_past_reset_clamps_retry_after_to_zero() {
    let reset_at = Utc
        .with_ymd_and_hms(2020, 1, 1, 0, 0, 0)
        .single()
        .expect("valid fixed timestamp");
    let info = UnifiedRateLimitInfo {
        is_rate_limited: true,
        limit: Some(0),
        remaining: Some(0),
        reset_at: Some(reset_at),
        tier: "starter".into(),
        auth_method: "jwt".into(),
    };

    let map = create_rate_limit_headers(&info);

    assert_eq!(header(&map, headers::X_RATE_LIMIT_LIMIT), Some("0"));
    assert_eq!(header(&map, headers::X_RATE_LIMIT_REMAINING), Some("0"));
    assert_eq!(
        header(&map, headers::X_RATE_LIMIT_RESET),
        Some("1577836800")
    );
    assert_eq!(header(&map, headers::RETRY_AFTER), Some("0"));
}

#[test]
fn test_rate_limit_headers_omit_unknown_values() {
    let info = UnifiedRateLimitInfo {
        is_rate_limited: false,
        limit: None,
        remaining: None,
        reset_at: None,
        tier: "starter".into(),
        auth_method: "jwt".into(),
    };

    let map = create_rate_limit_headers(&info);

    assert_eq!(header(&map, headers::X_RATE_LIMIT_LIMIT), None);
    assert_eq!(header(&map, headers::X_RATE_LIMIT_REMAINING), None);
    assert_eq!(header(&map, headers::X_RATE_LIMIT_RESET), None);
    assert_eq!(header(&map, headers::RETRY_AFTER), None);
    assert_eq!(header(&map, headers::X_RATE_LIMIT_TIER), Some("starter"));
}
