// ABOUTME: A delegated OAuth grant is refused on every REST path and still works on /mcp within its scopes, however wide it is
// ABOUTME: Pins each refusal beside the session's answer on the same route; the client a token names decides before its grant
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

use crate::common;
use crate::helpers;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::first_party_sign_in::{
    authorize_uri, first_party_router, fresh_state, location, FirstPartyClient, Pkce, DEFAULT_PEER,
};
use jsonwebtoken::dangerous::insecure_decode;
use jsonwebtoken::{encode, Algorithm, Header};
use pierre_auth::auth::{Claims, DelegatedGrant};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::first_party::{MOBILE_CLIENT_ID, WEB_CLIENT_ID};
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::errors::ErrorCode;
use pierre_core::models::{SessionRefreshToken, Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::permissions::UserRole;
use pierre_mcp_server::config::routes::{admin_config_router, AdminConfigState};
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::api_keys::ApiKeyRoutes;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_mcp_server::routes::imports::import_routes;
use pierre_mcp_server::routes::mcp::McpRoutes;
use pierre_mcp_server::routes::tenants::TenantRoutes;
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes, ImpersonationRoutes};
use pierre_routes_auth::AuthService;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use secrecy::ExposeSecret;
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
            &DelegatedGrant {
                client_id: "mcp_client_test",
                scopes: &scopes,
                audience: None,
            },
            &athlete.user_id,
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
        Arc::clone(&resources.common.repos.users),
        Arc::clone(&resources.auth.jwks_manager),
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
        .authenticate_scoped_request(
            Some(&token),
            &resources.common.config.oauth2_server.mcp_resources(),
        )
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
    let body = json!({ "name": "escalation" });

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

/// The admin routes' context, as the composition root builds it.
fn admin_api_context(resources: &Arc<ServerContext>) -> AdminApiContext {
    AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: resources.auth.admin_jwt_secret.to_string(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: resources.fitness.harness_config_registry.clone(),
        guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
        prompt_registry: resources.mcp.prompt_registry.clone(),
        tool_description_registry: resources.mcp.tool_description_registry.clone(),
        evidence_registry: resources.mcp.evidence_registry.clone(),
        messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
        cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
        persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
        training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
        contremaitre_config: None,
        app_behavior: resources.common.config.app_behavior.clone(),
    })
}

/// The console's admin mount, as the composition root builds it.
fn console_admin(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::cookie_admin_routes::<ServerContext>(admin_api_context(resources), resources)
}

#[tokio::test]
#[serial]
async fn a_delegated_grant_from_an_operator_cannot_reach_admin_routes() {
    let resources = create_test_server_resources().await.unwrap();
    let operator = athlete(
        &resources,
        "delegation-operator@example.com",
        UserRole::Admin,
    )
    .await;
    let token = delegated(&resources, &operator, &widest_delegation());

    let response = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &token)
        .send(console_admin(&resources))
        .await;
    assert_refused(response, "GET /api/admin/tokens");

    let response = AxumTestRequest::get("/api/admin/config")
        .header("authorization", &token)
        .send(admin_config(&resources))
        .await;
    assert_refused(response, "GET /api/admin/config");

    let response = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &operator.session)
        .send(console_admin(&resources))
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
        .header("cookie", &format!("{}={token}", auth_cookie_name()))
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

    // The athlete's own session is the app signed in, not an integration's
    // credential: `/mcp` refuses it as an invalid token (carnet#768).
    let response = call_get_training_plan(&resources, &athlete.session).await;
    assert_eq!(
        response.status(),
        401,
        "a first-party session token is refused on /mcp"
    );
    let challenge = response
        .header("www-authenticate")
        .unwrap_or_default()
        .to_owned();
    assert!(
        challenge.contains("error=\"invalid_token\""),
        "the refusal sends the client to authorize: {challenge}"
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
        .header("cookie", &format!("{}={token}", auth_cookie_name()))
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
        .header(
            "cookie",
            &format!("{}={}", auth_cookie_name(), athlete.session_token),
        )
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "the athlete's own session reaches the consent screen"
    );
    let page = response.text();
    assert!(
        page.contains("(fitness:read)</li>") && page.contains("(profile:read)</li>"),
        "the consent screen shows the default grant the code will carry"
    );
}

