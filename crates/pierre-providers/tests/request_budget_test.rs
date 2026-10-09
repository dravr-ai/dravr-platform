// ABOUTME: Pins the provider request budgets: Strava's windows counted in the database per signing OAuth app, and the walk's share
// ABOUTME: Each app, each intervals.icu athlete grant and each API key has its own windows; a tenant app's daily limit applies

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
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::{
    api_key_counter_key, budget_counter_key, budget_period, grant_counter_key, in_background,
    ProviderRateLimiter, RateLimitStatus, RequestBudget, FIFTEEN_MINUTES, ONE_DAY, PLATFORM_SCOPE,
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
            .acquire("coros", "tenant-app", Some(1))
            .await
            .unwrap(),
        RateLimitStatus::Allowed
    );
    assert!(matches!(
        limiter
            .acquire("coros", "tenant-app", Some(1))
            .await
            .unwrap(),
        RateLimitStatus::Exceeded { .. }
    ));
    assert_eq!(
        limiter.acquire("coros", APP, None).await.unwrap(),
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

/// The provider whose limits the tests below pin: Intervals.icu counts each
/// athlete's grant to an OAuth app, and each personal API key, on its own.
const INTERVALS: &str = "intervals_icu";

/// The Intervals.icu athlete whose grant most tests below spend.
const ATHLETE: &str = "i100";

/// Count `value` requests into the `window` bucket under `key` now.
async fn spend(
    counters: &Arc<dyn UsageCounterRepository>,
    key: &str,
    window: Duration,
    value: i64,
) {
    counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            key,
            &budget_period(Utc::now(), window),
            value,
        )
        .await
        .unwrap();
}

/// The requests counted in the `window` bucket under `key` now.
async fn spent(counters: &Arc<dyn UsageCounterRepository>, key: &str, window: Duration) -> i64 {
    counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            key,
            &budget_period(Utc::now(), window),
        )
        .await
        .unwrap()
        .value
}

/// The budget of `athlete`'s grant to the server's app.
fn grant(limiter: &Arc<ProviderRateLimiter>, athlete: &str) -> RequestBudget {
    RequestBudget::for_grant(
        Arc::clone(limiter),
        APP.to_owned(),
        None,
        athlete.to_owned(),
    )
}

/// Intervals.icu gives an OAuth app 100 requests a day for each athlete who
/// granted it (forum.intervals.icu/t/609): with 99 counted today the 100th is
/// taken, the 101st refused with the wait until midnight UTC, and the refused
/// request gives back the slot it took from the app's own window.
#[tokio::test]
async fn an_intervals_grant_refuses_its_hundred_and_first_request_of_the_day() {
    let counters = counters().await;
    let limiter = Arc::new(ProviderRateLimiter::new(Arc::clone(&counters)));
    let grant_key = grant_counter_key(INTERVALS, APP, ATHLETE);
    spend(&counters, &grant_key, ONE_DAY, 99).await;
    let budget = grant(&limiter, ATHLETE);

    budget.admit(INTERVALS).await.unwrap();
    let refused = budget.admit(INTERVALS).await.unwrap_err();

    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    let wait = refused
        .retry_after_secs()
        .expect("the refusal names its wait");
    assert!(wait > 0 && wait <= ONE_DAY.as_secs(), "{wait}");
    assert_eq!(spent(&counters, &grant_key, ONE_DAY).await, 100);
    assert_eq!(
        spent(&counters, &budget_counter_key(INTERVALS, APP), ONE_DAY).await,
        1,
        "only the request taken counts against the app"
    );
}

/// Each athlete's grant to one app has its own day: one athlete spending
/// theirs leaves another's untouched.
#[tokio::test]
async fn two_athletes_of_one_app_spend_their_own_grants() {
    let counters = counters().await;
    let limiter = Arc::new(ProviderRateLimiter::new(Arc::clone(&counters)));
    spend(
        &counters,
        &grant_counter_key(INTERVALS, APP, ATHLETE),
        ONE_DAY,
        100,
    )
    .await;

    let refused = grant(&limiter, ATHLETE).admit(INTERVALS).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    grant(&limiter, "i200")
        .admit(INTERVALS)
        .await
        .expect("another athlete's grant is untouched");
}

/// A backfill runs in the background share, a quarter of the athlete's 100:
/// its 26th request is refused, and the 75 left serve the reads the athlete
/// is waiting on, up to the day's 100th.
#[tokio::test]
async fn the_background_share_stops_a_backfill_before_the_athletes_reads_starve() {
    let counters = counters().await;
    let limiter = Arc::new(ProviderRateLimiter::new(Arc::clone(&counters)));
    let budget = grant(&limiter, ATHLETE);

    for _ in 0..25 {
        in_background(budget.admit(INTERVALS)).await.unwrap();
    }
    let stopped = in_background(budget.admit(INTERVALS)).await.unwrap_err();
    assert_eq!(stopped.code, ErrorCode::ExternalRateLimited);

    for read in 25..100 {
        budget
            .admit(INTERVALS)
            .await
            .unwrap_or_else(|e| panic!("read {read} is refused: {e}"));
    }
    let refused = budget.admit(INTERVALS).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
}

