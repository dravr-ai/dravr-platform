// ABOUTME: End-to-end tests for OAuth2 endpoint rate limiting with RFC-compliant headers
// ABOUTME: Validates per-IP rate limiting, 429 responses, and rate limit header correctness
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::header::{CONTENT_TYPE, RETRY_AFTER};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::Router;
use futures_util::future::join_all;
use pierre_auth::{
    config::RateLimitConfig,
    oauth2_server::{
        client_registration::ClientRegistrationManager, models::ClientRegistrationRequest,
        rate_limiting::OAuth2RateLimiter,
    },
    rate_limiting::OAuth2RateLimitConfig,
};
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::{backends::DatabaseProvider, database::generate_encryption_key};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use serde_json::{json, Value};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::time::{sleep, Duration};
use tower::ServiceExt;

/// Rate limit window, in seconds, for the two tests that cross a window boundary.
///
/// `OAuth2RateLimiter` measures the window with `std::time::Instant`, which
/// tokio's paused clock does not move, so those tests wait real time. The
/// window is a field of `RateLimitConfig`, so the wait is set by the
/// configuration under test rather than by the shipped 60-second default.
const TEST_WINDOW_SECS: u64 = 2;

/// Age, in seconds, at which the lazy cleanup pass drops an idle per-IP entry.
const TEST_STALE_ENTRY_TIMEOUT_SECS: u64 = 2;

/// Wait that clears both `TEST_WINDOW_SECS` and `TEST_STALE_ENTRY_TIMEOUT_SECS`
/// with a second of slack on a loaded runner.
const TEST_EXPIRY_WAIT: Duration = Duration::from_secs(3);

/// Distinct client IPs the cleanup test puts into the limiter's map.
const CLEANUP_TEST_IP_COUNT: u8 = 100;

/// Rate limit configuration whose window and stale-entry timeout expire in
/// seconds instead of minutes, for the tests that observe an expiry.
fn short_expiry_config() -> RateLimitConfig {
    RateLimitConfig {
        rate_limit_window_secs: TEST_WINDOW_SECS,
        stale_entry_timeout_secs: TEST_STALE_ENTRY_TIMEOUT_SECS,
        ..RateLimitConfig::default()
    }
}

/// Test rate limiting on client registration endpoint
#[tokio::test]
async fn test_rate_limit_client_registration() {
    let encryption_key = generate_encryption_key().to_vec();

    let database = Arc::new(create_test_db_with_key(encryption_key).await.unwrap());

    database.migrate().await.unwrap();

    let repos = database.repositories();
    let registration_manager = ClientRegistrationManager::new(repos.oauth2_server.clone());
    let rate_limiter = OAuth2RateLimiter::new();
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));

    // Default limit for registration endpoint is 10 requests per minute
    let endpoint = "register";

    // Make 10 successful requests
    for i in 1..=10 {
        let status = rate_limiter.check_rate_limit(endpoint, client_ip);
        assert!(!status.is_limited, "Request {i} should not be rate limited");
        assert_eq!(status.limit, 10);
        assert_eq!(status.remaining, 11 - i); // Remaining is count before current request
        assert!(status.retry_after_seconds.is_none());

        // Actually register a client to simulate real usage
        let registration_request = ClientRegistrationRequest {
            redirect_uris: vec![format!("https://example{i}.com/callback")],
            client_name: Some(format!("Test Client {i}")),
            client_uri: None,
            grant_types: None,
            response_types: None,
            scope: None,
        };

        let result = registration_manager
            .register_client(registration_request, MAX_PENDING_REGISTRATIONS)
            .await;
        assert!(result.is_ok(), "Registration {i} should succeed");
    }

    // 11th request should be rate limited
    let status = rate_limiter.check_rate_limit(endpoint, client_ip);
    assert!(status.is_limited, "Request 11 should be rate limited");
    assert_eq!(status.limit, 10);
    assert_eq!(status.remaining, 0);
    assert!(status.retry_after_seconds.is_some());
    assert!(status.retry_after_seconds.unwrap() <= 60);
}

