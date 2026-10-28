// ABOUTME: A push-only Garmin read serves what Garmin pushed without restamping it as a fresh sync
// ABOUTME: And no history capture is started for a provider that only pushes its data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin API reads in the live-read paths (carnet#1155).
//!
//! The Garmin API provider has no list to read: its activities are the rows
//! the push ingest cached. A live list read of it must hand those rows back
//! and leave the provider's freshness where the last push put it: written
//! back through the cache, every row's `synced_at` would read as now, and a
//! connection whose pushes stopped would look current. A history capture
//! would do the same and claim a depth no push reached.
#![cfg(feature = "provider-garmin")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_mcp_server::constants::oauth_providers;
use pierre_providers::core::{ActivityQueryParams, FitnessProvider, OwnerStores};
use pierre_providers::garmin_provider::GarminProvider;
use pierre_tool_runtime::activity_backfill::{spawn_activity_backfill, ActivityBackfillJob};
use pierre_tool_runtime::activity_fetch::read_live_window;
use pierre_tool_runtime::activity_fetch::sync_verdict::{HeadVerdict, LiveRead};
use pierre_tool_runtime::runtime::ToolRuntime;
use uuid::Uuid;

fn pushed(id: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id,
        "Pushed",
        SportType::Run,
        Utc::now() - Duration::days(days_ago),
        2_400,
        oauth_providers::GARMIN,
    )
    .source(oauth_providers::GARMIN)
    .distance_meters(8_000.0)
    .build()
}

async fn runtime_and_user() -> (Arc<dyn ToolRuntime>, Uuid, TenantId) {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    (resources, user_id, TenantId::from_uuid(Uuid::new_v4()))
}

/// The pushes, received three days ago.
async fn seed_pushes(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: TenantId,
) -> DateTime<Utc> {
    let received = Utc::now() - Duration::days(3);
    runtime
        .repos()
        .activity_cache
        .upsert_activities_synced_at(
            user_id,
            &tenant,
            oauth_providers::GARMIN,
            &[pushed("3001", 4), pushed("3002", 3)],
            received,
        )
        .await
        .unwrap();
    received
}

#[tokio::test]
async fn a_live_read_of_a_push_only_provider_serves_its_pushes_without_restamping_them() {
    let (runtime, user_id, tenant) = runtime_and_user().await;
    let received = seed_pushes(&runtime, user_id, tenant).await;
    let repos = runtime.repos();
    let provider = GarminProvider::new();
    provider.bind_owner_stores(OwnerStores {
        activity_cache: Arc::clone(&repos.activity_cache),
        activity_streams: Arc::clone(&repos.activity_streams),
        user_id,
        tenant_id: tenant,
    });
    let params = ActivityQueryParams {
        after: Some((Utc::now() - Duration::days(30)).timestamp()),
        before: None,
        limit: Some(50),
        offset: None,
    };
    let tenant_str = tenant.to_string();

    let head = read_live_window(
        &runtime,
        &provider,
        LiveRead {
            provider_slug: oauth_providers::GARMIN,
            user_id,
            tenant_id: Some(&tenant_str),
            params: &params,
            attempt_started_at: Utc::now(),
        },
        Some(tenant),
    )
    .await
    .expect("the pushed window is served");

    assert_eq!(head.verdict, HeadVerdict::Complete);
    let ids: Vec<&str> = head.activities.iter().map(Activity::id).collect();
    assert_eq!(ids, ["3002", "3001"], "the pushed rows, newest first");

    let last_sync = repos
        .activity_cache
        .latest_activity_sync(user_id, &tenant, oauth_providers::GARMIN)
        .await
        .unwrap()
        .expect("the pushes recorded a sync");
    assert_eq!(
        last_sync.timestamp(),
        received.timestamp(),
        "freshness is the last push, not the read"
    );
    assert!(
        repos
            .activity_cache
            .latest_activity_fetch_failure(user_id, &tenant, oauth_providers::GARMIN)
            .await
            .unwrap()
            .is_none(),
        "a read of what was pushed records no failed sync"
    );
}

#[tokio::test]
async fn an_empty_push_only_window_is_an_honest_zero_not_a_failed_sync() {
    let (runtime, user_id, tenant) = runtime_and_user().await;
    let repos = runtime.repos();
    let provider = GarminProvider::new();
    provider.bind_owner_stores(OwnerStores {
        activity_cache: Arc::clone(&repos.activity_cache),
        activity_streams: Arc::clone(&repos.activity_streams),
        user_id,
        tenant_id: tenant,
    });
    let params = ActivityQueryParams::with_pagination(Some(20), None);
    let tenant_str = tenant.to_string();

    let head = read_live_window(
        &runtime,
        &provider,
        LiveRead {
            provider_slug: oauth_providers::GARMIN,
            user_id,
            tenant_id: Some(&tenant_str),
            params: &params,
            attempt_started_at: Utc::now(),
        },
        Some(tenant),
    )
    .await
    .expect("nothing pushed yet is still an answer");

    assert!(head.activities.is_empty());
    assert_eq!(head.verdict, HeadVerdict::Complete);
    assert!(repos
        .activity_cache
        .latest_activity_sync(user_id, &tenant, oauth_providers::GARMIN)
        .await
        .unwrap()
        .is_none());
    assert!(repos
        .activity_cache
        .latest_activity_fetch_failure(user_id, &tenant, oauth_providers::GARMIN)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn no_history_capture_is_started_for_a_push_only_provider() {
    let (runtime, user_id, tenant) = runtime_and_user().await;
    let received = seed_pushes(&runtime, user_id, tenant).await;

    let started = spawn_activity_backfill(ActivityBackfillJob {
        resources: Arc::clone(&runtime),
        user_id,
        tenant_id: tenant,
        tenant_id_str: Some(tenant.to_string()),
        provider_name: oauth_providers::GARMIN.to_owned(),
        query_params: ActivityQueryParams {
            after: Some((Utc::now() - Duration::days(365)).timestamp()),
            ..ActivityQueryParams::default()
        },
        pierre_conversation_id: None,
    })
    .await;

    assert!(
        !started,
        "Garmin pushes from the connection on; there is no history to page"
    );
    let repos = runtime.repos();
    assert!(repos
        .activity_cache
        .get_backfill_coverage(user_id, &tenant, oauth_providers::GARMIN)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repos
            .activity_cache
            .latest_activity_sync(user_id, &tenant, oauth_providers::GARMIN)
            .await
            .unwrap()
            .map(|at| at.timestamp()),
        Some(received.timestamp())
    );
}
