// ABOUTME: Revoking a grant, a client's own revoke call, a spent family and a deleted account all end a refresh token
// ABOUTME: Each test drives the real routes and reads the client's next refresh: invalid_grant, or still alive for the untouched

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The ways a connected MCP app stops refreshing.
//!
//! Revoking a grant in "connected apps" used to set `oauth_client_grants.revoked_at`
//! and nothing else, so the client kept refreshing indefinitely: the refresh
//! grant reads only the refresh row. Each test below ends on the client's next
//! refresh, because that is the observable the user cares about.

use crate::common;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::Request;
use axum::Router;
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Duration, SubsecRound, Utc};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::endpoints::OAuth2AuthorizationServer;
use pierre_auth::oauth2_server::models::{
    AuthorizeRequest, ClientRegistrationRequest, TokenRequest,
};
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::refresh_rotation::REFRESH_FAMILY_MAX_DAYS;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::{OAuth2RefreshToken, OAuthClientGrant, TenantId, User};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::oauth_grants::OAuthGrantsRoutes;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use pierre_test_support::account::delete_account;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

const REDIRECT: &str = "https://app.example.com/callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const SCOPE: &str = "fitness:read profile:read";

fn oauth2_routes(resources: &Arc<ServerContext>) -> Router {
    let context = OAuth2Context {
        database: resources.agent.database.clone(),
        oauth2_server: resources.common.repos.oauth2_server.clone(),
        tenants: resources.common.repos.tenants.clone(),
        users: resources.common.repos.users.clone(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        config: Arc::new(resources.common.config.oauth2_server.clone()),
        refresh_token_expiry_days: 30,
        csrf_manager: resources.auth.csrf_manager.clone(),
        accounts: resources.account_service(),
        google_sign_in: None,
        strings: Arc::new(MessagingStringsRegistry::new()),
        first_party_only_sources: Arc::default(),
        rate_limiter: Arc::new(OAuth2RateLimiter::new(
            None,
            OAuth2RateLimiter::local_window_store(),
            &resources.common.config.rate_limiting,
        )),
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_002))))
}

/// A registered confidential client.
struct Client {
    id: String,
    secret: String,
}

/// An athlete who has consented to one client and holds its tokens.
struct Connection {
    client: Client,
    grant_id: String,
    refresh_token: String,
}

struct World {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    user: User,
    tenant_id: TenantId,
}

async fn world() -> World {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("revocation_{}@example.com", Uuid::new_v4());
    let (user_id, user) = common::create_test_user_with_email(&resources.agent.database, &email)
        .await
        .unwrap();
    let tenants = resources.common.repos.tenants.get_all().await.unwrap();
    let tenant_id = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("a fresh user owns a tenant")
        .id;
    World {
        resources,
        user_id,
        user,
        tenant_id,
    }
}

async fn register(world: &World) -> Client {
    let registered =
        ClientRegistrationManager::new(world.resources.common.repos.oauth2_server.clone())
            .register_client(
                ClientRegistrationRequest {
                    redirect_uris: vec![REDIRECT.to_owned()],
                    client_name: Some("Revocation client".to_owned()),
                    client_uri: None,
                    grant_types: None,
                    response_types: None,
                    scope: Some(SCOPE.to_owned()),
                },
                MAX_PENDING_REGISTRATIONS,
            )
            .await
            .unwrap();
    Client {
        id: registered.client_id,
        secret: registered.client_secret.expose_secret().to_owned(),
    }
}

fn challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

