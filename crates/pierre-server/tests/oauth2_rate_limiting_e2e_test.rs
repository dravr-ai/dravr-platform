// ABOUTME: End-to-end tests for OAuth2 endpoint rate limiting with RFC-compliant headers
// ABOUTME: Validates per-endpoint per-client windows, the deployed proxy chain, 429 and 503 responses, and header values
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
    config::{rate_limit::trusted_proxies, OAuth2ServerConfig, RateLimitConfig},
    oauth2_server::{
        client_registration::ClientRegistrationManager, models::ClientRegistrationRequest,
        rate_limiting::OAuth2RateLimiter,
    },
    rate_limiting::OAuth2Endpoint,
};
use pierre_cache::{CacheKey, CacheProvider, CacheResource};
use pierre_config::environment::ServerConfig;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::TenantId;
use pierre_database::backends::{DatabaseProvider, OAuth2ServerRepository};
use pierre_database::database::generate_encryption_key;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_routes_identity::oauth2::OAuth2Context;
use pierre_routes_identity::OAuth2Routes;
use serde_json::{json, Value};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::time::{sleep, Duration};
use tower::ServiceExt;
use uuid::Uuid;

/// Rate limit window, in seconds, for the test that crosses a window boundary.
///
/// The in-memory cache measures a window with `std::time::Instant`, which
/// tokio's paused clock does not move, so that test waits real time. The
/// window is a field of `RateLimitConfig`, so the wait is set by the
/// configuration under test rather than by the shipped 60-second default.
const TEST_WINDOW_SECS: u64 = 2;

/// Wait that clears `TEST_WINDOW_SECS` with a second of slack on a loaded runner.
const TEST_EXPIRY_WAIT: Duration = Duration::from_secs(3);

/// The redirect URI the client [`authorize_request`] names is registered for.
const AUTHORIZE_REDIRECT_URI: &str = "https://client.example.com/callback";

/// An S256 PKCE challenge, of the length `/oauth2/authorize` requires.
const AUTHORIZE_PKCE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

/// What a request the limiter could not count is refused with.
const LIMITER_UNAVAILABLE: &str = "Rate limiting is temporarily unavailable; retry later";

/// The hops the deployed backend saw appended after each client on
/// 2026-09-26: the load balancer's forwarding rule, the frontend nginx's
/// Cloud Run sandbox peer, and the `0.0.0.0` Cloud Run writes for the VPC hop
/// into the internal-ingress backend.
const DEPLOYED_TAIL: &str = "136.68.126.109, 169.254.169.126,0.0.0.0";

/// The backend's `TRUSTED_PROXY_CIDRS` in dev: Google's front-end ranges and
/// the load balancer's two addresses.
const DEV_TRUSTED_PROXY_CIDRS: &str =
    "35.191.0.0/16,130.211.0.0/22,136.68.126.109,2600:1901:0:3cf8::";

/// A limiter with no shared store, counting in its own process-local one,
/// with the limits and window of `config`.
fn limiter_with(config: &RateLimitConfig) -> OAuth2RateLimiter {
    OAuth2RateLimiter::new(None, OAuth2RateLimiter::local_window_store(), config)
}

/// A limiter with no shared store, with the shipped limits and window.
fn limiter() -> OAuth2RateLimiter {
    limiter_with(&RateLimitConfig::default())
}

/// The cache key the limiter counts `endpoint`'s window for `ip` under, so a
/// test can put something other than a count there.
fn window_key(endpoint: &str, ip: IpAddr) -> CacheKey {
    CacheKey::new(
        TenantId::nil(),
        Uuid::nil(),
        "_oauth2_rate_limit".to_owned(),
        CacheResource::Custom(format!("{endpoint}:{ip}")),
    )
}

