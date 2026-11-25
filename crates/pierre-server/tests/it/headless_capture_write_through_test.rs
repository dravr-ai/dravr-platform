// ABOUTME: A session-read capture whose head the service never saw is served but never written to the
// ABOUTME: activity cache on any of the three write-through paths; a complete capture is persisted
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The activity-cache write-through gate, end to end (carnet#149, carnet#151).
//!
//! The session read's Strava walk carries every complete week and tops it up with one
//! best-effort fetch of the in-progress week; when that fetch fails the
//! capture reports `head_complete: false`. Writing such a capture through
//! moves every row's `synced_at` to now and stamps the fetch mark, so
//! `latest_activity_sync` reads Fresh, `refresh_stale_head` stands down for
//! hours, and the week the capture missed is served as a quiet one.
//!
//! Three paths write a fetched window through, and each must decline a
//! headless capture: `get_activities`' inline recent-window fetch, the group
//! snapshot loop, and the inline historical backfill. Every test drives the
//! real `SciotteProvider` against a local session-reader stand-in whose
//! `head_complete` flag it flips, and asserts the cache's content — empty
//! while the head is missing, holding the ride once it is not.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dravr_tronc::dates::{Elapsed, Timestamp};
use dravr_tronc::mcp::tasks::TASKS_EXTENSION_ID;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::models::{Activity, ActivityBuilder, ConnectionType, SportType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_fetch::fetch_provider_head;
use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common::{create_test_server_resources_with_sciotte, create_test_user};
use crate::helpers::sciotte_mock::{
    seed_sciotte_session, spawn_mock_session_reader_serving, MOCK_SESSION_RIDE_ID,
};

/// Spawn this test's session-reader stand-in and hand back the `head_complete` flag
/// its activity list reads on every answer, with the URL it serves at. A
/// stand-in per test rather than one for the binary: the stand-in lives on the
/// runtime that spawned it, and every `#[tokio::test]` gets a runtime of its
/// own.
async fn session_reader_head_flag() -> (Arc<AtomicBool>, String) {
    let head_complete = Arc::new(AtomicBool::new(true));
    // Two days old: inside every window the three paths read, and inside the
    // retention prune the write-through applies.
    let ride_start = (Timestamp::now() - Elapsed::days(2)).to_rfc3339();
    let url = spawn_mock_session_reader_serving(Arc::clone(&head_complete), ride_start).await;
    (head_complete, url)
}

/// A fresh athlete with one live session-read (Strava mirror) connection and an
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
/// wide enough that nothing the session reader serves can fall outside it.
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
            Timestamp::now() - Elapsed::days(400),
            Timestamp::now() + Elapsed::days(1),
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
        vec![MOCK_SESSION_RIDE_ID],
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
        .map_or_default(|a| {
            a.iter()
                .filter_map(|x| x.get("id").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
}

/// `get_activities` with no window: the inline recent fetch in
/// `implementations/data.rs`.
#[tokio::test]
async fn recent_window_declines_a_headless_capture_and_persists_a_complete_one() {
    let (head_complete, reader_url) = session_reader_head_flag().await;
    let resources = create_test_server_resources_with_sciotte(&reader_url)
        .await
        .unwrap();
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
        vec![MOCK_SESSION_RIDE_ID],
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
    let (head_complete, reader_url) = session_reader_head_flag().await;
    let resources = create_test_server_resources_with_sciotte(&reader_url)
        .await
        .unwrap();
    let (user_id, tenant) = athlete_on_sciotte(&resources).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    head_complete.store(false, Ordering::SeqCst);
    let snapshots = fetch_member_snapshots(&runtime, &[user_id], tenant).await;
    assert_eq!(snapshots.len(), 1);
    assert!(
        !snapshots[0].recent_activities.is_empty(),
        "the headless capture still feeds the snapshot's sessions"
    );
    // The weekly figures are the stored Monday weeks Home counts: a capture
    // never stored gives the week no number, never a zero.
    assert_eq!(
        snapshots[0].weekly_volume_km, None,
        "an unstored week read as a figure"
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
    // Stored, the ride two days back is in this Monday week or the one
    // before, and both weeks it reaches carry their figures.
    let snapshot = &snapshots[0];
    let sessions =
        snapshot.weekly_volume_km.unwrap_or(0.0) + snapshot.previous_week_volume_km.unwrap_or(0.0);
    assert!(
        sessions > 0.0,
        "the stored ride is in neither Monday week: {:?} / {:?}",
        snapshot.weekly_volume_km,
        snapshot.previous_week_volume_km
    );
}

/// The inline historical backfill in `activity_backfill.rs`: an `after`
/// deeper than the backfill threshold on a mirror backend, from a client that
/// declares the tasks extension so the backfill runs on the caller's future.
#[tokio::test]
async fn historical_backfill_declines_a_headless_capture_and_persists_a_complete_one() {
    let (head_complete, reader_url) = session_reader_head_flag().await;
    let resources = create_test_server_resources_with_sciotte(&reader_url)
        .await
        .unwrap();
    let (user_id, tenant) = athlete_on_sciotte(&resources).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer")
        .with_client_capabilities(json!({ "extensions": { TASKS_EXTENSION_ID: {} } }));
    let after = (Timestamp::now() - Elapsed::days(200)).epoch_seconds();
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

/// One cached run under `provider`, started `age_days` ago.
fn old_run(id: &str, provider: &str, age_days: i64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("run {id}"),
        SportType::Run,
        Timestamp::now() - Elapsed::days(age_days),
        3_600,
        provider,
    )
    .source(provider)
    .build()
}

/// A write-through prunes nothing: retention is the one sweep's, by
/// provider, and it starts at no expiry. A Strava (usr/pwd) sync keeps its
/// own old rows and never touches the Garmin API's; Garmin pushes each
/// activity once, so a Garmin row a write took would be gone for good.
#[tokio::test]
async fn a_strava_write_through_prunes_no_old_row() {
    let (_head_complete, reader_url) = session_reader_head_flag().await;
    let resources = create_test_server_resources_with_sciotte(&reader_url)
        .await
        .unwrap();
    let (user_id, tenant) = athlete_on_sciotte(&resources).await;
    let cache = &resources.common.repos.activity_cache;
    let years_ago = 3 * 365;
    cache
        .upsert_activities(
            user_id,
            &tenant,
            "garmin",
            &[old_run("garmin-old", "garmin", years_ago)],
        )
        .await
        .unwrap();
    cache
        .upsert_activities(
            user_id,
            &tenant,
            "sciotte",
            &[old_run("strava-old", "sciotte", years_ago)],
        )
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let params = ActivityQueryParams {
        limit: Some(10),
        offset: None,
        before: None,
        after: Some((Timestamp::now() - Elapsed::days(7)).epoch_seconds()),
    };
    fetch_provider_head(&runtime, "sciotte", user_id, &tenant.to_string(), &params)
        .await
        .expect("the Strava head is read and written through");

    let held = |provider: &'static str, id: &'static str| {
        let cache = Arc::clone(cache);
        async move {
            cache
                .get_cached_activity(user_id, &tenant, provider, id)
                .await
                .unwrap()
                .is_some()
        }
    };
    assert!(
        held("sciotte", MOCK_SESSION_RIDE_ID).await,
        "the write-through landed"
    );
    assert!(
        held("sciotte", "strava-old").await,
        "bug: a write-through pruned the written provider's own old row"
    );
    assert!(
        held("garmin", "garmin-old").await,
        "bug: a Strava write-through must never prune a Garmin API row"
    );
}
