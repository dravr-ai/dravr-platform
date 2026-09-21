// ABOUTME: Pins that a Strava activity webhook fetches the owner's activities into the cache through the OAuth path
// ABOUTME: Drives the real /webhooks/strava route against a Strava-shaped mock; a no-op handler fails every assertion
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava webhook route suite (carnet#457).
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
//
// The route used to call enforme's `sync_user`, which is the sciotte TSB
// scraper — an OAuth-connected athlete's activity never reached the cache.
// It now goes through `fetch_provider_head`, the platform's real OAuth
// activity path, so these tests point the Strava provider at a local mock
// (`PIERRE_STRAVA_API_BASE_URL`, the registry seam) and read the
// `cached_activities` table back.
#![cfg(all(feature = "health-sync", feature = "provider-strava"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body};
use axum::extract::Query;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use pierre_core::models::{TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::webhooks::WebhookRoutes;
use serde_json::{json, Value};
use serial_test::serial;
use std::collections::HashMap;
use tokio::net::TcpListener;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

/// The Strava athlete id the seeded token carries and the events name.
const OWNER_ID: u64 = 4_242_424;

/// One week, mirroring the route's lookback: the fetch window must open
/// this far before the event, not at it.
const LOOKBACK_SECS: i64 = 7 * 86_400;

/// Environment set for the duration of one test and removed after it.
struct EnvGuard {
    keys: Vec<&'static str>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, String)]) -> Self {
        for (key, value) in vars {
            env::set_var(key, value);
        }
        Self {
            keys: vars.iter().map(|(key, _)| *key).collect(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            env::remove_var(key);
        }
    }
}

/// What the Strava-shaped mock saw: every `/athlete/activities` query string.
#[derive(Default)]
struct MockStrava {
    hits: AtomicUsize,
    queries: Mutex<Vec<HashMap<String, String>>>,
}

/// Stand up a mock `/athlete/activities` answering one activity and return
/// its base URL plus the recorder.
async fn mock_strava() -> (String, Arc<MockStrava>) {
    let recorder = Arc::new(MockStrava::default());
    let route_recorder = Arc::clone(&recorder);
    let app = Router::new().route(
        "/athlete/activities",
        get(move |Query(query): Query<HashMap<String, String>>| {
            let recorder = Arc::clone(&route_recorder);
            async move {
                recorder.hits.fetch_add(1, Ordering::SeqCst);
                recorder.queries.lock().unwrap().push(query);
                Json(json!([
                    {
                        "id": 9_001,
                        "name": "Lunch Ride",
                        "type": "Ride",
                        "sport_type": "Ride",
                        "start_date": Utc::now().to_rfc3339(),
                        "distance": 32_000.0,
                        "elapsed_time": 4_200,
                        "total_elevation_gain": 310.0
                    }
                ]))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), recorder)
}

/// A server context whose Strava provider talks to `api_base`.
///
/// The registry reads `PIERRE_STRAVA_API_BASE_URL` when the context is
/// built, so the guard must be alive before this call.
async fn context_pointed_at(api_base: &str) -> (Arc<ServerContext>, EnvGuard) {
    let guard = EnvGuard::set(&[
        ("PIERRE_STRAVA_API_BASE_URL", api_base.to_owned()),
        ("STRAVA_CLIENT_ID", "test_client".to_owned()),
        ("STRAVA_CLIENT_SECRET", "test_secret".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    (resources, guard)
}

/// Seed an OAuth-connected Strava athlete whose token carries `OWNER_ID`.
async fn seed_linked_athlete(resources: &ServerContext) -> (Uuid, TenantId) {
    let (user_id, _user, tenant_id) = common::create_test_user_with_plan(
        &resources.agent.database,
        "strava-webhook@example.com",
        "starter",
    )
    .await
    .unwrap();
    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: "strava".to_owned(),
        // >= 40 chars and not "at_"-prefixed: the provider's token validation.
        access_token: "strava_access_token_0123456789abcdef0123456789".to_owned(),
        refresh_token: Some("strava_refresh".to_owned()),
        token_type: "Bearer".to_owned(),
        expires_at: Some(now + chrono::Duration::hours(6)),
        scope: Some("activity:read_all".to_owned()),
        provider_user_id: Some(OWNER_ID.to_string()),
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    };
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();
    (user_id, tenant_id)
}

fn strava_event(aspect_type: &str, owner_id: u64, event_time: i64) -> Value {
    json!({
        "object_type": "activity",
        "object_id": 9_001,
        "aspect_type": aspect_type,
        "owner_id": owner_id,
        "subscription_id": 77,
        "event_time": event_time
    })
}

async fn post_event(resources: &Arc<ServerContext>, event: &Value) -> StatusCode {
    let response = WebhookRoutes::routes(Arc::clone(resources))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhooks/strava")
                .header("content-type", "application/json")
                .body(Body::from(event.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    response.status()
}

/// Wait for every turn the webhook spawned on the drain tracker to finish.
async fn await_spawned_turns(resources: &ServerContext) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !resources.common.turns.is_empty() {
        assert!(
            Instant::now() < deadline,
            "webhook-spawned turn did not finish within 15s"
        );
        sleep(Duration::from_millis(25)).await;
    }
}

async fn cached_strava_rows(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &TenantId,
) -> usize {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            tenant_id,
            Some("strava"),
            Utc::now() - chrono::Duration::days(30),
            Utc::now() + chrono::Duration::days(1),
            100,
        )
        .await
        .unwrap()
        .len()
}

/// An activity-create event for a linked athlete fetches through the OAuth
/// provider into `cached_activities`, stamps `last_sync`, and tells the
/// athlete's SSE stream how many activities landed. A handler that only logs
/// leaves zero rows, no stamp and an empty stream.
#[tokio::test]
#[serial]
async fn activity_create_fetches_the_owners_activities_into_the_cache() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base).await;
    let (user_id, tenant_id) = seed_linked_athlete(&resources).await;
    assert_eq!(
        cached_strava_rows(&resources, user_id, &tenant_id).await,
        0,
        "premise: nothing cached before the webhook"
    );
    let mut sse = resources
        .sse
        .sse_manager
        .register_notification_stream(user_id)
        .await;

    let event_time = Utc::now().timestamp();
    let status = post_event(&resources, &strava_event("create", OWNER_ID, event_time)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Strava must be acknowledged at once"
    );

    await_spawned_turns(&resources).await;

    assert_eq!(
        mock.hits.load(Ordering::SeqCst),
        1,
        "exactly one activities fetch"
    );
    let after: i64 = {
        let queries = mock.queries.lock().unwrap();
        queries[0]["after"]
            .parse()
            .expect("after bound is a unix timestamp")
    };
    assert_eq!(
        after,
        event_time - LOOKBACK_SECS,
        "the window opens one week before the event, so a late upload is inside it"
    );

    assert_eq!(
        cached_strava_rows(&resources, user_id, &tenant_id).await,
        1,
        "the fetched activity is written through to the cache"
    );
    let cached = resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant_id,
            Some("strava"),
            Utc::now() - chrono::Duration::days(30),
            Utc::now() + chrono::Duration::days(1),
            100,
        )
        .await
        .unwrap();
    assert_eq!(cached[0].id(), "9001");
    assert_eq!(cached[0].name(), "Lunch Ride");

    let last_sync = resources
        .common
        .repos
        .oauth_tokens
        .get_provider_last_sync(user_id, tenant_id, "strava")
        .await
        .unwrap()
        .expect("last_sync is stamped after the fetch");
    assert!(
        Utc::now() - last_sync < chrono::Duration::minutes(1),
        "last_sync is the fetch time"
    );

    let freshness = resources
        .common
        .repos
        .activity_cache
        .latest_activity_sync(user_id, &tenant_id, "strava")
        .await
        .unwrap();
    assert!(
        freshness.is_some(),
        "the fetch mark is written with the rows"
    );

    let message = sse
        .try_recv()
        .expect("the athlete's SSE stream carries the sync notification");
    let payload: Value = serde_json::from_str(message.trim_start_matches("data: ").trim()).unwrap();
    assert_eq!(payload["provider"], "strava");
    assert_eq!(payload["success"], true);
    assert_eq!(
        payload["message"], "Strava activity created: 1 recent activities fetched",
        "the notification says what actually happened"
    );
}

/// An owner id no token carries is acknowledged and nothing is fetched or
/// cached — a push event is never broadcast to every connected user.
#[tokio::test]
#[serial]
async fn unknown_owner_is_acknowledged_and_nothing_is_fetched() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base).await;
    let (user_id, tenant_id) = seed_linked_athlete(&resources).await;

    let status = post_event(
        &resources,
        &strava_event("create", OWNER_ID + 1, Utc::now().timestamp()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    await_spawned_turns(&resources).await;

    assert_eq!(
        mock.hits.load(Ordering::SeqCst),
        0,
        "no fetch for an unknown owner"
    );
    assert_eq!(cached_strava_rows(&resources, user_id, &tenant_id).await, 0);
    assert!(
        resources
            .common
            .repos
            .oauth_tokens
            .get_provider_last_sync(user_id, tenant_id, "strava")
            .await
            .unwrap()
            .is_none(),
        "the linked athlete is untouched by someone else's event"
    );
}

/// A delete aspect has nothing to fetch: acknowledged, no provider call, no
/// turn spawned.
#[tokio::test]
#[serial]
async fn delete_aspect_is_acknowledged_without_a_fetch() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base).await;
    let (user_id, tenant_id) = seed_linked_athlete(&resources).await;

    let status = post_event(
        &resources,
        &strava_event("delete", OWNER_ID, Utc::now().timestamp()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        resources.common.turns.len(),
        0,
        "a delete spawns no sync turn"
    );

    assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
    assert_eq!(cached_strava_rows(&resources, user_id, &tenant_id).await, 0);
}

/// A body Strava would never send is refused rather than acknowledged.
#[tokio::test]
#[serial]
async fn malformed_body_is_a_bad_request() {
    let (api_base, _mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base).await;
    let status = post_event(&resources, &json!({"hello": "world"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The subscription verification GET answers Strava's challenge when the
/// verify token matches and refuses it otherwise.
#[tokio::test]
#[serial]
async fn verification_echoes_the_challenge_only_for_the_configured_token() {
    let (api_base, _mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base).await;
    let _token = EnvGuard::set(&[("STRAVA_WEBHOOK_VERIFY_TOKEN", "verify-me".to_owned())]);

    let ok = WebhookRoutes::routes(Arc::clone(&resources))
        .oneshot(
            Request::builder()
                .uri("/webhooks/strava?hub.mode=subscribe&hub.challenge=abc123&hub.verify_token=verify-me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let body = to_bytes(ok.into_body(), 4096).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["hub.challenge"], "abc123");

    let refused = WebhookRoutes::routes(Arc::clone(&resources))
        .oneshot(
            Request::builder()
                .uri("/webhooks/strava?hub.mode=subscribe&hub.challenge=abc123&hub.verify_token=forged")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
}
