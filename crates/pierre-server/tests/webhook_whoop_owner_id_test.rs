// ABOUTME: Pins that a WHOOP connection captures its provider-side user id — at token exchange and on refresh
// ABOUTME: Mocks the WHOOP token and profile endpoints; without the id no WHOOP webhook can ever route to a user
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! WHOOP owner-id capture suite (carnet#457).
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
//
// WHOOP's token response carries no owner id, while its webhooks name the
// athlete by that id alone. The OAuth flow now reads `user/profile/basic`
// with the fresh access token and stores the id with the token; the refresh
// path fills a stored token that still lacks it and keeps one that has it.
// Both halves run against a local mock of WHOOP's token and profile
// endpoints through the same seams production leaves unset
// (`PIERRE_WHOOP_TOKEN_URL`, `PIERRE_WHOOP_API_BASE_URL`).
#![cfg(feature = "provider-whoop")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use pierre_core::models::{OAuthClientState, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::oauth_flow::OAuthService;
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use serial_test::serial;
use tokio::net::TcpListener;
use uuid::Uuid;

/// The WHOOP user id the mocked profile reports.
const WHOOP_USER_ID: u64 = 12_345;

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

/// What the WHOOP-shaped mock saw.
#[derive(Default)]
struct MockWhoopApi {
    token_hits: AtomicUsize,
    profile_hits: AtomicUsize,
    /// The `Authorization` header of every profile read.
    profile_bearers: Mutex<Vec<String>>,
}

/// Stand up a mock WHOOP: `POST /oauth/token` answers a token without any
/// owner id (as WHOOP does), `GET /user/profile/basic` answers the profile.
async fn mock_whoop() -> (String, Arc<MockWhoopApi>) {
    let recorder = Arc::new(MockWhoopApi::default());
    let token_recorder = Arc::clone(&recorder);
    let profile_recorder = Arc::clone(&recorder);
    let app = Router::new()
        .route(
            "/oauth/token",
            post(move || {
                let recorder = Arc::clone(&token_recorder);
                async move {
                    recorder.token_hits.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "access_token": "whoop_new_access",
                        "refresh_token": "whoop_new_refresh",
                        "token_type": "Bearer",
                        "expires_in": 3600,
                        "scope": "offline read:sleep read:recovery read:workout"
                    }))
                }
            }),
        )
        .route(
            "/user/profile/basic",
            get(move |headers: HeaderMap| {
                let recorder = Arc::clone(&profile_recorder);
                async move {
                    recorder.profile_hits.fetch_add(1, Ordering::SeqCst);
                    let bearer = headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    recorder.profile_bearers.lock().unwrap().push(bearer);
                    Json(json!({
                        "user_id": WHOOP_USER_ID,
                        "email": "athlete@example.com",
                        "first_name": "Ath",
                        "last_name": "Lete"
                    }))
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

/// A server context whose WHOOP token exchange, refresh and API reads all
/// reach the mock. The registry reads `PIERRE_WHOOP_API_BASE_URL` when the
/// context is built, so the guard is created first.
async fn context_pointed_at(base: &str) -> (Arc<ServerContext>, EnvGuard) {
    let guard = EnvGuard::set(&[
        ("PIERRE_WHOOP_TOKEN_URL", format!("{base}/oauth/token")),
        ("PIERRE_WHOOP_API_BASE_URL", base.to_owned()),
        ("WHOOP_CLIENT_ID", "whoop_client".to_owned()),
        ("WHOOP_CLIENT_SECRET", "whoop_secret".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    (resources, guard)
}

async fn linked_user(resources: &ServerContext, email: &str) -> (Uuid, TenantId) {
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&resources.agent.database, email, "starter")
            .await
            .unwrap();
    (user_id, tenant_id)
}

async fn stored_whoop_token(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
) -> UserOAuthToken {
    resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "whoop")
        .await
        .unwrap()
        .expect("a whoop token row exists")
}

/// A WHOOP code exchange stores the token WITH the owner id read from the
/// profile, using the freshly minted access token.
#[tokio::test]
#[serial]
async fn token_exchange_captures_the_whoop_user_id() {
    let (base, mock) = mock_whoop().await;
    let (resources, _env) = context_pointed_at(&base).await;
    let (user_id, tenant_id) = linked_user(&resources, "whoop-exchange@example.com").await;

    let state = format!("{user_id}:{}", Uuid::new_v4());
    resources
        .common
        .repos
        .oauth_client_state
        .store_oauth_client_state(&OAuthClientState {
            state: state.clone(),
            provider: "whoop".to_owned(),
            user_id: Some(user_id),
            tenant_id: Some(tenant_id.to_string()),
            redirect_uri: "http://localhost:8081/api/oauth/callback/whoop".to_owned(),
            scope: None,
            pkce_code_verifier: None,
            oauth_app_client_id: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(10),
            used: false,
        })
        .await
        .unwrap();

    let service = OAuthService::new(resources.data(), resources.common.config.clone());
    let callback = service
        .handle_callback("auth-code-1", &state, "whoop")
        .await
        .expect("the exchange succeeds against the mock token endpoint");
    assert_eq!(callback.provider, "whoop");

    assert_eq!(
        mock.token_hits.load(Ordering::SeqCst),
        1,
        "one code exchange"
    );
    assert_eq!(
        mock.profile_hits.load(Ordering::SeqCst),
        1,
        "the profile is read once, right after the exchange"
    );
    assert_eq!(
        *mock.profile_bearers.lock().unwrap(),
        vec!["Bearer whoop_new_access".to_owned()],
        "the profile is read with the access token the exchange just minted"
    );

    let stored = stored_whoop_token(&resources, user_id, tenant_id).await;
    assert_eq!(stored.access_token, "whoop_new_access");
    assert_eq!(
        stored.provider_user_id.as_deref(),
        Some("12345"),
        "the WHOOP user id is stored with the token"
    );

    let owner = resources
        .common
        .repos
        .oauth_tokens
        .find_user_by_provider_user_id("whoop", "12345")
        .await
        .unwrap()
        .expect("a WHOOP webhook naming 12345 now resolves to this user");
    assert_eq!(owner, (user_id, tenant_id.to_string()));
}

/// Seed an expired WHOOP token so the next lookup refreshes it.
async fn seed_expired_whoop_token(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    provider_user_id: Option<&str>,
) {
    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: "whoop".to_owned(),
        access_token: "whoop_expired_access".to_owned(),
        refresh_token: Some("whoop_old_refresh".to_owned()),
        token_type: "Bearer".to_owned(),
        expires_at: Some(now - chrono::Duration::hours(1)),
        scope: Some("read:sleep".to_owned()),
        provider_user_id: provider_user_id.map(str::to_owned),
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
}

/// A refresh of a WHOOP token that never captured its owner id (a connection
/// made before the flow read the profile) fills it from the profile and
/// persists it alongside the refreshed tokens.
#[tokio::test]
#[serial]
async fn refresh_fills_a_missing_whoop_user_id() {
    let (base, mock) = mock_whoop().await;
    let (resources, _env) = context_pointed_at(&base).await;
    let (user_id, tenant_id) = linked_user(&resources, "whoop-refresh-fill@example.com").await;
    seed_expired_whoop_token(&resources, user_id, tenant_id, None).await;

    let auth = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>);
    let token = auth
        .get_valid_token(user_id, "whoop", Some(&tenant_id.to_string()))
        .await
        .expect("lookup does not error")
        .expect("the expired token is refreshed");

    assert_eq!(mock.token_hits.load(Ordering::SeqCst), 1, "one refresh");
    assert_eq!(
        mock.profile_hits.load(Ordering::SeqCst),
        1,
        "one profile read"
    );
    assert_eq!(
        *mock.profile_bearers.lock().unwrap(),
        vec!["Bearer whoop_new_access".to_owned()],
        "the profile is read with the refreshed access token, not the expired one"
    );
    assert_eq!(token.access_token, "whoop_new_access");
    assert_eq!(
        token.provider_user_id.as_deref(),
        Some("12345"),
        "the refreshed token reports the captured id"
    );

    let stored = stored_whoop_token(&resources, user_id, tenant_id).await;
    assert_eq!(stored.access_token, "whoop_new_access");
    assert_eq!(stored.refresh_token.as_deref(), Some("whoop_new_refresh"));
    assert_eq!(
        stored.provider_user_id.as_deref(),
        Some("12345"),
        "the id is persisted with the refreshed tokens"
    );
    assert!(stored.expires_at.unwrap() > Utc::now());
}

/// A refresh of a WHOOP token that already carries its owner id keeps it and
/// reads no profile: an id that exists is never lost or re-fetched.
#[tokio::test]
#[serial]
async fn refresh_keeps_an_existing_whoop_user_id_without_a_profile_read() {
    let (base, mock) = mock_whoop().await;
    let (resources, _env) = context_pointed_at(&base).await;
    let (user_id, tenant_id) = linked_user(&resources, "whoop-refresh-keep@example.com").await;
    seed_expired_whoop_token(&resources, user_id, tenant_id, Some("777")).await;

    let auth = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>);
    let token = auth
        .get_valid_token(user_id, "whoop", Some(&tenant_id.to_string()))
        .await
        .unwrap()
        .expect("the expired token is refreshed");

    assert_eq!(mock.token_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        mock.profile_hits.load(Ordering::SeqCst),
        0,
        "an id that exists is not re-fetched"
    );
    assert_eq!(token.provider_user_id.as_deref(), Some("777"));

    let stored = stored_whoop_token(&resources, user_id, tenant_id).await;
    assert_eq!(stored.access_token, "whoop_new_access");
    assert_eq!(
        stored.provider_user_id.as_deref(),
        Some("777"),
        "the refresh keeps the stored id"
    );
}