// ============================================================================
// Session or delegation: the client a token names, then its grant
// ============================================================================
//
// No token the authorization server mints for an application holds `admin`
// today, so none holds all five scopes. These tests mint what it does not, to
// pin that such a token would still be the application's and never the
// athlete's session (carnet#1511).

/// The refusal a first-party session meets on the MCP and A2A entry point.
const SESSION_REFUSAL: &str =
    "A Dravr sign-in session is not accepted by the MCP and A2A endpoints; \
     authorize this client with OAuth or use an API key";

/// `claims` signed with the server's active key: how a test mints a token no
/// minter issues.
fn signed(resources: &Arc<ServerContext>, claims: &Claims) -> String {
    let key = resources.auth.jwks_manager.get_active_key().unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(&header, claims, &key.encoding_key().unwrap()).unwrap()
}

/// `token` naming no client: as one minted before the `client_id` claim
/// existed.
fn naming_no_client(resources: &Arc<ServerContext>, token: &str) -> String {
    let mut claims = insecure_decode::<Claims>(token).unwrap().claims;
    claims.client_id = None;
    signed(resources, &claims)
}

/// The session one of Dravr's own apps holds for `athlete`, naming
/// `client_id` when it signed in through one.
async fn first_party_session(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    client_id: Option<&str>,
) -> String {
    let repos = &resources.common.repos;
    let user = repos
        .users
        .get_global(athlete.user_id)
        .await
        .unwrap()
        .unwrap();
    let tenant = repos.tenants.first_for_user(user.id).await.unwrap();
    resources
        .auth
        .auth_manager
        .generate_client_session_token(
            &user,
            &resources.auth.jwks_manager,
            tenant.map(|tenant| tenant.to_string()),
            client_id,
        )
        .unwrap()
}

/// `GET /api/auth/session`, which answers a session with a fresh one.
async fn restore_session(resources: &Arc<ServerContext>, token: &str) -> AxumTestResponse {
    AxumTestRequest::get("/api/auth/session")
        .header("authorization", &format!("Bearer {token}"))
        .send(first_party_router(resources, DEFAULT_PEER))
        .await
}

/// Record that `athlete` proved their address: the waiting screen's route
/// answers 400 until then, whoever asks.
async fn confirm_email(resources: &Arc<ServerContext>, athlete: &Athlete) {
    resources
        .common
        .repos
        .email_verification
        .mark_verified(athlete.user_id)
        .await
        .unwrap();
}

/// `POST /api/user/import-tombstones/clear`, one of the routes the waiting
/// screen reaches with a session the status gate would refuse.
async fn clear_tombstones(resources: &Arc<ServerContext>, token: &str) -> AxumTestResponse {
    AxumTestRequest::post("/api/user/import-tombstones/clear")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({}))
        .send(first_party_router(resources, DEFAULT_PEER))
        .await
}

/// Where the consent screen sends a request for Dravr's web app.
#[derive(Debug, PartialEq, Eq)]
enum WebSignIn {
    /// Back to the app with an authorization code: the cookie was a session
    Code,
    /// To the login page: the cookie was no session
    LogIn,
    /// Anywhere else
    Elsewhere,
}

/// `GET /oauth2/authorize` for Dravr's web app with `token` in the web app's
/// session cookie: where the consent screen takes a session for its own. The
/// redirect is classified, never returned, so a failed assertion prints no
/// authorization code.
async fn sign_in_to_web_app(resources: &Arc<ServerContext>, token: &str) -> WebSignIn {
    sign_in_to_web_app_under(resources, &auth_cookie_name(), token).await
}

/// The authorization server's own sign-in cookie on this fixture's plain
/// HTTP issuer.
const AUTHORIZATION_SERVER_COOKIE: &str = "pierre_session";

