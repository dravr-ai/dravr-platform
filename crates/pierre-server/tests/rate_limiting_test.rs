// ABOUTME: Rate limiting integration tests for API throttling
// ABOUTME: Tests the request-budget calculators, the gate, the API-key window and the month reset
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Rate limiting integration tests
//!
//! The calculators are pure functions of the principal, its usage and `now`,
//! so they are pinned here against fixed instants. The middleware tests drive
//! real API keys through `McpAuthMiddleware` against a real database: the gate
//! admits a request while the key's window has room and refuses the next one
//! with a 429 and a retry window. Admission writes no `api_key_usage` row; the
//! request-budget layer writes it once the request has an outcome
//! (`rate_limiting_middleware_test`, `rate_limit_headers_e2e_test`), so these
//! tests seed the window with the rows that layer writes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use chrono::{DateTime, Duration, TimeZone, Timelike, Utc};
use pierre_auth::{
    api_keys::{
        ApiKey, ApiKeyManager, ApiKeyTier, ApiKeyUsage, CreateApiKeyRequest,
        CreateApiKeyRequestSimple,
    },
    auth::AuthManager,
    rate_limiting::{
        api_key_window_start, calculate_api_key_rate_limit, calculate_jwt_rate_limit, RequestBudget,
    },
};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{
    ApiKeyWindowUsage, JwtMonthlyUsage, MonthlyLimitOverride, User, UserTier,
};
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::repositories::analytics::next_utc_month_start;
use pierre_database::{backends::factory::Database, database::generate_encryption_key};
use pierre_middleware::rate_limiting::enforce_request_budget;
use pierre_middleware::McpAuthMiddleware;
use std::sync::Arc;
use uuid::Uuid;

/// Thirty days, the window every tier's keys are minted with.
const MONTH_WINDOW_SECS: u32 = 30 * 24 * 60 * 60;

async fn create_test_setup() -> (Arc<Database>, ApiKeyManager, Arc<McpAuthMiddleware>, User) {
    // Create test database
    let encryption_key = generate_encryption_key().to_vec();

    let database = Arc::new(create_test_db_with_key(encryption_key).await.unwrap());

    // Create auth manager and middleware
    let auth_manager = AuthManager::new(24);
    let jwks_manager = common::get_shared_test_jwks();
    let repos = Arc::new(database.repositories());
    let auth_middleware = Arc::new(McpAuthMiddleware::new(auth_manager, repos, jwks_manager));

    // Create API key manager
    let api_key_manager = ApiKeyManager::new();

    // Create test user with unique email
    let unique_id = Uuid::new_v4();
    let user = User::new(
        format!("ratelimit+{unique_id}@example.com"),
        "hashed_password".to_owned(),
        Some("Rate Limit Test User".to_owned()),
    );

    (database, api_key_manager, auth_middleware, user)
}

/// A fixed instant, so calculator results are exact rather than racing the clock.
fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 25, 12, 0, 0).unwrap()
}

/// An API key for `full_key`, hashed the way authentication looks it up.
fn api_key_for(
    manager: &ApiKeyManager,
    user: &User,
    full_key: &str,
    tier: ApiKeyTier,
    rate_limit_requests: u32,
    rate_limit_window_seconds: u32,
) -> ApiKey {
    ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id: user.id,
        name: format!("{tier:?} rate limit test"),
        key_prefix: manager.extract_key_prefix(full_key),
        key_hash: manager.hash_key(full_key),
        description: None,
        tier,
        rate_limit_requests,
        rate_limit_window_seconds,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    }
}

/// One recorded call for `api_key_id` at `at`.
fn call(api_key_id: &str, at: DateTime<Utc>, tool_name: &str, status_code: u16) -> ApiKeyUsage {
    ApiKeyUsage {
        id: None,
        api_key_id: api_key_id.to_owned(),
        timestamp: at,
        tool_name: tool_name.to_owned(),
        response_time_ms: Some(100),
        status_code,
        error_message: (status_code >= 400).then(|| format!("Error {status_code}")),
        request_size_bytes: None,
        response_size_bytes: None,
        ip_address: None,
        user_agent: None,
    }
}

