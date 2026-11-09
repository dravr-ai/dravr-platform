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
use std::time::Duration as StdDuration;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::Request;
use axum::Router;
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Duration, SubsecRound, Utc};
use pierre_auth::auth::DelegatedGrant;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::endpoints::OAuth2AuthorizationServer;
use pierre_auth::oauth2_server::grant_standing::GrantStandings;
use pierre_auth::oauth2_server::models::{
    AuthorizeRequest, ClientRegistrationRequest, TokenRequest,
};
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::refresh_rotation::REFRESH_FAMILY_MAX_DAYS;
use pierre_cache::{Cache, CacheConfig};
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::errors::ErrorCode;
use pierre_core::models::{OAuth2RefreshToken, OAuthClientGrant, TenantId, User, UserStatus};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::oauth_grants::OAuthGrantsRoutes;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use pierre_test_support::account::delete_account;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::time::sleep;
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
    access_token: String,
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
        access_token: token.access_token.expose_secret().to_owned(),
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

// ── An access token stops with its grant ────────────────────────────────────

/// `tools/call` on `/mcp` under `token`: the status it answers.
async fn mcp_status(world: &World, token: &str) -> u16 {
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
    ProviderToolRouter::build_http_app(&world.resources)
        .oneshot(request)
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// `POST /oauth2/revoke` by `client`.
async fn revoke_at_endpoint(world: &World, client: &Client, token: &str) -> AxumTestResponse {
    AxumTestRequest::post("/oauth2/revoke")
        .form(&json!({
            "token": token,
            "client_id": client.id,
            "client_secret": client.secret,
        }))
        .send(oauth2_routes(&world.resources))
        .await
}

/// `DELETE /api/me/oauth-grants/{id}` by the athlete.
async fn revoke_in_connected_apps(world: &World, grant_id: &str) {
    let session = common::generate_test_token(&world.resources, &world.user).await;
    let response = AxumTestRequest::delete(&format!("/api/me/oauth-grants/{grant_id}"))
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&world.resources)))
        .await;
    assert_eq!(response.status(), 204, "{}", response.body_text());
}

/// The tail this closes: a revoked grant's access token answered `/mcp` for
/// the rest of its hour. The first call primes the cached standing, so the
/// second also proves the revocation forgets it.
#[tokio::test]
async fn revoking_in_connected_apps_stops_the_access_token_on_its_next_call() {
    let world = world().await;
    let revoked = connect(&world).await;
    let untouched = connect(&world).await;
    assert_eq!(mcp_status(&world, &revoked.access_token).await, 200);
    assert_eq!(mcp_status(&world, &untouched.access_token).await, 200);

    revoke_in_connected_apps(&world, &revoked.grant_id).await;

    assert_eq!(
        mcp_status(&world, &revoked.access_token).await,
        401,
        "the revoked client's token stops at once"
    );
    assert_eq!(
        mcp_status(&world, &untouched.access_token).await,
        200,
        "another client's grant is not this one"
    );
}

/// The A2A door calls the same entry point with no resources.
#[tokio::test]
async fn the_scoped_entry_point_a2a_uses_refuses_a_revoked_token() {
    let world = world().await;
    let connection = connect(&world).await;
    let bearer = format!("Bearer {}", connection.access_token);
    let middleware = &world.resources.auth.auth_middleware;
    middleware
        .authenticate_scoped_request(Some(&bearer), &[])
        .await
        .unwrap();

    revoke_in_connected_apps(&world, &connection.grant_id).await;

    let refused = middleware
        .authenticate_scoped_request(Some(&bearer), &[])
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::AuthInvalid, "{refused}");
}

/// RFC 7009 §2.1: revoking a refresh token invalidates the access tokens of
/// the same grant.
#[tokio::test]
async fn a_client_revoking_its_refresh_token_stops_its_access_token() {
    let world = world().await;
    let connection = connect(&world).await;
    assert_eq!(mcp_status(&world, &connection.access_token).await, 200);

    let revoked = revoke_at_endpoint(&world, &connection.client, &connection.refresh_token).await;
    assert_eq!(revoked.status(), 200, "{}", revoked.body_text());

    assert_eq!(mcp_status(&world, &connection.access_token).await, 401);
}