/// Register a client, mint an authorization code for the athlete, exchange it
/// and record the consent grant: the state after "Connect" in a directory.
async fn connect(world: &World) -> Connection {
    let client = register(world).await;
    let server = OAuth2AuthorizationServer::from_config(
        &world.resources.common.config.oauth2_server,
        world.resources.common.repos.oauth2_server.clone(),
        world.resources.common.repos.tenants.clone(),
        world.resources.common.repos.users.clone(),
        world.resources.auth.auth_manager.clone(),
        world.resources.auth.jwks_manager.clone(),
        30,
    );
    let code = server
        .authorize(
            AuthorizeRequest {
                response_type: "code".to_owned(),
                client_id: client.id.clone(),
                redirect_uri: REDIRECT.to_owned(),
                scope: Some(SCOPE.to_owned()),
                state: None,
                code_challenge: Some(challenge()),
                code_challenge_method: Some("S256".to_owned()),
                resource: None,
            },
            Some(world.user_id),
            Some(world.tenant_id.to_string()),
        )
        .await
        .unwrap()
        .code;
    let token = server
        .token(TokenRequest {
            grant_type: "authorization_code".to_owned(),
            code: Some(code),
            redirect_uri: Some(REDIRECT.to_owned()),
            client_id: client.id.clone(),
            client_secret: SecretString::from(client.secret.clone()),
            scope: None,
            refresh_token: None,
            code_verifier: Some(VERIFIER.to_owned()),
            resource: None,
        })
        .await
        .unwrap();

    let grant_id = Uuid::new_v4().to_string();
    world
        .resources
        .common
        .repos
        .oauth2_server
        .store_client_grant(&OAuthClientGrant {
            id: grant_id.clone(),
            user_id: world.user_id.to_string(),
            tenant_id: world.tenant_id.to_string(),
            client_id: client.id.clone(),
            scope: SCOPE.to_owned(),
            granted_at: Utc::now(),
            revoked_at: None,
        })
        .await
        .unwrap();
    Connection {
        client,
        grant_id,
        refresh_token: token.refresh_token.unwrap().expose_secret().to_owned(),
    }
}

/// `POST /oauth2/token` with `grant_type=refresh_token`.
async fn refresh(world: &World, client: &Client, refresh_token: &str) -> AxumTestResponse {
    AxumTestRequest::post("/oauth2/token")
        .form(&json!({
            "grant_type": "refresh_token",
            "client_id": client.id,
            "client_secret": client.secret,
            "refresh_token": refresh_token,
        }))
        .send(oauth2_routes(&world.resources))
        .await
}

/// A refresh that must succeed, returning the rotated refresh token.
async fn refresh_ok(world: &World, client: &Client, refresh_token: &str) -> String {
    let response = refresh(world, client, refresh_token).await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = response.json();
    body["refresh_token"].as_str().unwrap().to_owned()
}

fn assert_invalid_grant(response: &AxumTestResponse) {
    assert_eq!(response.status(), 400, "{}", response.body_text());
    let body: Value = serde_json::from_str(&response.body_text()).unwrap();
    assert_eq!(body["error"], "invalid_grant", "{body}");
}

/// O6: the defect. Revoking the grant leaves the refresh token alive.
#[tokio::test]
async fn revoking_a_grant_ends_the_clients_refresh_family() {
    let world = world().await;
    let revoked = connect(&world).await;
    let untouched = connect(&world).await;

    // The client has already rotated once: the live token is a successor, so
    // revoking only the first token's row would not be enough.
    let live = refresh_ok(&world, &revoked.client, &revoked.refresh_token).await;

    let session = common::generate_test_token(&world.resources, &world.user).await;
    let response = AxumTestRequest::delete(&format!("/api/me/oauth-grants/{}", revoked.grant_id))
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&world.resources)))
        .await;
    assert_eq!(response.status(), 204, "{}", response.body_text());

    assert_invalid_grant(&refresh(&world, &revoked.client, &live).await);

    // Only that client's refresh tokens went: the athlete's other connection
    // keeps refreshing.
    refresh_ok(&world, &untouched.client, &untouched.refresh_token).await;
}

