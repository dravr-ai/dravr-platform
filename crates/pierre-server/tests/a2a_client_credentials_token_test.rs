// ABOUTME: An A2A client obtains its bearer at the tokenUrl its own agent card advertises (client_credentials)
// ABOUTME: Registration -> card -> /oauth2/token -> /a2a/jsonrpc, a wrong secret refused, deactivation revoking the credential
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The card's `oauth2ClientCredentials` flow works for the clients it names
//!
//! The agent card advertises an `oauth2ClientCredentials` scheme whose
//! `tokenUrl` is `/oauth2/token`, and the A2A principal accepts the
//! `client:{id}` tokens that grant mints. Registration through
//! `POST /a2a/clients` is what hands a client its `client_id` and
//! `client_secret`, so those two must be exactly what the token endpoint
//! authenticates, and deactivating the client must take the credential away.

mod common;
mod helpers;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use chrono::{Duration, Utc};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_a2a::{A2ARoutes, A2ARoutesState};
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use url::Url;

/// The A2A router over `resources`, as the server mounts it.
fn a2a_routes(resources: &Arc<ServerContext>) -> axum::Router {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    A2ARoutes::routes(A2ARoutesState {
        ctx: resources.clone(),
        client_manager: resources.a2a.a2a_client_manager.clone(),
        auth_middleware: resources.auth.auth_middleware.clone(),
        tool_runtime,
    })
}

/// The `OAuth2` router over `resources`, as the server mounts it.
fn oauth2_routes(resources: &Arc<ServerContext>) -> axum::Router {
    let context = OAuth2Context {
        database: resources.agent.database.clone(),
        oauth2_server: resources.common.repos.oauth2_server.clone(),
        tenants: resources.common.repos.tenants.clone(),
        users: resources.common.repos.users.clone(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        config: Arc::new(resources.common.config.oauth2_server.clone()),
        refresh_token_expiry_days: resources.common.config.auth.refresh_token_expiry_days,
        csrf_manager: resources.auth.csrf_manager.clone(),
        accounts: resources.oauth2_accounts(),
        google_sign_in: None,
        rate_limiter: Arc::new(OAuth2RateLimiter::new(
            None,
            OAuth2RateLimiter::local_window_store(),
            &resources.common.config.rate_limiting,
        )),
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))))
}

/// The path of the `tokenUrl` the public agent card advertises for its
/// `oauth2ClientCredentials` scheme, which is what the test routers serve.
async fn card_token_path(resources: &Arc<ServerContext>) -> String {
    let response = AxumTestRequest::get("/.well-known/agent-card.json")
        .send(a2a_routes(resources))
        .await;
    assert_eq!(response.status(), 200);
    let card: Value = response.json();
    let token_url = card["securitySchemes"]["oauth2ClientCredentials"]["oauth2SecurityScheme"]
        ["flows"]["clientCredentials"]["tokenUrl"]
        .as_str()
        .unwrap_or_else(|| panic!("the card advertises a clientCredentials tokenUrl: {card}"))
        .to_owned();
    // The card is built from the configured base URL, which the test server
    // leaves empty; resolving against a base reads an absolute and a
    // base-less `tokenUrl` alike.
    Url::parse("http://localhost")
        .and_then(|base| base.join(&token_url))
        .unwrap_or_else(|e| panic!("tokenUrl {token_url} is a URL: {e}"))
        .path()
        .to_owned()
}

/// Register an A2A client through `POST /a2a/clients` as a fresh user, and
/// return the user's JWT with the client's id and secret.
async fn register_client(resources: &Arc<ServerContext>, email: &str) -> (String, String, String) {
    let (_user, jwt) = common::create_test_tenant(resources, email)
        .await
        .expect("seed user + tenant + JWT");
    let response = AxumTestRequest::post("/a2a/clients")
        .header("Authorization", &format!("Bearer {jwt}"))
        .json(&json!({
            "name": "client-credentials-agent",
            "description": "obtains its token at the card's tokenUrl",
            "capabilities": ["fitness-data-analysis"],
            "contact_email": "ops@example.com",
        }))
        .send(a2a_routes(resources))
        .await;
    assert_eq!(response.status(), 201);
    let created: Value = response.json();
    let client_id = created["client_id"].as_str().unwrap().to_owned();
    let client_secret = created["client_secret"].as_str().unwrap().to_owned();
    (jwt, client_id, client_secret)
}

/// `POST` the `client_credentials` grant to `token_path`, form-encoded.
async fn request_token(
    resources: &Arc<ServerContext>,
    token_path: &str,
    client_id: &str,
    client_secret: &str,
) -> AxumTestResponse {
    AxumTestRequest::post(token_path)
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .send(oauth2_routes(resources))
        .await
}

/// `GetExtendedAgentCard` over the JSON-RPC binding with `bearer`.
async fn extended_card(resources: &Arc<ServerContext>, bearer: &str) -> Value {
    let response = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("Authorization", &format!("Bearer {bearer}"))
        .json(&json!({"jsonrpc": "2.0", "method": "GetExtendedAgentCard", "id": 1}))
        .send(a2a_routes(resources))
        .await;
    response.json()
}

