// ABOUTME: Pins the shared provider rate limiter: Strava's 15-minute and daily windows counted in the database, and the walk's share
// ABOUTME: Two limiters over one database never take more than the budget together; a spent window reopens when it ends

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
use pierre_database::repositories::UsageCounterRepository;
use pierre_services::provider_rate_limiter::{
    budget_counter_key, budget_period, ProviderRateLimiter, RateLimitStatus, FIFTEEN_MINUTES,
    ONE_DAY, PLATFORM_SCOPE,
};
use pierre_test_support::db::create_test_db;
use tokio::time::sleep;

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
                limiter.acquire_background("strava").await
            } else {
                limiter.acquire("strava").await
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
    let key = budget_counter_key("strava");
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
        limiter.acquire("strava").await.unwrap(),
        RateLimitStatus::Allowed
    );
    let RateLimitStatus::Exceeded { retry_after } = limiter.acquire("strava").await.unwrap() else {
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
    let key = budget_counter_key("strava");
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
        limiter.acquire("strava").await.unwrap(),
        RateLimitStatus::Allowed
    );
    assert!(matches!(
        limiter.acquire("strava").await.unwrap(),
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
            limiter.acquire_background("strava").await.unwrap(),
            RateLimitStatus::Allowed
        );
    }
    assert!(matches!(
        limiter.acquire_background("strava").await.unwrap(),
        RateLimitStatus::Exceeded { .. }
    ));
    for _ in 25..100 {
        assert_eq!(
            limiter.acquire("strava").await.unwrap(),
            RateLimitStatus::Allowed
        );
    }
    assert!(matches!(
        limiter.acquire("strava").await.unwrap(),
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
        limiter.acquire("strava").await.unwrap(),
        RateLimitStatus::Allowed
    );
    assert_eq!(
        limiter.acquire("strava").await.unwrap(),
        RateLimitStatus::Allowed
    );
    let RateLimitStatus::Exceeded { retry_after } = limiter.acquire("strava").await.unwrap() else {
        panic!("the window is spent after 2 requests");
    };
    assert!(retry_after <= window);
    sleep(retry_after + Duration::from_millis(20)).await;
    assert_eq!(
        limiter.acquire("strava").await.unwrap(),
        RateLimitStatus::Allowed
    );
}
