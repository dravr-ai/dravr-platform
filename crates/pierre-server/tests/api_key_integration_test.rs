// ABOUTME: Integration tests for API key authentication and MCP workflows
// ABOUTME: Tests API key creation, validation, rate limiting, and MCP protocol integration
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]

mod common;

use chrono::{Duration, Utc};
use pierre_auth::{
    api_keys::{ApiKey, ApiKeyManager, ApiKeyTier, ApiKeyUsage, CreateApiKeyRequest},
    auth::{AuthManager, AuthMethod},
    rate_limiting::{api_key_window_start, calculate_api_key_rate_limit, RequestBudget},
};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{ApiKeyWindowUsage, User};
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::{backends::factory::Database, database::generate_encryption_key};
use pierre_middleware::McpAuthMiddleware;
use std::sync::Arc;

async fn create_test_environment() -> (
    Arc<Database>,
    Arc<AuthManager>,
    Arc<McpAuthMiddleware>,
    User,
    String,
) {
    // Create test database
    let encryption_key = generate_encryption_key().to_vec();

    let database = Arc::new(create_test_db_with_key(encryption_key).await.unwrap());

    // Create auth manager
    let auth_manager = Arc::new(AuthManager::new(24));

    // Create auth middleware
    let jwks_manager = common::get_shared_test_jwks();
    let repos = Arc::new(database.repositories());
    let auth_middleware = Arc::new(McpAuthMiddleware::new(
        (*auth_manager).clone(),
        repos,
        jwks_manager,
    ));

    // Create test user
    let user = User::new(
        "integration@example.com".to_owned(),
        "hashed_password".to_owned(),
        Some("Integration Test User".to_owned()),
    );
    database.repositories().users.create(&user).await.unwrap();

    // Generate JWT token for the user
    let jwks_manager = common::get_shared_test_jwks();
    let jwt_token = auth_manager.generate_token(&user, &jwks_manager).unwrap();

    (database, auth_manager, auth_middleware, user, jwt_token)
}

/// The key's calls inside its own sliding window, as the gate reads them.
async fn window_usage(database: &Database, api_key: &ApiKey) -> ApiKeyWindowUsage {
    database
        .repositories()
        .usage
        .get_api_key_window_usage(&api_key.id, api_key_window_start(api_key, Utc::now()))
        .await
        .unwrap()
}

#[tokio::test]
async fn test_end_to_end_api_key_workflow() {
    let (database, _auth_manager, auth_middleware, user, _jwt_token) =
        create_test_environment().await;
    let api_key_manager = ApiKeyManager::new();

    // Step 1: Create an API key
    let request = CreateApiKeyRequest {
        name: "E2E Test Key".to_owned(),
        description: Some("End-to-end test API key".to_owned()),
        tier: ApiKeyTier::Professional,
        expires_in_days: Some(30),
        rate_limit_requests: None,
    };

    let (api_key, full_key) = api_key_manager.create_api_key(user.id, request).unwrap();
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    // Step 2: Authenticate using the API key
    let auth_result = auth_middleware
        .authenticate_request(Some(&full_key))
        .await
        .unwrap();

    assert_eq!(auth_result.user_id, user.id);
    assert!(matches!(auth_result.auth_method, AuthMethod::ApiKey { .. }));

    // Step 3: Authentication alone writes no usage row: the request-budget
    // layer writes it once the request has a real outcome, and this call ran
    // outside any request
    let usage = window_usage(&database, &api_key).await;
    assert_eq!(usage.count, 0);
    let budget = calculate_api_key_rate_limit(&api_key, &usage, Utc::now());
    assert!(!budget.is_exceeded());
    let RequestBudget::Metered { limit, used, .. } = budget else {
        panic!("a professional key is metered, got {budget:?}");
    };
    assert_eq!((limit, used), (100_000, 0)); // Professional tier
    assert_eq!(budget.remaining_after_this_request(), Some(99_999));

    // Step 4: Record a tool call the way the request-budget layer does
    let usage = ApiKeyUsage {
        id: None,
        api_key_id: api_key.id.clone(),
        timestamp: Utc::now(),
        tool_name: "get_activities".to_owned(),
        response_time_ms: Some(150),
        status_code: 200,
        error_message: None,
        request_size_bytes: Some(256),
        response_size_bytes: Some(1024),
        ip_address: Some("127.0.0.1".to_owned()),
        user_agent: Some("test-client".to_owned()),
    };

    database
        .repositories()
        .usage
        .record_api_key(&usage)
        .await
        .unwrap();

    // Step 5: The row is in the window
    assert_eq!(window_usage(&database, &api_key).await.count, 1);

    // Step 6: Get usage statistics
    let start_date = Utc::now() - Duration::days(1);
    let end_date = Utc::now() + Duration::days(1);
    let stats = database
        .repositories()
        .usage
        .get_api_key_stats(&api_key.id, start_date, end_date)
        .await
        .unwrap();

    assert_eq!(stats.total_requests, 1);
    assert_eq!(stats.successful_requests, 1);
    assert_eq!(stats.failed_requests, 0);
    assert_eq!(stats.total_response_time_ms, 150);
    assert_eq!(stats.tool_usage["get_activities"]["count"], 1);
    assert_eq!(
        stats.tool_usage.as_object().map(serde_json::Map::len),
        Some(1),
        "only the recorded call, no row invented by authentication: {}",
        stats.tool_usage
    );
}

