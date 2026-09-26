// ABOUTME: Pins that disconnecting a sciotte-backed provider drops its session on the sciotte service
// ABOUTME: DELETE /auth/sessions/{id} with the stored session id; a 404 or an unreachable service never blocks it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The platform is the session-of-record for a scrape session, but the shared
//! sciotte service keeps its own copy of the athlete's provider cookies until
//! the session idles out. A disconnect therefore asks the service to drop it
//! (`DELETE /auth/sessions/{id}`, carnet#566), through both disconnect paths:
//! the chokepoint every surface funnels into (`disconnect_provider`) and the
//! backend-pinned `DELETE /api/providers/sciotte/disconnect` route.
//!
//! The service answers `404 session_not_found` for a session it no longer
//! holds, which is the state the disconnect asked for; a service that cannot
//! be reached never blocks the local deletion either.
//!
//! One test, because `DRAVR_SCIOTTE_REMOTE_URL` is process-wide: separate
//! tests would race on it under the parallel harness.

mod common;
mod helpers;

use std::collections::HashSet;
use std::env;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::delete;
use axum::{Json, Router};
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_email};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth_providers::{
    GARMIN, SCIOTTE, SCIOTTE_GARMIN, SCIOTTE_TRAININGPEAKS, STRAVA, TOKEN_TYPE_SESSION,
    TRAININGPEAKS,
};
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{mint_link_token, MintProviderLinkTokenArgs};
use pierre_providers::sciotte_remote::{ENV_AUDIENCE, ENV_REMOTE_URL};
use pierre_routes_auth::AuthRoutes;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::{DisconnectReason, RevocationOutcome};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

/// What the stand-in service holds and every `DELETE` it was sent.
#[derive(Clone, Default)]
struct Scraper {
    held: Arc<Mutex<HashSet<String>>>,
    deletes: Arc<Mutex<Vec<String>>>,
}

impl Scraper {
    fn holding(sessions: &[&str]) -> Self {
        let scraper = Self::default();
        scraper
            .held
            .lock()
            .unwrap()
            .extend(sessions.iter().map(|s| (*s).to_owned()));
        scraper
    }

    fn deletes(&self) -> Vec<String> {
        self.deletes.lock().unwrap().clone()
    }

    fn holds(&self, session: &str) -> bool {
        self.held.lock().unwrap().contains(session)
    }
}

/// The service's `DELETE /auth/sessions/{id}`, answered as dravr-sciotte
/// answers it: `200 removed` for a held session, `404 session_not_found`
/// otherwise.
async fn delete_session(
    State(scraper): State<Scraper>,
    Path(session_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    scraper.deletes.lock().unwrap().push(session_id.clone());
    if scraper.held.lock().unwrap().remove(&session_id) {
        (
            StatusCode::OK,
            Json(json!({ "status": "removed", "session_id": session_id })),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "session_not_found", "session_id": session_id })),
        )
    }
}

async fn spawn_scraper(scraper: Scraper) -> String {
    let app = Router::new()
        .route("/auth/sessions/{id}", delete(delete_session))
        .with_state(scraper);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A loopback URL nothing listens on: the port is bound, read and released.
async fn unreachable_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

async fn user_with_tenant(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    let email = format!("session-drop-{}@example.test", Uuid::new_v4());
    let (user_id, _) = create_test_user_with_email(&resources.agent.database, &email)
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .expect("the test user has a tenant")
        .id;
    (user_id, tenant_id)
}

/// A user connected to `backend` through a scrape session stored as the
/// login route stores it: the serialized session in the token row.
async fn connected_by_session(
    resources: &Arc<ServerContext>,
    backend: &str,
    session_id: &str,
) -> (Uuid, TenantId) {
    let (user_id, tenant_id) = user_with_tenant(resources).await;
    let now = Utc::now();
    let session = json!({
        "session_id": session_id,
        "cookies": [],
        "created_at": now.to_rfc3339(),
    });
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant_id.to_string(),
            provider: backend.to_owned(),
            access_token: session.to_string(),
            refresh_token: None,
            token_type: TOKEN_TYPE_SESSION.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, backend, &ConnectionType::Manual, None)
        .await
        .unwrap();
    (user_id, tenant_id)
}