/// [`sign_in_to_web_app`] with `token` in the cookie named `cookie`.
async fn sign_in_to_web_app_under(
    resources: &Arc<ServerContext>,
    cookie: &str,
    token: &str,
) -> WebSignIn {
    let redirect_uri = FirstPartyClient::Web.redirect_uri(&resources.common.config.oauth2_server);
    let response = AxumTestRequest::get(&authorize_uri(
        WEB_CLIENT_ID,
        &redirect_uri,
        &Pkce::generate().challenge,
        &fresh_state(),
    ))
    .header("cookie", &format!("{cookie}={token}"))
    .send(first_party_router(resources, DEFAULT_PEER))
    .await;
    let sent_to = location(&response);
    if sent_to.starts_with(&redirect_uri) && sent_to.contains("code=") {
        WebSignIn::Code
    } else if sent_to.starts_with("/oauth2/login") {
        WebSignIn::LogIn
    } else {
        WebSignIn::Elsewhere
    }
}

/// All five scopes on a token an application holds: no REST route takes it
/// for the athlete's session. Restoring a session from it would hand the
/// application a first-party session in the athlete's name.
#[tokio::test]
#[serial]
async fn no_rest_route_takes_a_delegation_holding_every_scope_for_a_session() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "delegation-wide@example.com", UserRole::Admin).await;
    let token = delegated_token(&resources, &athlete, &OAuthScope::self_grant());

    let restored = restore_session(&resources, &token).await;
    assert!(
        restored.header("set-cookie").is_none(),
        "no session cookie is set for an application's token"
    );
    assert_refused(restored, "GET /api/auth/session");

    let refused = resources
        .auth
        .auth_middleware
        .authenticate_request(Some(&format!("Bearer {token}")))
        .await
        .expect_err("the REST entry point must refuse a delegation, however wide");
    assert_eq!(
        refused.code,
        ErrorCode::PermissionDenied,
        "refused with 403, not sent to sign in again"
    );
    assert_eq!(
        refused.sanitized_message(),
        REFUSAL,
        "refused as a delegation, not by some other gate"
    );
}

/// How a [`Reached`] route is asked.
enum Ask {
    Get,
    Post(Value),
    Delete,
}

/// One REST route an admin's own session reaches.
struct Reached {
    /// The route, for a failure message.
    what: &'static str,
    /// The router that mounts it, as the composition root builds it.
    router: fn(&Arc<ServerContext>) -> axum::Router,
    /// Its path on that router.
    path: String,
    /// Its method, and the body a write sends.
    ask: Ask,
    /// What the admin's own session is answered.
    session_status: u16,
}

impl Reached {
    /// The route asked under `bearer`.
    async fn asked(&self, resources: &Arc<ServerContext>, bearer: &str) -> AxumTestResponse {
        let request = match &self.ask {
            Ask::Get => AxumTestRequest::get(&self.path),
            Ask::Post(body) => AxumTestRequest::post(&self.path).json(body),
            Ask::Delete => AxumTestRequest::delete(&self.path),
        };
        request
            .header("authorization", bearer)
            .send((self.router)(resources))
            .await
    }
}

