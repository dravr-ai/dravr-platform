// ABOUTME: A Strava athlete holding a session beside a live grant reads old windows sciotte first, never the API
// ABOUTME: A walked window is served from the store under any key a walk covered, both keys merged; an unwalked one is read live

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reading the history a walk stored (carnet#1482).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A Strava athlete holding the grant and the usr/pwd session is walked
// through the session, so the walk's depth is recorded under `sciotte`. The
// read plan puts the session first and the grant behind it (carnet#1494),
// and an old window is history, so it keeps the walk's order even while the
// grant is alive (carnet#1482): a covered old window is answered from the
// store without one request to the API, serving the session's rows beside
// the grant's (`history_store_keys`), a ride recorded under both
// keys once; an old window no walk covered goes to the session's background
// backfill, never inline to the API.
//
// The depth is read under every history key of the backend the ask
// resolved to, never that backend's alone: a window a walk covered under
// either key is served from the store when the ask resolves to the session
// and when it resolves to the grant (a session the athlete no longer
// holds), and a grant's uncovered window is still the API's.
//
// Strava is the loopback stand-in of `history_walk_api_test`, which counts
// every listing it answers.
#![cfg(all(
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::helpers::history_walk_ledger::owe_history_walk;
use crate::helpers::sciotte_mock::seed_scrape_session;
use crate::history_walk_api_test::{mock_strava, server, strava_athlete, MockStrava};

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::constants::provider_capture::current_capture_version;
use pierre_core::models::{Activity, ActivityBuilder, ConnectionType, SportType, TenantId};
use pierre_database::repositories::{BackfillCoverage, BackfillJobKind};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

/// A Strava athlete, on a server whose Strava API is the counting stand-in,
/// who also holds a live usr/pwd session the walk went through.
async fn walked_athlete() -> (Arc<ServerContext>, Arc<MockStrava>, Uuid, TenantId) {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    seed_scrape_session(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        "walked-session",
    )
    .await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::SCIOTTE,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    (resources, mock, user_id, tenant)
}

/// A two-hour ride starting at `start`, recorded under `key`.
fn ride(id: &str, start: DateTime<Utc>, key: &str) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        "Sortie longue".to_owned(),
        SportType::Ride,
        start,
        7_200,
        key.to_owned(),
    )
    .distance_meters(60_000.0)
    .build()
}

/// Store `rides` under `key`.
async fn store(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    key: &str,
    rides: &[Activity],
) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, key, rides)
        .await
        .unwrap();
}

/// Record that the walk through the session reached `reached`.
async fn walked_to(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    reached: DateTime<Utc>,
) {
    walked_through(
        resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        reached,
    )
    .await;
}

/// Record that a walk through `key` reached `reached`, under `key` and at
/// its current capture.
async fn walked_through(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    key: &str,
    reached: DateTime<Utc>,
) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_backfill_coverage(
            user_id,
            &tenant,
            key,
            BackfillCoverage {
                oldest_reached_ts: reached.timestamp(),
                hit_feed_end: false,
                capture_version: current_capture_version(key),
            },
        )
        .await
        .unwrap();
}

/// A Strava athlete who holds only the grant now, on a server whose Strava
/// API is the counting stand-in: the session a walk went through is gone,
/// so the ask resolves to the grant.
async fn grant_athlete() -> (Arc<ServerContext>, Arc<MockStrava>, Uuid, TenantId) {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    (resources, mock, user_id, tenant)
}

/// `get_activities` for the closed window `[after, before]`, with no
/// provider argument: the ask resolves to the read plan's primary, the
/// session. A closed window has no head a stale-head top-up could read from
/// the API.
async fn ask_window(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    after: DateTime<Utc>,
    before: DateTime<Utc>,
) -> Value {
    ask_window_in(resources, user_id, tenant, (after, before), "summary").await
}

/// `get_activities` for the closed window `window` in `mode`, as
/// [`ask_window`] asks it.
async fn ask_window_in(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    (after, before): (DateTime<Utc>, DateTime<Utc>),
    mode: &str,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    GetActivitiesTool
        .execute(
            &runtime,
            &ctx,
            json!({
                "after": after.timestamp(),
                "before": before.timestamp(),
                "limit": 20,
                "mode": mode
            }),
        )
        .await
        .structured_content
        .expect("tool result carries structured content")
}