/// Test rate limiting on token endpoint
#[tokio::test]
async fn test_rate_limit_token_endpoint() {
    let rate_limiter = OAuth2RateLimiter::new();
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101));

    // Default limit for token endpoint is 30 requests per minute
    let endpoint = "token";

    // Make 30 successful requests
    for i in 1..=30 {
        let status = rate_limiter.check_rate_limit(endpoint, client_ip);
        assert!(!status.is_limited, "Request {i} should not be rate limited");
        assert_eq!(status.limit, 30);
        assert_eq!(status.remaining, 31 - i); // Remaining is count before current request
    }

    // 31st request should be rate limited
    let status = rate_limiter.check_rate_limit(endpoint, client_ip);
    assert!(status.is_limited, "Request 31 should be rate limited");
    assert_eq!(status.remaining, 0);
    assert!(status.retry_after_seconds.is_some());
}

/// Test rate limiting on authorization endpoint
#[tokio::test]
async fn test_rate_limit_authorization_endpoint() {
    let rate_limiter = OAuth2RateLimiter::new();
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 102));

    // Default limit for authorize endpoint is 60 requests per minute
    let endpoint = "authorize";

    // Make 60 successful requests
    for i in 1..=60 {
        let status = rate_limiter.check_rate_limit(endpoint, client_ip);
        assert!(!status.is_limited, "Request {i} should not be rate limited");
        assert_eq!(status.limit, 60);
        assert_eq!(status.remaining, 61 - i); // Remaining is count before current request
    }

    // 61st request should be rate limited
    let status = rate_limiter.check_rate_limit(endpoint, client_ip);
    assert!(status.is_limited, "Request 61 should be rate limited");
    assert_eq!(status.remaining, 0);
}

/// Test per-IP isolation - different IPs should have separate rate limits
#[tokio::test]
async fn test_per_ip_rate_limit_isolation() {
    let rate_limiter = OAuth2RateLimiter::new();
    let client_ip_1 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 103));
    let client_ip_2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 104));

    let endpoint = "register";

    // Exhaust rate limit for IP 1
    for _i in 1..=10 {
        let status = rate_limiter.check_rate_limit(endpoint, client_ip_1);
        assert!(!status.is_limited);
    }

    // IP 1 should now be rate limited
    let status = rate_limiter.check_rate_limit(endpoint, client_ip_1);
    assert!(status.is_limited, "IP 1 should be rate limited");

    // IP 2 should still have full quota
    let status = rate_limiter.check_rate_limit(endpoint, client_ip_2);
    assert!(!status.is_limited, "IP 2 should not be rate limited");
    assert_eq!(status.remaining, 10); // Remaining is count before current request
}

/// Test rate limit headers contain correct values
#[tokio::test]
async fn test_rate_limit_headers() {
    let rate_limiter = OAuth2RateLimiter::new();
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 105));
    let endpoint = "token";

    // First request
    let status = rate_limiter.check_rate_limit(endpoint, client_ip);
    assert_eq!(status.limit, 30, "X-RateLimit-Limit should be 30");
    assert_eq!(status.remaining, 30, "X-RateLimit-Remaining should be 30");
    assert!(status.reset_at > 0, "X-RateLimit-Reset should be set");

    // Second request
    let status = rate_limiter.check_rate_limit(endpoint, client_ip);
    assert_eq!(
        status.remaining, 29,
        "X-RateLimit-Remaining should decrement"
    );

    // Reset timestamp should be in the future (within next 60 seconds)
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    #[allow(clippy::cast_possible_wrap)]
    let now_i64 = now as i64;
    assert!(
        status.reset_at > now_i64,
        "Reset timestamp should be in the future"
    );
    assert!(
        status.reset_at <= now_i64 + 60,
        "Reset timestamp should be within 60 seconds"
    );
}

