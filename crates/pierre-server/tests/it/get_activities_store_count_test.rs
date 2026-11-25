// ABOUTME: get_activities' window total and totals are the store's count over the window it served
// ABOUTME: A row only the store holds counts, one before `after` does not, and a served row the store lacks is named

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One window, one count (carnet#1485, OQ6).
//!
//! `get_activities` used to sum its own page, so the same window carried one
//! count there and another from `query_activities`. Its `coverage.window`
//! `total` and `totals` now come from the store's read of the window it
//! served; `returned` stays the page. A served row the store does not hold
//! is not counted, and the coverage says how many there were.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dravr_tronc::dates::{Elapsed, Timestamp};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::constants::oauth_providers::STRAVA_ARCHIVE;
use pierre_core::models::{Activity, ActivityBuilder, ConnectionType, SportType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::activity_query::{parse_query, query_activities, ReadGate};
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::incomplete_window::incomplete_window_in_payload;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common::{create_test_server_resources_with_sciotte, create_test_user};
use crate::helpers::sciotte_mock::{
    seed_sciotte_session, spawn_mock_session_reader_serving, MOCK_SESSION_RIDE_ID,
};

/// A key no connection of the athlete reads live: rows a former connection
/// left in the store, which only the store read reaches.
const FORMER_KEY: &str = "wahoo";

/// The session reader stand-in serving one ride `age_days` old, with the flag its
/// `head_complete` verdict reads.
async fn session_reader_riding(age_days: i64) -> (Arc<AtomicBool>, String) {
    let head_complete = Arc::new(AtomicBool::new(true));
    let ride_start = (Timestamp::now() - Elapsed::days(age_days)).to_rfc3339();
    let url = spawn_mock_session_reader_serving(Arc::clone(&head_complete), ride_start).await;
    (head_complete, url)
}

/// The session reader stand-in serving one ride two days old.
async fn reader() -> (Arc<AtomicBool>, String) {
    session_reader_riding(2).await
}

/// A fresh athlete on one live session-read connection, an empty store, and the
/// context a tool call runs in.
async fn athlete(resources: &Arc<ServerContext>) -> (Uuid, TenantId, ToolContext) {
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
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    (user_id, tenant, ctx)
}

/// Noon UTC `days` ago: a window's `after`, mid-day so that a row earlier
/// on the same civil day falls outside it.
fn noon_days_ago(days: i64) -> Timestamp {
    (Timestamp::now() - Elapsed::days(days))
        .utc_date()
        .at_midnight()
        .and_utc()
        .unwrap()
        + Elapsed::hours(12)
}

fn week_ago_noon() -> Timestamp {
    noon_days_ago(7)
}

/// A run under `FORMER_KEY` started at `start`, `km` long.
fn former_run(id: &str, start: Timestamp, km: f64) -> Activity {
    ActivityBuilder::new(
        id,
        format!("run {id}"),
        SportType::Run,
        start,
        1_800,
        FORMER_KEY,
    )
    .source(FORMER_KEY)
    .distance_meters(km * 1_000.0)
    .build()
}

/// Every id the store holds for the athlete, under any key.
async fn stored_ids(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<String> {
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
        .expect("read the store")
        .iter()
        .map(|activity| activity.id().to_owned())
        .collect()
}

async fn get_activities(resources: &Arc<ServerContext>, ctx: &ToolContext, args: Value) -> Value {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let response = GetActivitiesTool.execute(&runtime, ctx, args).await;
    let payload = response.structured_content.expect("structured content");
    assert!(
        payload.get("error").is_none(),
        "bug: the window was not served: {payload}"
    );
    payload
}

fn served_ids(payload: &Value) -> Vec<String> {
    payload["activities"].as_array().map_or_default(|rows| {
        rows.iter()
            .filter_map(|row| row["id"].as_str().map(str::to_owned))
            .collect()
    })
}

/// The precondition the fold stands on: a live read the provider vouched
/// for is in the store before `get_activities` answers, so its page and its
/// count agree.
#[tokio::test]
async fn a_complete_live_read_is_stored_before_the_answer_and_counted_whole() {
    let (_head_complete, url) = reader().await;
    let resources = create_test_server_resources_with_sciotte(&url)
        .await
        .unwrap();
    let (user_id, tenant, ctx) = athlete(&resources).await;
    let args = json!({
        "provider": "sciotte",
        "after": week_ago_noon().epoch_seconds(),
        "limit": 10,
        "mode": "summary",
    });

    let payload = get_activities(&resources, &ctx, args).await;

    assert_eq!(
        served_ids(&payload),
        vec![MOCK_SESSION_RIDE_ID],
        "bug: the ride was not served: {payload}"
    );
    assert!(
        stored_ids(&resources, user_id, tenant)
            .await
            .contains(&MOCK_SESSION_RIDE_ID.to_owned()),
        "bug: a served ride the provider vouched for is not in the store when the answer returns"
    );
    let window = &payload["coverage"]["window"];
    assert_eq!(
        window["total"], 1,
        "bug: the page and its count disagree: {payload}"
    );
    assert_eq!(
        window["returned"], 1,
        "bug: returned is not the page: {payload}"
    );
    assert_eq!(
        window["totals"]["count"], 1,
        "bug: totals disagree with total: {payload}"
    );
    assert_eq!(
        window["totals"]["distance_km"], 21.0,
        "bug: the totals are not the stored ride's: {payload}"
    );
    assert!(
        window.get("unstored").is_none(),
        "bug: a stored ride is reported unstored: {payload}"
    );
}

/// The Q3/Q6 failure in small: a row the store holds under a former
/// connection's key is in `query_activities`' count but not on the page. The
/// window's total is the store's, so both answer one count; a row earlier on
/// the same civil day as `after` is outside the window and counts in neither.
#[tokio::test]
async fn the_window_total_is_the_stores_count_not_the_pages() {
    let (_head_complete, url) = reader().await;
    let resources = create_test_server_resources_with_sciotte(&url)
        .await
        .unwrap();
    let (user_id, tenant, ctx) = athlete(&resources).await;
    let after = week_ago_noon();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            user_id,
            &tenant,
            FORMER_KEY,
            &[
                former_run("former-in", after + Elapsed::days(1), 10.0),
                former_run("former-same-day-before", after - Elapsed::hours(1), 5.0),
                former_run("former-older", after - Elapsed::days(3), 7.0),
            ],
        )
        .await
        .unwrap();
    let args = json!({ "after": after.epoch_seconds(), "limit": 10, "mode": "summary" });

    let payload = get_activities(&resources, &ctx, args.clone()).await;

    assert_eq!(
        served_ids(&payload),
        vec![MOCK_SESSION_RIDE_ID],
        "bug: the page is not the live read: {payload}"
    );
    let window = &payload["coverage"]["window"];
    assert_eq!(
        window["total"], 2,
        "bug: the window total is not the store's count of the window: {payload}"
    );
    assert_eq!(
        window["returned"], 1,
        "bug: returned is not the page: {payload}"
    );
    assert_eq!(
        window["totals"]["count"], 2,
        "bug: totals count another set than total: {payload}"
    );
    assert_eq!(
        window["totals"]["distance_km"], 31.0,
        "bug: the totals are not the ride and the run inside the window: {payload}"
    );

    let again = get_activities(&resources, &ctx, args).await;
    assert_eq!(
        again["coverage"]["window"]["total"], 2,
        "bug: a second ask of the window counts it differently: {again}"
    );
}

/// A capture whose head the service never saw is served but, by design, never
/// stored (carnet#149). The count leaves it out and says so, rather than
/// counting a page the store cannot stand behind; the window is incomplete,
/// so a reply quoting its count is re-asked.
#[tokio::test]
async fn a_served_row_the_store_does_not_hold_is_named_unstored() {
    let (head_complete, url) = reader().await;
    let resources = create_test_server_resources_with_sciotte(&url)
        .await
        .unwrap();
    let (user_id, tenant, ctx) = athlete(&resources).await;
    head_complete.store(false, Ordering::SeqCst);
    let args = json!({
        "provider": "sciotte",
        "after": week_ago_noon().epoch_seconds(),
        "limit": 10,
        "mode": "summary",
    });

    let payload = get_activities(&resources, &ctx, args).await;

    assert_eq!(
        served_ids(&payload),
        vec![MOCK_SESSION_RIDE_ID],
        "bug: the headless capture is not served: {payload}"
    );
    assert!(
        stored_ids(&resources, user_id, tenant).await.is_empty(),
        "bug: a headless capture was stored"
    );
    let window = &payload["coverage"]["window"];
    assert_eq!(
        window["total"], 0,
        "bug: the count includes a row the store does not hold: {payload}"
    );
    assert_eq!(
        window["unstored"], 1,
        "bug: the served row the store lacks is not named: {payload}"
    );
    assert!(
        payload["coverage"]["note"]
            .as_str()
            .is_some_and(|note| note.contains("not in the store")),
        "bug: the note does not say a served row is left out of the count: {payload}"
    );
    assert_eq!(
        payload["coverage"]["complete"], false,
        "bug: a window whose count leaves out a served row reads complete: {payload}"
    );
    assert_eq!(
        payload["coverage"]["incomplete_reason"], "unstored_rows",
        "bug: the incomplete reason does not name the unstored rows: {payload}"
    );
    assert!(
        incomplete_window_in_payload("get_activities", &payload),
        "bug: the count guard does not re-ask a reply quoting a count short of the page: {payload}"
    );
}

/// A window answered from the response cache is counted from the store as a
/// live read's is. The ride is older than the detail window, so the hit needs
/// no stored detail; the session is then deleted, so only a hit can answer
/// the second ask at all.
#[tokio::test]
async fn a_window_the_response_cache_answers_is_counted_from_the_store() {
    let (_head_complete, url) = session_reader_riding(20).await;
    let resources = create_test_server_resources_with_sciotte(&url)
        .await
        .unwrap();
    let (user_id, tenant, ctx) = athlete(&resources).await;
    let after = noon_days_ago(30);
    let args = json!({ "after": after.epoch_seconds(), "limit": 10, "mode": "summary" });
    let first = get_activities(&resources, &ctx, args.clone()).await;
    assert_eq!(
        first["coverage"]["window"]["total"], 1,
        "bug: the live read's window is not the stored ride: {first}"
    );

    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            user_id,
            &tenant,
            FORMER_KEY,
            &[former_run("former-in", after + Elapsed::days(2), 10.0)],
        )
        .await
        .unwrap();
    resources
        .common
        .repos
        .oauth_tokens
        .delete_token(user_id, tenant, "sciotte")
        .await
        .unwrap();

    let hit = get_activities(&resources, &ctx, args).await;
    assert_eq!(
        served_ids(&hit),
        vec![MOCK_SESSION_RIDE_ID],
        "bug: the second ask was not the cached page: {hit}"
    );
    assert_eq!(
        hit["coverage"]["complete"], true,
        "bug: the second ask was a stand-in, not the cached page: {hit}"
    );
    let window = &hit["coverage"]["window"];
    assert_eq!(
        window["total"], 2,
        "bug: the cached page's total is its own length, not the store's count: {hit}"
    );
    assert_eq!(
        window["totals"]["distance_km"], 31.0,
        "bug: the cached page's totals are not the store's: {hit}"
    );
    assert_eq!(
        window["returned"], 1,
        "bug: returned is not the page: {hit}"
    );
}

/// Store the athlete's own Strava export of one ride, and a former
/// connection's run inside and one before the window, and return the
/// window's `after`: midnight on the athlete's calendar (UTC: no zone on
/// file), so the instants asked and the civil days `query_activities` reads
/// coincide.
async fn seed_import(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) -> Timestamp {
    let after = noon_days_ago(10) - Elapsed::hours(12);
    let imported = ActivityBuilder::new(
        "archive-ride",
        "imported ride",
        SportType::Ride,
        after + Elapsed::days(3),
        3_600,
        STRAVA_ARCHIVE,
    )
    .source(STRAVA_ARCHIVE)
    .distance_meters(40_000.0)
    .build();
    let cache = &resources.common.repos.activity_cache;
    cache
        .upsert_activities(user_id, &tenant, STRAVA_ARCHIVE, &[imported])
        .await
        .unwrap();
    cache
        .upsert_activities(
            user_id,
            &tenant,
            FORMER_KEY,
            &[
                former_run("former-in", after + Elapsed::days(1), 10.0),
                former_run("former-before", after - Elapsed::hours(6), 5.0),
            ],
        )
        .await
        .unwrap();
    after
}

/// The athlete's own Strava export is served from the store, under its own
/// key: unpinned beside every source, pinned as Strava's companion. Either
/// way the row is counted and is not reported unstored, and the unpinned
/// count is exactly the count `query_activities` gives over the same days.
#[tokio::test]
async fn an_imported_row_is_served_counted_and_stored_and_query_activities_agrees() {
    let (_head_complete, url) = reader().await;
    let resources = create_test_server_resources_with_sciotte(&url)
        .await
        .unwrap();
    let (user_id, tenant, ctx) = athlete(&resources).await;
    let after = seed_import(&resources, user_id, tenant).await;

    let unpinned = get_activities(
        &resources,
        &ctx,
        json!({ "after": after.epoch_seconds(), "limit": 10, "mode": "summary" }),
    )
    .await;
    let mut ids = served_ids(&unpinned);
    ids.sort();
    assert_eq!(
        ids,
        vec![MOCK_SESSION_RIDE_ID.to_owned(), "archive-ride".to_owned()],
        "bug: the unpinned page is not the live ride and the imported one: {unpinned}"
    );
    let window = &unpinned["coverage"]["window"];
    assert!(
        window.get("unstored").is_none(),
        "bug: an imported row read from the store is reported unstored: {unpinned}"
    );
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = parse_query(
        &json!({ "from": after.utc_date().to_string() }),
        Timestamp::now().utc_date(),
    )
    .unwrap();
    let counted = query_activities(&runtime, user_id, tenant, &asked, ReadGate::Model)
        .await
        .unwrap();
    assert_eq!(
        counted.totals.count,
        Some(3),
        "bug: query_activities does not count the ride, the import and the former run"
    );
    assert_eq!(
        window["total"].as_u64(),
        counted.totals.count,
        "bug: get_activities and query_activities count one window apart: {unpinned}"
    );
    assert_eq!(
        window["totals"]["distance_km"].as_f64(),
        counted.totals.distance_km,
        "bug: get_activities and query_activities sum one window apart: {unpinned}"
    );

    let pinned = get_activities(
        &resources,
        &ctx,
        json!({
            "provider": "sciotte",
            "after": after.epoch_seconds(),
            "limit": 10,
            "mode": "summary",
        }),
    )
    .await;
    let window = &pinned["coverage"]["window"];
    assert_eq!(
        window["total"], 2,
        "bug: a pinned window does not count the ride and its own import: {pinned}"
    );
    assert_eq!(
        window["returned"], 2,
        "bug: returned is not the page: {pinned}"
    );
    assert!(
        window.get("unstored").is_none(),
        "bug: a pinned import read from the store is reported unstored: {pinned}"
    );
}

/// A list pinned to one source and a count pinned to the same source read
/// one key set: that source's history keys and the athlete's own imports of
/// it. A pinned `query_activities` used to read the history keys alone, so
/// it counted the live ride where the pinned list counted the ride and its
/// import.
#[tokio::test]
async fn a_pinned_list_and_a_pinned_query_count_one_window() {
    let (_head_complete, url) = reader().await;
    let resources = create_test_server_resources_with_sciotte(&url)
        .await
        .unwrap();
    let (user_id, tenant, ctx) = athlete(&resources).await;
    let after = seed_import(&resources, user_id, tenant).await;

    let pinned = get_activities(
        &resources,
        &ctx,
        json!({
            "provider": "sciotte",
            "after": after.epoch_seconds(),
            "limit": 10,
            "mode": "summary",
        }),
    )
    .await;
    let window = &pinned["coverage"]["window"];
    assert_eq!(
        window["total"], 2,
        "bug: a pinned window does not count the ride and its own import: {pinned}"
    );

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = parse_query(
        &json!({ "from": after.utc_date().to_string(), "provider": "sciotte" }),
        Timestamp::now().utc_date(),
    )
    .unwrap();
    let counted = query_activities(&runtime, user_id, tenant, &asked, ReadGate::Model)
        .await
        .unwrap();
    assert_eq!(
        window["total"].as_u64(),
        counted.totals.count,
        "bug: a pinned list and a pinned query count one source's window apart: {pinned}"
    );
    assert_eq!(
        window["totals"]["distance_km"].as_f64(),
        counted.totals.distance_km,
        "bug: a pinned list and a pinned query sum one source's window apart: {pinned}"
    );
}