#[tokio::test]
async fn test_api_key_rate_limiting() {
    let (database, _auth_manager, auth_middleware, user, _jwt_token) =
        create_test_environment().await;
    let api_key_manager = ApiKeyManager::new();

    // Create a Starter tier key with low limit for testing
    let request = CreateApiKeyRequest {
        name: "Rate Limit Test Key".to_owned(),
        description: None,
        tier: ApiKeyTier::Starter,
        expires_in_days: None,
        rate_limit_requests: None,
    };

    let (mut api_key, full_key) = api_key_manager.create_api_key(user.id, request).unwrap();

    // Override rate limit for testing (simulate a very low limit)
    api_key.rate_limit_requests = 2;
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    // Two calls already made inside the key's window
    for _ in 0..2 {
        database
            .repositories()
            .usage
            .record_api_key(&ApiKeyUsage {
                id: None,
                api_key_id: api_key.id.clone(),
                timestamp: Utc::now(),
                tool_name: "GET /api/usage/status".to_owned(),
                response_time_ms: Some(12),
                status_code: 200,
                error_message: None,
                request_size_bytes: None,
                response_size_bytes: None,
                ip_address: None,
                user_agent: None,
            })
            .await
            .unwrap();
    }
    let usage = window_usage(&database, &api_key).await;
    assert_eq!(usage.count, 2);
    let budget = calculate_api_key_rate_limit(&api_key, &usage, Utc::now());
    assert!(budget.is_exceeded());
    assert_eq!(budget.remaining_after_this_request(), Some(0));

    // The third is refused as a 429 with a retry window, not a 401
    let refusal = auth_middleware
        .authenticate_request(Some(&full_key))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, ErrorCode::RateLimitExceeded);
    assert!(refusal.to_string().contains("Rate limit exceeded"));
    assert!(refusal.retry_after_secs().unwrap() >= 1);
}

#[tokio::test]
async fn test_enterprise_tier_unlimited_usage() {
    let (database, _auth_manager, auth_middleware, user, _jwt_token) =
        create_test_environment().await;
    let api_key_manager = ApiKeyManager::new();

    // Create an Enterprise tier key
    let request = CreateApiKeyRequest {
        name: "Enterprise Test Key".to_owned(),
        description: None,
        tier: ApiKeyTier::Enterprise,
        expires_in_days: None,
        rate_limit_requests: None,
    };

    let (api_key, full_key) = api_key_manager.create_api_key(user.id, request).unwrap();
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    // Record high usage
    for i in 0..1000 {
        let usage = ApiKeyUsage {
            id: None,
            api_key_id: api_key.id.clone(),
            timestamp: Utc::now(),
            tool_name: format!("bulk_tool_{i}"),
            response_time_ms: Some(50),
            status_code: 200,
            error_message: None,
            request_size_bytes: None,
            response_size_bytes: None,
            ip_address: None,
            user_agent: None,
        };
        database
            .repositories()
            .usage
            .record_api_key(&usage)
            .await
            .unwrap();
    }

    // Verify high usage is recorded
    let usage = window_usage(&database, &api_key).await;
    assert_eq!(usage.count, 1000);

    // Enterprise tier is never rate limited
    assert_eq!(
        calculate_api_key_rate_limit(&api_key, &usage, Utc::now()),
        RequestBudget::Unlimited
    );

    // Authentication still succeeds. It writes no row itself: the
    // request-budget layer records the call once the request has an outcome.
    auth_middleware
        .authenticate_request(Some(&full_key))
        .await
        .unwrap();
    assert_eq!(window_usage(&database, &api_key).await.count, 1000);
}