/// One request to the full application, from `addr` as the server's
/// `ConnectInfo` would name it.
async fn from_address(app: &Router, request: Request<Body>, addr: SocketAddr) -> Response {
    let mut request = request;
    request.extensions_mut().insert(ConnectInfo(addr));
    app.clone().oneshot(request).await.unwrap()
}

/// Assert `response` is the per-IP refusal: 429, the RFC 6749
/// `too_many_requests` body, and a `Retry-After` inside the one-minute window.
async fn assert_rate_limited(response: Response, endpoint: &str) {
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "{endpoint} past its limit"
    );
    let retry_after: u64 = response
        .headers()
        .get(RETRY_AFTER)
        .unwrap_or_else(|| panic!("{endpoint} refusal carries Retry-After"))
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        (1..=60).contains(&retry_after),
        "{endpoint} Retry-After {retry_after} should be within the one-minute window"
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"], "too_many_requests");
    assert_eq!(body["error_description"], "Rate limit exceeded");
}

/// The 429 the `OAuth2` endpoints answer through the full application carries
/// `Retry-After`, in the RFC's body shape, on both rate-limited endpoints.
#[tokio::test]
async fn test_retry_after_header() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let addr = SocketAddr::from(([203, 0, 113, 7], 40_000));

    // Registration admits ten per minute from one address.
    let register = |i: u32| {
        Request::post("/oauth2/register")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "redirect_uris": [format!("https://retry{i}.example.com/callback")],
                    "client_name": format!("Retry-After client {i}"),
                })
                .to_string(),
            ))
            .unwrap()
    };
    for i in 1..=10 {
        let response = from_address(&app, register(i), addr).await;
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "registration {i} is within the limit"
        );
        assert!(response.headers().get(RETRY_AFTER).is_none());
    }
    assert_rate_limited(from_address(&app, register(11), addr).await, "register").await;

    // The token endpoint admits thirty per minute; a malformed grant still
    // counts, since the limiter runs before the grant is read. The limiter
    // keeps one counter per address across its endpoints, so this half starts
    // from a second, fresh address.
    let token_addr = SocketAddr::from(([203, 0, 113, 8], 40_000));
    let token = || {
        Request::post("/oauth2/token")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=client_credentials"))
            .unwrap()
    };
    for i in 1..=30 {
        let response = from_address(&app, token(), token_addr).await;
        assert_ne!(
            response.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "token request {i} is within the limit"
        );
    }
    assert_rate_limited(from_address(&app, token(), token_addr).await, "token").await;

    // Another address has its own bucket.
    let elsewhere = SocketAddr::from(([198, 51, 100, 9], 40_000));
    assert_eq!(
        from_address(&app, register(12), elsewhere).await.status(),
        StatusCode::CREATED
    );
}

/// Test rate limit window reset after expiration
#[tokio::test]
async fn test_rate_limit_window_reset() {
    // The window lives in RateLimitConfig, so this test drives the limiter
    // through a real boundary at TEST_WINDOW_SECS instead of the 60-second
    // default that `OAuth2RateLimiter::with_config` leaves in place.
    let config = RateLimitConfig {
        oauth_register_rpm: 3, // Only 3 requests allowed
        ..short_expiry_config()
    };

    let rate_limiter = OAuth2RateLimiter::from_rate_limit_config(config);
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 107));
    let endpoint = "register";

    // Make 3 requests
    for i in 1..=3 {
        let status = rate_limiter.check_rate_limit(endpoint, client_ip);
        assert!(!status.is_limited, "Request {i} should succeed");
    }

    // 4th request should be rate limited
    let status = rate_limiter.check_rate_limit(endpoint, client_ip);
    assert!(status.is_limited, "Request 4 should be rate limited");

    // Wait for the configured window to expire
    sleep(TEST_EXPIRY_WAIT).await;

    // After window reset, should be able to make requests again
    let status = rate_limiter.check_rate_limit(endpoint, client_ip);
    assert!(
        !status.is_limited,
        "After window reset, requests should succeed again"
    );
    assert_eq!(status.remaining, 3); // Remaining is count before current request
}