/// Test rate limiting on client registration endpoint
#[tokio::test]
async fn test_rate_limit_client_registration() {
    let encryption_key = generate_encryption_key().to_vec();

    let database = Arc::new(create_test_db_with_key(encryption_key).await.unwrap());

    database.migrate().await.unwrap();

    let repos = database.repositories();
    let registration_manager = ClientRegistrationManager::new(repos.oauth2_server.clone());
    let rate_limiter = limiter();
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));

    // Default limit for registration endpoint is 10 requests per minute
    let endpoint = OAuth2Endpoint::Register;

    // Make 10 successful requests
    for i in 1..=10 {
        let status = rate_limiter
            .check_rate_limit(endpoint, client_ip)
            .await
            .unwrap();
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
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip)
        .await
        .unwrap();
    assert!(status.is_limited, "Request 11 should be rate limited");
    assert_eq!(status.limit, 10);
    assert_eq!(status.remaining, 0);
    assert!(status.retry_after_seconds.is_some());
    assert!(status.retry_after_seconds.unwrap() <= 60);
}

/// Test rate limiting on token endpoint
#[tokio::test]
async fn test_rate_limit_token_endpoint() {
    let rate_limiter = limiter();
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101));

    // Default limit for token endpoint is 30 requests per minute
    let endpoint = OAuth2Endpoint::Token;

    // Make 30 successful requests
    for i in 1..=30 {
        let status = rate_limiter
            .check_rate_limit(endpoint, client_ip)
            .await
            .unwrap();
        assert!(!status.is_limited, "Request {i} should not be rate limited");
        assert_eq!(status.limit, 30);
        assert_eq!(status.remaining, 31 - i); // Remaining is count before current request
    }

    // 31st request should be rate limited
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip)
        .await
        .unwrap();
    assert!(status.is_limited, "Request 31 should be rate limited");
    assert_eq!(status.remaining, 0);
    assert!(status.retry_after_seconds.is_some());
}

/// Test rate limiting on authorization endpoint
#[tokio::test]
async fn test_rate_limit_authorization_endpoint() {
    let rate_limiter = limiter();
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 102));

    // Default limit for authorize endpoint is 60 requests per minute
    let endpoint = OAuth2Endpoint::Authorize;

    // Make 60 successful requests
    for i in 1..=60 {
        let status = rate_limiter
            .check_rate_limit(endpoint, client_ip)
            .await
            .unwrap();
        assert!(!status.is_limited, "Request {i} should not be rate limited");
        assert_eq!(status.limit, 60);
        assert_eq!(status.remaining, 61 - i); // Remaining is count before current request
    }

    // 61st request should be rate limited
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip)
        .await
        .unwrap();
    assert!(status.is_limited, "Request 61 should be rate limited");
    assert_eq!(status.remaining, 0);
}

/// Test per-IP isolation - different IPs should have separate rate limits
#[tokio::test]
async fn test_per_ip_rate_limit_isolation() {
    let rate_limiter = limiter();
    let client_ip_1 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 103));
    let client_ip_2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 104));

    let endpoint = OAuth2Endpoint::Register;

    // Exhaust rate limit for IP 1
    for _i in 1..=10 {
        let status = rate_limiter
            .check_rate_limit(endpoint, client_ip_1)
            .await
            .unwrap();
        assert!(!status.is_limited);
    }

    // IP 1 should now be rate limited
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip_1)
        .await
        .unwrap();
    assert!(status.is_limited, "IP 1 should be rate limited");

    // IP 2 should still have full quota
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip_2)
        .await
        .unwrap();
    assert!(!status.is_limited, "IP 2 should not be rate limited");
    assert_eq!(status.remaining, 10); // Remaining is count before current request
}