/// The ids a `get_activities` payload served, sorted.
fn served_ids(payload: &Value) -> Vec<String> {
    assert!(
        payload.get("error").is_none(),
        "the window must be served, got: {payload}"
    );
    assert_ne!(
        payload.get("status").and_then(Value::as_str),
        Some("backfilling"),
        "a walked window never waits on a background backfill, got: {payload}"
    );
    let mut ids: Vec<String> = payload
        .get("activities")
        .and_then(Value::as_array)
        .expect("activities array present")
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect();
    ids.sort_unstable();
    ids
}

/// The walk went through the session, so its depth is recorded under
/// `sciotte`; the Strava read of an old window it covered is answered from
/// the store, with no request to the API.
#[tokio::test]
async fn a_window_the_session_walked_is_served_without_the_api() {
    let (resources, mock, user_id, tenant) = walked_athlete().await;
    let now = Utc::now();
    store(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        &[
            ride("15550000001", now - Duration::days(700), "sciotte"),
            ride("15550000002", now - Duration::days(720), "sciotte"),
        ],
    )
    .await;
    // The API holds the same history: a live read would list it.
    mock.serve(vec![
        (now - Duration::days(700)).timestamp(),
        (now - Duration::days(720)).timestamp(),
    ]);
    walked_to(&resources, user_id, tenant, now - Duration::days(1_000)).await;

    let payload = ask_window(
        &resources,
        user_id,
        tenant,
        now - Duration::days(800),
        now - Duration::days(600),
    )
    .await;

    assert_eq!(
        served_ids(&payload),
        vec!["15550000001".to_owned(), "15550000002".to_owned()],
        "bug: the session's walked rows answer the Strava window"
    );
    assert_eq!(
        mock.listing_count(),
        0,
        "bug: a window the session's walk covered was read from the Strava API"
    );
}

/// No walk covered the window: a Strava athlete with a live grant and a
/// session still reads it sciotte first (carnet#1482, overriding the
/// carnet#1494 rule that kept deep windows on the API while a grant is
/// alive): the window goes to the session's background backfill, and the
/// Strava API is never listed.
#[tokio::test]
async fn a_window_no_walk_covered_goes_to_the_session_not_the_api() {
    let (resources, mock, user_id, tenant) = walked_athlete().await;
    let now = Utc::now();
    let start = now - Duration::days(700);
    store(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        &[ride("15550000001", start, "sciotte")],
    )
    .await;
    mock.serve(vec![start.timestamp()]);

    let payload = ask_window(
        &resources,
        user_id,
        tenant,
        now - Duration::days(800),
        now - Duration::days(600),
    )
    .await;

    assert!(
        payload.get("error").is_none(),
        "the window must be answered, got: {payload}"
    );
    assert_eq!(
        payload.get("status").and_then(Value::as_str),
        Some("backfilling"),
        "bug: an uncovered old window goes to the session's backfill, got: {payload}"
    );
    assert_eq!(
        mock.listing_count(),
        0,
        "bug: an uncovered old window was read inline from the Strava API beside a live session"
    );
}

/// The session's walk is running and has not reached the window yet: the
/// store answers with what it holds and says the walk is still filling it
/// (`history_walk_running`, a reason the turn's count guard re-asks on), and
/// nothing else pages the window: no backfill of its own, no API listing.
#[tokio::test]
async fn a_window_the_running_walk_has_not_reached_is_served_as_incomplete() {
    let (resources, mock, user_id, tenant) = walked_athlete().await;
    let now = Utc::now();
    let start = now - Duration::days(700);
    store(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        &[ride("15550000001", start, "sciotte")],
    )
    .await;
    mock.serve(vec![start.timestamp()]);
    let repos = &resources.common.repos;
    owe_history_walk(repos, user_id, tenant, oauth_providers::SCIOTTE).await;

    let payload = ask_window(
        &resources,
        user_id,
        tenant,
        now - Duration::days(800),
        now - Duration::days(600),
    )
    .await;

    assert_eq!(
        served_ids(&payload),
        vec!["15550000001".to_owned()],
        "bug: the store answers what the running walk has landed"
    );
    let coverage = &payload["coverage"];
    assert_eq!(coverage["complete"], false, "bug: {payload}");
    assert_eq!(
        coverage["incomplete_reason"], "history_walk_running",
        "bug: the window must say the walk is still filling it: {payload}"
    );
    assert!(
        coverage["note"]
            .as_str()
            .is_some_and(|note| note.contains("Do not state a count")),
        "bug: an incomplete window forbids stating a count: {payload}"
    );
    assert_eq!(
        repos
            .activity_backfill_jobs
            .backfill_job_state(
                user_id,
                tenant,
                oauth_providers::SCIOTTE,
                BackfillJobKind::Backfill
            )
            .await
            .unwrap(),
        None,
        "bug: a window the walk is filling started a backfill of its own"
    );
    assert_eq!(
        mock.listing_count(),
        0,
        "bug: a window the walk is filling was read from the Strava API"
    );
}