/// Every route below acts with its caller's whole authority and reads no
/// scope: the admin console, the impersonation routes, the data export,
/// imports, the tenant switch (which mints a session) and the plan card's
/// writes. All five scopes on an application's token open none of them, and
/// the admin's own session is answered on each as it was, so each refusal is
/// the token's and not the route's.
#[tokio::test]
#[serial]
async fn no_route_an_admins_session_reaches_takes_a_delegation_holding_every_scope() {
    let resources = create_test_server_resources().await.unwrap();
    let admin = athlete(
        &resources,
        "delegation-wide-admin@example.com",
        UserRole::Admin,
    )
    .await;
    let member = athlete(
        &resources,
        "delegation-wide-member@example.com",
        UserRole::User,
    )
    .await;
    let wide = delegated(&resources, &admin, &OAuthScope::self_grant());
    let session = format!(
        "Bearer {}",
        first_party_session(&resources, &admin, Some(WEB_CLIENT_ID)).await
    );
    let tenant = resources
        .common
        .repos
        .tenants
        .first_for_user(admin.user_id)
        .await
        .unwrap()
        .expect("the fixture gives the admin a tenant");
    // A thread nobody opened: the session is answered that it is not there,
    // which is past the credential check.
    let plan_card = format!("/api/chat/conversations/{}/subject-plan", Uuid::new_v4());

    let reached = [
        Reached {
            what: "GET /api/admin/tokens",
            router: console_admin,
            path: "/api/admin/tokens".to_owned(),
            ask: Ask::Get,
            session_status: 200,
        },
        Reached {
            what: "GET /api/admin/config",
            router: admin_config,
            path: "/api/admin/config".to_owned(),
            ask: Ask::Get,
            session_status: 200,
        },
        Reached {
            what: "POST /api/admin/impersonate",
            router: |resources| ImpersonationRoutes::routes(Arc::clone(resources)),
            path: "/api/admin/impersonate".to_owned(),
            ask: Ask::Post(json!({
                "target_user_id": member.user_id.to_string(),
                "reason": "Ticket 1511",
            })),
            session_status: 200,
        },
        Reached {
            what: "GET /api/admin/impersonate/sessions",
            router: |resources| ImpersonationRoutes::routes(Arc::clone(resources)),
            path: "/api/admin/impersonate/sessions".to_owned(),
            ask: Ask::Get,
            session_status: 200,
        },
        Reached {
            what: "POST /api/admin/impersonate/end",
            router: |resources| ImpersonationRoutes::routes(Arc::clone(resources)),
            path: "/api/admin/impersonate/end".to_owned(),
            ask: Ask::Post(json!({})),
            session_status: 200,
        },
        Reached {
            what: "GET /api/user/export",
            router: |resources| first_party_router(resources, DEFAULT_PEER),
            path: "/api/user/export".to_owned(),
            ask: Ask::Get,
            session_status: 200,
        },
        Reached {
            what: "GET /api/imports",
            router: |resources| import_routes().with_state(Arc::clone(resources)),
            path: "/api/imports".to_owned(),
            ask: Ask::Get,
            session_status: 200,
        },
        Reached {
            what: "POST /tenants/switch",
            router: |resources| TenantRoutes::routes(Arc::clone(resources)),
            path: "/tenants/switch".to_owned(),
            ask: Ask::Post(json!({ "tenant_id": tenant.to_string() })),
            session_status: 200,
        },
        Reached {
            what: "POST …/subject-plan/publish",
            router: |resources| ChatRoutes::routes(Arc::clone(resources)),
            path: format!("{plan_card}/publish"),
            ask: Ask::Post(json!({})),
            session_status: 404,
        },
        Reached {
            what: "DELETE …/subject-plan/draft",
            router: |resources| ChatRoutes::routes(Arc::clone(resources)),
            path: format!("{plan_card}/draft"),
            ask: Ask::Delete,
            session_status: 404,
        },
    ];

    for route in &reached {
        assert_refused(route.asked(&resources, &wide).await, route.what);
        // The status alone: the tenant switch answers a session with a token.
        assert_eq!(
            route.asked(&resources, &session).await.status(),
            route.session_status,
            "{}: the admin's own session is answered as before",
            route.what
        );
    }

    // The admin API takes an admin token, a credential of its own kind, and
    // no user's: an application's token is no more one for holding `admin`.
    let admin_api = AxumTestRequest::get("/admin/token-info")
        .header("authorization", &wide)
        .send(AdminRoutes::routes(admin_api_context(&resources)))
        .await;
    assert_eq!(
        admin_api.status(),
        401,
        "GET /admin/token-info: an application's token is not an admin token"
    );
}

/// `/mcp` and A2A carry the same token as the delegation it is, and do not
/// refuse it as a session lifted from the app.
#[tokio::test]
#[serial]
async fn the_scoped_entry_point_carries_a_delegation_holding_every_scope() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "delegation-scoped@example.com", UserRole::Admin).await;
    let token = delegated(&resources, &athlete, &OAuthScope::self_grant());

    let scoped = resources
        .auth
        .auth_middleware
        .authenticate_scoped_request(
            Some(&token),
            &resources.common.config.oauth2_server.mcp_resources(),
        )
        .await
        .expect("the scoped entry point carries a delegation, however wide");
    assert_eq!(
        scoped.user_id, athlete.user_id,
        "the delegation acts for the athlete who consented"
    );
    assert_eq!(
        scoped.scopes,
        OAuthScope::self_grant(),
        "the grant is carried as issued, for the scope gate to read"
    );
    assert_eq!(
        scoped.client_id.as_deref(),
        Some("mcp_client_test"),
        "the application the token names is carried with it"
    );
}