/// Test rate limit headers contain correct values
#[tokio::test]
async fn test_rate_limit_headers() {
    let rate_limiter = limiter();
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 105));
    let endpoint = OAuth2Endpoint::Token;

    // First request
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip)
        .await
        .unwrap();
    assert_eq!(status.limit, 30, "X-RateLimit-Limit should be 30");
    assert_eq!(status.remaining, 30, "X-RateLimit-Remaining should be 30");
    assert!(status.reset_at > 0, "X-RateLimit-Reset should be set");

    // Second request
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip)
        .await
        .unwrap();
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
/// `Retry-After`, in the RFC's body shape, on both rate-limited endpoints, and
/// one address spending all of registration's allowance still has all of the
/// token endpoint's.
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

    // The token endpoint admits thirty per minute from the same address, whose
    // registration window is spent; a malformed grant still counts, since the
    // limiter runs before the grant is read.
    let token = || {
        Request::post("/oauth2/token")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("grant_type=client_credentials"))
            .unwrap()
    };
    for i in 1..=30 {
        let response = from_address(&app, token(), addr).await;
        assert_ne!(
            response.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "token request {i} is within the limit"
        );
    }
    assert_rate_limited(from_address(&app, token(), addr).await, "token").await;

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
    // through a real boundary at TEST_WINDOW_SECS instead of the shipped
    // 60-second default.
    let config = RateLimitConfig {
        oauth_register_rpm: 3, // Only 3 requests allowed
        rate_limit_window_secs: TEST_WINDOW_SECS,
        ..RateLimitConfig::default()
    };

    let rate_limiter = limiter_with(&config);
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 107));
    let endpoint = OAuth2Endpoint::Register;

    // Make 3 requests
    for i in 1..=3 {
        let status = rate_limiter
            .check_rate_limit(endpoint, client_ip)
            .await
            .unwrap();
        assert!(!status.is_limited, "Request {i} should succeed");
    }

    // 4th request should be rate limited
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip)
        .await
        .unwrap();
    assert!(status.is_limited, "Request 4 should be rate limited");

    // Wait for the configured window to expire
    sleep(TEST_EXPIRY_WAIT).await;

    // After window reset, should be able to make requests again
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip)
        .await
        .unwrap();
    assert!(
        !status.is_limited,
        "After window reset, requests should succeed again"
    );
    assert_eq!(status.remaining, 3); // Remaining is count before current request
}

/// Test custom rate limit configuration
#[tokio::test]
async fn test_custom_rate_limit_config() {
    let config = RateLimitConfig {
        oauth_register_rpm: 5,
        oauth_token_rpm: 15,
        oauth_authorize_rpm: 25,
        ..RateLimitConfig::default()
    };

    let rate_limiter = limiter_with(&config);
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 108));

    // Test custom register limit
    let status = rate_limiter
        .check_rate_limit(OAuth2Endpoint::Register, client_ip)
        .await
        .unwrap();
    assert_eq!(status.limit, 5, "Custom register limit should be 5");
    assert_eq!(status.remaining, 5);

    // Test custom token limit
    let status = rate_limiter
        .check_rate_limit(OAuth2Endpoint::Token, client_ip)
        .await
        .unwrap();
    assert_eq!(status.limit, 15, "Custom token limit should be 15");
    assert_eq!(status.remaining, 15);

    // Test custom authorize limit
    let status = rate_limiter
        .check_rate_limit(OAuth2Endpoint::Authorize, client_ip)
        .await
        .unwrap();
    assert_eq!(status.limit, 25, "Custom authorize limit should be 25");
    assert_eq!(status.remaining, 25);
}

/// Test rate limiting behavior at exact limit boundary
#[tokio::test]
async fn test_rate_limit_boundary() {
    let rate_limiter = limiter();
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 109));
    let endpoint = OAuth2Endpoint::Register;

    // Make exactly 10 requests (at limit)
    for i in 1..=10 {
        let status = rate_limiter
            .check_rate_limit(endpoint, client_ip)
            .await
            .unwrap();
        assert!(!status.is_limited, "Request {i} should succeed");
        if i == 10 {
            assert_eq!(
                status.remaining, 1,
                "Remaining should be 1 at limit boundary"
            );
        }
    }

    // One more request should be rejected
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip)
        .await
        .unwrap();
    assert!(status.is_limited, "Request beyond limit should be rejected");
}

