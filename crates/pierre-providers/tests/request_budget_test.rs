// ABOUTME: Pins the provider request budgets: Strava's windows counted in the database per signing OAuth app, and the walk's share
// ABOUTME: Two limiters never take more than the budget together; each app has its own windows; a tenant app's daily limit applies

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider rate limiter suite (carnet#582).
//!
//! Every limiter here counts in a real `usage_counters` table. Two limiters
//! over one database are two instances of the backend: they share nothing
//! but the database, as Cloud Run instances do.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_core::errors::ErrorCode;
use pierre_database::repositories::UsageCounterRepository;
use pierre_providers::request_budget::{
    budget_counter_key, budget_period, in_background, ProviderRateLimiter, RateLimitStatus,
    RequestBudget, FIFTEEN_MINUTES, ONE_DAY, PLATFORM_SCOPE,
};
use pierre_test_support::db::create_test_db;
use tokio::time::sleep;

/// The OAuth app every request here is signed by.
const APP: &str = "server-app";

async fn counters() -> Arc<dyn UsageCounterRepository> {
    let db = create_test_db().await.expect("test db");
    Arc::clone(&db.repositories().usage_counters)
}

/// How many of `attempts` requests two limiters sharing `counters` take
/// together, each asking `attempts / 2` times at once, at the background
/// share when `background`.
async fn taken_by_two_instances(
    counters: &Arc<dyn UsageCounterRepository>,
    budget: u32,
    attempts: usize,
    background: bool,
) -> usize {
    let instances = [
        Arc::new(ProviderRateLimiter::new(Arc::clone(counters))),
        Arc::new(ProviderRateLimiter::new(Arc::clone(counters))),
    ];
    for instance in &instances {
        instance.set_budgets("strava", &[(budget, ONE_DAY)]);
    }
    let mut tasks = Vec::new();
    for attempt in 0..attempts {
        let limiter = Arc::clone(&instances[attempt % 2]);
        tasks.push(tokio::spawn(async move {
            let status = if background {
                in_background(limiter.acquire("strava", APP, None)).await
            } else {
                limiter.acquire("strava", APP, None).await
            };
            status.unwrap() == RateLimitStatus::Allowed
        }));
    }
    let mut taken = 0;
    for task in tasks {
        if task.await.unwrap() {
            taken += 1;
        }
    }
    taken
}

/// Two instances asking at once for more than the budget take exactly the
/// budget between them, for live requests and for the walk's share alike.
#[tokio::test]
async fn two_instances_sharing_one_database_never_exceed_the_budget_together() {
    let live = counters().await;
    assert_eq!(taken_by_two_instances(&live, 10, 30, false).await, 10);

    let background = counters().await;
    assert_eq!(
        taken_by_two_instances(&background, 12, 30, true).await,
        3,
        "a quarter of 12"
    );
}

/// Strava's 15-minute budget is the standard read limit, 100: with 99
/// counted in the current window one more request is taken, the next is
/// refused until the window ends, and a refused request takes nothing from
/// the day.
#[tokio::test]
async fn strava_has_a_fifteen_minute_budget_of_one_hundred_by_default() {
    let counters = counters().await;
    let limiter = ProviderRateLimiter::new(Arc::clone(&counters));
    let key = budget_counter_key("strava", APP);
    let now = Utc::now();
    counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &key,
            &budget_period(now, FIFTEEN_MINUTES),
            99,
        )
        .await
        .unwrap();

    assert_eq!(
        limiter.acquire("strava", APP, None).await.unwrap(),
        RateLimitStatus::Allowed
    );
    let RateLimitStatus::Exceeded { retry_after } =
        limiter.acquire("strava", APP, None).await.unwrap()
    else {
        panic!("the window is spent after 100 requests");
    };
    assert!(retry_after <= FIFTEEN_MINUTES);
    let day = counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &key,
            &budget_period(now, ONE_DAY),
        )
        .await
        .unwrap();
    assert_eq!(day.value, 1, "only the request taken counts for the day");
}

/// Strava's daily budget is the standard read limit, 1,000: with 999 counted
/// today one more request is taken and the next refused, and the refused
/// request's 15-minute slot is given back.
#[tokio::test]
async fn strava_has_a_daily_budget_of_one_thousand_by_default() {
    let counters = counters().await;
    let limiter = ProviderRateLimiter::new(Arc::clone(&counters));
    let key = budget_counter_key("strava", APP);
    let now = Utc::now();
    counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &key,
            &budget_period(now, ONE_DAY),
            999,
        )
        .await
        .unwrap();

    assert_eq!(
        limiter.acquire("strava", APP, None).await.unwrap(),
        RateLimitStatus::Allowed
    );
    assert!(matches!(
        limiter.acquire("strava", APP, None).await.unwrap(),
        RateLimitStatus::Exceeded { .. }
    ));
    let short = counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &key,
            &budget_period(now, FIFTEEN_MINUTES),
        )
        .await
        .unwrap();
    assert_eq!(
        short.value, 1,
        "the refused request gave its 15-minute slot back"
    );
}