/// The consent screen validates the web app's cookie without the middleware.
/// Taken for a session there, the token would sign its application in to
/// Dravr's own app as the athlete.
#[tokio::test]
#[serial]
async fn the_consent_screen_takes_no_delegation_holding_every_scope_for_a_session() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "delegation-bridge@example.com", UserRole::User).await;
    let token = delegated_token(&resources, &athlete, &OAuthScope::self_grant());

    assert_eq!(
        sign_in_to_web_app(&resources, &token).await,
        WebSignIn::LogIn,
        "no session: the application is sent to log in"
    );
    assert_eq!(
        sign_in_to_web_app(&resources, &athlete.session_token).await,
        WebSignIn::Code,
        "the athlete's own session gets its code"
    );
}

/// The authorization server's own cookie is read by the same rule: its
/// sign-in token, named for an application, is not its session.
#[tokio::test]
#[serial]
async fn the_consent_screen_takes_no_applications_token_in_its_own_cookie() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(
        &resources,
        "delegation-own-cookie@example.com",
        UserRole::User,
    )
    .await;
    let repos = &resources.common.repos;
    let user = repos
        .users
        .get_global(athlete.user_id)
        .await
        .unwrap()
        .unwrap();
    let tenant = repos.tenants.first_for_user(user.id).await.unwrap();
    let own = resources
        .auth
        .auth_manager
        .generate_authorization_session_token(
            &user,
            &resources.auth.jwks_manager,
            tenant.map(|tenant| tenant.to_string()),
        )
        .unwrap();
    let mut held_by_an_application = insecure_decode::<Claims>(&own).unwrap().claims;
    held_by_an_application.client_id = Some("mcp_client_test".to_owned());

    assert_eq!(
        sign_in_to_web_app_under(
            &resources,
            AUTHORIZATION_SERVER_COOKIE,
            &signed(&resources, &held_by_an_application)
        )
        .await,
        WebSignIn::LogIn,
        "no session: the application is sent to log in"
    );
    assert_eq!(
        sign_in_to_web_app_under(&resources, AUTHORIZATION_SERVER_COOKIE, &own).await,
        WebSignIn::Code,
        "the authorization server's own sign-in gets its code"
    );
}

/// Nor do the routes the waiting screen reaches, which validate the session
/// themselves because a Pending account cannot pass the middleware.
#[tokio::test]
#[serial]
async fn the_waiting_screen_takes_no_delegation_holding_every_scope_for_a_session() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "delegation-waiting@example.com", UserRole::User).await;
    let token = delegated_token(&resources, &athlete, &OAuthScope::self_grant());
    // With the address confirmed the route serves a session, so a 401 is the
    // session check and nothing after it.
    confirm_email(&resources, &athlete).await;

    let waiting = clear_tombstones(&resources, &token).await;
    assert_eq!(
        waiting.status(),
        401,
        "the waiting screen's route takes no delegation: {}",
        waiting.body_text()
    );
    let waiting = clear_tombstones(&resources, &athlete.session_token).await;
    assert_eq!(
        waiting.status(),
        200,
        "the waiting screen's route serves the athlete's own session: {}",
        waiting.body_text()
    );
}

/// A session is Dravr's own app signed in, whichever app it names and when
/// it names none (a token from before the claim, and an impersonation's
/// shape): REST serves it, `/mcp` refuses it as lifted from the app, and the
/// consent and waiting screens take it.
#[tokio::test]
#[serial]
async fn a_first_party_session_is_a_session_whichever_app_it_names() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "session-apps@example.com", UserRole::User).await;
    let auth = &resources.auth;
    let middleware = &auth.auth_middleware;
    confirm_email(&resources, &athlete).await;

    for client_id in [Some(WEB_CLIENT_ID), Some(MOBILE_CLIENT_ID), None] {
        let token = first_party_session(&resources, &athlete, client_id).await;
        let bearer = format!("Bearer {token}");

        let session = middleware
            .authenticate_request(Some(&bearer))
            .await
            .unwrap_or_else(|e| panic!("{client_id:?}: REST serves a session: {e}"));
        assert_eq!(session.scopes, OAuthScope::self_grant(), "{client_id:?}");
        assert_eq!(
            session.client_id.as_deref(),
            client_id,
            "the session still names the app it was signed in to"
        );

        let refused = middleware
            .authenticate_scoped_request(
                Some(&bearer),
                &resources.common.config.oauth2_server.mcp_resources(),
            )
            .await
            .expect_err("a session is not an integration's credential");
        assert_eq!(refused.code, ErrorCode::AuthInvalid, "{client_id:?}");
        assert_eq!(refused.message, SESSION_REFUSAL, "{client_id:?}");

        auth.auth_manager
            .validate_session_token(&token, &auth.jwks_manager)
            .unwrap_or_else(|e| panic!("{client_id:?}: a session validates as one: {e}"));
        assert_eq!(
            sign_in_to_web_app(&resources, &token).await,
            WebSignIn::Code,
            "{client_id:?}: a session gets its code"
        );
        assert_eq!(
            restore_session(&resources, &token).await.status(),
            200,
            "{client_id:?}: a session is restored"
        );
        assert_eq!(
            clear_tombstones(&resources, &token).await.status(),
            200,
            "{client_id:?}: the waiting screen's route serves a session"
        );
    }
}