/// The session's stored window serves the grant's rows beside its own, and
/// a ride recorded under both keys is one session.
#[tokio::test]
async fn a_stored_session_window_serves_both_keys_and_merges_twins() {
    let (resources, mock, user_id, tenant) = walked_athlete().await;
    let now = Utc::now();
    let twin_start = now - Duration::days(680);
    store(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        &[
            ride("15550000001", now - Duration::days(700), "sciotte"),
            ride("15550000003", twin_start, "sciotte"),
        ],
    )
    .await;
    store(
        &resources,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        &[
            ride("15550000002", now - Duration::days(720), "strava"),
            ride("15550000003", twin_start, "strava"),
        ],
    )
    .await;
    walked_to(&resources, user_id, tenant, now - Duration::days(1_000)).await;

    // The store holds four recordings in the window: two under each key.
    for key in [oauth_providers::SCIOTTE, oauth_providers::STRAVA] {
        let stored = resources
            .common
            .repos
            .activity_cache
            .get_cached_activities(
                user_id,
                &tenant,
                Some(key),
                now - Duration::days(800),
                now - Duration::days(600),
                50,
            )
            .await
            .unwrap();
        assert_eq!(stored.len(), 2, "two rides are stored under {key}");
    }

    let payload = ask_window(
        &resources,
        user_id,
        tenant,
        now - Duration::days(800),
        now - Duration::days(600),
    )
    .await;

    assert_eq!(
        served_ids(&payload),
        vec![
            "15550000001".to_owned(),
            "15550000002".to_owned(),
            "15550000003".to_owned(),
        ],
        "bug: the Strava window serves the session's rows beside its own, \
         the ride recorded under both keys once"
    );
    assert_eq!(
        mock.listing_count(),
        0,
        "bug: a covered window was read from the Strava API"
    );
}

/// A walk through the grant, before the athlete signed a session in,
/// recorded its depth under `strava`; the session's read of an old window
/// that walk covered is answered from the store, never sent to the
/// session's background backfill.
#[tokio::test]
async fn a_window_the_api_walked_is_served_to_the_session_from_the_store() {
    let (resources, mock, user_id, tenant) = walked_athlete().await;
    let now = Utc::now();
    store(
        &resources,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        &[
            ride("15550000001", now - Duration::days(700), "strava"),
            ride("15550000002", now - Duration::days(720), "strava"),
        ],
    )
    .await;
    mock.serve(vec![
        (now - Duration::days(700)).timestamp(),
        (now - Duration::days(720)).timestamp(),
    ]);
    walked_through(
        &resources,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        now - Duration::days(1_000),
    )
    .await;

    let payload = ask_window(
        &resources,
        user_id,
        tenant,
        now - Duration::days(800),
        now - Duration::days(600),
    )
    .await;

    assert_eq!(
        served_ids(&payload),
        vec!["15550000001".to_owned(), "15550000002".to_owned()],
        "bug: the grant's walked rows answer the session's window"
    );
    assert_eq!(
        mock.listing_count(),
        0,
        "bug: a window the grant's walk covered was read from the Strava API"
    );
}