/// Test concurrent requests from same IP
#[tokio::test]
async fn test_concurrent_requests_same_ip() {
    let rate_limiter = Arc::new(limiter());
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 110));
    let endpoint = OAuth2Endpoint::Token;

    let mut handles = vec![];

    // Spawn 30 concurrent requests
    for _i in 0..30 {
        let limiter = Arc::clone(&rate_limiter);
        let handle = tokio::spawn(async move {
            let status = limiter.check_rate_limit(endpoint, client_ip).await.unwrap();
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
    let status = rate_limiter
        .check_rate_limit(endpoint, client_ip)
        .await
        .unwrap();
    assert!(status.is_limited, "Request 31 should be rate limited");
}

/// Spending one endpoint's allowance leaves every other endpoint's allowance
/// for the same address whole, in either order.
#[tokio::test]
async fn test_endpoints_keep_separate_windows_per_ip() {
    let rate_limiter = limiter();

    // Token first: thirty admitted, the next refused.
    let token_first = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 111));
    for _ in 1..=30 {
        let status = rate_limiter
            .check_rate_limit(OAuth2Endpoint::Token, token_first)
            .await
            .unwrap();
        assert!(!status.is_limited);
    }
    let status = rate_limiter
        .check_rate_limit(OAuth2Endpoint::Token, token_first)
        .await
        .unwrap();
    assert!(status.is_limited, "token is past its limit");

    // Registration from the same address still has its ten.
    for i in 1..=10 {
        let status = rate_limiter
            .check_rate_limit(OAuth2Endpoint::Register, token_first)
            .await
            .unwrap();
        assert!(
            !status.is_limited,
            "registration {i} is within its own limit"
        );
        assert_eq!(status.limit, 10);
        assert_eq!(status.remaining, 11 - i);
    }
    let status = rate_limiter
        .check_rate_limit(OAuth2Endpoint::Authorize, token_first)
        .await
        .unwrap();
    assert!(!status.is_limited);
    assert_eq!(status.remaining, 60, "authorize is untouched");

    // Registration first: ten admitted, the next refused.
    let register_first = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 112));
    for _ in 1..=10 {
        let status = rate_limiter
            .check_rate_limit(OAuth2Endpoint::Register, register_first)
            .await
            .unwrap();
        assert!(!status.is_limited);
    }
    let status = rate_limiter
        .check_rate_limit(OAuth2Endpoint::Register, register_first)
        .await
        .unwrap();
    assert!(status.is_limited, "registration is past its limit");

    // The token endpoint from the same address still has its thirty.
    for i in 1..=30 {
        let status = rate_limiter
            .check_rate_limit(OAuth2Endpoint::Token, register_first)
            .await
            .unwrap();
        assert!(
            !status.is_limited,
            "token request {i} is within its own limit"
        );
        assert_eq!(status.remaining, 31 - i);
    }
}

/// Two limiters over one cache, as two replicas over one Redis are, count into
/// the same window: what one admitted, the other has already spent.
#[tokio::test]
async fn test_limiters_sharing_a_cache_share_windows() {
    let cache = Arc::new(common::create_test_cache().await.unwrap());
    let config = RateLimitConfig::default();
    let replica_a = OAuth2RateLimiter::new(
        Some(Arc::clone(&cache)),
        OAuth2RateLimiter::local_window_store(),
        &config,
    );
    let replica_b = OAuth2RateLimiter::new(
        Some(Arc::clone(&cache)),
        OAuth2RateLimiter::local_window_store(),
        &config,
    );
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 113));

    for _ in 1..=6 {
        let status = replica_a
            .check_rate_limit(OAuth2Endpoint::Register, client_ip)
            .await
            .unwrap();
        assert!(!status.is_limited);
    }
    for i in 7..=10 {
        let status = replica_b
            .check_rate_limit(OAuth2Endpoint::Register, client_ip)
            .await
            .unwrap();
        assert!(!status.is_limited, "registration {i} is within the limit");
        assert_eq!(status.remaining, 11 - i);
    }
    let status = replica_a
        .check_rate_limit(OAuth2Endpoint::Register, client_ip)
        .await
        .unwrap();
    assert!(
        status.is_limited,
        "the eleventh, on either replica, is refused"
    );
    assert_eq!(status.remaining, 0);
}

/// A token request, which the limiter counts before it reads the grant.
fn token_request() -> Request<Body> {
    Request::post("/oauth2/token")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("grant_type=client_credentials"))
        .unwrap()
}

