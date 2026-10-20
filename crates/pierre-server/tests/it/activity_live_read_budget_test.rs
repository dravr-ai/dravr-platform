// ABOUTME: A live activity read that outlives the refresh budget answers from the cache, never fails the call
// ABOUTME: The read keeps running past the budget and its write-through still lands for the next turn
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A healthy scrape takes about two minutes and the chat tool loop gives a
//! tool call ninety seconds, so `get_activities` reading a scraped provider
//! inline could fail on a provider that was working. These tests drive
//! [`read_live_window_within_budget`] with a provider that answers only when
//! the test lets it.

use crate::common;

use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, ActivityBuilder, Athlete, SportType, Stats, TenantId};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_database::repositories::activity_cache::{
    ActivityFetchFailure, ActivityFetchFailureRecord,
};
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_tool_runtime::activity_fetch::budgeted_read::{
    read_live_window_within_budget, BudgetedRead, OwnedLiveRead,
};
use pierre_tool_runtime::activity_fetch::refresh_stale_head;
use pierre_tool_runtime::runtime::ToolRuntime;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

const PROVIDER: &str = "sciotte";
const BUDGET: StdDuration = StdDuration::from_millis(200);

/// A provider whose list read waits until the test notifies it.
struct GatedProvider {
    config: ProviderConfig,
    gate: Arc<Notify>,
    rows: Vec<Activity>,
}

#[async_trait]
impl FitnessProvider for GatedProvider {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        true
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        Ok(())
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        self.gate.notified().await;
        Ok(self.rows.clone())
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        Err(AppError::not_found(id.to_owned()))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        Err(AppError::internal("not read by this test"))
    }
}

fn activity(id: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id,
        "Tempo run",
        SportType::Run,
        Utc::now() - Duration::days(days_ago),
        3_000,
        PROVIDER,
    )
    .distance_meters(10_000.0)
    .build()
}

fn gated(gate: &Arc<Notify>, rows: Vec<Activity>) -> Box<GatedProvider> {
    Box::new(GatedProvider {
        config: ProviderConfig {
            name: PROVIDER.to_owned(),
            auth_url: String::new(),
            token_url: String::new(),
            api_base_url: String::new(),
            revoke_url: None,
            default_scopes: Vec::new(),
        },
        gate: Arc::clone(gate),
        rows,
    })
}

fn window(user_id: Uuid, tenant_id: TenantId) -> OwnedLiveRead {
    OwnedLiveRead {
        provider_slug: PROVIDER.to_owned(),
        user_id,
        tenant_id,
        params: ActivityQueryParams {
            after: Some((Utc::now() - Duration::days(30)).timestamp()),
            before: None,
            limit: Some(50),
            offset: None,
        },
        attempt_started_at: Utc::now(),
    }
}

fn ids(rows: &[Activity]) -> Vec<&str> {
    let mut ids: Vec<&str> = rows.iter().map(Activity::id).collect();
    ids.sort_unstable();
    ids
}

async fn runtime_and_user() -> (Arc<dyn ToolRuntime>, Uuid, TenantId) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = TenantId::from_uuid(Uuid::new_v4());
    (resources, user_id, tenant_id)
}

