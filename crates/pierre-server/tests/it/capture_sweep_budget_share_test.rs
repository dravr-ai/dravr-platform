// ABOUTME: The capture sweep spends only the background share of a provider app's request budget
// ABOUTME: Pins carnet#1491: the sweep read every Strava head at the full share the athletes' reads need

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1491. Nobody waits on the capture sweep, but it read every head at
//! the full share of the signing app's windows. On 2026-11-10, after a runaway
//! backfill had spent Strava's day, every sweep read was refused as a spent
//! foreground budget and paged as a failed head read.
//!
//! The real Strava provider is pointed at a loopback stand-in that counts the
//! activity-list reads it receives, so what reached Strava is counted, not
//! inferred.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use pierre_core::models::{ConnectionType, UserOAuthToken};
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::{
    budget_counter_key, budget_period, ProviderRateLimiter, BACKGROUND_SHARE_PERCENT, ONE_DAY,
    PLATFORM_SCOPE,
};
use pierre_tool_runtime::activity_fetch::fetch_provider_head;
use pierre_tool_runtime::capture_sweep::{
    refresh_captures, RefreshOutcome, SweepBudget, SweepSelection,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::SecretString;
use serde_json::Value;
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::common;

/// The Strava app the test server signs with (`common::set_server_app`).
const APP: &str = "test_client";
/// The app's day in this test: small, so its background share is a count.
const DAY_BUDGET: u32 = 8;

/// A Strava stand-in with an empty feed that counts every list read.
async fn mock_strava() -> (String, Arc<AtomicUsize>) {
    let reads = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/athlete/activities",
            get(|State(reads): State<Arc<AtomicUsize>>| async move {
                reads.fetch_add(1, Ordering::SeqCst);
                Json(Value::Array(Vec::new()))
            }),
        )
        .with_state(Arc::clone(&reads));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), reads)
}

/// The sweep spends only the background share of the app's day: with that
/// share already counted it sends Strava nothing and reports the fetch
/// failed, while a read an athlete waits on still goes through.
#[tokio::test]
async fn the_sweep_takes_only_the_background_share_of_the_app() {
    let (base, reads) = mock_strava().await;
    let mut config = common::test_server_config_with_api_base("strava", &base);
    common::set_server_app(&mut config, "strava", APP, "test_secret");
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let repos = &resources.common.repos;
    let limiter = ProviderRateLimiter::new(
        Arc::clone(&repos.usage_counters),
        Arc::clone(&repos.provider_connections),
    );
    limiter.set_budgets("strava", &[(DAY_BUDGET, ONE_DAY)]);
    let mut context = (*resources).clone();
    context.fitness.provider_registry = Arc::new(
        ProviderRegistry::new()
            .with_endpoint_overrides(&resources.common.config.oauth.endpoints)
            .with_request_limiter(Arc::new(limiter)),
    );
    let runtime: Arc<dyn ToolRuntime> = Arc::new(context);

    let (user_id, _user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        "swept@budget.test",
        "starter",
    )
    .await
    .unwrap();
    let now = Utc::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: "strava".to_owned(),
            // >= 40 chars and not "at_"-prefixed: the provider's token validation.
            access_token: SecretString::from(format!("strava_access_token_{:0>30}", 1)),
            refresh_token: Some(SecretString::from("strava_refresh")),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + Duration::hours(6)),
            scope: Some("activity:read_all".to_owned()),
            provider_user_id: Some("1".to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .usage_counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &budget_counter_key("strava", APP),
            &budget_period(Utc::now(), ONE_DAY),
            i64::from(DAY_BUDGET * BACKGROUND_SHARE_PERCENT / 100),
        )
        .await
        .unwrap();

    let report = refresh_captures(&runtime, SweepBudget::default(), SweepSelection::default())
        .await
        .expect("refresh report");
    let line = report
        .connections
        .iter()
        .find(|c| c.user_id == user_id.to_string() && c.provider == "strava")
        .expect("the connection was walked");
    assert!(
        matches!(line.outcome, RefreshOutcome::Failed { .. }),
        "the sweep's read is refused past the background share: {:?}",
        line.outcome
    );
    assert_eq!(
        reads.load(Ordering::SeqCst),
        0,
        "bug: the sweep read Strava past the background share of the app's day"
    );
    assert_eq!(
        repos
            .activity_cache
            .latest_activity_fetch_failure(user_id, &tenant, "strava")
            .await
            .unwrap(),
        None,
        "bug: a refusal of the sweep's share was recorded as a failed sync, pausing the reads athletes wait on"
    );

    let params = ActivityQueryParams {
        limit: Some(50),
        offset: None,
        before: None,
        after: Some((now - Duration::days(7)).timestamp()),
    };
    fetch_provider_head(&runtime, "strava", user_id, &tenant.to_string(), &params)
        .await
        .expect("a read an athlete waits on keeps the rest of the day");
    assert_eq!(
        reads.load(Ordering::SeqCst),
        1,
        "bug: a read an athlete waits on did not reach Strava past the background share"
    );
}