/// Test custom rate limit configuration
#[tokio::test]
async fn test_custom_rate_limit_config() {
    let mut config = OAuth2RateLimitConfig::new();
    config.register_rpm = 5;
    config.token_rpm = 15;
    config.authorize_rpm = 25;

    let rate_limiter = OAuth2RateLimiter::with_config(config);
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 108));

    // Test custom register limit
    let status = rate_limiter.check_rate_limit("register", client_ip);
    assert_eq!(status.limit, 5, "Custom register limit should be 5");

    // Test custom token limit
    let status = rate_limiter.check_rate_limit("token", client_ip);
    assert_eq!(status.limit, 15, "Custom token limit should be 15");

    // Test custom authorize limit
    let status = rate_limiter.check_rate_limit("authorize", client_ip);
    assert_eq!(status.limit, 25, "Custom authorize limit should be 25");
}

/// Test rate limiting behavior at exact limit boundary
#[tokio::test]
async fn test_rate_limit_boundary() {
    let rate_limiter = OAuth2RateLimiter::new();
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 109));
    let endpoint = "register";

    // Make exactly 10 requests (at limit)
    for i in 1..=10 {
        let status = rate_limiter.check_rate_limit(endpoint, client_ip);
        assert!(!status.is_limited, "Request {i} should succeed");
        if i == 10 {
            assert_eq!(
                status.remaining, 1,
                "Remaining should be 1 at limit boundary"
            );
        }
    }

    // One more request should be rejected
    let status = rate_limiter.check_rate_limit(endpoint, client_ip);
    assert!(status.is_limited, "Request beyond limit should be rejected");
}

/// Test concurrent requests from same IP
#[tokio::test]
async fn test_concurrent_requests_same_ip() {
    let rate_limiter = Arc::new(OAuth2RateLimiter::new());
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 110));
    let endpoint = "token";

    let mut handles = vec![];

    // Spawn 30 concurrent requests
    for _i in 0..30 {
        let limiter = Arc::clone(&rate_limiter);
        let handle = tokio::spawn(async move {
            let status = limiter.check_rate_limit(endpoint, client_ip);
            status.is_limited
        });
        handles.push(handle);
    }

    // Wait for all requests to complete
    let results: Vec<bool> = join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // All 30 requests should succeed (limit is 30)
    let limited_count = results.iter().filter(|&&limited| limited).count();
    assert_eq!(
        limited_count, 0,
        "All 30 concurrent requests should succeed"
    );

    // 31st request should be rate limited
    let status = rate_limiter.check_rate_limit(endpoint, client_ip);
    assert!(status.is_limited, "Request 31 should be rate limited");
}

/// Test that rate limiter cleans up old entries
#[tokio::test]
async fn test_rate_limiter_cleanup() {
    // The cleanup pass runs only once the map exceeds cleanup_threshold, so the
    // threshold sits below CLEANUP_TEST_IP_COUNT here; at the shipped threshold
    // of 1000 these entries never reach it and the pass never runs.
    let config = RateLimitConfig {
        cleanup_threshold: usize::from(CLEANUP_TEST_IP_COUNT) / 2,
        ..short_expiry_config()
    };
    let rate_limiter = OAuth2RateLimiter::from_rate_limit_config(config);
    let endpoint = "register";

    // Create requests from many different IPs
    for i in 1..=CLEANUP_TEST_IP_COUNT {
        let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, i));
        let status = rate_limiter.check_rate_limit(endpoint, client_ip);
        assert!(
            !status.is_limited,
            "First request from IP {i} should not be limited"
        );
    }

    // Wait for the configured stale-entry timeout to elapse
    sleep(TEST_EXPIRY_WAIT).await;

    // Make another request to trigger cleanup
    let new_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
    let status = rate_limiter.check_rate_limit(endpoint, new_ip);
    assert!(
        !status.is_limited,
        "New request should succeed after cleanup"
    );
}
