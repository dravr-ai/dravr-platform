// ABOUTME: The per-athlete windows the live provider context reads of a signed-in session are counted in
// ABOUTME: 200 a day and 30 in 15 minutes per athlete, apart from the walks' windows and any app's

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The windows of the provider context reads (carnet#1622).
//!
//! A tool that analyses an activity or reads the athlete's profile reads
//! Strava's own zones, best efforts, records, weight and gear live, beside
//! its answer. Through a signed-in session (sciotte reading strava.com as the
//! athlete), each of those reads is a page of the athlete's own session, so
//! they are counted per athlete ([`RequestBudget::for_session_context`]):
//! [`CONTEXT_READ_BURST_CALLS`] in [`FIFTEEN_MINUTES`] and
//! [`CONTEXT_READ_DAILY_CALLS`] a day (JF's call on carnet#1622).
//!
//! The windows are the context reads' own. They never touch the walks'
//! windows or the app's API windows, so an athlete who asks many questions
//! never slows their walks, and a walk never starves their answers. A refused
//! read leaves the provider's block out of the answer; the tool's own read is
//! never counted here.
//!
//! [`RequestBudget::for_session_context`]: super::RequestBudget::for_session_context

use uuid::Uuid;

use super::{Budget, Window, FIFTEEN_MINUTES, ONE_DAY};

/// The most provider context reads one athlete's signed-in session makes in
/// a day (JF's call on carnet#1622).
pub(crate) const CONTEXT_READ_DAILY_CALLS: u32 = 200;

/// The most provider context reads one athlete's signed-in session makes in
/// [`FIFTEEN_MINUTES`] (JF's call on carnet#1622).
///
/// An agent looping over a tool cannot send the day's reads to the session
/// back to back.
pub const CONTEXT_READ_BURST_CALLS: u32 = 30;

/// The `counter_key` of the context-read windows of `user_id`'s session at
/// `backend`.
#[must_use]
pub fn context_read_counter_key(backend: &str, user_id: Uuid) -> String {
    format!("provider_context_reads:{backend}:{user_id}")
}

/// The windows one context read of `user_id`'s session at `backend` is taken
/// from: the burst first, so a refusal by it takes nothing from the day's,
/// each taken whole whatever the running task.
pub(super) fn context_read_windows(backend: &str, user_id: Uuid) -> Vec<Window> {
    let key = context_read_counter_key(backend, user_id);
    [
        (CONTEXT_READ_BURST_CALLS, FIFTEEN_MINUTES),
        (CONTEXT_READ_DAILY_CALLS, ONE_DAY),
    ]
    .into_iter()
    .map(|(max_calls, window)| Window {
        key: key.clone(),
        budget: Budget { max_calls, window },
        share_percent: 100,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use dravr_tronc::dates::Timestamp;
    use pierre_core::errors::ErrorCode;
    use pierre_database::RepositoryRegistry;
    use pierre_test_support::db::create_test_db;

    use super::super::{
        budget_counter_key, budget_period, detail_walk_athlete_counter_key,
        detail_walk_counter_key, in_background, in_detail_walk, ProviderRateLimiter, RequestBudget,
        PLATFORM_SCOPE,
    };
    use super::*;

    const SESSION: &str = "sciotte";

    async fn limiter() -> (Arc<RepositoryRegistry>, Arc<ProviderRateLimiter>) {
        let db = create_test_db()
            .await
            .unwrap_or_else(|e| panic!("test db: {e}"));
        let repos = Arc::clone(db.repositories());
        let limiter = Arc::new(ProviderRateLimiter::new(
            Arc::clone(&repos.usage_counters),
            Arc::clone(&repos.provider_connections),
        ));
        (repos, limiter)
    }

    /// What `key` holds in its `window` bucket now; `0` when nothing was
    /// counted.
    async fn spent(repos: &RepositoryRegistry, key: &str, window: Duration) -> i64 {
        repos
            .usage_counters
            .get_counter(
                PLATFORM_SCOPE,
                PLATFORM_SCOPE,
                key,
                &budget_period(Timestamp::now(), window),
            )
            .await
            .map_or(0, |counter| counter.value)
    }

    fn context_budget(limiter: &Arc<ProviderRateLimiter>, user_id: Uuid) -> RequestBudget {
        RequestBudget::for_session_context(limiter, SESSION, user_id)
            .unwrap_or_else(|| panic!("bug: the signed-in session has context windows"))
    }

    #[tokio::test]
    async fn only_a_counted_session_has_context_windows() {
        let (_, limiter) = limiter().await;
        let user_id = Uuid::new_v4();
        assert!(
            RequestBudget::for_session_context(&limiter, SESSION, user_id).is_some(),
            "bug: the signed-in session's reads are counted"
        );
        for backend in ["strava", "intervals_icu", "sciotte_garmin"] {
            assert!(
                RequestBudget::for_session_context(&limiter, backend, user_id).is_none(),
                "{backend} reads are no session's context reads"
            );
        }
    }

    #[tokio::test]
    async fn an_athletes_context_reads_stop_at_the_burst_and_spare_other_athletes() {
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        let budget = context_budget(&limiter, athlete);
        for call in 0..CONTEXT_READ_BURST_CALLS {
            assert!(
                budget.admit(SESSION).await.is_ok(),
                "read {call} is within the burst of {CONTEXT_READ_BURST_CALLS}"
            );
        }
        let refused = budget
            .admit(SESSION)
            .await
            .expect_err("the read past the burst is refused");
        assert_eq!(
            refused.code,
            ErrorCode::ExternalRateLimited,
            "bug: a spent burst refuses as a rate limit"
        );
        let key = context_read_counter_key(SESSION, athlete);
        assert_eq!(
            spent(&repos, &key, ONE_DAY).await,
            i64::from(CONTEXT_READ_BURST_CALLS),
            "a refused read takes nothing from the day"
        );
        assert!(
            context_budget(&limiter, Uuid::new_v4())
                .admit(SESSION)
                .await
                .is_ok(),
            "another athlete's session has its own windows"
        );
    }

    #[tokio::test]
    async fn a_spent_day_refuses_the_athletes_context_reads() {
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        let key = context_read_counter_key(SESSION, athlete);
        repos
            .usage_counters
            .increment_counter(
                PLATFORM_SCOPE,
                PLATFORM_SCOPE,
                &key,
                &budget_period(Timestamp::now(), ONE_DAY),
                i64::from(CONTEXT_READ_DAILY_CALLS),
            )
            .await
            .unwrap_or_else(|e| panic!("bug: the day could not be spent: {e}"));
        let refused = context_budget(&limiter, athlete)
            .admit(SESSION)
            .await
            .expect_err("a spent day refuses the read");
        assert_eq!(
            refused.code,
            ErrorCode::ExternalRateLimited,
            "bug: a spent day refuses as a rate limit"
        );
        assert_eq!(
            spent(&repos, &key, FIFTEEN_MINUTES).await,
            0,
            "the burst taken before the day refused is given back"
        );
    }

    #[tokio::test]
    async fn context_reads_never_touch_the_walk_or_app_windows() {
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        let budget = context_budget(&limiter, athlete);
        // Counted the same in the background: the windows are taken whole.
        in_background(budget.admit(SESSION))
            .await
            .unwrap_or_else(|e| panic!("bug: the first read is admitted: {e}"));
        budget
            .admit(SESSION)
            .await
            .unwrap_or_else(|e| panic!("bug: the second read is admitted: {e}"));
        // Inside the athlete's own detail walk too: no walk window is added.
        in_detail_walk(athlete, budget.admit(SESSION))
            .await
            .unwrap_or_else(|e| panic!("bug: a read inside the walk is admitted: {e}"));
        let key = context_read_counter_key(SESSION, athlete);
        assert_eq!(
            spent(&repos, &key, FIFTEEN_MINUTES).await,
            3,
            "bug: each read is one in the burst"
        );
        assert_eq!(
            spent(&repos, &key, ONE_DAY).await,
            3,
            "bug: each read is one in the day"
        );
        for walk in [
            detail_walk_counter_key(&key),
            detail_walk_athlete_counter_key(&key, athlete),
        ] {
            assert_eq!(
                spent(&repos, &walk, ONE_DAY).await,
                0,
                "bug: a context read took a walk window"
            );
        }
        let app = budget_counter_key("strava", "");
        assert_eq!(spent(&repos, &app, ONE_DAY).await, 0, "no app window");
        assert_eq!(
            spent(
                &repos,
                &detail_walk_athlete_counter_key(&app, athlete),
                ONE_DAY
            )
            .await,
            0,
            "no walk window"
        );
    }
}