#[tokio::test]
async fn test_api_key_expiration() {
    let (database, _auth_manager, auth_middleware, user, _jwt_token) =
        create_test_environment().await;
    let api_key_manager = ApiKeyManager::new();

    // Create an expired API key
    let request = CreateApiKeyRequest {
        name: "Expired Test Key".to_owned(),
        description: None,
        tier: ApiKeyTier::Starter,
        expires_in_days: Some(1),
        rate_limit_requests: None,
    };

    let (mut api_key, full_key) = api_key_manager.create_api_key(user.id, request).unwrap();

    // Manually set expiration to past date
    api_key.expires_at = Some(Utc::now() - Duration::days(1));
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    // Authentication should fail due to expiration
    let auth_result = auth_middleware.authenticate_request(Some(&full_key)).await;
    assert!(auth_result.is_err());
    assert!(auth_result.unwrap_err().to_string().contains("expired"));
}

#[tokio::test]
async fn test_deactivated_api_key() {
    let (database, _auth_manager, auth_middleware, user, _jwt_token) =
        create_test_environment().await;
    let api_key_manager = ApiKeyManager::new();

    // Create an API key
    let request = CreateApiKeyRequest {
        name: "Deactivated Test Key".to_owned(),
        description: None,
        tier: ApiKeyTier::Professional,
        expires_in_days: None,
        rate_limit_requests: None,
    };

    let (api_key, full_key) = api_key_manager.create_api_key(user.id, request).unwrap();
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    // Verify it works initially
    let auth_result = auth_middleware
        .authenticate_request(Some(&full_key))
        .await
        .unwrap();
    assert_eq!(auth_result.user_id, user.id);

    // Deactivate the key
    database
        .repositories()
        .api_keys
        .deactivate(&api_key.id, user.id)
        .await
        .unwrap();

    // Authentication should now fail
    let auth_result = auth_middleware.authenticate_request(Some(&full_key)).await;
    assert!(auth_result.is_err());
    let error_msg = auth_result.unwrap_err().to_string();
    assert!(error_msg.contains("API key not found or invalid"));
}

#[tokio::test]
async fn test_invalid_api_key_format() {
    let (_database, _auth_manager, auth_middleware, _user, _jwt_token) =
        create_test_environment().await;

    // Test various invalid key formats
    let invalid_keys = vec![
        "invalid_key",
        "pk_test_abcdefghijklmnopqrstuvwxyz123456", // Wrong prefix
        "pk_live_short",                            // Too short
        "pk_live_abcdefghijklmnopqrstuvwxyz12345",  // Too short by 1
        "pk_live_abcdefghijklmnopqrstuvwxyz1234567", // Too long by 1
        "",                                         // Empty
        "bearer token",                             // JWT-like format
    ];

    for invalid_key in invalid_keys {
        let auth_result = auth_middleware
            .authenticate_request(Some(invalid_key))
            .await;
        assert!(
            auth_result.is_err(),
            "Key '{}' should be invalid",
            invalid_key
        );
    }
}