/// Revoking the access token alone withdraws the grant it rides on, refresh
/// token included.
#[tokio::test]
async fn a_client_revoking_its_access_token_withdraws_the_grant() {
    let world = world().await;
    let connection = connect(&world).await;
    let other = connect(&world).await;
    assert_eq!(mcp_status(&world, &connection.access_token).await, 200);

    let foreign = revoke_at_endpoint(&world, &other.client, &connection.access_token).await;
    assert_eq!(foreign.status(), 400, "{}", foreign.body_text());
    let body: Value = serde_json::from_str(&foreign.body_text()).unwrap();
    assert_eq!(body["error"], "unauthorized_client");
    assert_eq!(
        mcp_status(&world, &connection.access_token).await,
        200,
        "another client cannot revoke it"
    );

    let revoked = revoke_at_endpoint(&world, &connection.client, &connection.access_token).await;
    assert_eq!(revoked.status(), 200, "{}", revoked.body_text());

    assert_eq!(mcp_status(&world, &connection.access_token).await, 401);
    assert_invalid_grant(&refresh(&world, &connection.client, &connection.refresh_token).await);
    let session = common::generate_test_token(&world.resources, &world.user).await;
    let listed: Value = AxumTestRequest::get("/api/me/oauth-grants")
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&world.resources)))
        .await
        .json();
    assert!(
        listed["grants"]
            .as_array()
            .unwrap()
            .iter()
            .all(|grant| grant["id"] != connection.grant_id.as_str()),
        "the app leaves the athlete's connected apps: {listed}"
    );
}

/// A session presented at the endpoint revokes nothing: it is no delegated
/// grant, and a first-party session stays out of all of this.
#[tokio::test]
async fn a_session_token_at_the_revocation_endpoint_is_left_alone() {
    let world = world().await;
    let connection = connect(&world).await;
    let session = common::generate_test_token(&world.resources, &world.user).await;

    let answered = revoke_at_endpoint(&world, &connection.client, &session).await;
    assert_eq!(answered.status(), 200, "{}", answered.body_text());

    assert_eq!(mcp_status(&world, &connection.access_token).await, 200);
    let still_signed_in = AxumTestRequest::get("/api/me/oauth-grants")
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&world.resources)))
        .await;
    assert_eq!(still_signed_in.status(), 200);
}

/// A token with no grant row runs to its expiry, because nobody withdrew
/// anything — until its own client revokes it, which records the withdrawal.
#[tokio::test]
async fn a_token_with_no_grant_row_is_served_until_its_client_revokes_it() {
    let world = world().await;
    let connection = connect(&world).await;
    let session = common::generate_test_token(&world.resources, &world.user).await;
    sqlx_delete_grants(&world).await;
    world
        .resources
        .account_service()
        .grant_standings()
        .forget(
            world.user_id,
            &world.tenant_id.to_string(),
            &connection.client.id,
        )
        .await;
    assert_eq!(
        mcp_status(&world, &connection.access_token).await,
        200,
        "no row reads as never withdrawn"
    );

    let revoked = revoke_at_endpoint(&world, &connection.client, &connection.access_token).await;
    assert_eq!(revoked.status(), 200, "{}", revoked.body_text());

    assert_eq!(mcp_status(&world, &connection.access_token).await, 401);
    let listed: Value = AxumTestRequest::get("/api/me/oauth-grants")
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&world.resources)))
        .await
        .json();
    assert_eq!(
        listed["grants"].as_array().unwrap().len(),
        0,
        "the withdrawal's own row lists as no connected app: {listed}"
    );
}

/// Delete every grant row, as an authorization from before the table.
async fn sqlx_delete_grants(world: &World) {
    match world.resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query("DELETE FROM oauth_client_grants")
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query("DELETE FROM oauth_client_grants")
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// Consenting again does not bring back what the revocation ended: the old
/// token stays refused while the new consent's token is served.
#[tokio::test]
async fn consenting_again_does_not_revive_a_token_from_before_the_revocation() {
    let world = world().await;
    let connection = connect(&world).await;
    revoke_in_connected_apps(&world, &connection.grant_id).await;
    assert_eq!(mcp_status(&world, &connection.access_token).await, 401);

    // Past the second of the revocation, which `iat` cannot tell apart.
    sleep(StdDuration::from_millis(1_100)).await;
    world
        .resources
        .common
        .repos
        .oauth2_server
        .store_client_grant(&OAuthClientGrant {
            id: Uuid::new_v4().to_string(),
            user_id: world.user_id.to_string(),
            tenant_id: world.tenant_id.to_string(),
            client_id: connection.client.id.clone(),
            scope: SCOPE.to_owned(),
            granted_at: Utc::now(),
            revoked_at: None,
        })
        .await
        .unwrap();
    world
        .resources
        .account_service()
        .grant_standings()
        .forget(
            world.user_id,
            &world.tenant_id.to_string(),
            &connection.client.id,
        )
        .await;

    assert_eq!(
        mcp_status(&world, &connection.access_token).await,
        401,
        "the token the athlete revoked stays revoked"
    );
    let fresh = world
        .resources
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &world.resources.auth.jwks_manager,
            &DelegatedGrant {
                client_id: &connection.client.id,
                scopes: &SCOPE.split(' ').map(str::to_owned).collect::<Vec<_>>(),
                audience: None,
            },
            &world.user_id,
            &[],
            Some(world.tenant_id.to_string()),
        )
        .unwrap();
    assert_eq!(mcp_status(&world, &fresh).await, 200, "the new consent's");
}