/// A refresh family opened before sessions named a client names none. The
/// session minter refuses a third-party client; no client is not one, so the
/// family still refreshes into a session, naming none.
#[tokio::test]
#[serial]
async fn a_refresh_family_naming_no_client_still_mints_a_session() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "family-no-client@example.com", UserRole::User).await;
    let presented = "a-family-opened-before-the-claim";
    let now = chrono::Utc::now();
    resources
        .common
        .repos
        .session_refresh_tokens
        .store_token(
            presented,
            &SessionRefreshToken {
                family_id: Uuid::new_v4().to_string(),
                user_id: athlete.user_id,
                tenant_id: None,
                client_id: None,
                created_at: now,
                expires_at: now + chrono::Duration::days(30),
            },
        )
        .await
        .unwrap();

    let service = AuthService::new(
        resources.auth.auth_manager.clone(),
        resources.auth.jwks_manager.clone(),
        resources.common.config.clone(),
        resources.data(),
    );
    // Matched, never unwrapped: the refreshed session holds tokens.
    let refreshed = match service.refresh_session(presented).await {
        Ok(refreshed) => refreshed,
        Err(refused) => panic!("a family naming no client refreshes: {refused}"),
    };
    let jwt = refreshed.login.jwt_token.expect("a refresh mints a JWT");
    let claims = resources
        .auth
        .auth_manager
        .validate_session_token(jwt.expose_secret(), &resources.auth.jwks_manager)
        .unwrap_or_else(|e| panic!("the refreshed token is a session: {e}"));
    assert_eq!(
        claims.client_id, None,
        "the refreshed session names no client, as its family does"
    );
}

/// A narrow grant naming no client, as a delegated token minted before the
/// `client_id` claim existed: still a delegation everywhere.
#[tokio::test]
#[serial]
async fn a_narrow_grant_naming_no_client_stays_a_delegation() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "delegation-unnamed@example.com", UserRole::User).await;
    let auth = &resources.auth;
    let middleware = &auth.auth_middleware;
    let token = naming_no_client(
        &resources,
        &delegated_token(&resources, &athlete, &widest_delegation()),
    );
    let bearer = format!("Bearer {token}");

    let refused = middleware
        .authenticate_request(Some(&bearer))
        .await
        .expect_err("the REST entry point must refuse a narrow grant");
    assert_eq!(
        refused.sanitized_message(),
        REFUSAL,
        "refused as a delegation, not by some other gate"
    );

    let scoped = middleware
        .authenticate_scoped_request(
            Some(&bearer),
            &resources.common.config.oauth2_server.mcp_resources(),
        )
        .await
        .expect("the scoped entry point carries it");
    assert_eq!(
        scoped.scopes,
        widest_delegation(),
        "the grant is carried as issued, for the scope gate to read"
    );
    assert_eq!(scoped.client_id, None, "the token names no client");

    auth.auth_manager
        .validate_session_token(&token, &auth.jwks_manager)
        .expect_err("a narrow grant is not a session");
    assert_eq!(
        sign_in_to_web_app(&resources, &token).await,
        WebSignIn::LogIn,
        "no session: sent to log in"
    );
    assert_eq!(
        clear_tombstones(&resources, &token).await.status(),
        401,
        "the waiting screen's route takes no narrow grant for a session"
    );
}