#[tokio::test]
async fn a_registered_client_gets_a_token_at_the_card_token_url_and_calls_a2a_with_it() {
    let resources = common::create_test_server_resources().await.unwrap();
    let token_path = card_token_path(&resources).await;
    assert_eq!(token_path, "/oauth2/token");
    let (_jwt, client_id, client_secret) =
        register_client(&resources, "a2a-cc-token@example.com").await;

    let response = request_token(&resources, &token_path, &client_id, &client_secret).await;
    assert_eq!(response.status(), 200);
    let token: Value = response.json();
    assert_eq!(token["token_type"], "Bearer");
    // The read-only default a client that asks for nothing is granted, which
    // carries the `fitness:read` the card's security requirement names.
    assert_eq!(
        token["scope"],
        OAuthScope::render_granted(&OAuthScope::default_grant())
    );
    assert!(token["scope"]
        .as_str()
        .unwrap()
        .split(' ')
        .any(|scope| scope == OAuthScope::FitnessRead.as_str()));
    assert!(token.get("refresh_token").is_none_or(Value::is_null));
    let access_token = token["access_token"].as_str().unwrap().to_owned();

    let claims = resources
        .auth
        .auth_manager
        .validate_token(&access_token, &resources.auth.jwks_manager)
        .expect("the minted token validates");
    assert_eq!(claims.sub, format!("client:{client_id}"));

    let envelope = extended_card(&resources, &access_token).await;
    assert!(
        envelope["error"].is_null(),
        "the client-credentials token authenticates A2A: {envelope}"
    );
    assert_eq!(
        envelope["result"]["capabilities"]["extendedAgentCard"],
        true
    );
}

#[tokio::test]
async fn a_wrong_secret_is_refused_as_invalid_client() {
    let resources = common::create_test_server_resources().await.unwrap();
    let token_path = card_token_path(&resources).await;
    let (_jwt, client_id, client_secret) =
        register_client(&resources, "a2a-cc-wrong@example.com").await;

    let response = request_token(
        &resources,
        &token_path,
        &client_id,
        &format!("{client_secret}x"),
    )
    .await;
    assert_eq!(response.status(), 400);
    let refusal: Value = response.json();
    assert_eq!(refusal["error"], "invalid_client");
    assert!(refusal.get("access_token").is_none());
}

#[tokio::test]
async fn a_deactivated_client_is_refused_at_the_token_url_and_by_a2a() {
    let resources = common::create_test_server_resources().await.unwrap();
    let token_path = card_token_path(&resources).await;
    let (jwt, client_id, client_secret) =
        register_client(&resources, "a2a-cc-deactivated@example.com").await;

    let response = request_token(&resources, &token_path, &client_id, &client_secret).await;
    assert_eq!(response.status(), 200);
    let issued: Value = response.json();
    let access_token = issued["access_token"].as_str().unwrap().to_owned();

    let response = AxumTestRequest::delete(&format!("/a2a/clients/{client_id}"))
        .header("Authorization", &format!("Bearer {jwt}"))
        .send(a2a_routes(&resources))
        .await;
    assert_eq!(response.status(), 200);

    // The credential itself is gone: the same id and secret are an unknown client.
    let response = request_token(&resources, &token_path, &client_id, &client_secret).await;
    assert_eq!(response.status(), 400);
    assert_eq!(response.json::<Value>()["error"], "invalid_client");
    assert!(resources
        .common
        .repos
        .oauth2_server
        .get_client(&client_id)
        .await
        .unwrap()
        .is_none());

    // A token minted before the deactivation is refused by the A2A principal.
    let envelope = extended_card(&resources, &access_token).await;
    assert!(
        envelope["result"].is_null(),
        "no result for a deactivated client: {envelope}"
    );
    let message = envelope["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("deactivated"),
        "the refusal names the deactivation: {envelope}"
    );
}

#[tokio::test]
async fn the_retention_sweep_never_deletes_an_a2a_client_credential() {
    let resources = common::create_test_server_resources().await.unwrap();
    let token_path = card_token_path(&resources).await;
    let (_jwt, client_id, client_secret) =
        register_client(&resources, "a2a-cc-sweep@example.com").await;

    // A dynamic registration nobody authorized, which the sweep below must
    // delete: proof the sweep ran with cutoffs that reach every row it may touch.
    let oauth2_server = resources.common.repos.oauth2_server.clone();
    let dynamic = ClientRegistrationManager::new(oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec!["https://example.com/callback".to_owned()],
                client_name: Some("abandoned".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap();

    let far_future = Utc::now() + Duration::days(3650);
    let sweep = oauth2_server
        .delete_stale_clients(far_future, far_future)
        .await
        .unwrap();
    assert_eq!(
        sweep.expired + sweep.abandoned,
        1,
        "the sweep deleted the dynamic registration and nothing else: {sweep:?}"
    );
    assert!(oauth2_server
        .get_client(&dynamic.client_id)
        .await
        .unwrap()
        .is_none());

    let kept = oauth2_server
        .get_client(&client_id)
        .await
        .unwrap()
        .expect("the A2A client's registration survives the sweep");
    assert_eq!(kept.expires_at, None);
    assert_eq!(kept.grant_types, vec!["client_credentials".to_owned()]);
    assert!(kept.redirect_uris.is_empty());

    let response = request_token(&resources, &token_path, &client_id, &client_secret).await;
    assert_eq!(response.status(), 200);
}
