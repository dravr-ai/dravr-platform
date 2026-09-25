// ABOUTME: A delegated OAuth grant is refused on every REST path and still works on /mcp within its scopes
// ABOUTME: Pins 403 on API-key minting, chat, admin, an ordinary read and the consent screen; the session is unchanged
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # A delegation is only as wide as the path that reads it
//!
//! The `OAuth2` authorization server mints access tokens for third-party
//! applications, narrowed to the scopes the athlete consented to. Only the
//! MCP (and A2A) tool dispatch reads those scopes. Every REST handler acts
//! with the athlete's whole authority, so a `fitness:read` token accepted
//! there could mint a full-grant API key that outlives the grant, drive a
//! chat turn that runs under the self grant, and reach the admin console
//! when the athlete is an operator.
//!
//! Each refusal below is paired with the same request under the athlete's
//! own session, answered as before: a gate that refused everything would
//! pass the refusals alone.

mod common;
mod helpers;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::errors::ErrorCode;
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::permissions::UserRole;
use pierre_mcp_server::config::routes::{admin_config_router, AdminConfigState};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::api_keys::ApiKeyRoutes;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_mcp_server::routes::mcp::McpRoutes;
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use pierre_routes_web_admin::WebAdminRoutes;
use serde_json::{json, Value};
use serial_test::serial;
use uuid::Uuid;

/// The sentence a refused delegation carries — `PermissionDenied`, so it
/// reaches the client verbatim and tells the application where its token is
/// accepted.
const REFUSAL: &str =
    "This access token was delegated to an application and is accepted only by the MCP and A2A endpoints";

/// An athlete with a tenant, and the first-party session the web app holds.
struct Athlete {
    user_id: Uuid,
    /// `"Bearer <session jwt>"`.
    session: String,
    /// The raw session JWT, for the cookie form.
    session_token: String,
}

async fn athlete(resources: &Arc<ServerContext>, email: &str, role: UserRole) -> Athlete {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap();
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Delegator".to_owned()),
    );
    user.is_admin = role.is_admin_or_higher();
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    let user_id = user.id;
    let repos = &resources.common.repos;
    repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("Tenant for {email}"),
            slug: format!("tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();

    let session_token = generate_test_token(resources, &user).await;
    Athlete {
        user_id,
        session: format!("Bearer {session_token}"),
        session_token,
    }
}

/// The token the authorization server's token endpoint mints for a
/// third-party application — the same call, with no active tenant, exactly as
/// `OAuth2AuthorizationServer::generate_access_token` makes it.
fn delegated_token(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    grant: &[OAuthScope],
) -> String {
    let scopes: Vec<String> = grant
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect();
    resources
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &resources.auth.jwks_manager,
            &athlete.user_id,
            &scopes,
            &[],
            None,
        )
        .unwrap()
}

fn delegated(resources: &Arc<ServerContext>, athlete: &Athlete, grant: &[OAuthScope]) -> String {
    format!("Bearer {}", delegated_token(resources, athlete, grant))
}

/// Every delegable scope at once: the widest grant an application can hold.
fn widest_delegation() -> Vec<OAuthScope> {
    OAuthScope::ALL
        .into_iter()
        .filter(|scope| scope.is_delegable())
        .collect()
}

fn assert_refused(response: AxumTestResponse, what: &str) {
    assert_eq!(
        response.status(),
        403,
        "{what}: a delegated grant must be refused with 403"
    );
    let body: Value = response.json();
    assert_eq!(
        body["message"], REFUSAL,
        "{what}: the refusal must be the delegation refusal, not some other gate: {body}"
    );
}

fn admin_config(resources: &Arc<ServerContext>) -> axum::Router {
    let service = resources
        .agent
        .admin_config
        .as_ref()
        .expect("test resources initialise the admin config service");
    let admin_auth = AdminAuthService::new(
        Arc::clone(&resources.common.repos.admin),
        Arc::clone(&resources.auth.jwks_manager),
        0,
    );
    let state = Arc::new(AdminConfigState::new(
        Arc::clone(service),
        Arc::clone(resources),
        admin_auth,
    ));
    axum::Router::new().nest("/api/admin/config", admin_config_router(state))
}

// ============================================================================
// The middleware: one refusal, one scoped entry point
// ============================================================================