/// A well-formed authorization request from `client_id` with no session,
/// which an admitted request answers with the login redirect.
///
/// The client is registered for [`AUTHORIZE_REDIRECT_URI`] and the request
/// carries a PKCE challenge, since `/oauth2/authorize` checks both before it
/// sends anyone to log in.
fn authorize_request(client_id: &str) -> Request<Body> {
    Request::get(format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}\
         &code_challenge={AUTHORIZE_PKCE_CHALLENGE}&code_challenge_method=S256",
        urlencoding::encode(client_id),
        urlencoding::encode(AUTHORIZE_REDIRECT_URI),
    ))
    .body(Body::empty())
    .unwrap()
}

/// Register the client [`authorize_request`] authorizes for, and return its id.
async fn register_authorize_client(oauth2_server: Arc<dyn OAuth2ServerRepository>) -> String {
    ClientRegistrationManager::new(oauth2_server)
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![AUTHORIZE_REDIRECT_URI.to_owned()],
                client_name: Some("Limiter client".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap()
        .client_id
}

/// `request` with `X-Forwarded-For: forwarded_for`.
fn forwarded(mut request: Request<Body>, forwarded_for: &str) -> Request<Body> {
    request
        .headers_mut()
        .insert("x-forwarded-for", forwarded_for.parse().unwrap());
    request
}

/// The `OAuth2` routes alone, over `limiter`, and the id of the client
/// [`authorize_request`] authorizes for in their store.
async fn oauth2_routes(limiter: OAuth2RateLimiter) -> (Router, String) {
    let database = Arc::new(
        create_test_db_with_key(generate_encryption_key().to_vec())
            .await
            .unwrap(),
    );
    let repos = database.repositories();
    let client_id = register_authorize_client(Arc::clone(&repos.oauth2_server)).await;
    let routes = OAuth2Routes::routes(OAuth2Context {
        database: Arc::clone(&database),
        oauth2_server: Arc::clone(&repos.oauth2_server),
        tenants: Arc::clone(&repos.tenants),
        users: Arc::clone(&repos.users),
        auth_manager: common::create_test_auth_manager(),
        jwks_manager: common::get_shared_test_jwks(),
        config: Arc::new(OAuth2ServerConfig::default()),
        rate_limiter: Arc::new(limiter),
    });
    (routes, client_id)
}

/// `response`'s body as text.
async fn page(response: Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

/// A shared store that cannot count a request (Redis unreachable, or a key it
/// cannot increment) does not refuse sign-in: the request is counted in this
/// process's window, which admits and refuses exactly as the shared one would.
#[tokio::test]
async fn test_a_shared_store_that_cannot_count_falls_back_to_this_process() {
    let shared = Arc::new(common::create_test_cache().await.unwrap());
    let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 114));
    let poisoned = window_key("register", client_ip);
    shared
        .set(&poisoned, &"not a count", Duration::from_mins(1))
        .await
        .unwrap();
    let limiter = OAuth2RateLimiter::new(
        Some(Arc::clone(&shared)),
        OAuth2RateLimiter::local_window_store(),
        &RateLimitConfig::default(),
    );

    for i in 1..=10 {
        let status = limiter
            .check_rate_limit(OAuth2Endpoint::Register, client_ip)
            .await
            .unwrap();
        assert!(!status.is_limited, "registration {i} is within the limit");
        assert_eq!(status.limit, 10);
        assert_eq!(status.remaining, 11 - i, "metered, not waved through");
    }
    let status = limiter
        .check_rate_limit(OAuth2Endpoint::Register, client_ip)
        .await
        .unwrap();
    assert!(status.is_limited, "the eleventh is refused by the fallback");
    assert!(status.retry_after_seconds.is_some());

    // The shared store was never written; another endpoint's window there
    // still counts in it.
    let unreadable: Option<String> = shared.get(&poisoned).await.unwrap();
    assert_eq!(unreadable.as_deref(), Some("not a count"));
    limiter
        .check_rate_limit(OAuth2Endpoint::Token, client_ip)
        .await
        .unwrap();
    let token_hits: Option<u64> = shared.get(&window_key("token", client_ip)).await.unwrap();
    assert_eq!(token_hits, Some(1));
}