/// The window: a change nobody forgot reaches a reader when the entry expires,
/// and until then costs no read.
#[tokio::test]
async fn a_standing_is_held_for_its_window_and_forgotten_on_demand() {
    let world = world().await;
    let connection = connect(&world).await;
    let cache = Arc::new(
        Cache::new(CacheConfig {
            enable_background_cleanup: false,
            ..CacheConfig::default()
        })
        .await
        .unwrap(),
    );
    let repository = world.resources.common.repos.oauth2_server.clone();
    let tenant = world.tenant_id.to_string();
    let client = connection.client.id.as_str();
    let standings =
        GrantStandings::new(repository.clone(), cache).with_ttl(StdDuration::from_secs(1));
    let read = || standings.read(world.user_id, &tenant, client);
    assert!(read().await.unwrap().active);

    assert!(repository
        .revoke_client_grant(&connection.grant_id, &world.user_id.to_string(), &tenant)
        .await
        .unwrap());
    assert!(
        read().await.unwrap().active,
        "held inside its window, so no read reached the database"
    );

    standings.forget(world.user_id, &tenant, client).await;
    assert!(!read().await.unwrap().active, "forgotten, so read afresh");

    // An entry nobody forgets ends with its window.
    repository
        .store_client_grant(&OAuthClientGrant {
            id: Uuid::new_v4().to_string(),
            user_id: world.user_id.to_string(),
            tenant_id: tenant.clone(),
            client_id: client.to_owned(),
            scope: SCOPE.to_owned(),
            granted_at: Utc::now(),
            revoked_at: None,
        })
        .await
        .unwrap();
    assert!(!read().await.unwrap().active, "still inside the window");
    sleep(StdDuration::from_millis(1_200)).await;
    assert!(read().await.unwrap().active, "the window is over");
}

/// A grant that cannot be read refuses the call, as a quota that cannot be
/// read does.
#[tokio::test]
async fn a_grant_that_cannot_be_read_refuses_the_call() {
    let world = world().await;
    let connection = connect(&world).await;
    assert_eq!(mcp_status(&world, &connection.access_token).await, 200);
    world
        .resources
        .account_service()
        .grant_standings()
        .forget(
            world.user_id,
            &world.tenant_id.to_string(),
            &connection.client.id,
        )
        .await;
    match world.resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query("DROP TABLE oauth_client_grants")
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query("DROP TABLE oauth_client_grants CASCADE")
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
    assert_ne!(mcp_status(&world, &connection.access_token).await, 200);
}

/// Suspending the athlete stops the delegated token at once, without any
/// revocation; lifting it serves the token again, since nothing was withdrawn.
#[tokio::test]
async fn a_suspended_athletes_token_is_refused_at_mcp() {
    let world = world().await;
    let connection = connect(&world).await;
    assert_eq!(mcp_status(&world, &connection.access_token).await, 200);
    let users = &world.resources.common.repos.users;

    users
        .update_status(world.user_id, UserStatus::Suspended, None)
        .await
        .unwrap();
    let refused = mcp_status(&world, &connection.access_token).await;
    assert!(
        refused == 401 || refused == 403,
        "a suspended athlete's token is refused, not served: {refused}"
    );

    users
        .update_status(world.user_id, UserStatus::Active, None)
        .await
        .unwrap();
    assert_eq!(mcp_status(&world, &connection.access_token).await, 200);
}

/// A connection is validated as `/mcp` would serve it, so this route does not
/// call a revoked token good.
#[tokio::test]
async fn validate_and_refresh_calls_a_revoked_token_invalid() {
    let world = world().await;
    let connection = connect(&world).await;
    let verdict = |token: String| {
        AxumTestRequest::post("/oauth2/validate-and-refresh")
            .header("authorization", &format!("Bearer {token}"))
            .json(&json!({}))
            .send(oauth2_routes(&world.resources))
    };
    let before: Value = verdict(connection.access_token.clone()).await.json();
    assert_eq!(before["status"], "valid", "{before}");

    revoke_in_connected_apps(&world, &connection.grant_id).await;

    let after: Value = verdict(connection.access_token.clone()).await.json();
    assert_eq!(after["status"], "invalid", "{after}");
    assert_eq!(after["reason"], "grant_revoked", "{after}");
}

/// The screen names the app and where it is, not its opaque client id.
#[tokio::test]
async fn connected_apps_lists_a_clients_name_and_host() {
    let world = world().await;
    let connection = connect(&world).await;
    let session = common::generate_test_token(&world.resources, &world.user).await;
    let listed: Value = AxumTestRequest::get("/api/me/oauth-grants")
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&world.resources)))
        .await
        .json();
    let grant = &listed["grants"][0];
    assert_eq!(grant["client_id"], connection.client.id.as_str());
    assert_eq!(grant["client_name"], "Revocation client");
    assert_eq!(grant["client_host"], "app.example.com");
}
