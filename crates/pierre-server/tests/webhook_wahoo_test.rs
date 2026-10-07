// ABOUTME: Pins that a Wahoo workout_summary webhook fetches the owner's workouts into the cache through the OAuth path
// ABOUTME: and that the body's webhook_token gates it; drives /webhooks/wahoo/workouts against a mock Cloud API
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Wahoo webhook route suite (carnet#34).
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(all(feature = "health-sync", feature = "provider-wahoo"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use pierre_core::models::{TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::wahoo_webhook::WAHOO_WEBHOOK_TOKEN_ENV;
use pierre_mcp_server::routes::webhooks::WebhookRoutes;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

/// The Wahoo user id the seeded token carries and the events name.
const WAHOO_USER_ID: i64 = 60_462;

/// The token the portal is configured with.
const PORTAL_TOKEN: &str = "portal-webhook-token-0123456789";

/// The completed workout the mock lists.
const WORKOUT_ID: i64 = 56_519;

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

/// Stand up a mock `GET /v1/workouts` listing one completed workout, and
/// return its base URL plus the hit counter.
async fn mock_wahoo() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let starts = (Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
    let app = Router::new().route(
        "/v1/workouts",
        get(move || {
            let counter = Arc::clone(&counter);
            let starts = starts.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "workouts": [{
                        "id": WORKOUT_ID,
                        "starts": starts,
                        "minutes": 62,
                        "name": "Morning ride",
                        "workout_type_id": 0,
                        "workout_summary": {
                            "id": 8_297,
                            "name": "Morning ride",
                            "distance_accum": "32000.00",
                            "duration_total_accum": "3720.00",
                            "power_avg": "210.00",
                            "power_bike_np_last": "228.00",
                            "power_bike_tss_last": "78.40",
                            "heart_rate_avg": "141.00"
                        }
                    }],
                    "total": 1, "page": 1, "per_page": 50
                }))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}/v1"), hits)
}

/// A server context whose Wahoo provider talks to `api_base`, with the portal
/// token configured when `token` is given. The registry reads
/// `PIERRE_WAHOO_API_BASE_URL` when the context is built.
async fn context_pointed_at(api_base: &str, token: Option<&str>) -> (Arc<ServerContext>, EnvGuard) {
    let mut vars = vec![
        ("PIERRE_WAHOO_API_BASE_URL", api_base.to_owned()),
        ("WAHOO_CLIENT_ID", "test_client".to_owned()),
        ("WAHOO_CLIENT_SECRET", "test_secret".to_owned()),
    ];
    if let Some(token) = token {
        vars.push((WAHOO_WEBHOOK_TOKEN_ENV, token.to_owned()));
    } else {
        env::remove_var(WAHOO_WEBHOOK_TOKEN_ENV);
    }
    let guard = EnvGuard::set(&vars);
    let resources = common::create_test_server_resources().await.unwrap();
    (resources, guard)
}

/// Seed an OAuth-connected Wahoo athlete whose token carries `WAHOO_USER_ID`.
async fn seed_linked_athlete(resources: &ServerContext) -> (Uuid, TenantId) {
    let (user_id, _user, tenant_id) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("wahoo-webhook-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: "wahoo".to_owned(),
        access_token: "wahoo_access_token_0123456789abcdef".to_owned(),
        refresh_token: Some("wahoo_refresh".to_owned()),
        token_type: "Bearer".to_owned(),
        expires_at: Some(now + chrono::Duration::hours(2)),
        scope: Some("user_read workouts_read offline_data".to_owned()),
        provider_user_id: Some(WAHOO_USER_ID.to_string()),
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

fn summary_event(token: &str) -> Value {
    json!({
        "event_type": "workout_summary",
        "webhook_token": token,
        "user": { "id": WAHOO_USER_ID },
        "workout_summary": {
            "id": 8_297,
            "power_avg": "210.00",
            "workout": {
                "id": WORKOUT_ID,
                "starts": (Utc::now() - chrono::Duration::hours(2)).to_rfc3339(),
                "minutes": 62
            }
        }
    })
}

async fn post_body(resources: &Arc<ServerContext>, body: String) -> StatusCode {
    WebhookRoutes::routes(Arc::clone(resources))
        .oneshot(
            Request::builder()
                .method("POST")
                // The path the developer portal is configured with.
                .uri("/webhooks/wahoo/workouts")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
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

async fn cached_wahoo_ids(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &TenantId,
) -> Vec<String> {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            tenant_id,
            Some("wahoo"),
            Utc::now() - chrono::Duration::days(30),
            Utc::now() + chrono::Duration::days(1),
            100,
        )
        .await
        .unwrap()
        .iter()
        .map(|activity| activity.id().to_owned())
        .collect()
}

/// A workout summary for a linked athlete fetches through the OAuth provider
/// into the cache, keyed by the Wahoo workout id, stamps `last_sync`, and
/// tells the athlete's live stream.
#[tokio::test]
#[serial]
async fn a_workout_summary_fetches_the_owners_workouts_into_the_cache() {
    let (api_base, hits) = mock_wahoo().await;
    let (resources, _env) = context_pointed_at(&api_base, Some(PORTAL_TOKEN)).await;
    let (user_id, tenant_id) = seed_linked_athlete(&resources).await;
    let mut sse = resources
        .sse
        .sse_manager
        .register_notification_stream(user_id)
        .await;

    let status = post_body(&resources, summary_event(PORTAL_TOKEN).to_string()).await;
    assert_eq!(status, StatusCode::OK, "Wahoo is acknowledged at once");
    await_spawned_turns(&resources).await;

    assert_eq!(hits.load(Ordering::SeqCst), 1, "one workouts page read");
    assert_eq!(
        cached_wahoo_ids(&resources, user_id, &tenant_id).await,
        vec![WORKOUT_ID.to_string()],
        "the completed workout is written through to the cache under its Wahoo id"
    );
    let last_sync = resources
        .common
        .repos
        .oauth_tokens
        .get_provider_last_sync(user_id, tenant_id, "wahoo")
        .await
        .unwrap()
        .expect("last_sync is stamped after the fetch");
    assert!(Utc::now() - last_sync < chrono::Duration::minutes(1));
    let message = sse.try_recv().expect("the athlete's stream hears of it");
    let payload: Value = serde_json::from_str(message.trim_start_matches("data: ").trim()).unwrap();
    assert_eq!(payload["provider"], "wahoo");
}

/// A body whose token is not the portal's is refused and nothing is fetched.
#[tokio::test]
#[serial]
async fn a_wrong_token_is_refused_and_nothing_is_fetched() {
    let (api_base, hits) = mock_wahoo().await;
    let (resources, _env) = context_pointed_at(&api_base, Some(PORTAL_TOKEN)).await;
    seed_linked_athlete(&resources).await;

    let status = post_body(&resources, summary_event("guessed-token").to_string()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let unsigned = summary_event(PORTAL_TOKEN);
    let mut unsigned = unsigned.as_object().cloned().unwrap();
    unsigned.remove("webhook_token");
    let status = post_body(&resources, Value::Object(unsigned).to_string()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no token is no access");
    await_spawned_turns(&resources).await;
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

/// With no token configured every event is refused with a retryable 503: an
/// empty expected token would otherwise match an empty presented one.
#[tokio::test]
#[serial]
async fn an_unset_token_refuses_every_event() {
    let (api_base, hits) = mock_wahoo().await;
    let (resources, _env) = context_pointed_at(&api_base, None).await;
    seed_linked_athlete(&resources).await;

    let status = post_body(&resources, summary_event("").to_string()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    await_spawned_turns(&resources).await;
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

/// An event that is not a workout summary is acknowledged — a retry would not
/// change it — and nothing is fetched.
#[tokio::test]
#[serial]
async fn another_event_type_is_acknowledged_and_ignored() {
    let (api_base, hits) = mock_wahoo().await;
    let (resources, _env) = context_pointed_at(&api_base, Some(PORTAL_TOKEN)).await;
    seed_linked_athlete(&resources).await;

    let mut event = summary_event(PORTAL_TOKEN);
    event["event_type"] = json!("plan_updated");
    assert_eq!(
        post_body(&resources, event.to_string()).await,
        StatusCode::OK
    );
    await_spawned_turns(&resources).await;
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial]
async fn a_malformed_body_is_a_bad_request() {
    let (api_base, _hits) = mock_wahoo().await;
    let (resources, _env) = context_pointed_at(&api_base, Some(PORTAL_TOKEN)).await;
    assert_eq!(
        post_body(&resources, "not json".to_owned()).await,
        StatusCode::BAD_REQUEST
    );
}