/// The walk went through a session the athlete no longer holds, so its
/// depth is under `sciotte` and the ask resolves to the grant: an old
/// window that walk covered is answered from the store, with no request to
/// the Strava API.
#[tokio::test]
async fn a_window_the_session_walked_is_served_to_the_grant_from_the_store() {
    let (resources, mock, user_id, tenant) = grant_athlete().await;
    let now = Utc::now();
    store(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        &[
            ride("15550000001", now - Duration::days(700), "sciotte"),
            ride("15550000002", now - Duration::days(720), "sciotte"),
        ],
    )
    .await;
    mock.serve(vec![
        (now - Duration::days(700)).timestamp(),
        (now - Duration::days(720)).timestamp(),
    ]);
    walked_to(&resources, user_id, tenant, now - Duration::days(1_000)).await;

    let payload = ask_window(
        &resources,
        user_id,
        tenant,
        now - Duration::days(800),
        now - Duration::days(600),
    )
    .await;

    assert_eq!(
        served_ids(&payload),
        vec!["15550000001".to_owned(), "15550000002".to_owned()],
        "bug: the session's walked rows answer the grant's window"
    );
    assert_eq!(
        mock.listing_count(),
        0,
        "bug: a window the session's walk covered was read from the Strava API"
    );
}

/// The grant's stored window serves the rows the session stored beside its
/// own (`history_store_keys(strava)`), and a ride recorded under both keys is one
/// session.
#[tokio::test]
async fn a_stored_strava_window_serves_the_session_rows_and_merges_twins() {
    let (resources, mock, user_id, tenant) = grant_athlete().await;
    let now = Utc::now();
    let twin_start = now - Duration::days(680);
    store(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        &[
            ride("15550000001", now - Duration::days(700), "sciotte"),
            ride("15550000003", twin_start, "sciotte"),
        ],
    )
    .await;
    store(
        &resources,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        &[
            ride("15550000002", now - Duration::days(720), "strava"),
            ride("15550000003", twin_start, "strava"),
        ],
    )
    .await;
    walked_to(&resources, user_id, tenant, now - Duration::days(1_000)).await;

    let payload = ask_window(
        &resources,
        user_id,
        tenant,
        now - Duration::days(800),
        now - Duration::days(600),
    )
    .await;

    assert_eq!(
        served_ids(&payload),
        vec![
            "15550000001".to_owned(),
            "15550000002".to_owned(),
            "15550000003".to_owned(),
        ],
        "bug: the Strava window serves the session's rows beside its own, \
         the ride recorded under both keys once"
    );
    assert_eq!(
        mock.listing_count(),
        0,
        "bug: a covered window was read from the Strava API"
    );
}

/// No walk covered the window and no session stands beside the grant: the
/// old window is still read inline from the Strava API, as any grant's
/// window is.
#[tokio::test]
async fn a_window_no_walk_covered_is_read_from_the_api_by_the_grant() {
    let (resources, mock, user_id, tenant) = grant_athlete().await;
    let now = Utc::now();
    let start = now - Duration::days(700);
    mock.serve(vec![start.timestamp()]);

    let payload = ask_window(
        &resources,
        user_id,
        tenant,
        now - Duration::days(800),
        now - Duration::days(600),
    )
    .await;

    assert_eq!(
        served_ids(&payload),
        vec![start.timestamp().to_string()],
        "the grant's uncovered window is the API's answer"
    );
    assert!(
        mock.listing_count() > 0,
        "bug: an uncovered old window of a grant alone was never read from the Strava API"
    );
}

/// A grant serving an old window from the store still details its own rows
/// as its live read does: a `mode=detailed` ask of a window the grant's walk
/// covered lists nothing from the Strava API and reads the served ride's
/// detail from it. A row the session stored is the session's to detail.
#[tokio::test]
async fn a_detailed_ask_of_a_walked_window_still_details_through_the_grant() {
    let (resources, mock, user_id, tenant) = grant_athlete().await;
    let now = Utc::now();
    let start = now - Duration::days(700);
    let id = start.timestamp().to_string();
    store(
        &resources,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        &[ride(&id, start, "strava")],
    )
    .await;
    walked_through(
        &resources,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        now - Duration::days(1_000),
    )
    .await;

    let payload = ask_window_in(
        &resources,
        user_id,
        tenant,
        (now - Duration::days(800), now - Duration::days(600)),
        "detailed",
    )
    .await;

    assert_eq!(served_ids(&payload), vec![id], "the walked ride is served");
    assert_eq!(
        mock.listing_count(),
        0,
        "bug: a window the grant's walk covered was listed from the Strava API"
    );
    assert!(
        mock.detail_count() > 0,
        "bug: a detailed ask served from the store read no detail through the grant"
    );
}