/// A code issued just before the athlete withdrew consent starts no new family.
#[tokio::test]
async fn a_code_issued_before_its_grant_was_revoked_is_not_redeemed() {
    let world = world().await;
    let connection = connect(&world).await;
    let client = &connection.client;
    let server = OAuth2AuthorizationServer::from_config(
        &world.resources.common.config.oauth2_server,
        world.resources.common.repos.oauth2_server.clone(),
        world.resources.common.repos.tenants.clone(),
        world.resources.common.repos.users.clone(),
        world.resources.auth.auth_manager.clone(),
        world.resources.auth.jwks_manager.clone(),
        30,
    );
    let code = server
        .authorize(
            AuthorizeRequest {
                response_type: "code".to_owned(),
                client_id: client.id.clone(),
                redirect_uri: REDIRECT.to_owned(),
                scope: Some(SCOPE.to_owned()),
                state: None,
                code_challenge: Some(challenge()),
                code_challenge_method: Some("S256".to_owned()),
                resource: None,
            },
            Some(world.user_id),
            Some(world.tenant_id.to_string()),
        )
        .await
        .unwrap()
        .code;

    assert!(world
        .resources
        .common
        .repos
        .oauth2_server
        .revoke_client_grant(
            &connection.grant_id,
            &world.user_id.to_string(),
            &world.tenant_id.to_string(),
        )
        .await
        .unwrap());

    let refused = server
        .token(TokenRequest {
            grant_type: "authorization_code".to_owned(),
            code: Some(code),
            redirect_uri: Some(REDIRECT.to_owned()),
            client_id: client.id.clone(),
            client_secret: SecretString::from(client.secret.clone()),
            scope: None,
            refresh_token: None,
            code_verifier: Some(VERIFIER.to_owned()),
            resource: None,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.error, "invalid_grant");
}

/// A successor written while the grant was being revoked does not outlive it:
/// the rotation checks the grant after writing, which is the state this
/// reproduces by writing the successor after the revocation.
#[tokio::test]
async fn a_token_written_after_its_grant_was_revoked_does_not_survive() {
    let world = world().await;
    let connection = connect(&world).await;
    let repos = &world.resources.common.repos;
    let user_id = world.user_id.to_string();
    let tenant_id = world.tenant_id.to_string();
    assert!(repos
        .oauth2_server
        .revoke_client_grant(&connection.grant_id, &user_id, &tenant_id)
        .await
        .unwrap());

    let late = format!("late-{}", Uuid::new_v4());
    repos
        .oauth2_server
        .store_refresh_token(&OAuth2RefreshToken {
            token: SecretString::from(late.clone()),
            client_id: connection.client.id.clone(),
            user_id: world.user_id,
            tenant_id: tenant_id.clone(),
            scope: Some(SCOPE.to_owned()),
            expires_at: Utc::now() + Duration::days(1),
            created_at: Utc::now(),
            revoked: false,
            family_id: Uuid::new_v4().to_string(),
            resource: None,
            family_created_at: Some(Utc::now()),
        })
        .await
        .unwrap();

    assert!(repos
        .oauth2_server
        .revoke_refresh_tokens_of_withdrawn_grant(&user_id, &tenant_id, &connection.client.id)
        .await
        .unwrap());
    assert_invalid_grant(&refresh(&world, &connection.client, &late).await);
}

/// O5: a client that is being disconnected revokes its own token.
#[tokio::test]
async fn a_client_revokes_its_refresh_token_at_the_revocation_endpoint() {
    let world = world().await;
    let connection = connect(&world).await;
    let other = connect(&world).await;
    let routes = oauth2_routes(&world.resources);
    let revoke = |client: &Client, token: &str| {
        AxumTestRequest::post("/oauth2/revoke")
            .form(&json!({
                "token": token,
                "token_type_hint": "refresh_token",
                "client_id": client.id,
                "client_secret": client.secret,
            }))
            .send(routes.clone())
    };

    // Another client's token is refused and survives.
    let foreign = revoke(&other.client, &connection.refresh_token).await;
    assert_eq!(foreign.status(), 400, "{}", foreign.body_text());
    let body: Value = serde_json::from_str(&foreign.body_text()).unwrap();
    assert_eq!(body["error"], "unauthorized_client");

    // A wrong secret revokes nothing.
    let forged = Client {
        id: connection.client.id.clone(),
        secret: "not-the-secret".to_owned(),
    };
    let rejected = revoke(&forged, &connection.refresh_token).await;
    assert_eq!(rejected.status(), 400, "{}", rejected.body_text());
    let body: Value = serde_json::from_str(&rejected.body_text()).unwrap();
    assert_eq!(body["error"], "invalid_client");

    // An unknown token is a success: the endpoint is no probe.
    assert_eq!(
        revoke(&connection.client, "no-such-token").await.status(),
        200
    );

    // Missing token is a malformed request.
    let missing = AxumTestRequest::post("/oauth2/revoke")
        .form(&json!({ "client_id": connection.client.id, "client_secret": connection.client.secret }))
        .send(routes.clone())
        .await;
    assert_eq!(missing.status(), 400);

    // The rotated token is as revocable as the first: revoke the live one.
    let live = refresh_ok(&world, &connection.client, &connection.refresh_token).await;
    let revoked = revoke(&connection.client, &live).await;
    assert_eq!(revoked.status(), 200, "{}", revoked.body_text());
    assert_invalid_grant(&refresh(&world, &connection.client, &live).await);
}

/// Store a refresh token for a registered client with a chosen family start.
async fn stored_refresh_token(
    world: &World,
    client: &Client,
    family_created_at: Option<DateTime<Utc>>,
) -> String {
    let raw = format!("rt-{}", Uuid::new_v4());
    world
        .resources
        .common
        .repos
        .oauth2_server
        .store_refresh_token(&OAuth2RefreshToken {
            token: SecretString::from(raw.clone()),
            client_id: client.id.clone(),
            user_id: world.user_id,
            tenant_id: world.tenant_id.to_string(),
            scope: Some(SCOPE.to_owned()),
            expires_at: Utc::now() + Duration::days(30),
            created_at: Utc::now(),
            revoked: false,
            family_id: Uuid::new_v4().to_string(),
            family_created_at,
            resource: None,
        })
        .await
        .unwrap();
    raw
}

/// O9: sliding rotation has an absolute end. `NULL` is a legacy family.
#[tokio::test]
async fn a_refresh_family_has_an_absolute_lifetime_and_a_legacy_one_has_none() {
    let world = world().await;
    let client = register(&world).await;
    let repos = &world.resources.common.repos.oauth2_server;

    // Older than the family lifetime, though its own window is still open.
    let spent = stored_refresh_token(
        &world,
        &client,
        Some(Utc::now() - Duration::days(REFRESH_FAMILY_MAX_DAYS + 1)),
    )
    .await;
    assert_invalid_grant(&refresh(&world, &client, &spent).await);

    // Close to the end: it still refreshes, but the successor cannot outlive
    // the family.
    let started = (Utc::now() - Duration::days(REFRESH_FAMILY_MAX_DAYS - 2)).trunc_subsecs(0);
    let aging = stored_refresh_token(&world, &client, Some(started)).await;
    let successor = refresh_ok(&world, &client, &aging).await;
    let stored = repos
        .get_refresh_token_by_value(&successor)
        .await
        .unwrap()
        .expect("the successor is stored");
    let deadline = started + Duration::days(REFRESH_FAMILY_MAX_DAYS);
    assert!(
        stored.expires_at <= deadline + Duration::seconds(1),
        "the successor expires at {}, past the family deadline {deadline}",
        stored.expires_at
    );
    assert_eq!(stored.family_created_at, Some(started));

    // Issued before the column existed: no absolute limit, and a successor
    // keeps the family unlimited.
    let legacy = stored_refresh_token(&world, &client, None).await;
    let successor = refresh_ok(&world, &client, &legacy).await;
    let stored = repos
        .get_refresh_token_by_value(&successor)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.family_created_at, None);
    assert!(stored.expires_at > Utc::now() + Duration::days(29));
}