async fn seed_calls(database: &Database, api_key_id: &str, count: u32, at: DateTime<Utc>) {
    let usage = database.repositories().usage;
    for i in 0..count {
        usage
            .record_api_key(&call(api_key_id, at, &format!("seeded_tool_{i}"), 200))
            .await
            .unwrap();
    }
}

async fn window_usage(database: &Database, api_key: &ApiKey) -> ApiKeyWindowUsage {
    database
        .repositories()
        .usage
        .get_api_key_window_usage(&api_key.id, api_key_window_start(api_key, Utc::now()))
        .await
        .unwrap()
}

fn user_on(tier: UserTier) -> User {
    let mut user = User::new(
        format!("tier+{}@example.com", Uuid::new_v4()),
        "hashed_password".to_owned(),
        None,
    );
    user.tier = tier;
    user
}

/// `user`'s budget under `monthly_override`, `used_this_month` requests
/// counted.
fn budget_under(
    user: &User,
    monthly_override: MonthlyLimitOverride,
    used_this_month: u32,
    now: DateTime<Utc>,
) -> RequestBudget {
    calculate_jwt_rate_limit(
        user,
        JwtMonthlyUsage {
            used: used_this_month,
            monthly_override,
        },
        now,
    )
}

/// `user`'s budget with no override set, `used_this_month` requests counted.
fn tier_budget(user: &User, used_this_month: u32, now: DateTime<Utc>) -> RequestBudget {
    budget_under(user, MonthlyLimitOverride::NotSet, used_this_month, now)
}

#[tokio::test]
async fn test_starter_tier_rate_limiting() {
    let (database, api_key_manager, auth_middleware, user) = create_test_setup().await;
    database.repositories().users.create(&user).await.unwrap();

    let full_key = "pk_live_ratelimitstarterkey1234567890123";
    let api_key = api_key_for(
        &api_key_manager,
        &user,
        full_key,
        ApiKeyTier::Starter,
        5,
        MONTH_WINDOW_SECS,
    );
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    // Four calls in the window: the fifth is admitted. Admission itself
    // writes no row, so the window still holds what was seeded.
    seed_calls(&database, &api_key.id, 4, Utc::now()).await;
    auth_middleware
        .authenticate_request(Some(full_key))
        .await
        .unwrap();
    assert_eq!(
        window_usage(&database, &api_key).await.count,
        4,
        "authentication alone writes no api_key_usage row"
    );

    // The fifth call's row, as the request-budget layer writes it, fills the
    // window.
    seed_calls(&database, &api_key.id, 1, Utc::now()).await;

    // The sixth is refused as a 429 with the seconds until the oldest call
    // leaves the 30-day window.
    let refusal = auth_middleware
        .authenticate_request(Some(full_key))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, ErrorCode::RateLimitExceeded);
    assert_eq!(refusal.http_status(), 429);
    let retry_after = refusal.retry_after_secs().unwrap();
    let window = u64::from(MONTH_WINDOW_SECS);
    assert!(
        retry_after <= window && retry_after + 120 >= window,
        "retry window {retry_after}s should be the 30-day window less the seconds since the first call"
    );
    let details = refusal.details.as_deref().unwrap();
    assert_eq!(details["current"], 5);
    assert_eq!(details["limit"], 5);
    assert_eq!(
        window_usage(&database, &api_key).await.count,
        5,
        "a refused request is not counted"
    );
}

#[tokio::test]
async fn test_professional_tier_rate_limiting() {
    let (database, api_key_manager, auth_middleware, user) = create_test_setup().await;
    database.repositories().users.create(&user).await.unwrap();

    let full_key = "pk_live_professionallimitkey123456789012";
    let api_key = api_key_for(
        &api_key_manager,
        &user,
        full_key,
        ApiKeyTier::Professional,
        100_000,
        MONTH_WINDOW_SECS,
    );
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();
    seed_calls(&database, &api_key.id, 1_000, Utc::now()).await;

    auth_middleware
        .authenticate_request(Some(full_key))
        .await
        .unwrap();

    let usage = window_usage(&database, &api_key).await;
    assert_eq!(usage.count, 1_000, "admission writes no row");
    let budget = calculate_api_key_rate_limit(&api_key, &usage, Utc::now());
    assert!(!budget.is_exceeded());
    assert_eq!(budget.remaining_after_this_request(), Some(98_999));
    let RequestBudget::Metered { limit, used, .. } = budget else {
        panic!("a professional key is metered, got {budget:?}");
    };
    assert_eq!((limit, used), (100_000, 1_000));
}