#[tokio::test]
#[serial]
async fn the_rest_entry_point_refuses_a_delegation_the_scoped_one_carries_it() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(
        &resources,
        "delegation-middleware@example.com",
        UserRole::User,
    )
    .await;
    let middleware = &resources.auth.auth_middleware;
    let token = delegated(&resources, &athlete, &[OAuthScope::FitnessRead]);

    let refused = middleware
        .authenticate_request(Some(&token))
        .await
        .expect_err("the REST entry point must refuse a delegated grant");
    assert_eq!(refused.code, ErrorCode::PermissionDenied);
    assert_eq!(refused.http_status(), 403);
    assert_eq!(refused.sanitized_message(), REFUSAL);

    let scoped = middleware
        .authenticate_scoped_request(Some(&token))
        .await
        .expect("the scoped entry point accepts a delegated grant");
    assert_eq!(scoped.user_id, athlete.user_id);
    assert_eq!(
        scoped.scopes,
        vec![OAuthScope::FitnessRead],
        "the delegation keeps exactly the grant the athlete consented to"
    );

    let session = middleware
        .authenticate_request(Some(&athlete.session))
        .await
        .expect("the athlete's own session still authenticates on REST");
    assert_eq!(session.scopes, OAuthScope::self_grant());
}

// ============================================================================
// REST: credential minting, chat, admin, an ordinary read
// ============================================================================

#[tokio::test]
#[serial]
async fn a_delegated_grant_cannot_mint_an_api_key() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "delegation-keys@example.com", UserRole::User).await;
    let body = json!({ "name": "escalation", "rate_limit_requests": 1000 });

    let response = AxumTestRequest::post("/api/keys")
        .header(
            "authorization",
            &delegated(&resources, &athlete, &[OAuthScope::FitnessRead]),
        )
        .json(&body)
        .send(ApiKeyRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_refused(response, "POST /api/keys");

    let minted = resources
        .common
        .repos
        .api_keys
        .get_for_user(athlete.user_id)
        .await
        .unwrap();
    assert!(
        minted.is_empty(),
        "the refused request minted no key: {minted:?}"
    );

    let response = AxumTestRequest::post("/api/keys")
        .header("authorization", &athlete.session)
        .json(&body)
        .send(ApiKeyRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(
        response.status(),
        201,
        "the athlete's own session still mints a key"
    );
    let created: Value = response.json();
    assert!(
        created["api_key"]
            .as_str()
            .is_some_and(|key| key.starts_with("pk_")),
        "a real key comes back: {created}"
    );
}

#[tokio::test]
#[serial]
async fn a_delegated_grant_cannot_drive_chat() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "delegation-chat@example.com", UserRole::User).await;
    let body = json!({ "title": "Opened by an application" });

    let response = AxumTestRequest::post("/api/chat/conversations")
        .header(
            "authorization",
            &delegated(&resources, &athlete, &widest_delegation()),
        )
        .json(&body)
        .send(ChatRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_refused(response, "POST /api/chat/conversations");

    let response = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &athlete.session)
        .json(&body)
        .send(ChatRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(
        response.status(),
        201,
        "the athlete's own session still opens a conversation"
    );
    let created: Value = response.json();
    assert_eq!(created["title"], "Opened by an application");
}

#[tokio::test]
#[serial]
async fn a_delegated_grant_from_an_operator_cannot_reach_admin_routes() {
    let resources = create_test_server_resources().await.unwrap();
    let operator = athlete(
        &resources,
        "delegation-operator@example.com",
        UserRole::SuperAdmin,
    )
    .await;
    let token = delegated(&resources, &operator, &widest_delegation());

    let response = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &token)
        .send(WebAdminRoutes::routes(resources.web_admin_context()))
        .await;
    assert_refused(response, "GET /api/admin/tokens");

    let response = AxumTestRequest::get("/api/admin/config")
        .header("authorization", &token)
        .send(admin_config(&resources))
        .await;
    assert_refused(response, "GET /api/admin/config");

    let response = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &operator.session)
        .send(WebAdminRoutes::routes(resources.web_admin_context()))
        .await;
    assert_eq!(
        response.status(),
        200,
        "the operator's session still reaches the console"
    );

    let response = AxumTestRequest::get("/api/admin/config")
        .header("authorization", &operator.session)
        .send(admin_config(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "the operator's session still reads the configuration"
    );
}

