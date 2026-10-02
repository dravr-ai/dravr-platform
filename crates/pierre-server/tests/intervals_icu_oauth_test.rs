// ABOUTME: Pins the Intervals.icu OAuth link end to end — code exchange, stored token, bearer reads, revocation
// ABOUTME: Mocks the Intervals.icu token, athlete and disconnect-app endpoints; an API-key link stays as it was
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Intervals.icu OAuth link suite (carnet#47).
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
//
// Intervals.icu answers a code exchange with a bearer token that never expires,
// no refresh token, and the athlete inline with a *string* id. Each of those
// broke the shared flow before: the numeric-only athlete id failed the whole
// token response, a missing `expires_in` was stored as an hour, and the
// provider sent every token as an HTTP Basic API key. These run the real flow
// against a local mock through the `PIERRE_INTERVALS_ICU_*` seams.
#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::{Arc, Mutex};

use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Form, Json, Router};
use chrono::Utc;
use pierre_core::models::{
    ConnectionType, OAuthClientState, TenantId, UserOAuthToken, API_KEY_TOKEN_TYPE,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::{DisconnectReason, RevocationOutcome};
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use serial_test::serial;
use std::collections::HashMap;
use tokio::net::TcpListener;
use uuid::Uuid;

const PROVIDER: &str = "intervals_icu";

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

/// What the Intervals.icu-shaped mock saw, one line per request:
/// `"<route> <authorization header>"`.
#[derive(Default)]
struct MockIntervals {
    requests: Mutex<Vec<String>>,
    /// The form fields of every code exchange.
    exchanges: Mutex<Vec<HashMap<String, String>>>,
}

impl MockIntervals {
    fn record(&self, route: &str, headers: &HeaderMap) {
        let auth = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        self.requests
            .lock()
            .unwrap()
            .push(format!("{route} {auth}"));
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

/// Stand up a mock Intervals.icu: the token exchange answers exactly as the
/// published example does, the athlete read and the deauthorize record the
/// credential they were called with.
async fn mock_intervals() -> (String, Arc<MockIntervals>) {
    let recorder = Arc::new(MockIntervals::default());
    let token_recorder = Arc::clone(&recorder);
    let athlete_recorder = Arc::clone(&recorder);
    let revoke_recorder = Arc::clone(&recorder);
    let app = Router::new()
        .route(
            "/api/oauth/token",
            post(move |Form(form): Form<HashMap<String, String>>| {
                let recorder = Arc::clone(&token_recorder);
                async move {
                    recorder.exchanges.lock().unwrap().push(form);
                    Json(json!({
                        "token_type": "Bearer",
                        "access_token": "icu-oauth-token",
                        "scope": "ACTIVITY:READ,WELLNESS:READ,CALENDAR:WRITE,SETTINGS:READ",
                        "athlete": { "id": "2049151", "name": "Test Athlete" }
                    }))
                }
            }),
        )
        .route(
            "/api/v1/athlete/{id}",
            get(move |Path(id): Path<String>, headers: HeaderMap| {
                let recorder = Arc::clone(&athlete_recorder);
                async move {
                    recorder.record(&format!("GET /api/v1/athlete/{id}"), &headers);
                    Json(json!({ "id": "i2049151", "name": "Test Athlete" }))
                }
            }),
        )
        .route(
            "/api/v1/disconnect-app",
            delete(move |headers: HeaderMap| {
                let recorder = Arc::clone(&revoke_recorder);
                async move {
                    recorder.record("DELETE /api/v1/disconnect-app", &headers);
                    StatusCode::OK
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

/// A server context whose Intervals.icu exchange, reads and revocation all
/// reach the mock. The registry reads the `PIERRE_INTERVALS_ICU_*` overrides
/// when the context is built, so the guard is created first.
async fn context_pointed_at(base: &str) -> (Arc<ServerContext>, EnvGuard) {
    let guard = EnvGuard::set(&[
        (
            "PIERRE_INTERVALS_ICU_TOKEN_URL",
            format!("{base}/api/oauth/token"),
        ),
        ("PIERRE_INTERVALS_ICU_API_BASE_URL", base.to_owned()),
        (
            "PIERRE_INTERVALS_ICU_REVOKE_URL",
            format!("{base}/api/v1/disconnect-app"),
        ),
        ("INTERVALS_ICU_CLIENT_ID", "icu_client".to_owned()),
        ("INTERVALS_ICU_CLIENT_SECRET", "icu_secret".to_owned()),
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

async fn stored_token(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<UserOAuthToken> {
    resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, PROVIDER)
        .await
        .unwrap()
}

/// Run the OAuth callback for `user_id` the way the redirect lands it.
async fn complete_oauth_link(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) {
    let state = format!("{user_id}:{}", Uuid::new_v4());
    resources
        .common
        .repos
        .oauth_client_state
        .store_oauth_client_state(&OAuthClientState {
            state: state.clone(),
            provider: PROVIDER.to_owned(),
            user_id: Some(user_id),
            tenant_id: Some(tenant_id.to_string()),
            redirect_uri: "http://localhost:8081/api/oauth/callback/intervals_icu".to_owned(),
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
        .handle_callback("icu-code-1", &state, PROVIDER)
        .await
        .expect("the exchange succeeds against the mock token endpoint");
    assert_eq!(callback.provider, PROVIDER);
    assert!(
        callback.expires_at.is_none(),
        "a token issued without expires_in reports no expiry"
    );
}

/// Seed a link made by pasting an athlete id + API key.
async fn seed_api_key_link(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) {
    let now = Utc::now();
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant_id.to_string(),
            provider: PROVIDER.to_owned(),
            access_token: "pasted-api-key".to_owned(),
            refresh_token: None,
            token_type: API_KEY_TOKEN_TYPE.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: Some("i777".to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, PROVIDER, &ConnectionType::Manual, None)
        .await
        .unwrap();
}

/// The code exchange stores a bearer token with the athlete's string id and
/// no expiry, under the configured client, and registers an OAuth connection.
#[tokio::test]
#[serial]
async fn code_exchange_stores_a_non_expiring_bearer_token_with_the_athlete_id() {
    let (base, mock) = mock_intervals().await;
    let (resources, _env) = context_pointed_at(&base).await;
    let (user_id, tenant_id) = linked_user(&resources, "icu-exchange@example.com").await;

    complete_oauth_link(&resources, user_id, tenant_id).await;

    let exchanges = mock.exchanges.lock().unwrap().clone();
    assert_eq!(exchanges.len(), 1, "one code exchange");
    assert_eq!(
        exchanges[0].get("client_id").map(String::as_str),
        Some("icu_client")
    );
    assert_eq!(
        exchanges[0].get("client_secret").map(String::as_str),
        Some("icu_secret")
    );
    assert_eq!(
        exchanges[0].get("code").map(String::as_str),
        Some("icu-code-1")
    );

    let stored = stored_token(&resources, user_id, tenant_id)
        .await
        .expect("an intervals_icu token row exists");
    assert_eq!(stored.access_token, "icu-oauth-token");
    assert_eq!(stored.token_type, "Bearer");
    assert_eq!(stored.refresh_token, None);
    assert_eq!(
        stored.expires_at, None,
        "a token Intervals.icu issued without expires_in never expires"
    );
    assert_eq!(
        stored.provider_user_id.as_deref(),
        Some("2049151"),
        "the athlete's string id is read from the token response"
    );

    let connections = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    let connection = connections
        .iter()
        .find(|connection| connection.provider == PROVIDER)
        .expect("a connection is registered");
    assert_eq!(connection.connection_type, ConnectionType::OAuth);
}

/// The serving path sends the stored OAuth token as a bearer credential
/// addressed to athlete `0`, never as a Basic API key.
#[tokio::test]
#[serial]
async fn an_oauth_link_reads_with_its_bearer_token() {
    let (base, mock) = mock_intervals().await;
    let (resources, _env) = context_pointed_at(&base).await;
    let (user_id, tenant_id) = linked_user(&resources, "icu-bearer@example.com").await;
    complete_oauth_link(&resources, user_id, tenant_id).await;

    let auth = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>);
    let provider = auth
        .create_authenticated_provider(PROVIDER, user_id, Some(&tenant_id.to_string()))
        .await
        .unwrap_or_else(|response| panic!("provider builds: {:?}", response.error));
    let athlete = provider.get_athlete().await.expect("the athlete reads");
    assert_eq!(athlete.id, "i2049151");

    assert_eq!(
        mock.requests(),
        vec!["GET /api/v1/athlete/0 Bearer icu-oauth-token".to_owned()]
    );
}

/// An API-key link still reads with HTTP Basic on its athlete id, and needs no
/// OAuth client to do so.
#[tokio::test]
#[serial]
async fn an_api_key_link_still_reads_with_basic_auth() {
    let (base, mock) = mock_intervals().await;
    let (resources, _env) = context_pointed_at(&base).await;
    let (user_id, tenant_id) = linked_user(&resources, "icu-key@example.com").await;
    seed_api_key_link(&resources, user_id, tenant_id).await;
    // No OAuth app configured: an API key must not need one.
    env::remove_var("INTERVALS_ICU_CLIENT_ID");
    env::remove_var("INTERVALS_ICU_CLIENT_SECRET");

    let auth = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>);
    let provider = auth
        .create_authenticated_provider(PROVIDER, user_id, Some(&tenant_id.to_string()))
        .await
        .unwrap_or_else(|response| panic!("provider builds: {:?}", response.error));
    provider.get_athlete().await.expect("the athlete reads");

    // base64("API_KEY:pasted-api-key")
    assert_eq!(
        mock.requests(),
        vec!["GET /api/v1/athlete/i777 Basic QVBJX0tFWTpwYXN0ZWQtYXBpLWtleQ==".to_owned()]
    );
}

/// Disconnecting an OAuth link withdraws the grant at Intervals.icu with the
/// bearer token; disconnecting an API-key link calls nothing upstream.
#[tokio::test]
#[serial]
async fn disconnect_revokes_an_oauth_grant_but_not_an_api_key() {
    let (base, mock) = mock_intervals().await;
    let (resources, _env) = context_pointed_at(&base).await;
    let service = OAuthService::new(resources.data(), resources.common.config.clone());

    let (oauth_user, oauth_tenant) = linked_user(&resources, "icu-revoke@example.com").await;
    complete_oauth_link(&resources, oauth_user, oauth_tenant).await;
    let outcome = service
        .disconnect_provider(
            oauth_user,
            PROVIDER,
            Some(oauth_tenant.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("disconnect succeeds");
    assert!(
        matches!(outcome, RevocationOutcome::Revoked),
        "got {outcome:?}"
    );
    assert_eq!(
        mock.requests(),
        vec!["DELETE /api/v1/disconnect-app Bearer icu-oauth-token".to_owned()]
    );
    assert!(stored_token(&resources, oauth_user, oauth_tenant)
        .await
        .is_none());

    let (key_user, key_tenant) = linked_user(&resources, "icu-key-revoke@example.com").await;
    seed_api_key_link(&resources, key_user, key_tenant).await;
    let outcome = service
        .disconnect_provider(
            key_user,
            PROVIDER,
            Some(key_tenant.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("disconnect succeeds");
    assert!(
        matches!(outcome, RevocationOutcome::NoGrant),
        "got {outcome:?}"
    );
    assert_eq!(
        mock.requests().len(),
        1,
        "an API key is never sent to the deauthorize endpoint"
    );
    assert!(stored_token(&resources, key_user, key_tenant)
        .await
        .is_none());
}