#[tokio::test]
async fn test_enterprise_tier_unlimited() {
    let (database, api_key_manager, auth_middleware, user) = create_test_setup().await;
    database.repositories().users.create(&user).await.unwrap();

    let full_key = "pk_live_enterpriseunlimitedkey1234567890";
    let api_key = api_key_for(
        &api_key_manager,
        &user,
        full_key,
        ApiKeyTier::Enterprise,
        2,
        MONTH_WINDOW_SECS,
    );
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();
    // Past its nominal rate_limit_requests: an Enterprise key ignores it.
    seed_calls(&database, &api_key.id, 10, Utc::now()).await;

    for _ in 0..3 {
        auth_middleware
            .authenticate_request(Some(full_key))
            .await
            .unwrap();
    }

    let usage = window_usage(&database, &api_key).await;
    assert_eq!(
        usage.count, 10,
        "an unlimited key is admitted past its nominal limit, and admission writes no row"
    );
    assert_eq!(
        calculate_api_key_rate_limit(&api_key, &usage, Utc::now()),
        RequestBudget::Unlimited
    );
}

#[test]
fn test_api_key_reset_is_the_oldest_call_plus_the_window() {
    let now = fixed_now();
    let manager = ApiKeyManager::new();
    let user = user_on(UserTier::Starter);
    let api_key = api_key_for(
        &manager,
        &user,
        "pk_live_windowresetkey1234567890123456",
        ApiKeyTier::Starter,
        100,
        3_600,
    );

    let oldest = now - Duration::seconds(600);
    let budget = calculate_api_key_rate_limit(
        &api_key,
        &ApiKeyWindowUsage {
            count: 7,
            oldest: Some(oldest),
        },
        now,
    );
    assert_eq!(
        budget,
        RequestBudget::Metered {
            limit: 100,
            used: 7,
            resets_at: oldest + Duration::seconds(3_600),
        },
        "the first slot frees when the oldest call leaves the one-hour window, not next month"
    );

    let empty = calculate_api_key_rate_limit(
        &api_key,
        &ApiKeyWindowUsage {
            count: 0,
            oldest: None,
        },
        now,
    );
    assert_eq!(
        empty,
        RequestBudget::Metered {
            limit: 100,
            used: 0,
            resets_at: now + Duration::seconds(3_600),
        }
    );
    assert_eq!(
        api_key_window_start(&api_key, now),
        now - Duration::seconds(3_600)
    );
}

#[test]
fn test_enterprise_api_key_is_unlimited() {
    let manager = ApiKeyManager::new();
    let user = user_on(UserTier::Starter);
    let api_key = api_key_for(
        &manager,
        &user,
        "pk_live_enterprisecalckey123456789012345",
        ApiKeyTier::Enterprise,
        1_000_000_000,
        MONTH_WINDOW_SECS,
    );
    let budget = calculate_api_key_rate_limit(
        &api_key,
        &ApiKeyWindowUsage {
            count: 5_000_000,
            oldest: Some(fixed_now()),
        },
        fixed_now(),
    );
    assert_eq!(budget, RequestBudget::Unlimited);
    assert!(!budget.is_exceeded());
    assert_eq!(budget.remaining_after_this_request(), None);

    // The tier itself names no monthly limit and no trial period
    assert_eq!(api_key.tier.monthly_limit(), None);
    assert!(!api_key.tier.is_trial());
    assert_eq!(api_key.tier.as_str(), "enterprise");
    assert_eq!(api_key.tier.default_trial_days(), None);
}