/// When neither store can count a request it is refused, never admitted
/// unmetered: a 503 `temporarily_unavailable` body on the token endpoint, and
/// the hosted error page with a 503, and no `Retry-After`, instead of the
/// login redirect on the authorize endpoint.
#[tokio::test]
async fn test_uncountable_window_refuses_with_temporarily_unavailable() {
    let addr = SocketAddr::from(([203, 0, 113, 21], 40_000));
    let shared = Arc::new(common::create_test_cache().await.unwrap());
    let local = OAuth2RateLimiter::local_window_store();
    for endpoint in ["token", "authorize"] {
        let key = window_key(endpoint, addr.ip());
        shared
            .set(&key, &"not a count", Duration::from_mins(1))
            .await
            .unwrap();
        local
            .set(&key, &"not a count", Duration::from_mins(1))
            .await
            .unwrap();
    }
    let (app, client_id) = oauth2_routes(OAuth2RateLimiter::new(
        Some(shared),
        local,
        &RateLimitConfig::default(),
    ))
    .await;

    let response = from_address(&app, token_request(), addr).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get(RETRY_AFTER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"], "temporarily_unavailable");
    assert_eq!(body["error_description"], LIMITER_UNAVAILABLE);

    let response = from_address(&app, authorize_request(&client_id), addr).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get(RETRY_AFTER).is_none());
    // The hosted page names no error detail, so its status is what tells an
    // uncountable window (503, no wait) from a spent one (429, Retry-After).
    let page = page(response).await;
    assert!(page.contains("Connection Failed"), "{page}");

    // Another address's windows are counted as usual: its token request
    // reaches the grant check, its authorization request the login redirect.
    let elsewhere = SocketAddr::from(([203, 0, 113, 22], 40_000));
    let response = from_address(&app, token_request(), elsewhere).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"], "invalid_request");
    let response = from_address(&app, authorize_request(&client_id), elsewhere).await;
    assert!(
        response.status().is_redirection(),
        "admitted authorization redirects to login, got {}",
        response.status()
    );
}

/// Past its sixty, `/oauth2/authorize` answers the hosted error page with a
/// 429 and the window's `Retry-After`.
#[tokio::test]
async fn test_authorize_past_its_limit_answers_a_429_page_with_retry_after() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let addr = SocketAddr::from(([203, 0, 113, 31], 40_000));
    let client_id =
        register_authorize_client(Arc::clone(&resources.common.repos.oauth2_server)).await;

    for i in 1..=60 {
        let response = from_address(&app, authorize_request(&client_id), addr).await;
        assert!(
            response.status().is_redirection(),
            "authorization {i} is within the limit, got {}",
            response.status()
        );
    }
    let response = from_address(&app, authorize_request(&client_id), addr).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: u64 = response
        .headers()
        .get(RETRY_AFTER)
        .expect("the refusal carries Retry-After")
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry_after), "{retry_after}");
    let page = page(response).await;
    assert!(page.contains("Connection Failed"), "{page}");
}

