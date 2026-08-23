// ABOUTME: Local stand-in for the dravr-sciotte scraper plus a seeded scrape session
// ABOUTME: Lets integration tests exercise the live sciotte fetch path without Chrome
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use pierre_core::constants::oauth_providers::TOKEN_TYPE_SESSION;
use pierre_core::models::{TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::json;
use tokio::net::TcpListener;
use uuid::Uuid;

/// Spawn a local stand-in for the `dravr-sciotte` scraper service: session
/// import always succeeds and the activity list serves one canned ride.
/// Returns the base URL for `DRAVR_SCIOTTE_REMOTE_URL`.
// Shared across test binaries; any single binary may use only part of it.
#[allow(dead_code)]
pub async fn spawn_mock_scraper() -> String {
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
            get(|| async {
                Json(json!({
                    "count": 1,
                    "activities": [{
                        "id": "15551234567",
                        "name": "Sortie vélo matinale",
                        "sport_type": "ride",
                        "start_date": "2026-08-10T12:00:00Z",
                        "duration_seconds": 2700,
                        "provider": "strava",
                        "distance_meters": 21000.0,
                        "elevation_gain": 250.0
                    }]
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

/// Seed a live sciotte scrape session the way the hosted login stores it:
/// a `UserOAuthToken` row whose `access_token` is the serialized
/// `AuthSession` (the provider deserializes it in `set_credentials`).
// Shared across test binaries; any single binary may use only part of it.
#[allow(dead_code)]
pub async fn seed_sciotte_session(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    let session_json = json!({
        "session_id": "cap-verified-session",
        "cookies": [{
            "name": "_strava4_session",
            "value": "test-cookie",
            "domain": ".strava.com",
            "path": "/",
            "secure": true,
            "http_only": true
        }],
        "created_at": Utc::now().to_rfc3339(),
        "expires_at": (Utc::now() + chrono::Duration::hours(6)).to_rfc3339(),
    })
    .to_string();

    let mut token = UserOAuthToken::new(
        user_id,
        tenant_id.to_string(),
        "sciotte".to_owned(),
        session_json,
        None,
        Some(Utc::now() + chrono::Duration::hours(6)),
        None,
    );
    TOKEN_TYPE_SESSION.clone_into(&mut token.token_type);
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .expect("upsert sciotte session token");
}
