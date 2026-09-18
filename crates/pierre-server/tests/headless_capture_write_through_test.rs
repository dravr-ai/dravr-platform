// ABOUTME: A sciotte capture whose head the scraper never saw is served but never written to the
// ABOUTME: activity cache on any of the three write-through paths; a complete capture is persisted
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The activity-cache write-through gate, end to end (carnet#149, carnet#151).
//!
//! sciotte's Strava walk carries every complete week and tops it up with one
//! best-effort fetch of the in-progress week; when that fetch fails the
//! capture reports `head_complete: false`. Writing such a capture through
//! moves every row's `synced_at` to now and stamps the fetch mark, so
//! `latest_activity_sync` reads Fresh, `refresh_stale_head` stands down for
//! hours, and the week the capture missed is served as a quiet one.
//!
//! Three paths write a fetched window through, and each must decline a
//! headless capture: `get_activities`' inline recent-window fetch, the group
//! snapshot loop, and the inline historical backfill. Every test drives the
//! real `SciotteProvider` against a local scraper stand-in whose
//! `head_complete` flag it flips, and asserts the cache's content — empty
//! while the head is missing, holding the ride once it is not.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{Duration, Utc};
use dravr_tronc::mcp::tasks::TASKS_EXTENSION_ID;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::models::{Activity, ConnectionType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};
use crate::helpers::sciotte_mock::{
    seed_sciotte_session, spawn_mock_scraper_serving, MOCK_SCRAPER_RIDE_ID,
};

/// `DRAVR_SCIOTTE_REMOTE_URL` is process-wide and each test points it at its
/// own scraper stand-in, so the tests run one at a time. A stand-in per test
/// rather than one for the binary: the stand-in lives on the runtime that
/// spawned it, and every `#[tokio::test]` gets a runtime of its own.
static SERIAL: Mutex<()> = Mutex::const_new(());

/// Spawn this test's scraper stand-in, point the provider at it, and hand back
/// the `head_complete` flag its activity list reads on every answer.
async fn scraper_head_flag() -> Arc<AtomicBool> {
    let head_complete = Arc::new(AtomicBool::new(true));
    // Two days old: inside every window the three paths read, and inside the
    // retention prune the write-through applies.
    let ride_start = (Utc::now() - Duration::days(2)).to_rfc3339();
    let url = spawn_mock_scraper_serving(Arc::clone(&head_complete), ride_start).await;
    env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &url);
    // The remote client is both-or-neither: a URL with no audience disables
    // it, because unsigned requests are refused by the scraper rather than
    // served.
    env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-test");
    head_complete
}

/// A fresh athlete with one live sciotte (Strava mirror) connection and an
/// empty activity cache.
async fn athlete_on_sciotte(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenants = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::Manual, None)
        .await
        .unwrap();
    seed_sciotte_session(resources, user_id, tenant).await;
    (user_id, tenant)
}

/// Every row the cache holds for the athlete, any provider, over a window
/// wide enough that nothing the scraper serves can fall outside it.
async fn cached_rows(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<Activity> {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant,
            None,
            Utc::now() - Duration::days(400),
            Utc::now() + Duration::days(1),
            100,
        )
        .await
        .expect("read activity cache")
}

async fn latest_sync(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) -> bool {
    resources
        .common
        .repos
        .activity_cache
        .latest_activity_sync(user_id, &tenant, "sciotte")
        .await
        .expect("read freshness")
        .is_some()
}

fn assert_cache_empty(rows: &[Activity], fresh: bool, path: &str) {
    assert!(
        rows.is_empty(),
        "{path}: a capture missing its head must not be written through, cache holds {:?}",
        rows.iter().map(Activity::id).collect::<Vec<_>>()
    );
    assert!(
        !fresh,
        "{path}: a capture missing its head must not stamp the fetch as fresh"
    );
}

fn assert_cache_holds_ride(rows: &[Activity], fresh: bool, path: &str) {
    let ids: Vec<&str> = rows.iter().map(Activity::id).collect();
    assert_eq!(
        ids,
        vec![MOCK_SCRAPER_RIDE_ID],
        "{path}: a complete capture is written through, cache holds {ids:?}"
    );
    assert!(
        fresh,
        "{path}: a complete capture stamps the fetch as fresh"
    );
}