#[test]
fn test_jwt_budget_follows_the_user_tier() {
    let now = fixed_now();
    let starter = user_on(UserTier::Starter);

    assert_eq!(
        tier_budget(&starter, 3, now),
        RequestBudget::Metered {
            limit: 10_000,
            used: 3,
            resets_at: next_utc_month_start(now),
        }
    );
    assert_eq!(
        tier_budget(&starter, 3, now).remaining_after_this_request(),
        Some(9_996)
    );
    assert!(!tier_budget(&starter, 9_999, now).is_exceeded());
    assert!(tier_budget(&starter, 10_000, now).is_exceeded());
    assert_eq!(
        tier_budget(&starter, 10_000, now).remaining_after_this_request(),
        Some(0)
    );

    let professional = user_on(UserTier::Professional);
    let RequestBudget::Metered { limit, .. } = tier_budget(&professional, 0, now) else {
        panic!("a professional user is metered");
    };
    assert_eq!(limit, 100_000);

    let enterprise = user_on(UserTier::Enterprise);
    assert_eq!(
        tier_budget(&enterprise, 5_000_000, now),
        RequestBudget::Unlimited
    );
}

#[test]
fn test_a_user_override_replaces_the_tier_monthly_limit() {
    let now = fixed_now();
    let starter = user_on(UserTier::Starter);
    let enterprise = user_on(UserTier::Enterprise);

    // Without a row the tier decides
    assert_eq!(
        MonthlyLimitOverride::NotSet.resolve(&starter.tier),
        Some(10_000)
    );
    assert_eq!(MonthlyLimitOverride::NotSet.resolve(&enterprise.tier), None);

    // An override below the tier is the limit, and refuses at its own count
    let lowered = MonthlyLimitOverride::Limit(5);
    assert_eq!(lowered.resolve(&starter.tier), Some(5));
    assert_eq!(
        budget_under(&starter, lowered, 4, now),
        RequestBudget::Metered {
            limit: 5,
            used: 4,
            resets_at: next_utc_month_start(now),
        }
    );
    assert!(!budget_under(&starter, lowered, 4, now).is_exceeded());
    assert!(budget_under(&starter, lowered, 5, now).is_exceeded());

    // An override above the tier admits past the tier's ceiling
    let raised = MonthlyLimitOverride::Limit(25_000);
    assert!(tier_budget(&starter, 10_000, now).is_exceeded());
    assert_eq!(
        budget_under(&starter, raised, 10_000, now).remaining_after_this_request(),
        Some(14_999)
    );

    // A NULL monthly limit lifts the tier's ceiling
    assert_eq!(
        budget_under(&starter, MonthlyLimitOverride::Unlimited, 50_000, now),
        RequestBudget::Unlimited
    );

    // An override on an unlimited tier imposes its limit
    let RequestBudget::Metered { limit, .. } =
        budget_under(&enterprise, MonthlyLimitOverride::Limit(1_500), 0, now)
    else {
        panic!("an override caps an enterprise user");
    };
    assert_eq!(limit, 1_500);
}

#[test]
fn test_enforce_request_budget_refuses_with_the_seconds_to_reset() {
    let now = fixed_now();

    let exceeded = RequestBudget::Metered {
        limit: 5,
        used: 5,
        resets_at: now + Duration::seconds(90),
    };
    let refusal = enforce_request_budget(exceeded, now).unwrap_err();
    assert_eq!(refusal.code, ErrorCode::RateLimitExceeded);
    assert_eq!(refusal.retry_after_secs(), Some(90));
    let details = refusal.details.as_deref().unwrap();
    assert_eq!(details["limit_type"], "requests");
    assert_eq!(details["current"], 5);
    assert_eq!(details["limit"], 5);

    // A reset instant already past still names a wait of at least a second.
    let overshot = RequestBudget::Metered {
        limit: 5,
        used: 7,
        resets_at: now - Duration::seconds(10),
    };
    assert_eq!(
        enforce_request_budget(overshot, now)
            .unwrap_err()
            .retry_after_secs(),
        Some(1)
    );

    let within = RequestBudget::Metered {
        limit: 5,
        used: 4,
        resets_at: now + Duration::seconds(90),
    };
    assert!(enforce_request_budget(within, now).is_ok());
    assert!(enforce_request_budget(RequestBudget::Unlimited, now).is_ok());
}