#[tokio::test]
async fn test_concurrent_api_key_usage() {
    let (database, _auth_manager, auth_middleware, user, _jwt_token) =
        create_test_environment().await;
    let api_key_manager = ApiKeyManager::new();

    // Create an API key
    let request = CreateApiKeyRequest {
        name: "Concurrent Test Key".to_owned(),
        description: None,
        tier: ApiKeyTier::Professional,
        expires_in_days: None,
        rate_limit_requests: None,
    };

    let (api_key, full_key) = api_key_manager.create_api_key(user.id, request).unwrap();
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    // Simulate concurrent requests
    let mut handles = vec![];
    for _ in 0..10 {
        let auth_middleware_clone = auth_middleware.clone();
        let full_key_clone = full_key.clone();

        let handle = tokio::spawn(async move {
            let auth_result = auth_middleware_clone
                .authenticate_request(Some(&full_key_clone))
                .await
                .unwrap();

            auth_result.user_id
        });

        handles.push(handle);
    }

    // Wait for all requests to complete
    let mut results = vec![];
    for handle in handles {
        results.push(handle.await.unwrap());
    }

    // Verify all requests succeeded and returned the correct user ID
    assert_eq!(results.len(), 10);
    for user_id in results {
        assert_eq!(user_id, user.id);
    }

    // Admission writes no row; each request's row is the request-budget
    // layer's, pinned per concurrent request in rate_limit_headers_e2e_test.
    assert_eq!(window_usage(&database, &api_key).await.count, 0);
}

#[tokio::test]
async fn test_usage_analytics() {
    let (database, _auth_manager, _auth_middleware, user, _jwt_token) =
        create_test_environment().await;
    let api_key_manager = ApiKeyManager::new();

    // Create an API key
    let request = CreateApiKeyRequest {
        name: "Analytics Test Key".to_owned(),
        description: None,
        tier: ApiKeyTier::Professional,
        expires_in_days: None,
        rate_limit_requests: None,
    };

    let (api_key, _full_key) = api_key_manager.create_api_key(user.id, request).unwrap();
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    // Record diverse usage patterns
    let tools = ["get_activities", "get_athlete", "analyze_activity"];
    let status_codes = [200, 200, 400, 200, 500]; // Mix of success and errors

    for (i, &status_code) in status_codes.iter().enumerate() {
        let usage = ApiKeyUsage {
            id: None,
            api_key_id: api_key.id.clone(),
            timestamp: Utc::now() - Duration::hours(i as i64),
            tool_name: tools[i % tools.len()].to_owned(),
            response_time_ms: Some((100 + i * 50) as u32),
            status_code,
            error_message: if status_code >= 400 {
                Some(format!("Error {status_code}"))
            } else {
                None
            },
            request_size_bytes: Some(256),
            response_size_bytes: Some(1024),
            ip_address: Some("127.0.0.1".to_owned()),
            user_agent: Some("test-client".to_owned()),
        };
        database
            .repositories()
            .usage
            .record_api_key(&usage)
            .await
            .unwrap();
    }

    // Get usage statistics
    let start_date = Utc::now() - Duration::days(1);
    let end_date = Utc::now() + Duration::hours(1);
    let stats = database
        .repositories()
        .usage
        .get_api_key_stats(&api_key.id, start_date, end_date)
        .await
        .unwrap();

    // Verify statistics
    assert_eq!(stats.total_requests, 5);
    assert_eq!(stats.successful_requests, 3); // 200 status codes
    assert_eq!(stats.failed_requests, 2); // 400 and 500 status codes
    assert_eq!(stats.total_response_time_ms, 100 + 150 + 200 + 250 + 300); // Sum of response times

    // Verify tool usage breakdown is captured
    assert!(stats.tool_usage.is_object());
}

#[tokio::test]
async fn test_create_api_key_invalid_auth() {
    let (_database, _auth_manager, auth_middleware, _user, _jwt_token) =
        create_test_environment().await;

    // Test with no authorization header
    let result = auth_middleware.authenticate_request(None).await;
    assert!(result.is_err());
    // Just verify auth fails - don't check exact error message

    // Test with invalid bearer token format
    let result = auth_middleware
        .authenticate_request(Some("invalid_token"))
        .await;
    assert!(result.is_err());

    // Test with malformed bearer token
    let result = auth_middleware.authenticate_request(Some("Bearer ")).await;
    assert!(result.is_err());

    // Test with completely fake token
    let result = auth_middleware
        .authenticate_request(Some("Bearer fake_token_12345"))
        .await;
    assert!(result.is_err());
}