fn served_ids(payload: &Value) -> Vec<String> {
    payload
        .get("activities")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("id").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `get_activities` with no window: the inline recent fetch in
/// `implementations/data.rs`.
#[tokio::test]
async fn recent_window_declines_a_headless_capture_and_persists_a_complete_one() {
    let _serial = SERIAL.lock().await;
    let head_complete = scraper_head_flag().await;
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_on_sciotte(&resources).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let args = json!({ "provider": "sciotte", "limit": 10, "mode": "summary" });

    head_complete.store(false, Ordering::SeqCst);
    let response = GetActivitiesTool.execute(&runtime, &ctx, args).await;
    let payload = response.structured_content.expect("structured content");
    assert!(
        payload.get("error").is_none(),
        "the headless capture is still served: {payload}"
    );
    assert_eq!(
        served_ids(&payload),
        vec![MOCK_SCRAPER_RIDE_ID],
        "the headless capture is served to the athlete, just not persisted"
    );
    let rows = cached_rows(&resources, user_id, tenant).await;
    let fresh = latest_sync(&resources, user_id, tenant).await;
    assert_cache_empty(&rows, fresh, "recent window");

    head_complete.store(true, Ordering::SeqCst);
    // A different page size, so the TTL'd response cache — keyed on the
    // window, and warmed by the first answer — does not serve this ask and
    // the provider is fetched again.
    let args = json!({ "provider": "sciotte", "limit": 20, "mode": "summary" });
    let response = GetActivitiesTool.execute(&runtime, &ctx, args).await;
    let payload = response.structured_content.expect("structured content");
    assert!(
        payload.get("error").is_none(),
        "the complete capture is served: {payload}"
    );
    let rows = cached_rows(&resources, user_id, tenant).await;
    let fresh = latest_sync(&resources, user_id, tenant).await;
    assert_cache_holds_ride(&rows, fresh, "recent window");
}

/// The group snapshot loop in `group_fitness.rs`, reached through
/// `fetch_member_snapshots` on a member whose cache is empty.
#[tokio::test]
async fn group_snapshot_declines_a_headless_capture_and_persists_a_complete_one() {
    let _serial = SERIAL.lock().await;
    let head_complete = scraper_head_flag().await;
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_on_sciotte(&resources).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    head_complete.store(false, Ordering::SeqCst);
    let snapshots = fetch_member_snapshots(&runtime, &[user_id], tenant).await;
    assert_eq!(snapshots.len(), 1);
    assert!(
        snapshots[0].weekly_activity_count >= 1,
        "the headless capture still feeds the snapshot, got weekly_activity_count={}",
        snapshots[0].weekly_activity_count
    );
    let rows = cached_rows(&resources, user_id, tenant).await;
    let fresh = latest_sync(&resources, user_id, tenant).await;
    assert_cache_empty(&rows, fresh, "group snapshot");

    head_complete.store(true, Ordering::SeqCst);
    let snapshots = fetch_member_snapshots(&runtime, &[user_id], tenant).await;
    assert_eq!(snapshots.len(), 1);
    let rows = cached_rows(&resources, user_id, tenant).await;
    let fresh = latest_sync(&resources, user_id, tenant).await;
    assert_cache_holds_ride(&rows, fresh, "group snapshot");
}

/// The inline historical backfill in `activity_backfill.rs`: an `after`
/// deeper than the backfill threshold on a mirror backend, from a client that
/// declares the tasks extension so the backfill runs on the caller's future.
#[tokio::test]
async fn historical_backfill_declines_a_headless_capture_and_persists_a_complete_one() {
    let _serial = SERIAL.lock().await;
    let head_complete = scraper_head_flag().await;
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_on_sciotte(&resources).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer")
        .with_client_capabilities(json!({ "extensions": { TASKS_EXTENSION_ID: {} } }));
    let after = (Utc::now() - Duration::days(200)).timestamp();
    let args = json!({ "provider": "sciotte", "after": after, "limit": 10, "mode": "summary" });

    head_complete.store(false, Ordering::SeqCst);
    let response = GetActivitiesTool
        .execute(&runtime, &ctx, args.clone())
        .await;
    let payload = response.structured_content.expect("structured content");
    let error = payload
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        error.contains("try again shortly"),
        "a headless backfill is a failed one, so the next ask re-runs it; got {payload}"
    );
    let rows = cached_rows(&resources, user_id, tenant).await;
    let fresh = latest_sync(&resources, user_id, tenant).await;
    assert_cache_empty(&rows, fresh, "historical backfill");
    let coverage = resources
        .common
        .repos
        .activity_cache
        .get_backfill_coverage(user_id, &tenant, "sciotte")
        .await
        .expect("read coverage");
    assert!(
        coverage.is_none(),
        "a headless backfill must not record coverage, got {coverage:?}"
    );

    head_complete.store(true, Ordering::SeqCst);
    let response = GetActivitiesTool.execute(&runtime, &ctx, args).await;
    let payload = response.structured_content.expect("structured content");
    assert!(
        payload.get("error").is_none(),
        "the complete backfill is served: {payload}"
    );
    let rows = cached_rows(&resources, user_id, tenant).await;
    let fresh = latest_sync(&resources, user_id, tenant).await;
    assert_cache_holds_ride(&rows, fresh, "historical backfill");
    let coverage = resources
        .common
        .repos
        .activity_cache
        .get_backfill_coverage(user_id, &tenant, "sciotte")
        .await
        .expect("read coverage");
    assert!(
        coverage.is_some_and(|c| c.oldest_reached_ts <= after),
        "a complete backfill records the requested floor as covered"
    );
}