/// The walk takes a quarter of Strava's window, 25 of its 100 requests, and
/// the other 75 stay open to live requests.
#[tokio::test]
async fn the_walk_takes_a_quarter_of_each_window_and_live_requests_keep_the_rest() {
    let limiter = ProviderRateLimiter::new(counters().await);
    limiter.set_budgets("strava", &[(100, ONE_DAY)]);
    for _ in 0..25 {
        assert_eq!(
            in_background(limiter.acquire("strava", APP, None))
                .await
                .unwrap(),
            RateLimitStatus::Allowed
        );
    }
    assert!(matches!(
        in_background(limiter.acquire("strava", APP, None))
            .await
            .unwrap(),
        RateLimitStatus::Exceeded { .. }
    ));
    for _ in 25..100 {
        assert_eq!(
            limiter.acquire("strava", APP, None).await.unwrap(),
            RateLimitStatus::Allowed
        );
    }
    assert!(matches!(
        limiter.acquire("strava", APP, None).await.unwrap(),
        RateLimitStatus::Exceeded { .. }
    ));
}

/// A spent window refuses until it ends, and the next one takes requests
/// again.
#[tokio::test]
async fn a_spent_window_refuses_until_it_ends() {
    let window = Duration::from_secs(1);
    let limiter = ProviderRateLimiter::new(counters().await);
    limiter.set_budgets("strava", &[(2, window)]);
    // Start at the top of a window, so the three requests share one.
    let into_window = u64::try_from(Utc::now().timestamp_millis().rem_euclid(1_000)).unwrap();
    sleep(Duration::from_millis(1_000 - into_window + 20)).await;

    assert_eq!(
        limiter.acquire("strava", APP, None).await.unwrap(),
        RateLimitStatus::Allowed
    );
    assert_eq!(
        limiter.acquire("strava", APP, None).await.unwrap(),
        RateLimitStatus::Allowed
    );
    let RateLimitStatus::Exceeded { retry_after } =
        limiter.acquire("strava", APP, None).await.unwrap()
    else {
        panic!("the window is spent after 2 requests");
    };
    assert!(retry_after <= window);
    sleep(retry_after + Duration::from_millis(20)).await;
    assert_eq!(
        limiter.acquire("strava", APP, None).await.unwrap(),
        RateLimitStatus::Allowed
    );
}

/// A provider counts each OAuth app on its own, so one app spending its
/// window leaves another's untouched: a Strava pool app, the server's app and
/// a tenant's app each have their own windows.
#[tokio::test]
async fn each_signing_app_spends_its_own_windows() {
    let limiter = ProviderRateLimiter::new(counters().await);
    limiter.set_budgets("strava", &[(2, ONE_DAY)]);

    for _ in 0..2 {
        assert_eq!(
            limiter.acquire("strava", "pool-app", None).await.unwrap(),
            RateLimitStatus::Allowed
        );
    }
    assert!(matches!(
        limiter.acquire("strava", "pool-app", None).await.unwrap(),
        RateLimitStatus::Exceeded { .. }
    ));
    assert_eq!(
        limiter.acquire("strava", APP, None).await.unwrap(),
        RateLimitStatus::Allowed,
        "another app's window is untouched"
    );
}

/// A tenant's own app carries the daily budget its operator registered, in
/// place of the provider's, and keeps the provider's other windows.
#[tokio::test]
async fn a_tenant_apps_own_daily_limit_replaces_the_providers() {
    let counters = counters().await;
    let limiter = ProviderRateLimiter::new(Arc::clone(&counters));
    limiter.set_budgets("strava", &[(100, FIFTEEN_MINUTES), (1_000, ONE_DAY)]);

    for _ in 0..3 {
        assert_eq!(
            limiter
                .acquire("strava", "tenant-app", Some(3))
                .await
                .unwrap(),
            RateLimitStatus::Allowed
        );
    }
    assert!(
        matches!(
            limiter
                .acquire("strava", "tenant-app", Some(3))
                .await
                .unwrap(),
            RateLimitStatus::Exceeded { .. }
        ),
        "the registered daily limit is the app's whole day"
    );
    // The refusal gave back the fifteen-minute slot it took first.
    let key = budget_counter_key("strava", "tenant-app");
    let period = budget_period(Utc::now(), FIFTEEN_MINUTES);
    assert_eq!(
        counters
            .get_counter(PLATFORM_SCOPE, PLATFORM_SCOPE, &key, &period)
            .await
            .unwrap()
            .value,
        3
    );
}

/// A provider no budget is configured for gets one from an app that
/// registered its own daily limit.
#[tokio::test]
async fn an_apps_daily_limit_applies_where_the_provider_has_none() {
    let limiter = ProviderRateLimiter::new(counters().await);

    assert_eq!(
        limiter
            .acquire("intervals_icu", "tenant-app", Some(1))
            .await
            .unwrap(),
        RateLimitStatus::Allowed
    );
    assert!(matches!(
        limiter
            .acquire("intervals_icu", "tenant-app", Some(1))
            .await
            .unwrap(),
        RateLimitStatus::Exceeded { .. }
    ));
    assert_eq!(
        limiter.acquire("intervals_icu", APP, None).await.unwrap(),
        RateLimitStatus::Allowed,
        "no budget configured and none registered: nothing to refuse"
    );
}

/// A credential's budget refuses with the typed rate-limit error, naming the
/// wait until the spent window resets.
#[tokio::test]
async fn a_spent_budget_refuses_with_the_rate_limit_error_and_its_wait() {
    let limiter = Arc::new(ProviderRateLimiter::new(counters().await));
    limiter.set_budgets("strava", &[(1, ONE_DAY)]);
    let budget = RequestBudget::new(Arc::clone(&limiter), APP.to_owned(), None);

    budget.admit("strava").await.unwrap();
    let refused = budget.admit("strava").await.unwrap_err();

    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    let wait = refused
        .retry_after_secs()
        .expect("the refusal names its wait");
    assert!(wait > 0 && wait <= ONE_DAY.as_secs(), "{wait}");
}