#[tokio::test]
async fn test_window_usage_counts_only_calls_inside_the_window() {
    let (database, api_key_manager, _auth_middleware, user) = create_test_setup().await;
    database.repositories().users.create(&user).await.unwrap();

    let api_key = api_key_for(
        &api_key_manager,
        &user,
        "pk_live_monthlyusagecalckey123456789012",
        ApiKeyTier::Professional,
        100_000,
        MONTH_WINDOW_SECS,
    );
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    // Whole seconds, so the oldest call reads back equal on every engine.
    let now = Utc::now().with_nanosecond(0).unwrap();
    let usage = database.repositories().usage;
    for hours_ago in 0..5 {
        usage
            .record_api_key(&call(
                &api_key.id,
                now - Duration::hours(hours_ago),
                "inside_window",
                200,
            ))
            .await
            .unwrap();
    }
    // Forty days ago: outside the 30-day window.
    for hours_ago in 0..3 {
        usage
            .record_api_key(&call(
                &api_key.id,
                now - Duration::days(40) - Duration::hours(hours_ago),
                "outside_window",
                200,
            ))
            .await
            .unwrap();
    }

    let counted = usage
        .get_api_key_window_usage(&api_key.id, api_key_window_start(&api_key, now))
        .await
        .unwrap();
    assert_eq!(
        counted,
        ApiKeyWindowUsage {
            count: 5,
            oldest: Some(now - Duration::hours(4)),
        }
    );
}

#[tokio::test]
async fn test_rate_limit_edge_cases() {
    let (database, api_key_manager, auth_middleware, user) = create_test_setup().await;
    database.repositories().users.create(&user).await.unwrap();

    let full_key = "pk_live_edgecaselimitkey1234567890123456";
    let api_key = api_key_for(
        &api_key_manager,
        &user,
        full_key,
        ApiKeyTier::Starter,
        10,
        MONTH_WINDOW_SECS,
    );
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();
    let seeded_at = Utc::now() - Duration::days(29);
    seed_calls(&database, &api_key.id, 10, seeded_at).await;

    // At exactly the limit the budget is spent: nothing remains, and the
    // next request is refused.
    let usage = window_usage(&database, &api_key).await;
    assert_eq!(usage.count, 10);
    let budget = calculate_api_key_rate_limit(&api_key, &usage, Utc::now());
    assert!(budget.is_exceeded());
    assert_eq!(budget.remaining_after_this_request(), Some(0));

    let refusal = auth_middleware
        .authenticate_request(Some(full_key))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, ErrorCode::RateLimitExceeded);
    // The calls were seeded 29 days into a 30-day window: they leave it in a day.
    let retry_after = refusal.retry_after_secs().unwrap();
    assert!(
        (86_280..=86_400).contains(&retry_after),
        "retry window {retry_after}s should be about one day"
    );
}

#[tokio::test]
async fn test_rate_limit_with_mixed_status_codes() {
    let (database, api_key_manager, _auth_middleware, user) = create_test_setup().await;
    database.repositories().users.create(&user).await.unwrap();

    let api_key = api_key_for(
        &api_key_manager,
        &user,
        "pk_live_mixedstatuskey123456789012345678",
        ApiKeyTier::Professional,
        100_000,
        MONTH_WINDOW_SECS,
    );
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    let status_codes = [200, 201, 400, 401, 403, 404, 500, 502];
    for (i, &status_code) in status_codes.iter().enumerate() {
        database
            .repositories()
            .usage
            .record_api_key(&call(
                &api_key.id,
                Utc::now(),
                &format!("mixed_tool_{i}"),
                status_code,
            ))
            .await
            .unwrap();
    }

    // All requests count toward the rate limit, regardless of status code
    let usage = window_usage(&database, &api_key).await;
    assert_eq!(usage.count, 8);
    let budget = calculate_api_key_rate_limit(&api_key, &usage, Utc::now());
    assert!(!budget.is_exceeded());
    assert_eq!(budget.remaining_after_this_request(), Some(99_991));

    // Statistics still split them by status
    let stats = database
        .repositories()
        .usage
        .get_api_key_stats(
            &api_key.id,
            Utc::now() - Duration::hours(1),
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    assert_eq!(stats.total_requests, 8);
    assert_eq!(stats.successful_requests, 2); // 200, 201
    assert_eq!(stats.failed_requests, 6); // 400, 401, 403, 404, 500, 502
}

#[tokio::test]
async fn test_trial_tier_rate_limiting() {
    let (database, api_key_manager, auth_middleware, user) = create_test_setup().await;
    database.repositories().users.create(&user).await.unwrap();

    // A pk_trial_ key authenticates on the same path as pk_live_
    let full_key = "pk_trial_trialtierlimitkey123456789012345";
    let mut api_key = api_key_for(
        &api_key_manager,
        &user,
        full_key,
        ApiKeyTier::Trial,
        1_000,
        MONTH_WINDOW_SECS,
    );
    api_key.expires_at = Some(Utc::now() + Duration::days(14));
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();

    auth_middleware
        .authenticate_request(Some(full_key))
        .await
        .unwrap();
    seed_calls(&database, &api_key.id, 1_000, Utc::now()).await;

    let usage = window_usage(&database, &api_key).await;
    assert_eq!(usage.count, 1_000);
    assert!(calculate_api_key_rate_limit(&api_key, &usage, Utc::now()).is_exceeded());

    let refusal = auth_middleware
        .authenticate_request(Some(full_key))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, ErrorCode::RateLimitExceeded);
}

