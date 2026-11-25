// ABOUTME: Local stand-in for the session-read service plus a seeded signed-in session
// ABOUTME: Lets integration tests exercise the live session-read fetch path without Chrome
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::routing::{get, post};
use axum::{Json, Router};
use dravr_tronc::dates::Elapsed;
use dravr_tronc::dates::Timestamp;
use pierre_core::constants::oauth_providers::TOKEN_TYPE_SESSION;
use pierre_core::models::{TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

/// The id of the one canned ride the mock session reader serves.
// Shared across test binaries; any single binary may use only part of it.
#[allow(dead_code)]
pub(crate) const MOCK_SESSION_RIDE_ID: &str = "15551234567";

/// Spawn a local stand-in for the `dravr-sciotte` service: session
/// import always succeeds and the activity list serves one canned ride whose
/// capture reached the list head.
/// Returns the base URL the server's session-read client is built to reach it at.
// Shared across test binaries; any single binary may use only part of it.
#[allow(dead_code)]
pub(crate) async fn spawn_mock_session_reader() -> String {
    spawn_mock_session_reader_serving(
        Arc::new(AtomicBool::new(true)),
        "2026-08-10T12:00:00Z".to_owned(),
    )
    .await
}

/// [`spawn_mock_session_reader`] with the capture's `head_complete` verdict and the
/// ride's start instant under the caller's control.
///
/// `head_complete` is read on every `/api/activities` answer, so one reader
/// can first play a capture whose head the service never saw and then, flipped,
/// one that reached it — the two halves of the write-through gate.
// Shared across test binaries; any single binary may use only part of it.
#[allow(dead_code)]
pub(crate) async fn spawn_mock_session_reader_serving(
    head_complete: Arc<AtomicBool>,
    ride_start: String,
) -> String {
    let ride = json!({
        "id": MOCK_SESSION_RIDE_ID,
        "name": "Sortie vélo matinale",
        "sport_type": "ride",
        "start_date": ride_start,
        "duration_seconds": 2700,
        "provider": "strava",
        "distance_meters": 21000.0,
        "elevation_gain": 250.0
    });
    serve_mock_session_reader(head_complete, ride).await
}

/// [`spawn_mock_session_reader`] serving `activity`, the service's JSON for one
/// activity, as the whole list, its capture having reached the list head.
// Shared across test binaries; any single binary may use only part of it.
#[allow(dead_code)]
pub(crate) async fn spawn_mock_session_reader_with_activity(activity: Value) -> String {
    serve_mock_session_reader(Arc::new(AtomicBool::new(true)), activity).await
}

/// Serve the service's routes on loopback, its activity list holding
/// `activity` alone; the base URL to reach them at.
async fn serve_mock_session_reader(head_complete: Arc<AtomicBool>, activity: Value) -> String {
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
        )
        .route(
            "/api/athlete",
            get(|| async { Json(json!({ "display_name": "Cap Tester" })) }),
        )
        .route(
            "/api/activities",
            get(move || async move {
                Json(json!({
                    "count": 1,
                    "activities": [activity],
                    "head_complete": head_complete.load(Ordering::SeqCst)
                }))
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Seed a live signed-in session the way the hosted login stores it:
/// a `UserOAuthToken` row whose `access_token` is the serialized
/// `AuthSession` (the provider deserializes it in `set_credentials`).
// Shared across test binaries; any single binary may use only part of it.
#[allow(dead_code)]
pub(crate) async fn seed_sciotte_session(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    seed_signed_in_session(
        resources,
        user_id,
        tenant_id,
        "sciotte",
        "cap-verified-session",
    )
    .await;
}

/// [`seed_sciotte_session`] for any session-read backend (`sciotte_garmin`,
/// `sciotte_trainingpeaks`) under a caller-chosen session id.
///
/// The provider sends the stored session's id as `X-Session-Id` on every
/// session read, so a stand-in reader can answer each seeded athlete differently.
// Shared across test binaries; any single binary may use only part of it.
#[allow(dead_code)]
pub(crate) async fn seed_signed_in_session(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    session_id: &str,
) {
    let session_json = json!({
        "session_id": session_id,
        "cookies": [{
            "name": "_strava4_session",
            "value": "test-cookie",
            "domain": ".strava.com",
            "path": "/",
            "secure": true,
            "http_only": true
        }],
        "created_at": Timestamp::now().to_rfc3339(),
        "expires_at": (Timestamp::now() + Elapsed::hours(6)).to_rfc3339(),
    })
    .to_string();

    let mut token = UserOAuthToken::new(
        user_id,
        tenant_id.to_string(),
        backend.to_owned(),
        session_json,
        None,
        Some(Timestamp::now() + Elapsed::hours(6)),
        None,
    );
    TOKEN_TYPE_SESSION.clone_into(&mut token.token_type);
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .expect("upsert signed-in session token");
}