/// A new connection stamps its family so the lifetime has a start.
#[tokio::test]
async fn a_connection_stamps_when_its_refresh_family_began() {
    let world = world().await;
    let connection = connect(&world).await;
    let stored = world
        .resources
        .common
        .repos
        .oauth2_server
        .get_refresh_token_by_value(&connection.refresh_token)
        .await
        .unwrap()
        .unwrap();
    let stamped = stored.family_created_at.expect("a new family is stamped");
    assert!((Utc::now() - stamped).num_seconds().abs() < 60);
}

/// Design exit criterion 7: deleting the account ends every token it holds.
#[tokio::test]
async fn deleting_the_account_ends_the_refresh_family_and_the_access_token() {
    let world = world().await;
    let connection = connect(&world).await;
    let access = common::generate_delegated_test_token(&world.resources, &world.user).await;

    let tools_call = |token: &str| {
        let request = Request::post("/mcp")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": { "name": "get_connection_status", "arguments": {} }
                })
                .to_string(),
            ))
            .unwrap();
        ProviderToolRouter::build_http_app(&world.resources).oneshot(request)
    };
    let before = tools_call(&access).await.unwrap();
    assert_eq!(
        before.status(),
        200,
        "control: the token works while the account exists"
    );

    delete_account(&world.resources.common.repos, world.user_id, None)
        .await
        .unwrap();

    assert_invalid_grant(&refresh(&world, &connection.client, &connection.refresh_token).await);
    let after = tools_call(&access).await.unwrap();
    assert_eq!(
        after.status(),
        401,
        "an access token of a deleted account is refused"
    );
}
