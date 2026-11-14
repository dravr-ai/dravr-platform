// ABOUTME: Pins the provider request budgets: Strava's windows counted in the database per signing OAuth app, and the walks' shares
// ABOUTME: Each app and API key has its own windows; a pool is sized by its grants; a detail walk has an app and an athlete day

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider rate limiter suite (carnet#582).
//!
//! Every limiter here counts in a real `usage_counters` table. Two limiters
//! over one database are two instances of the backend: they share nothing
//! but the database, as Cloud Run instances do.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use secrecy::SecretString;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_core::errors::ErrorCode;
use pierre_core::models::{ConnectionType, TenantId, User, UserOAuthToken, API_KEY_TOKEN_TYPE};
use pierre_database::RepositoryRegistry;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::{
    api_key_counter_key, budget_counter_key, budget_period, detail_walk_athlete_counter_key,
    detail_walk_counter_key, in_background, in_detail_walk, is_background, refused_before_send,
    ProviderRateLimiter, RateLimitStatus, RequestBudget, DETAIL_WALK_DAILY_CALLS, FIFTEEN_MINUTES,
    ONE_DAY, PLATFORM_SCOPE,
};
use pierre_test_support::db::create_test_db;
use tokio::time::sleep;
use uuid::Uuid;

/// The OAuth app every request here is signed by.
const APP: &str = "server-app";

/// A fresh database's repositories.
async fn database() -> Arc<RepositoryRegistry> {
    let db = create_test_db().await.expect("test db");
    Arc::clone(db.repositories())
}

/// A limiter over `repos`: one instance of the backend, counting its windows
/// in `usage_counters` and the grants that size a pool through
/// `provider_connections`.
fn limiter_over(repos: &RepositoryRegistry) -> ProviderRateLimiter {
    ProviderRateLimiter::new(
        Arc::clone(&repos.usage_counters),
        Arc::clone(&repos.provider_connections),
    )
}

/// A limiter over a fresh database.
async fn fresh_limiter() -> ProviderRateLimiter {
    let repos = database().await;
    limiter_over(&repos)
}

/// How many of `attempts` requests two limiters sharing `repos` take
/// together, each asking `attempts / 2` times at once, at the background
/// share when `background`.
async fn taken_by_two_instances(
    repos: &RepositoryRegistry,
    budget: u32,
    attempts: usize,
    background: bool,
) -> usize {
    let instances = [Arc::new(limiter_over(repos)), Arc::new(limiter_over(repos))];
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
    let live = database().await;
    assert_eq!(taken_by_two_instances(&live, 10, 30, false).await, 10);

    let background = database().await;
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
    let repos = database().await;
    let counters = Arc::clone(&repos.usage_counters);
    let limiter = limiter_over(&repos);
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
    let repos = database().await;
    let counters = Arc::clone(&repos.usage_counters);
    let limiter = limiter_over(&repos);
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
    let limiter = fresh_limiter().await;
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
    let limiter = fresh_limiter().await;
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
    let limiter = fresh_limiter().await;
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
    let repos = database().await;
    let counters = Arc::clone(&repos.usage_counters);
    let limiter = limiter_over(&repos);
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
    let limiter = fresh_limiter().await;

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
    let limiter = Arc::new(fresh_limiter().await);
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

/// The provider whose limits the tests below pin: Intervals.icu sizes an
/// OAuth app's windows by the athletes who granted it, and limits each
/// personal API key on its own.
const INTERVALS: &str = "intervals_icu";

/// The Intervals.icu athlete whose API key the key tests spend.
const ATHLETE: &str = "i100";

/// Count `value` requests into the `window` bucket under `key` now.
async fn spend(repos: &RepositoryRegistry, key: &str, window: Duration, value: i64) {
    repos
        .usage_counters
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
async fn spent(repos: &RepositoryRegistry, key: &str, window: Duration) -> i64 {
    repos
        .usage_counters
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

/// Link `athletes` new users to Intervals.icu, each a distinct Intervals.icu
/// athlete, with a token of `token_type`: an OAuth grant as the code exchange
/// stores it, or an API key as the pasted link stores it.
async fn link_athletes(repos: &RepositoryRegistry, athletes: u32, token_type: &str) {
    let connection = if token_type == API_KEY_TOKEN_TYPE {
        ConnectionType::Manual
    } else {
        ConnectionType::OAuth
    };
    let tenant = TenantId::generate();
    for athlete in 0..athletes {
        let user_id = repos
            .users
            .create(&User::new(
                format!("icu-pool-{}@example.com", Uuid::new_v4()),
                "argon2-hash-placeholder".to_owned(),
                None,
            ))
            .await
            .unwrap();
        let now = Utc::now();
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id,
                tenant_id: tenant.to_string(),
                provider: INTERVALS.to_owned(),
                access_token: SecretString::from(format!("icu-credential-{athlete}")),
                refresh_token: None,
                token_type: token_type.to_owned(),
                expires_at: None,
                scope: None,
                provider_user_id: Some(format!("i{}", 9_000 + athlete)),
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(user_id, tenant, INTERVALS, &connection, None)
            .await
            .unwrap();
    }
}

/// Link `athletes` new users to Intervals.icu by OAuth: the grants the
/// server's app signs.
async fn grant_server_app(repos: &RepositoryRegistry, athletes: u32) {
    link_athletes(repos, athletes, "Bearer").await;
}

/// The budget of a credential the server's Intervals.icu app signs.
fn server_app(limiter: &Arc<ProviderRateLimiter>) -> RequestBudget {
    RequestBudget::new(Arc::clone(limiter), APP.to_owned(), None)
}

/// An Intervals.icu OAuth app few athletes granted has the provider's
/// minimum day, 5,000 requests (forum.intervals.icu/t/609), for one athlete
/// as for all: with 4,999 counted the 5,000th is taken, the next refused with
/// the wait until midnight UTC, giving back its 15-minute slot. The one
/// athlete who granted the app may spend the whole day.
#[tokio::test]
async fn an_intervals_app_few_athletes_granted_has_the_minimum_day() {
    let repos = database().await;
    grant_server_app(&repos, 1).await;
    let limiter = Arc::new(limiter_over(&repos));
    let app = budget_counter_key(INTERVALS, APP);
    spend(&repos, &app, ONE_DAY, 4_999).await;
    let athlete = server_app(&limiter);

    athlete.admit(INTERVALS).await.unwrap();
    let refused = athlete.admit(INTERVALS).await.unwrap_err();

    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    let wait = refused
        .retry_after_secs()
        .expect("the refusal names its wait");
    assert!(wait > 0 && wait <= ONE_DAY.as_secs(), "{wait}");
    assert_eq!(spent(&repos, &app, ONE_DAY).await, 5_000);
    assert_eq!(
        spent(&repos, &app, FIFTEEN_MINUTES).await,
        1,
        "the refused request gave its 15-minute slot back"
    );
}

/// Past the 50 athletes the minimum covers, each athlete who granted the app
/// adds 100 requests to its day: with 51 the day holds 5,100, the 5,100th
/// request is taken and the next refused.
#[tokio::test]
async fn each_athlete_past_the_fiftieth_adds_a_hundred_to_the_apps_day() {
    let repos = database().await;
    grant_server_app(&repos, 51).await;
    let limiter = Arc::new(limiter_over(&repos));
    spend(&repos, &budget_counter_key(INTERVALS, APP), ONE_DAY, 5_099).await;

    server_app(&limiter)
        .admit(INTERVALS)
        .await
        .expect("the 5,100th request of the day is taken");
    let refused = server_app(&limiter).admit(INTERVALS).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
}

/// Intervals.icu's 15-minute limit rolls, at an eighth of the day and at
/// least 2,500; the fixed 15-minute bucket holds half of it, 1,250, so two
/// adjacent buckets never pass the rolling limit. With 1,249 counted in the
/// bucket one more is taken and the next refused until the bucket ends,
/// taking nothing from the day.
#[tokio::test]
async fn an_intervals_apps_fifteen_minute_bucket_holds_half_the_rolling_limit() {
    let repos = database().await;
    let limiter = Arc::new(limiter_over(&repos));
    let app = budget_counter_key(INTERVALS, APP);
    spend(&repos, &app, FIFTEEN_MINUTES, 1_249).await;

    server_app(&limiter).admit(INTERVALS).await.unwrap();
    let refused = server_app(&limiter).admit(INTERVALS).await.unwrap_err();

    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    let wait = refused
        .retry_after_secs()
        .expect("the refusal names its wait");
    assert!(wait <= FIFTEEN_MINUTES.as_secs(), "{wait}");
    assert_eq!(
        spent(&repos, &app, ONE_DAY).await,
        1,
        "only the request taken counts for the day"
    );
}

/// Every athlete the app signs for spends one pool: two athletes' reads are
/// counted in the same windows, so once one of them takes the day's last
/// request the other is refused too.
#[tokio::test]
async fn two_athletes_of_one_app_spend_one_pool() {
    let repos = database().await;
    grant_server_app(&repos, 2).await;
    let registry = ProviderRegistry::new().with_request_limiter(Arc::new(limiter_over(&repos)));
    let first = registry.request_budget(APP, None).expect("a counted app");
    let second = registry.request_budget(APP, None).expect("a counted app");
    let app = budget_counter_key(INTERVALS, APP);
    spend(&repos, &app, ONE_DAY, 4_998).await;

    first.admit(INTERVALS).await.unwrap();
    second.admit(INTERVALS).await.unwrap();
    assert_eq!(spent(&repos, &app, ONE_DAY).await, 5_000);
    for athlete in [&first, &second] {
        let refused = athlete.admit(INTERVALS).await.unwrap_err();
        assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    }
}

/// A backfill runs in the background share, a quarter of the app's pool:
/// with 1,249 of the day's 5,000 counted one more backfill request is taken
/// and the next refused, while the reads athletes are waiting on keep the
/// other three quarters.
#[tokio::test]
async fn a_backfill_stops_at_a_quarter_of_the_pool_and_reads_keep_the_rest() {
    let repos = database().await;
    let limiter = Arc::new(limiter_over(&repos));
    let app = budget_counter_key(INTERVALS, APP);
    spend(&repos, &app, ONE_DAY, 1_249).await;
    let budget = server_app(&limiter);

    in_background(budget.admit(INTERVALS)).await.unwrap();
    let stopped = in_background(budget.admit(INTERVALS)).await.unwrap_err();
    assert_eq!(stopped.code, ErrorCode::ExternalRateLimited);

    budget
        .admit(INTERVALS)
        .await
        .expect("a read an athlete is waiting on is past the background share");
    assert_eq!(spent(&repos, &app, ONE_DAY).await, 1_251);
}

/// A tenant's own Intervals.icu app carries the daily budget its operator
/// registered in place of the pool the server's app grants size.
#[tokio::test]
async fn a_tenant_apps_registered_day_replaces_the_pool() {
    let repos = database().await;
    let limiter = Arc::new(limiter_over(&repos));
    let tenant_app = RequestBudget::new(Arc::clone(&limiter), "tenant-app".to_owned(), Some(3));

    for request in 0..3 {
        tenant_app
            .admit(INTERVALS)
            .await
            .unwrap_or_else(|e| panic!("request {request} is refused: {e}"));
    }
    let refused = tenant_app.admit(INTERVALS).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    server_app(&limiter)
        .admit(INTERVALS)
        .await
        .expect("the server's app keeps its own pool");
}

/// The grant count is read once and kept, not read on every request: an
/// instance that sized the pool before 51 athletes granted the app keeps the
/// minimum day and refuses the 5,001st request, while an instance that reads
/// the count afresh sizes the pool at 5,100 and takes it.
#[tokio::test]
async fn the_grant_count_is_kept_between_requests_not_read_for_each() {
    let repos = database().await;
    let counted_early = Arc::new(limiter_over(&repos));
    server_app(&counted_early).admit(INTERVALS).await.unwrap();

    grant_server_app(&repos, 51).await;
    spend(&repos, &budget_counter_key(INTERVALS, APP), ONE_DAY, 4_999).await;

    let refused = server_app(&counted_early)
        .admit(INTERVALS)
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    let counted_now = Arc::new(limiter_over(&repos));
    server_app(&counted_now)
        .admit(INTERVALS)
        .await
        .expect("a fresh count of 51 athletes sizes the day at 5,100");
}

/// Only an OAuth grant sizes the pool: an API-key link is no grant, so 51
/// athletes who pasted a key leave the server's app at the minimum day, and
/// its 5,001st request is refused.
#[tokio::test]
async fn an_api_key_link_is_no_grant_of_the_apps_pool() {
    let repos = database().await;
    link_athletes(&repos, 51, API_KEY_TOKEN_TYPE).await;
    let limiter = Arc::new(limiter_over(&repos));
    spend(&repos, &budget_counter_key(INTERVALS, APP), ONE_DAY, 5_000).await;

    let refused = server_app(&limiter).admit(INTERVALS).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
}

/// A personal Intervals.icu API key has 2,500 requests per 15 minutes
/// (forum.intervals.icu/t/609): with 2,499 counted in the window one more is
/// taken and the next refused until the window ends, taking nothing from the
/// key's day.
#[tokio::test]
async fn an_api_key_has_a_fifteen_minute_window_of_its_own() {
    let repos = database().await;
    let limiter = Arc::new(limiter_over(&repos));
    let key = api_key_counter_key(INTERVALS, ATHLETE);
    spend(&repos, &key, FIFTEEN_MINUTES, 2_499).await;
    let budget = RequestBudget::for_api_key(Arc::clone(&limiter), ATHLETE.to_owned());

    budget.admit(INTERVALS).await.unwrap();
    let refused = budget.admit(INTERVALS).await.unwrap_err();

    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    let wait = refused
        .retry_after_secs()
        .expect("the refusal names its wait");
    assert!(wait <= FIFTEEN_MINUTES.as_secs(), "{wait}");
    assert_eq!(
        spent(&repos, &key, ONE_DAY).await,
        1,
        "only the request taken counts for the day"
    );
}

/// A personal Intervals.icu API key has 5,000 requests a day
/// (forum.intervals.icu/t/609): with 4,999 counted today one more is taken
/// and the next refused, giving back its 15-minute slot.
#[tokio::test]
async fn an_api_key_has_a_daily_window_of_its_own() {
    let repos = database().await;
    let limiter = Arc::new(limiter_over(&repos));
    let key = api_key_counter_key(INTERVALS, ATHLETE);
    spend(&repos, &key, ONE_DAY, 4_999).await;
    let budget = RequestBudget::for_api_key(Arc::clone(&limiter), ATHLETE.to_owned());

    budget.admit(INTERVALS).await.unwrap();
    let refused = budget.admit(INTERVALS).await.unwrap_err();

    assert_eq!(refused.code, ErrorCode::ExternalRateLimited);
    assert_eq!(
        spent(&repos, &key, FIFTEEN_MINUTES).await,
        1,
        "the refused request gave its 15-minute slot back"
    );
}

/// A personal API key is held to its own windows only: with the server
/// app's day spent, the key's requests are still taken, and none of them is
/// counted against the app.
#[tokio::test]
async fn an_api_key_is_held_to_no_app_window() {
    let repos = database().await;
    let limiter = Arc::new(limiter_over(&repos));
    let app = budget_counter_key(INTERVALS, APP);
    spend(&repos, &app, ONE_DAY, 5_000).await;
    let budget = RequestBudget::for_api_key(Arc::clone(&limiter), ATHLETE.to_owned());

    for request in 0..101 {
        budget
            .admit(INTERVALS)
            .await
            .unwrap_or_else(|e| panic!("request {request} is refused: {e}"));
    }
    assert_eq!(
        spent(&repos, &api_key_counter_key(INTERVALS, ATHLETE), ONE_DAY).await,
        101
    );
    assert_eq!(spent(&repos, &app, ONE_DAY).await, 5_000);
}

/// A key whose account is unknown is counted nowhere rather than under an
/// empty account, where every such key would share one set of windows; a key
/// whose account is known is counted in its own.
#[tokio::test]
async fn a_key_with_no_account_shares_no_windows() {
    let repos = database().await;
    let registry = ProviderRegistry::new().with_request_limiter(Arc::new(limiter_over(&repos)));

    assert!(registry.api_key_budget("").is_none());
    registry
        .api_key_budget(ATHLETE)
        .expect("a key with its account carries its budget")
        .admit(INTERVALS)
        .await
        .unwrap();
    assert_eq!(
        spent(&repos, &api_key_counter_key(INTERVALS, ATHLETE), ONE_DAY).await,
        1
    );
}

/// The athlete whose detail walk the tests below run, each a fresh one.
fn walker() -> Uuid {
    Uuid::new_v4()
}

/// One Strava request signed by `app` with `daily_limit`, made inside
/// `user_id`'s detail walk.
async fn walk_call(
    limiter: &ProviderRateLimiter,
    user_id: Uuid,
    app: &str,
    daily_limit: Option<u32>,
) -> RateLimitStatus {
    in_detail_walk(user_id, limiter.acquire("strava", app, daily_limit))
        .await
        .unwrap()
}

/// The detail walk's app window and `user_id`'s window under `app`'s
/// Strava windows, as spent today.
async fn walk_spent(repos: &RepositoryRegistry, app: &str, user_id: Uuid) -> (i64, i64) {
    let holder = budget_counter_key("strava", app);
    (
        spent(repos, &detail_walk_counter_key(&holder), ONE_DAY).await,
        spent(
            repos,
            &detail_walk_athlete_counter_key(&holder, user_id),
            ONE_DAY,
        )
        .await,
    )
}

/// One athlete's detail walk makes at most 50 Strava calls a day
/// (carnet#1483): with a day roomy enough that no other window refuses
/// first, the 50th call is taken and the 51st refused by the athlete's own
/// window, taking nothing from the app's, while another athlete's walk is
/// still admitted.
#[tokio::test]
async fn an_athletes_detail_walk_is_refused_its_fifty_first_call_of_the_day() {
    let repos = database().await;
    let limiter = limiter_over(&repos);
    limiter.set_budgets("strava", &[(1_000, ONE_DAY)]);
    let athlete = walker();

    for call in 0..DETAIL_WALK_DAILY_CALLS {
        assert_eq!(
            walk_call(&limiter, athlete, APP, None).await,
            RateLimitStatus::Allowed,
            "call {call} of the athlete's day is within the 50"
        );
    }
    let RateLimitStatus::Exceeded { retry_after } = walk_call(&limiter, athlete, APP, None).await
    else {
        panic!("the 51st call of the athlete's day is refused");
    };
    assert!(retry_after <= ONE_DAY, "{retry_after:?}");
    assert_eq!(
        walk_spent(&repos, APP, athlete).await,
        (50, 50),
        "the refused call took no slot of the walk's windows"
    );
    assert_eq!(
        spent(&repos, &budget_counter_key("strava", APP), ONE_DAY).await,
        50,
        "the refused call took no slot of the app's day"
    );
    assert_eq!(
        walk_call(&limiter, walker(), APP, None).await,
        RateLimitStatus::Allowed,
        "another athlete's walk has a window of its own"
    );
}

/// Every athlete's detail walk spends one app window, half the background
/// share of the app's day: at 480 a day the background share is 120 and the
/// walk window 60, so one athlete taking their 50 and a second their 10
/// spends it, and the second athlete's 11th call is refused with their own
/// window still open and the app's day still roomy.
#[tokio::test]
async fn the_detail_walk_app_window_refuses_across_two_athletes() {
    let repos = database().await;
    let limiter = limiter_over(&repos);
    limiter.set_budgets("strava", &[(480, ONE_DAY)]);
    let (first, second) = (walker(), walker());

    for call in 0..50 {
        assert_eq!(
            walk_call(&limiter, first, APP, None).await,
            RateLimitStatus::Allowed,
            "the first athlete's call {call} is within both windows"
        );
    }
    for call in 0..10 {
        assert_eq!(
            walk_call(&limiter, second, APP, None).await,
            RateLimitStatus::Allowed,
            "the second athlete's call {call} is within the app window"
        );
    }
    assert!(
        matches!(
            walk_call(&limiter, second, APP, None).await,
            RateLimitStatus::Exceeded { .. }
        ),
        "the app window of 60 is spent"
    );
    assert_eq!(
        walk_spent(&repos, APP, second).await,
        (60, 10),
        "bug: the app window does not hold both athletes' 60 calls, or the second athlete's window not their 10"
    );
    assert_eq!(
        spent(&repos, &budget_counter_key("strava", APP), ONE_DAY).await,
        60,
        "the app's day kept background room: the walk window refused"
    );
    assert!(
        matches!(
            walk_call(&limiter, walker(), APP, None).await,
            RateLimitStatus::Exceeded { .. }
        ),
        "bug: a third athlete with an untouched window is admitted past the spent app window"
    );
}

/// The walk windows are sized from the day the signing app actually has,
/// never from Strava's default: a tenant app registered at 80 a day has a
/// background share of 20, so a walk window of 10, and the athlete's window
/// is held to it (the smaller of 50 and the app's).
#[tokio::test]
async fn a_registered_daily_limit_sizes_the_detail_walk_windows() {
    let repos = database().await;
    let limiter = limiter_over(&repos);
    let athlete = walker();

    for call in 0..10 {
        assert_eq!(
            walk_call(&limiter, athlete, "tenant-app", Some(80)).await,
            RateLimitStatus::Allowed,
            "call {call} is within the tenant app's walk window"
        );
    }
    assert!(
        matches!(
            walk_call(&limiter, athlete, "tenant-app", Some(80)).await,
            RateLimitStatus::Exceeded { .. }
        ),
        "bug: an app registered at 80 a day admits an 11th walk call"
    );
    assert_eq!(
        walk_spent(&repos, "tenant-app", athlete).await,
        (10, 10),
        "bug: the tenant app's walk windows are not sized from its registered 80 a day"
    );
    assert_eq!(
        walk_call(&limiter, athlete, APP, None).await,
        RateLimitStatus::Allowed,
        "the server's app has walk windows of its own"
    );
}

/// The day a walk's ETA is paced by is the athlete's walk window as the
/// limiter sizes it for the signing app: 50 at Strava's 1,000, held to the
/// app's walk window below 400 (a tenant app at 80 a day: 10), none at all
/// below 8 (a background share of 1, half of it rounded down: 0), and no
/// walk window at a provider that has none.
#[tokio::test]
async fn a_budget_names_the_athletes_detail_walk_day_as_the_limiter_sizes_it() {
    let repos = database().await;
    let limiter = Arc::new(limiter_over(&repos));
    let budget =
        |daily_limit| RequestBudget::new(Arc::clone(&limiter), APP.to_owned(), daily_limit);

    assert_eq!(
        budget(None).detail_walk_daily_calls("strava").await,
        Some(DETAIL_WALK_DAILY_CALLS),
        "Strava's 1,000 a day leaves the athlete their whole 50"
    );
    assert_eq!(
        budget(Some(80)).detail_walk_daily_calls("strava").await,
        Some(10),
        "bug: the athlete's window is not held to the app's walk window"
    );
    assert_eq!(
        budget(Some(7)).detail_walk_daily_calls("strava").await,
        Some(0),
        "bug: an app registered at 7 a day reads as having room for a walk"
    );
    assert_eq!(
        budget(None).detail_walk_daily_calls("garmin").await,
        None,
        "bug: a provider with no walk windows names one"
    );
}

/// A walk call refused by a later window gives back the slots the earlier
/// ones took: refused by the app's 15-minute background share, the
/// athlete's and the app's walk windows are back to nothing; refused by the
/// app's walk window, the athlete's is too.
#[tokio::test]
async fn a_refused_detail_walk_call_gives_back_the_slots_it_took() {
    let repos = database().await;
    let limiter = limiter_over(&repos);
    let holder = budget_counter_key("strava", APP);
    let athlete = walker();
    spend(&repos, &holder, FIFTEEN_MINUTES, 25).await;

    let RateLimitStatus::Exceeded { retry_after } = walk_call(&limiter, athlete, APP, None).await
    else {
        panic!("the 15-minute background share is spent");
    };
    assert!(retry_after <= FIFTEEN_MINUTES, "{retry_after:?}");
    assert_eq!(
        walk_spent(&repos, APP, athlete).await,
        (0, 0),
        "bug: a call the 15-minute share refused kept its walk window slots"
    );
    assert_eq!(
        spent(&repos, &holder, ONE_DAY).await,
        0,
        "bug: a call the 15-minute share refused kept its slot in the app's day"
    );

    // A second app, whose 15-minute share is untouched: its walk window of
    // 125 (half the background share of 1,000) is what refuses.
    let pool = budget_counter_key("strava", "pool-app");
    let other = walker();
    spend(&repos, &detail_walk_counter_key(&pool), ONE_DAY, 125).await;
    assert!(
        matches!(
            walk_call(&limiter, other, "pool-app", None).await,
            RateLimitStatus::Exceeded { .. }
        ),
        "bug: a spent app walk window of 125 admits a call"
    );
    assert_eq!(
        walk_spent(&repos, "pool-app", other).await,
        (125, 0),
        "bug: a call the app walk window refused kept the athlete's slot"
    );
    assert_eq!(
        spent(&repos, &pool, FIFTEEN_MINUTES).await,
        0,
        "the app's 15-minute share was never reached"
    );
}

/// A detail walk is work nobody waits on, so the app's background share
/// still applies inside it: with a quarter of Strava's day spent, a walk
/// call with both walk windows open is refused.
#[tokio::test]
async fn the_background_share_still_applies_inside_a_detail_walk() {
    let repos = database().await;
    let limiter = limiter_over(&repos);
    let athlete = walker();
    assert!(
        in_detail_walk(athlete, async { is_background() }).await,
        "bug: a detail walk's scope does not read as background work"
    );
    spend(&repos, &budget_counter_key("strava", APP), ONE_DAY, 250).await;

    assert!(
        matches!(
            walk_call(&limiter, athlete, APP, None).await,
            RateLimitStatus::Exceeded { .. }
        ),
        "bug: a walk call is admitted past the app's spent background share"
    );
    assert_eq!(
        walk_spent(&repos, APP, athlete).await,
        (0, 0),
        "bug: a walk call the background share refused kept its walk window slots"
    );
    assert_eq!(
        limiter.acquire("strava", APP, None).await.unwrap(),
        RateLimitStatus::Allowed,
        "a live read keeps the rest of the day, and counts in no walk window"
    );
    assert_eq!(
        walk_spent(&repos, APP, athlete).await,
        (0, 0),
        "bug: a live read outside a walk counted in a walk window"
    );
}

/// Only a provider with detail-walk windows has them: a Garmin read inside
/// a detail walk takes only Garmin's background share, so it is admitted
/// past 50 calls and counts in no walk window.
#[tokio::test]
async fn a_provider_without_detail_walk_windows_takes_only_its_background_share() {
    let repos = database().await;
    let limiter = limiter_over(&repos);
    let athlete = walker();

    for call in 0..=DETAIL_WALK_DAILY_CALLS {
        assert_eq!(
            in_detail_walk(athlete, limiter.acquire("garmin", APP, None))
                .await
                .unwrap(),
            RateLimitStatus::Allowed,
            "Garmin call {call} fits Garmin's background share"
        );
    }
    let holder = budget_counter_key("garmin", APP);
    assert_eq!(
        spent(&repos, &detail_walk_counter_key(&holder), ONE_DAY).await,
        0,
        "bug: a Garmin read counted in an app walk window Garmin does not have"
    );
    assert_eq!(
        spent(
            &repos,
            &detail_walk_athlete_counter_key(&holder, athlete),
            ONE_DAY
        )
        .await,
        0,
        "bug: a Garmin read counted in an athlete walk window Garmin does not have"
    );
}

/// A credential's budget refuses a walk call past the athlete's day with the
/// limiter's own refusal, sent nothing, and waits until the athlete's day
/// ends at midnight UTC, the end a paused walk resumes at.
#[tokio::test]
async fn a_detail_walk_refusal_waits_until_the_athletes_day_ends() {
    let repos = database().await;
    let limiter = Arc::new(limiter_over(&repos));
    let athlete = walker();
    let holder = budget_counter_key("strava", APP);
    spend(
        &repos,
        &detail_walk_athlete_counter_key(&holder, athlete),
        ONE_DAY,
        i64::from(DETAIL_WALK_DAILY_CALLS),
    )
    .await;
    let budget = RequestBudget::new(Arc::clone(&limiter), APP.to_owned(), None);

    let refused = in_detail_walk(athlete, budget.admit("strava"))
        .await
        .unwrap_err();

    assert_eq!(
        refused.code,
        ErrorCode::ExternalRateLimited,
        "bug: a walk call past the athlete's day is not the limiter's own refusal"
    );
    assert!(refused_before_send(&refused), "nothing was sent");
    let now = Utc::now().timestamp();
    let to_midnight = i64::try_from(ONE_DAY.as_secs()).unwrap() - now.rem_euclid(86_400);
    let wait = i64::try_from(
        refused
            .retry_after_secs()
            .expect("the refusal names its wait"),
    )
    .unwrap();
    assert!(
        (to_midnight - wait).abs() <= 5,
        "the wait {wait}s is the athlete's day, ending in {to_midnight}s"
    );
    assert_eq!(
        spent(&repos, &holder, FIFTEEN_MINUTES).await,
        0,
        "bug: a walk call the athlete's window refused kept the app's 15-minute slot"
    );
}