#[tokio::test]
async fn a_read_past_the_budget_answers_from_the_cache_and_still_lands() {
    let (runtime, user_id, tenant_id) = runtime_and_user().await;
    runtime
        .repos()
        .activity_cache
        .upsert_activities(user_id, &tenant_id, PROVIDER, &[activity("cached", 3)])
        .await
        .unwrap();

    // The provider answers only once the test lets it, which is after the
    // turn: a read awaited inline would never return, so bound the wait.
    let gate = Arc::new(Notify::new());
    let outcome = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(
            &runtime,
            gated(&gate, vec![activity("cached", 3), activity("fresh", 0)]),
            window(user_id, tenant_id),
            BUDGET,
        ),
    )
    .await
    .expect("the turn waited on a read that had not answered");
    let BudgetedRead::Cached(rows) = outcome else {
        panic!("a read past the budget must answer from the cached window");
    };
    assert_eq!(ids(&rows), ["cached"]);

    // The read was not dropped: once the provider answers, its rows are
    // written through for the next turn.
    gate.notify_one();
    let landed = timeout(StdDuration::from_secs(10), async {
        loop {
            let cached = runtime
                .repos()
                .activity_cache
                .get_cached_activities(
                    user_id,
                    &tenant_id,
                    Some(PROVIDER),
                    Utc::now() - Duration::days(30),
                    Utc::now() + Duration::days(1),
                    50,
                )
                .await
                .unwrap();
            if cached.iter().any(|a| a.id() == "fresh") {
                return;
            }
            sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        landed.is_ok(),
        "the detached read never wrote its rows through"
    );
}

#[tokio::test]
async fn with_nothing_cached_the_turn_waits_for_the_read() {
    let (runtime, user_id, tenant_id) = runtime_and_user().await;
    let gate = Arc::new(Notify::new());
    let release = Arc::clone(&gate);
    tokio::spawn(async move {
        sleep(BUDGET * 3).await;
        release.notify_one();
    });

    let outcome = read_live_window_within_budget(
        &runtime,
        gated(&gate, vec![activity("fresh", 0)]),
        window(user_id, tenant_id),
        BUDGET,
    )
    .await;
    let BudgetedRead::Answered { read, provider } = outcome else {
        panic!("with no cached window the read itself must answer");
    };
    assert_eq!(ids(&read.unwrap().activities), ["fresh"]);
    assert!(provider.is_some(), "the provider comes back with the read");
}

#[tokio::test]
async fn a_read_inside_the_budget_answers_live() {
    let (runtime, user_id, tenant_id) = runtime_and_user().await;
    let gate = Arc::new(Notify::new());
    gate.notify_one();

    let outcome = read_live_window_within_budget(
        &runtime,
        gated(&gate, vec![activity("fresh", 0)]),
        window(user_id, tenant_id),
        StdDuration::from_secs(10),
    )
    .await;
    let BudgetedRead::Answered { read, .. } = outcome else {
        panic!("a read inside the budget answers live");
    };
    assert_eq!(ids(&read.unwrap().activities), ["fresh"]);
}

/// A stale head whose last refreshes failed is not read again this turn, and
/// the window says so instead of passing for current.
#[tokio::test]
async fn a_stale_head_held_back_by_its_failures_is_reported_behind() {
    let (runtime, user_id, tenant_id) = runtime_and_user().await;
    let cache = &runtime.repos().activity_cache;
    let last_sync = Utc::now() - Duration::days(2);
    cache
        .upsert_activities_synced_at(
            user_id,
            &tenant_id,
            PROVIDER,
            &[activity("cached", 3)],
            last_sync,
        )
        .await
        .unwrap();
    let failed = Utc::now() - Duration::minutes(1);
    cache
        .record_activity_fetch_failure(
            user_id,
            &tenant_id,
            PROVIDER,
            &ActivityFetchFailureRecord {
                failed_at: failed,
                failure: ActivityFetchFailure::FetchError,
                consecutive: 1,
                streak: 1,
                streak_started_at: failed,
            },
        )
        .await
        .unwrap();

    let mut served = vec![activity("cached", 3)];
    let behind =
        refresh_stale_head(&runtime, PROVIDER, user_id, tenant_id, None, &mut served).await;
    assert!(
        behind,
        "a stale head that was not refreshed must read as behind"
    );
    assert_eq!(ids(&served), ["cached"]);

    // A closed window has no head to be behind.
    let closed = Some((Utc::now() - Duration::days(10)).timestamp());
    let behind =
        refresh_stale_head(&runtime, PROVIDER, user_id, tenant_id, closed, &mut served).await;
    assert!(!behind, "a closed window is never behind");
}