async fn assert_disconnected(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    let repos = &resources.common.repos;
    assert!(
        repos
            .oauth_tokens
            .get_token(user_id, tenant_id, backend)
            .await
            .unwrap()
            .is_none(),
        "the {backend} session row is deleted"
    );
    let connections = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert!(
        connections.iter().all(|c| c.provider != backend),
        "the {backend} connection row is deleted: {connections:?}"
    );
}

async fn disconnect(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> RevocationOutcome {
    OAuthService::new(resources.data(), resources.common.config.clone())
        .disconnect_provider(
            user_id,
            provider,
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .unwrap_or_else(|e| panic!("disconnecting {provider} succeeds: {e}"))
}

/// A held Strava scrape session is dropped from the service by its id.
async fn a_held_session_is_dropped(resources: &Arc<ServerContext>, scraper: &Scraper) {
    let (user_id, tenant_id) = connected_by_session(resources, SCIOTTE, "sess-strava-held").await;

    let outcome = disconnect(resources, user_id, tenant_id, STRAVA).await;

    assert_eq!(
        scraper.deletes(),
        vec!["sess-strava-held".to_owned()],
        "exactly one DELETE, naming the stored session"
    );
    assert!(
        !scraper.holds("sess-strava-held"),
        "the service no longer holds the cookies"
    );
    assert_eq!(
        outcome,
        RevocationOutcome::NoGrant,
        "a scrape session is no grant at the provider"
    );
    assert_disconnected(resources, user_id, tenant_id, SCIOTTE).await;
}

/// A session the service no longer holds answers 404, and the disconnect
/// still completes.
async fn a_session_already_gone_still_disconnects(
    resources: &Arc<ServerContext>,
    scraper: &Scraper,
) {
    let (user_id, tenant_id) =
        connected_by_session(resources, SCIOTTE_GARMIN, "sess-garmin-idled-out").await;

    disconnect(resources, user_id, tenant_id, GARMIN).await;

    assert_eq!(
        scraper.deletes().last().map(String::as_str),
        Some("sess-garmin-idled-out"),
        "the DELETE names the stored session even when the service lost it"
    );
    assert_disconnected(resources, user_id, tenant_id, SCIOTTE_GARMIN).await;
}

/// The backend-pinned sciotte route drops the session the same way.
async fn the_sciotte_route_drops_the_session(resources: &Arc<ServerContext>, scraper: &Scraper) {
    let (user_id, tenant_id) = connected_by_session(resources, SCIOTTE, "sess-route-held").await;
    let token = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            provider: "sciotte",
            target: "strava",
            channel: "slack",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    )
    .expect("mint a sciotte link token");

    let resp = AxumTestRequest::delete("/api/providers/sciotte/disconnect")
        .header("authorization", &format!("Bearer {token}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;

    let status = resp.status();
    let body = resp.text();
    assert_eq!(status, 204, "{body}");
    assert_eq!(
        scraper.deletes().last().map(String::as_str),
        Some("sess-route-held")
    );
    assert!(!scraper.holds("sess-route-held"));
    assert_disconnected(resources, user_id, tenant_id, SCIOTTE).await;
}

/// A service that cannot be reached never blocks the local deletion: the
/// session's idle lifetime on the service is the backstop.
async fn an_unreachable_service_never_blocks_the_disconnect(resources: &Arc<ServerContext>) {
    env::set_var(ENV_REMOTE_URL, unreachable_url().await);
    let (user_id, tenant_id) =
        connected_by_session(resources, SCIOTTE_TRAININGPEAKS, "sess-tp-unreachable").await;

    disconnect(resources, user_id, tenant_id, TRAININGPEAKS).await;

    assert_disconnected(resources, user_id, tenant_id, SCIOTTE_TRAININGPEAKS).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sciotte_disconnect_drops_the_session_on_the_service() {
    let scraper = Scraper::holding(&["sess-strava-held", "sess-route-held"]);
    env::set_var(ENV_REMOTE_URL, spawn_scraper(scraper.clone()).await);
    env::remove_var(ENV_AUDIENCE);
    let resources = create_test_server_resources().await.unwrap();

    a_held_session_is_dropped(&resources, &scraper).await;
    a_session_already_gone_still_disconnects(&resources, &scraper).await;
    the_sciotte_route_drops_the_session(&resources, &scraper).await;
    an_unreachable_service_never_blocks_the_disconnect(&resources).await;

    env::remove_var(ENV_REMOTE_URL);
}