#[tokio::test]
async fn test_tier_conversion_scenarios() {
    let (database, api_key_manager, _auth_middleware, user) = create_test_setup().await;
    database.repositories().users.create(&user).await.unwrap();

    let now = fixed_now();
    let half_used = |count| ApiKeyWindowUsage {
        count,
        oldest: Some(now - Duration::days(1)),
    };

    let mut trial_key = api_key_for(
        &api_key_manager,
        &user,
        "pk_trial_conversionkey123456789012345",
        ApiKeyTier::Trial,
        1_000,
        MONTH_WINDOW_SECS,
    );
    trial_key.expires_at = Some(Utc::now() + Duration::days(14));
    let starter_key = api_key_for(
        &api_key_manager,
        &user,
        "pk_live_upgradedstarterkey123456789012",
        ApiKeyTier::Starter,
        10_000,
        MONTH_WINDOW_SECS,
    );
    let professional_key = api_key_for(
        &api_key_manager,
        &user,
        "pk_live_professionalkey123456789012345",
        ApiKeyTier::Professional,
        100_000,
        MONTH_WINDOW_SECS,
    );
    let enterprise_key = api_key_for(
        &api_key_manager,
        &user,
        "pk_live_enterprisekey123456789012345678",
        ApiKeyTier::Enterprise,
        1_000_000_000,
        MONTH_WINDOW_SECS,
    );
    for key in [&trial_key, &starter_key, &professional_key, &enterprise_key] {
        database.repositories().api_keys.create(key).await.unwrap();
    }

    assert!(trial_key.expires_at.is_some());
    assert!(starter_key.expires_at.is_none());

    // Each tier's budget at half its limit: remaining counts this request.
    let resets_at = now - Duration::days(1) + Duration::seconds(i64::from(MONTH_WINDOW_SECS));
    for (key, used, limit) in [
        (&trial_key, 500, 1_000),
        (&starter_key, 5_000, 10_000),
        (&professional_key, 50_000, 100_000),
    ] {
        let budget = calculate_api_key_rate_limit(key, &half_used(used), now);
        assert_eq!(
            budget,
            RequestBudget::Metered {
                limit,
                used,
                resets_at,
            },
            "{:?}",
            key.tier
        );
        assert_eq!(
            budget.remaining_after_this_request(),
            Some(limit - used - 1)
        );
    }
    assert_eq!(
        calculate_api_key_rate_limit(&enterprise_key, &half_used(1_000_000), now),
        RequestBudget::Unlimited
    );
}