/// An Intervals.icu OAuth app stops at 50,000 requests a day
/// (forum.intervals.icu/t/609), whichever of its athletes made them, while
/// each of their grants still has room.
#[tokio::test]
async fn an_intervals_app_stops_at_its_daily_ceiling_across_its_athletes() {
    let counters = counters().await;
    let limiter = Arc::new(ProviderRateLimiter::new(Arc::clone(&counters)));
    spend(
        &counters,
        &budget_counter_key(INTERVALS, APP),
        ONE_DAY,
        49_999,
    )
    .await;

    grant(&limiter, "i300")
        .admit(INTERVALS)
        .await
        .expect("the app's 50,000th request is taken");
    let refused = grant(&limiter, "i301").admit(INTERVALS).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    assert_eq!(
        spent(
            &counters,
            &grant_counter_key(INTERVALS, APP, "i301"),
            ONE_DAY
        )
        .await,
        0,
        "a request the app refused takes nothing from the athlete's grant"
    );
}

/// A personal Intervals.icu API key has 2,500 requests per 15 minutes
/// (forum.intervals.icu/t/609): with 2,499 counted in the window one more is
/// taken and the next refused until the window ends, taking nothing from the
/// key's day.
#[tokio::test]
async fn an_api_key_has_a_fifteen_minute_window_of_its_own() {
    let counters = counters().await;
    let limiter = Arc::new(ProviderRateLimiter::new(Arc::clone(&counters)));
    let key = api_key_counter_key(INTERVALS, ATHLETE);
    spend(&counters, &key, FIFTEEN_MINUTES, 2_499).await;
    let budget = RequestBudget::for_api_key(Arc::clone(&limiter), ATHLETE.to_owned());

    budget.admit(INTERVALS).await.unwrap();
    let refused = budget.admit(INTERVALS).await.unwrap_err();

    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    let wait = refused
        .retry_after_secs()
        .expect("the refusal names its wait");
    assert!(wait <= FIFTEEN_MINUTES.as_secs(), "{wait}");
    assert_eq!(
        spent(&counters, &key, ONE_DAY).await,
        1,
        "only the request taken counts for the day"
    );
}

/// A personal Intervals.icu API key has 5,000 requests a day
/// (forum.intervals.icu/t/609): with 4,999 counted today one more is taken
/// and the next refused, giving back its 15-minute slot.
#[tokio::test]
async fn an_api_key_has_a_daily_window_of_its_own() {
    let counters = counters().await;
    let limiter = Arc::new(ProviderRateLimiter::new(Arc::clone(&counters)));
    let key = api_key_counter_key(INTERVALS, ATHLETE);
    spend(&counters, &key, ONE_DAY, 4_999).await;
    let budget = RequestBudget::for_api_key(Arc::clone(&limiter), ATHLETE.to_owned());

    budget.admit(INTERVALS).await.unwrap();
    let refused = budget.admit(INTERVALS).await.unwrap_err();

    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    assert_eq!(
        spent(&counters, &key, FIFTEEN_MINUTES).await,
        1,
        "the refused request gave its 15-minute slot back"
    );
}

/// A personal API key is held to its own windows only: its 101st request of
/// the day is taken, where an OAuth grant would refuse it, and no app's
/// window is counted.
#[tokio::test]
async fn an_api_key_is_held_to_no_grant_or_app_window() {
    let counters = counters().await;
    let limiter = Arc::new(ProviderRateLimiter::new(Arc::clone(&counters)));
    let budget = RequestBudget::for_api_key(Arc::clone(&limiter), ATHLETE.to_owned());

    for request in 0..101 {
        budget
            .admit(INTERVALS)
            .await
            .unwrap_or_else(|e| panic!("request {request} is refused: {e}"));
    }
    assert_eq!(
        spent(&counters, &api_key_counter_key(INTERVALS, ATHLETE), ONE_DAY).await,
        101
    );
    assert_eq!(
        spent(
            &counters,
            &grant_counter_key(INTERVALS, APP, ATHLETE),
            ONE_DAY
        )
        .await,
        0
    );
    assert_eq!(
        spent(&counters, &budget_counter_key(INTERVALS, APP), ONE_DAY).await,
        0
    );
}

/// A key whose account is unknown is counted nowhere rather than under an
/// empty account, where every such key would share one set of windows; a key
/// whose account is known is counted in its own.
#[tokio::test]
async fn a_key_with_no_account_shares_no_windows() {
    let counters = counters().await;
    let registry = ProviderRegistry::new()
        .with_request_limiter(Arc::new(ProviderRateLimiter::new(Arc::clone(&counters))));

    assert!(registry.api_key_budget("").is_none());
    registry
        .api_key_budget(ATHLETE)
        .expect("a key with its account carries its budget")
        .admit(INTERVALS)
        .await
        .unwrap();
    assert_eq!(
        spent(&counters, &api_key_counter_key(INTERVALS, ATHLETE), ONE_DAY).await,
        1
    );
}
