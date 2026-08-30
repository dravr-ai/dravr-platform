// ABOUTME: Serving a covered historical window from the durable cache must not re-stamp its freshness
// ABOUTME: Writing read-back rows through moves synced_at to now, which disarms the stale-head top-up

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_activities` write-through fired for every non-empty result, including
//! the rows the historical branch had just READ out of the durable cache. That
//! write changed no data — it only moved their `synced_at` to now.
//!
//! `latest_activity_sync` takes the max of that column, `DataFreshness` reads
//! the result as `Fresh`, and `refresh_stale_head` returns early on `Fresh`. So
//! the one path that tops up the head was disarmed by the act of serving the
//! stale window, and the next ask re-armed the disarming. A capture that stops
//! reports itself current forever.
//!
//! Observed on dev: jf@dravr.ai's sciotte capture froze at 2026-08-28 02:59Z,
//! and two days later all 109 cached rows carried one identical `synced_at`
//! while `activity_fetch_freshness` still held the last real provider fetch,
//! five days older. A 5.3 km trail run recorded in between never landed, and
//! nothing in the system could notice.
//!
//! The architecture note above the branch already states the invariant this
//! pins: the durable cache is "Read ONLY in the historical branch, written
//! through on the recent path."

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::models::{ActivityBuilder, ConnectionType, SportType};
use pierre_database::repositories::BackfillCoverage;
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};

use crate::common::{create_test_server_resources, create_test_user};

/// A deep window whose depth a backfill already confirmed is answered from the
/// durable cache. Those rows came from that table, so serving them must leave
/// the provider-sync clock exactly where it was — otherwise the next turn reads
/// a capture that has not run in days as `Fresh`.
#[tokio::test]
async fn serving_a_covered_window_from_cache_does_not_restamp_its_freshness() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    // sciotte is a scrape-backed mirror backend — the only kind whose deep
    // windows route to the durable cache instead of an inline provider fetch.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    // Two rides well past the 90-day historical threshold, so the ask is
    // unambiguously a deep window.
    let long_ride = ActivityBuilder::new(
        "sciotte-ride-1".to_owned(),
        "Sortie longue".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(200),
        7_200,
        "sciotte".to_owned(),
    )
    .distance_meters(120_000.0)
    .build();
    let trail_run = ActivityBuilder::new(
        "sciotte-run-1".to_owned(),
        "Tourbierer".to_owned(),
        SportType::TrailRunning,
        Utc::now() - Duration::days(210),
        1_712,
        "sciotte".to_owned(),
    )
    .distance_meters(5_308.1)
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "sciotte", &[long_ride, trail_run])
        .await
        .unwrap();

    // A backfill that reached the feed end: the window is covered, so the
    // historical branch serves it from the cache and never calls the provider.
    resources
        .common
        .repos
        .activity_cache
        .upsert_backfill_coverage(
            user_id,
            &tenant,
            "sciotte",
            BackfillCoverage {
                oldest_reached_ts: (Utc::now() - Duration::days(400)).timestamp(),
                hit_feed_end: true,
            },
        )
        .await
        .unwrap();

    let sync_before = resources
        .common
        .repos
        .activity_cache
        .latest_activity_sync(user_id, &tenant, "sciotte")
        .await
        .unwrap()
        .expect("the seeded rows carry a sync timestamp");

    // Enough separation that a re-stamp is unambiguous rather than a sub-millisecond
    // tie the assertion could pass through by accident.
    tokio::time::sleep(StdDuration::from_millis(1_100)).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetActivitiesTool
        .execute(
            &runtime,
            &ctx,
            json!({
                "after": (Utc::now() - Duration::days(300)).timestamp(),
                "limit": 10,
                "mode": "summary"
            }),
        )
        .await;

    let payload = response
        .structured_content
        .expect("tool result carries structured content");
    assert!(
        payload.get("error").is_none(),
        "the covered window must be served, got: {payload}"
    );

    // Guard against a vacuous pass: freshness staying put proves nothing if the
    // serve never happened. The cached rows must actually reach the athlete.
    let activities = payload
        .get("activities")
        .and_then(Value::as_array)
        .expect("activities array present");
    let mut ids: Vec<&str> = activities
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec!["sciotte-ride-1", "sciotte-run-1"],
        "both cached rows must be served from the covered window"
    );
    let run_distance = activities
        .iter()
        .find(|a| a.get("id").and_then(Value::as_str) == Some("sciotte-run-1"))
        .and_then(|a| a.get("distance_meters"))
        .and_then(Value::as_f64)
        .expect("the served run keeps its distance");
    assert!(
        (run_distance - 5_308.1).abs() < 1.0,
        "the served rows are the real cached records, not a placeholder; got {run_distance} m"
    );

    let sync_after = resources
        .common
        .repos
        .activity_cache
        .latest_activity_sync(user_id, &tenant, "sciotte")
        .await
        .unwrap()
        .expect("sync timestamp still present after the serve");

    assert_eq!(
        sync_after, sync_before,
        "serving a window out of the durable cache re-stamped its freshness: the rows \
         came FROM the cache, so the provider-sync clock must not move. Moving it makes \
         DataFreshness report Fresh, which makes refresh_stale_head return early, which \
         is how a capture that stopped days ago keeps reporting itself current."
    );
}