#[tokio::test]
async fn test_legacy_conversion_functionality() {
    let (database, api_key_manager, _auth_middleware, user) = create_test_setup().await;

    // Store the user in the database first
    database.repositories().users.create(&user).await.unwrap();

    // Test legacy API key creation using the old CreateApiKeyRequest format
    let legacy_request = CreateApiKeyRequest {
        name: "Legacy API Key".to_owned(),
        description: Some("Created using legacy format".to_owned()),
        tier: ApiKeyTier::Professional,
        rate_limit_requests: Some(50_000), // Custom limit
        expires_in_days: Some(365),        // Custom expiration
    };

    let (legacy_key, legacy_full_key) = api_key_manager
        .create_api_key(user.id, legacy_request)
        .unwrap();

    // Verify legacy key properties
    assert_eq!(legacy_key.tier, ApiKeyTier::Professional);
    assert_eq!(legacy_key.rate_limit_requests, 50_000);
    assert!(legacy_key.expires_at.is_some());
    assert_eq!(
        legacy_key.description,
        Some("Created using legacy format".to_owned())
    );

    // Test new simplified API key creation
    let simple_request = CreateApiKeyRequestSimple {
        name: "Simple API Key".to_owned(),
        description: Some("Created using simplified format".to_owned()),
        rate_limit_requests: 25_000, // Maps to Professional tier
        expires_in_days: None,
    };

    let (simple_key, simple_full_key) = api_key_manager
        .create_api_key_simple(user.id, simple_request)
        .unwrap();

    // Verify simple key properties (tier is automatically determined)
    assert_eq!(simple_key.tier, ApiKeyTier::Professional);
    assert_eq!(simple_key.rate_limit_requests, 25_000);
    assert!(simple_key.expires_at.is_none());

    // Test trial key creation using legacy method
    let trial_key_result = api_key_manager.create_trial_key(
        user.id,
        "Legacy Trial Key".to_owned(),
        Some("Auto-generated trial key".to_owned()),
    );

    let (trial_key, trial_full_key) = trial_key_result.unwrap();

    // Verify trial key properties
    assert_eq!(trial_key.tier, ApiKeyTier::Trial);
    assert_eq!(trial_key.rate_limit_requests, 1_000);
    assert!(trial_key.expires_at.is_some());
    assert!(trial_full_key.starts_with("pk_trial_"));

    // Test key format validation
    assert!(api_key_manager
        .validate_key_format(&legacy_full_key)
        .is_ok());
    assert!(api_key_manager
        .validate_key_format(&simple_full_key)
        .is_ok());
    assert!(api_key_manager.validate_key_format(&trial_full_key).is_ok());

    // Test key type detection
    assert!(!api_key_manager.is_trial_key(&legacy_full_key));
    assert!(!api_key_manager.is_trial_key(&simple_full_key));
    assert!(api_key_manager.is_trial_key(&trial_full_key));

    // Store all keys in database
    for key in [&legacy_key, &simple_key, &trial_key] {
        database.repositories().api_keys.create(key).await.unwrap();
    }

    // Test that all keys are valid
    assert!(api_key_manager.is_key_valid(&legacy_key).is_ok());
    assert!(api_key_manager.is_key_valid(&simple_key).is_ok());
    assert!(api_key_manager.is_key_valid(&trial_key).is_ok());
}

#[test]
fn test_monthly_reset_calculations() {
    let at = |y, m, d, h, min, s| Utc.with_ymd_and_hms(y, m, d, h, min, s).unwrap();

    // A 29th, 30th or 31st never names a day the next month lacks, the
    // year rolls over in December, and the first instant of a month resets
    // at the next one.
    for (now, expected) in [
        (at(2026, 1, 29, 10, 0, 0), at(2026, 2, 1, 0, 0, 0)),
        (at(2026, 1, 31, 23, 59, 59), at(2026, 2, 1, 0, 0, 0)),
        (at(2026, 3, 31, 12, 0, 0), at(2026, 4, 1, 0, 0, 0)),
        (at(2026, 8, 31, 8, 30, 0), at(2026, 9, 1, 0, 0, 0)),
        (at(2026, 10, 31, 23, 0, 0), at(2026, 11, 1, 0, 0, 0)),
        (at(2026, 12, 15, 12, 0, 0), at(2027, 1, 1, 0, 0, 0)),
        (at(2028, 1, 30, 6, 0, 0), at(2028, 2, 1, 0, 0, 0)),
        (at(2026, 9, 1, 0, 0, 0), at(2026, 10, 1, 0, 0, 0)),
    ] {
        let reset = next_utc_month_start(now + Duration::nanoseconds(123_456_789));
        assert_eq!(reset, expected, "next month start after {now}");
        assert_eq!(reset.nanosecond(), 0, "the reset is a whole second");
    }
}
