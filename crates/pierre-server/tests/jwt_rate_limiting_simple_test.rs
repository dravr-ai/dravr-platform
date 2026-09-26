// ABOUTME: Simple test to verify JWT rate limiting works correctly
// ABOUTME: Tests critical security fix for JWT tokens having unlimited API access
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Simple test to verify JWT rate limiting works
//!
//! This is a focused test to verify that the critical security vulnerability
//! where JWT tokens had unlimited API access has been fixed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use chrono::Utc;
use pierre_auth::auth::AuthManager;
use pierre_auth::rate_limiting::{calculate_jwt_rate_limit, RequestBudget};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{JwtMonthlyUsage, MonthlyLimitOverride, User};
use pierre_database::database::generate_encryption_key;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::repositories::analytics::next_utc_month_start;
use pierre_middleware::rate_limiting::enforce_request_budget;
use pierre_middleware::McpAuthMiddleware;
use std::sync::Arc;

#[tokio::test]
async fn test_jwt_tokens_now_have_rate_limiting() {
    // Create test database
    let encryption_key = generate_encryption_key().to_vec();

    let database = Arc::new(create_test_db_with_key(encryption_key).await.unwrap());

    // Create auth manager and middleware
    let auth_manager = AuthManager::new(24);
    let jwks_manager = common::get_shared_test_jwks();
    let repos = Arc::new(database.repositories());
    let auth_middleware = Arc::new(McpAuthMiddleware::new(
        auth_manager,
        repos,
        jwks_manager.clone(),
    ));

    // Create and store a test user (defaults to Starter tier with 10,000 requests/month)
    let user = User::new(
        "jwt_test@example.com".to_owned(),
        "hashed_password".to_owned(),
        Some("JWT Test User".to_owned()),
    );
    database.repositories().users.create(&user).await.unwrap();

    // Create a JWT token for the user (using same secret for consistency)
    let token_auth_manager = AuthManager::new(24);
    let token = token_auth_manager
        .generate_token(&user, &jwks_manager)
        .expect("Failed to generate JWT token");

    // Authenticating counts the request: one jwt_usage row per admitted JWT
    // request, which the next request's budget is read from.
    let repos = database.repositories();
    assert_eq!(
        repos.usage.get_jwt_current_usage(user.id).await.unwrap(),
        JwtMonthlyUsage {
            used: 0,
            monthly_override: MonthlyLimitOverride::NotSet,
        }
    );
    auth_middleware
        .authenticate_request(Some(&format!("Bearer {token}")))
        .await
        .expect("JWT authentication should succeed");
    let usage = repos.usage.get_jwt_current_usage(user.id).await.unwrap();
    assert_eq!(usage.used, 1, "the admitted request is counted");

    // CRITICAL SECURITY FIX VERIFICATION
    // Before: JWT tokens had no budget (unlimited access)
    // After: JWT tokens are metered by the user's tier, 10,000 for Starter,
    // resetting at the first instant of the next UTC month.
    let now = Utc::now();
    let budget = calculate_jwt_rate_limit(&user, usage, now);
    assert_eq!(
        budget,
        RequestBudget::Metered {
            limit: 10_000,
            used: 1,
            resets_at: next_utc_month_start(now),
        },
        "SECURITY FIX: JWT should have the Starter tier budget, not unlimited!"
    );
    assert_eq!(budget.remaining_after_this_request(), Some(9_998));

    // At the tier's limit the gate refuses with a 429 and the seconds to reset
    let spent = calculate_jwt_rate_limit(
        &user,
        JwtMonthlyUsage {
            used: 10_000,
            ..usage
        },
        now,
    );
    let refusal = enforce_request_budget(spent, now).unwrap_err();
    assert_eq!(refusal.code, ErrorCode::RateLimitExceeded);
    let expected_wait = (next_utc_month_start(now) - now).num_seconds();
    assert_eq!(
        refusal.retry_after_secs(),
        Some(u64::try_from(expected_wait.max(1)).unwrap())
    );
}