#[tokio::test]
#[serial]
async fn a_delegated_grant_is_refused_an_ordinary_read_even_in_the_cookie() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "delegation-read@example.com", UserRole::User).await;
    let token = delegated_token(&resources, &athlete, &widest_delegation());

    let response = AxumTestRequest::get("/api/keys")
        .header("authorization", &format!("Bearer {token}"))
        .send(ApiKeyRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_refused(response, "GET /api/keys (header)");

    let response = AxumTestRequest::get("/api/keys")
        .header("cookie", &format!("auth_token={token}"))
        .send(ApiKeyRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_refused(response, "GET /api/keys (cookie)");

    let response = AxumTestRequest::get("/api/keys")
        .header("authorization", &athlete.session)
        .send(ApiKeyRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(
        response.status(),
        200,
        "the athlete's own session still reads"
    );
}

// ============================================================================
// MCP: the path delegated tokens exist for
// ============================================================================

async fn call_get_training_plan(
    resources: &Arc<ServerContext>,
    authorization: &str,
) -> AxumTestResponse {
    AxumTestRequest::post("/mcp")
        .header("authorization", authorization)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get_training_plan", "arguments": {} }
        }))
        .send(McpRoutes::routes(Arc::clone(resources)))
        .await
}

#[tokio::test]
#[serial]
async fn mcp_serves_a_delegated_grant_within_its_scopes() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "delegation-mcp@example.com", UserRole::User).await;

    let response = call_get_training_plan(
        &resources,
        &delegated(&resources, &athlete, &[OAuthScope::FitnessRead]),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "fitness:read covers get_training_plan on /mcp"
    );
    let body: Value = response.json();
    assert_eq!(
        body["result"]["isError"],
        json!(false),
        "the tool ran: {body}"
    );
    let text = body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        text.contains("no active training plan"),
        "the tool's own answer comes back, not a refusal: {text}"
    );

    // Outside its scopes it is refused on the wire with the RFC 6750
    // challenge naming the grant it lacks.
    let response = call_get_training_plan(
        &resources,
        &delegated(&resources, &athlete, &[OAuthScope::ProfileRead]),
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "profile:read does not cover the plan"
    );
    let challenge = response
        .header("www-authenticate")
        .unwrap_or_default()
        .to_owned();
    assert!(
        challenge.contains("error=\"insufficient_scope\"")
            && challenge.contains("scope=\"fitness:read\""),
        "the challenge names the missing grant: {challenge}"
    );

    let response = call_get_training_plan(&resources, &athlete.session).await;
    assert_eq!(
        response.status(),
        200,
        "the athlete's own session is unchanged on /mcp"
    );
}

// ============================================================================
// The consent screen: a delegation is not the athlete's session
// ============================================================================

fn oauth2_routes(resources: &Arc<ServerContext>) -> axum::Router {
    let context = OAuth2Context {
        database: resources.agent.database.clone(),
        oauth2_server: resources.common.repos.oauth2_server.clone(),
        tenants: resources.common.repos.tenants.clone(),
        users: resources.common.repos.users.clone(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        config: Arc::new(resources.common.config.oauth2_server.clone()),
        rate_limiter: Arc::new(OAuth2RateLimiter::from_rate_limit_config(
            resources.common.config.rate_limiting.clone(),
        )),
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))))
}

#[tokio::test]
#[serial]
async fn a_delegated_grant_cannot_stand_in_for_the_session_on_the_consent_screen() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "delegation-consent@example.com", UserRole::User).await;
    let client = ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec!["https://app.example.com/callback".to_owned()],
                client_name: Some("Second application".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap();
    let authorize = format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&state=consent-state&code_challenge={}&code_challenge_method=S256",
        client.client_id,
        urlencoding::encode("https://app.example.com/callback"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
    );

    // A `fitness:read` token placed in the cookie would otherwise approve a
    // grant for another client on the athlete's behalf.
    let token = delegated_token(&resources, &athlete, &[OAuthScope::FitnessRead]);
    let response = AxumTestRequest::get(&authorize)
        .header("cookie", &format!("auth_token={token}"))
        .send(oauth2_routes(&resources))
        .await;
    assert!(
        response.status_code().is_redirection(),
        "no session: the athlete is sent to log in, got {}",
        response.status()
    );
    let location = response.header("location").unwrap_or_default().to_owned();
    assert!(
        location.starts_with("/oauth2/login"),
        "redirected to the login page: {location}"
    );

    let response = AxumTestRequest::get(&authorize)
        .header("cookie", &format!("auth_token={}", athlete.session_token))
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "the athlete's own session reaches the consent screen"
    );
    let page = response.text();
    assert!(
        page.contains("<li>fitness:read</li>") && page.contains("<li>profile:read</li>"),
        "the consent screen shows the default grant the code will carry"
    );
}