/// Behind a trusted proxy every request has the proxy's address as its peer;
/// each client is keyed by the address the proxies recorded for it, never by
/// the peer and never by an entry the client wrote itself.
#[tokio::test]
async fn test_clients_behind_one_proxy_keep_separate_windows() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    // nginx inside the private network; the entries to its right are the
    // internal hops the chain appends after the client's own address.
    let proxy = SocketAddr::from(([10, 8, 0, 3], 51_000));
    let alice = "198.51.100.41, 169.254.1.1";
    let bob = "198.51.100.42, 169.254.1.1";

    for i in 1..=30 {
        let response = from_address(&app, forwarded(token_request(), alice), proxy).await;
        assert_ne!(
            response.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "alice's token request {i} is within her limit"
        );
    }
    assert_rate_limited(
        from_address(&app, forwarded(token_request(), alice), proxy).await,
        "token",
    )
    .await;

    // Bob reaches the server through the same proxy with his own window.
    let response = from_address(&app, forwarded(token_request(), bob), proxy).await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "bob is admitted"
    );

    // Alice cannot open a fresh window by forging an entry in front of hers.
    let forged = format!("203.0.113.250, {alice}");
    assert_rate_limited(
        from_address(&app, forwarded(token_request(), &forged), proxy).await,
        "token",
    )
    .await;

    // A peer outside the trusted networks is the client: whatever it claims
    // in X-Forwarded-For, it spends its own window.
    let direct = SocketAddr::from(([203, 0, 113, 77], 52_000));
    for i in 1..=30 {
        let claim = format!("198.51.100.{i}");
        let response = from_address(&app, forwarded(token_request(), &claim), direct).await;
        assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS, "{i}");
    }
    assert_rate_limited(
        from_address(&app, forwarded(token_request(), "198.51.100.200"), direct).await,
        "token",
    )
    .await;

    // With no shared store the windows live in the limiter's own store,
    // never in the server's in-memory cache.
    let client = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 41));
    assert!(!resources
        .common
        .cache
        .exists(&window_key("token", client))
        .await
        .unwrap());
}

/// carnet#623, the live regression: through the load balancer, nginx and the
/// VPC hop, every request reached the backend ending in `0.0.0.0`, so a
/// second client's first token request landed in the first client's spent
/// window. Each client now keeps its own.
#[tokio::test]
async fn test_clients_through_the_deployed_chain_keep_separate_windows() {
    let resources = common::create_test_server_resources_with_config(ServerConfig {
        activity_fetch_limit: 100,
        rate_limiting: RateLimitConfig {
            trusted_proxies: trusted_proxies(DEV_TRUSTED_PROXY_CIDRS),
            ..RateLimitConfig::default()
        },
        ..ServerConfig::default()
    })
    .await
    .unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    // The backend's own peer: its Cloud Run sandbox proxy.
    let peer = SocketAddr::from(([169, 254, 169, 126], 51_000));
    let alice = format!("198.51.100.41,{DEPLOYED_TAIL}");

    for i in 1..=30 {
        let response = from_address(&app, forwarded(token_request(), &alice), peer).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "alice's token request {i} is within her limit"
        );
    }
    assert_rate_limited(
        from_address(&app, forwarded(token_request(), &alice), peer).await,
        "token",
    )
    .await;

    // A second client through the same hops: its first call is admitted.
    let bob = format!("136.86.205.10,{DEPLOYED_TAIL}");
    let response = from_address(&app, forwarded(token_request(), &bob), peer).await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "bob's first token request has his own window"
    );

    // Entries alice writes in front of her own, trusted-looking or not, do
    // not open her a fresh window.
    for forged in ["203.0.113.250", "0.0.0.0", "136.68.126.109"] {
        let chain = format!("{forged}, {alice}");
        assert_rate_limited(
            from_address(&app, forwarded(token_request(), &chain), peer).await,
            "token",
        )
        .await;
    }
}

/// An IPv6 client is metered by its /64: rotating through the addresses a
/// host is handed opens no fresh window.
#[tokio::test]
async fn test_ipv6_clients_share_a_window_per_slash_64() {
    let rate_limiter = limiter();
    let first: IpAddr = "2001:db8:0:7::1".parse().unwrap();
    for i in 1..=10u16 {
        let rotated = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 7, i, 0, 0, i));
        let status = rate_limiter
            .check_rate_limit(OAuth2Endpoint::Register, rotated)
            .await
            .unwrap();
        assert!(!status.is_limited, "{rotated}");
    }
    let status = rate_limiter
        .check_rate_limit(OAuth2Endpoint::Register, first)
        .await
        .unwrap();
    assert!(status.is_limited, "the /64 spent its ten");

    let next_subnet: IpAddr = "2001:db8:0:8::1".parse().unwrap();
    let status = rate_limiter
        .check_rate_limit(OAuth2Endpoint::Register, next_subnet)
        .await
        .unwrap();
    assert!(!status.is_limited);
    assert_eq!(status.remaining, 10);
}
