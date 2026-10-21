// ABOUTME: The protected-resource metadata, the /mcp 401 challenge and RFC 8707 resource indicators all name MCP_RESOURCE_URL
// ABOUTME: A resource on authorize and token binds the token's audience, a satellite's included; another is refused
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Before carnet#484 the RFC 9728 document published the issuer as its
//! `resource` and the 401 challenge pointed at `BASE_URL`, so a client dialling
//! the MCP endpoint under a second hostname was told the resource was a host
//! it had not connected to — which RFC 9728 §3.3 makes it reject. Nor did the
//! authorization server read an RFC 8707 `resource` parameter, so no token was
//! ever bound to the resource it was minted for.
//!
//! Every test here runs a deployment whose authorization server and MCP
//! resource are different hosts, which is the case the old single value could
//! not describe.

use crate::common;
use crate::helpers;
use secrecy::ExposeSecret;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use common::{create_test_server_resources_with_config, create_test_tenant};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use jsonwebtoken::dangerous::insecure_decode;
use jsonwebtoken::{encode, Algorithm, Header};
use pierre_auth::auth::{Claims, DelegatedGrant};
use pierre_auth::config::{resolve_issuer_url, resolve_mcp_resource_url, OAuth2ServerConfig};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::constants::service_names::MCP;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::mcp::McpRoutes;
use pierre_mcp_transport::tenant_isolation::extract_tenant_context_internal;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

/// The authorization server's host.
const ISSUER: &str = "https://app.example.test";
/// The MCP resource server's host — deliberately not the issuer's.
const MCP_RESOURCE: &str = "https://mcp.example.test";
/// A resource server this deployment does not serve.
const OTHER_RESOURCE: &str = "https://elsewhere.example.test";
/// A satellite MCP server this authorization server also mints for
/// (`OAUTH2_SATELLITE_RESOURCES`), served under a path.
const SATELLITE_RESOURCE: &str = "https://commere.example.test/mcp";
const REDIRECT: &str = "https://client.example.test/callback";
const STATE: &str = "resource-indicator-state";
const VERIFIER: &str = "resource-indicator-pkce-verifier-0123456789-abcdefghijklmnop";

/// A deployment whose issuer and MCP resource are the two hosts above.
fn split_hosts() -> OAuth2ServerConfig {
    OAuth2ServerConfig {
        issuer_url: ISSUER.to_owned(),
        mcp_resource_url: MCP_RESOURCE.to_owned(),
        ..OAuth2ServerConfig::default()
    }
}

async fn resources_with(oauth2_server: OAuth2ServerConfig) -> Arc<ServerContext> {
    Box::pin(create_test_server_resources_with_config(ServerConfig {
        oauth2_server,
        activity_fetch_limit: 100,
        ..ServerConfig::default()
    }))
    .await
    .unwrap()
}

/// The OAuth routes exactly as the server mounts them: over the context's own
/// `oauth2_server` config slice.
fn oauth2_routes(resources: &Arc<ServerContext>) -> axum::Router {
    let context = OAuth2Context {
        database: resources.agent.database.clone(),
        oauth2_server: resources.common.repos.oauth2_server.clone(),
        tenants: resources.common.repos.tenants.clone(),
        users: resources.common.repos.users.clone(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        config: Arc::new(resources.common.config.oauth2_server.clone()),
        rate_limiter: Arc::new(OAuth2RateLimiter::new(
            None,
            OAuth2RateLimiter::local_window_store(),
            &resources.common.config.rate_limiting,
        )),
        refresh_token_expiry_days: 30,
        csrf_manager: resources.auth.csrf_manager.clone(),
        accounts: resources.account_service(),
        google_sign_in: None,
        strings: Arc::new(MessagingStringsRegistry::new()),
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_484))))
}

fn pkce_challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

/// Register a client for the authorization code grant; `(client_id, secret)`.
async fn register(resources: &Arc<ServerContext>) -> (String, String) {
    let registered = ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![REDIRECT.to_owned()],
                client_name: Some("Resource indicator client".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap();
    (
        registered.client_id,
        registered.client_secret.expose_secret().to_owned(),
    )
}

/// `/oauth2/authorize` for `client_id` with PKCE, naming `resource` if given.
fn authorize_uri(client_id: &str, resource: Option<&str>) -> String {
    let mut uri = format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        urlencoding::encode(STATE),
        pkce_challenge(),
    );
    if let Some(resource) = resource {
        uri.push_str("&resource=");
        uri.push_str(&urlencoding::encode(resource));
    }
    uri
}

/// The query of the redirect a response carries.
fn redirect_query(response: &AxumTestResponse) -> Vec<(String, String)> {
    assert_eq!(
        response.status(),
        303,
        "the answer is a redirect to the client"
    );
    let location = response
        .header("location")
        .expect("a redirect has a Location");
    let url = Url::parse(location).unwrap_or_else(|e| panic!("{location}: {e}"));
    assert!(
        location.starts_with(REDIRECT),
        "redirected to the client: {location}"
    );
    url.query_pairs().into_owned().collect()
}

fn param<'a>(query: &'a [(String, String)], name: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// A tenant member signed in to the authorization server, as the athlete who
/// consents: the `/oauth2/login` cookie's token, which only the authorize flow
/// accepts (carnet#787).
async fn athlete(resources: &Arc<ServerContext>, email: &str) -> (User, String) {
    let (user, _app_session) = create_test_tenant(resources, email).await.unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .map(|tenant| tenant.id.to_string());
    let session = resources
        .auth
        .auth_manager
        .generate_authorization_session_token(&user, &resources.auth.jwks_manager, tenant_id)
        .unwrap();
    (user, session)
}

/// Walk the athlete through authorize and consent; the authorization code.
///
/// `resource` rides on the authorize request and, as the consent page
/// echoes it, on the consent form — which is the round trip that has to keep
/// it for the code to be bound.
async fn authorization_code(
    resources: &Arc<ServerContext>,
    client_id: &str,
    session: &str,
    resource: Option<&str>,
) -> String {
    let cookie = format!("__Host-pierre_session={session}");
    let consent_page = AxumTestRequest::get(&authorize_uri(client_id, resource))
        .header("cookie", &cookie)
        .send(oauth2_routes(resources))
        .await;
    assert_eq!(
        consent_page.status(),
        200,
        "a first authorization asks for consent"
    );
    let page = consent_page.text();
    assert!(
        page.contains(&format!(
            "<input type=\"hidden\" name=\"resource\" value=\"{}\">",
            resource.unwrap_or_default()
        )),
        "the consent form carries the requested resource: {page}"
    );

    // The token the rendered form carries is the one the submission proves.
    let csrf_token = page
        .split("name=\"csrf_token\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("the consent form carries its synchronizer token: {page}"))
        .to_owned();

    let challenge = pkce_challenge();
    let form = [
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", REDIRECT),
        ("state", STATE),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("resource", resource.unwrap_or_default()),
        ("csrf_token", csrf_token.as_str()),
        ("decision", "approve"),
    ];
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &cookie)
        .form(&form)
        .send(oauth2_routes(resources))
        .await;
    let query = redirect_query(&approved);
    assert_eq!(param(&query, "state"), Some(STATE));
    param(&query, "code")
        .unwrap_or_else(|| panic!("an approved consent redirects with a code: {query:?}"))
        .to_owned()
}

/// `POST /oauth2/token` with `fields`.
async fn token_request(
    resources: &Arc<ServerContext>,
    fields: &[(&str, &str)],
) -> AxumTestResponse {
    AxumTestRequest::post("/oauth2/token")
        .form(&fields)
        .send(oauth2_routes(resources))
        .await
}

/// `tools/list` at `/mcp`, with `bearer` when given.
async fn tools_list(resources: &Arc<ServerContext>, bearer: Option<&str>) -> AxumTestResponse {
    let mut request = AxumTestRequest::post("/mcp").json(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    }));
    if let Some(token) = bearer {
        request = request.header("authorization", &format!("Bearer {token}"));
    }
    request.send(McpRoutes::routes(resources.clone())).await
}

/// The challenge a refused `/mcp` request carries, naming the metadata of the
/// configured MCP resource — never the issuer's host.
fn metadata_challenge(error: Option<&str>) -> String {
    let base =
        format!("Bearer resource_metadata=\"{MCP_RESOURCE}/.well-known/oauth-protected-resource\"");
    error.map_or_else(
        || base.clone(),
        |error| format!("{base}, error=\"{error}\""),
    )
}

#[tokio::test]
async fn the_protected_resource_names_mcp_resource_url_and_the_issuer_stays_the_authorization_server(
) {
    let resources = resources_with(split_hosts()).await;

    let response = AxumTestRequest::get("/.well-known/oauth-protected-resource")
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let metadata: Value = response.json();
    assert_eq!(metadata["resource"], MCP_RESOURCE);
    assert_eq!(metadata["authorization_servers"], json!([ISSUER]));

    let discovery: Value = AxumTestRequest::get("/.well-known/oauth-authorization-server")
        .send(oauth2_routes(&resources))
        .await
        .json();
    assert_eq!(
        discovery["issuer"], ISSUER,
        "the authorization server did not move"
    );
}

#[tokio::test]
async fn an_unset_mcp_resource_url_publishes_base_url() {
    // What `OAuth2ServerConfig::from_env` resolves for a deployment that sets
    // BASE_URL and neither OAUTH2_ISSUER_URL nor MCP_RESOURCE_URL.
    let resources = resources_with(OAuth2ServerConfig {
        issuer_url: resolve_issuer_url(None, Some(ISSUER), 8081),
        mcp_resource_url: resolve_mcp_resource_url(None, Some(ISSUER), 8081),
        ..OAuth2ServerConfig::default()
    })
    .await;

    let metadata: Value = AxumTestRequest::get("/.well-known/oauth-protected-resource")
        .send(oauth2_routes(&resources))
        .await
        .json();
    assert_eq!(
        metadata["resource"], ISSUER,
        "unset is today's single-host document"
    );
    assert_eq!(metadata["authorization_servers"], json!([ISSUER]));

    let refused = tools_list(&resources, None).await;
    assert_eq!(refused.status(), 401);
    assert_eq!(
        refused.header("www-authenticate"),
        Some(
            format!("Bearer resource_metadata=\"{ISSUER}/.well-known/oauth-protected-resource\"")
                .as_str()
        )
    );
}

#[tokio::test]
async fn the_mcp_401_challenge_points_at_the_configured_resource() {
    let resources = resources_with(split_hosts()).await;

    let anonymous = tools_list(&resources, None).await;
    assert_eq!(anonymous.status(), 401);
    assert_eq!(
        anonymous.header("www-authenticate"),
        Some(metadata_challenge(None).as_str())
    );

    let bogus = tools_list(&resources, Some("not-a-jwt")).await;
    assert_eq!(bogus.status(), 401);
    assert_eq!(
        bogus.header("www-authenticate"),
        Some(metadata_challenge(Some("invalid_token")).as_str())
    );
}

#[tokio::test]
async fn authorize_and_token_with_the_resource_mint_a_token_bound_to_it() {
    let resources = resources_with(split_hosts()).await;
    let (client_id, client_secret) = register(&resources).await;
    let (_user, session) = athlete(&resources, "bound@example.test").await;

    // The MCP TypeScript SDK sends the published identifier as `URL.href`,
    // with the trailing slash a bare origin gains.
    let code = authorization_code(
        &resources,
        &client_id,
        &session,
        Some("https://mcp.example.test/"),
    )
    .await;

    let exchanged = token_request(
        &resources,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", REDIRECT),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
            ("code_verifier", VERIFIER),
            ("resource", MCP_RESOURCE),
        ],
    )
    .await;
    assert_eq!(exchanged.status(), 200);
    let tokens: Value = exchanged.json();
    let access_token = tokens["access_token"].as_str().unwrap().to_owned();
    let refresh_token = tokens["refresh_token"].as_str().unwrap().to_owned();

    let auth = &resources.auth;
    let claims = auth
        .auth_manager
        .validate_resource_token(&access_token, &auth.jwks_manager, &[MCP_RESOURCE])
        .unwrap();
    assert_eq!(
        claims.aud, MCP_RESOURCE,
        "the token is audience-bound to the resource"
    );
    assert!(
        auth.auth_manager
            .validate_token(&access_token, &auth.jwks_manager)
            .is_err(),
        "a bound token does not pass as the platform audience"
    );

    let served = tools_list(&resources, Some(&access_token)).await;
    assert_eq!(
        served.status(),
        200,
        "the MCP resource accepts its own audience"
    );
    let listed: Value = served.json();
    assert!(
        !listed["result"]["tools"].as_array().unwrap().is_empty(),
        "and serves the catalog: {listed}"
    );

    // The binding lives on the grant, so a refresh that names no resource
    // still mints for the resource the athlete authorized.
    let refreshed = token_request(
        &resources,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh_token),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
        ],
    )
    .await;
    assert_eq!(refreshed.status(), 200);
    let refreshed: Value = refreshed.json();
    let refreshed_claims = auth
        .auth_manager
        .validate_resource_token(
            refreshed["access_token"].as_str().unwrap(),
            &auth.jwks_manager,
            &[MCP_RESOURCE],
        )
        .unwrap();
    assert_eq!(refreshed_claims.aud, MCP_RESOURCE);
}

#[tokio::test]
async fn a_resource_the_server_does_not_serve_is_invalid_target_at_authorize() {
    let resources = resources_with(split_hosts()).await;
    let (client_id, _secret) = register(&resources).await;
    let (_user, session) = athlete(&resources, "authorize-target@example.test").await;

    let refused = AxumTestRequest::get(&authorize_uri(&client_id, Some(OTHER_RESOURCE)))
        .header("cookie", &format!("__Host-pierre_session={session}"))
        .send(oauth2_routes(&resources))
        .await;
    let query = redirect_query(&refused);
    assert_eq!(param(&query, "error"), Some("invalid_target"));
    assert_eq!(param(&query, "state"), Some(STATE));
    assert_eq!(
        param(&query, "code"),
        None,
        "no code for a resource not served here"
    );

    // The issuer's host is not the MCP resource either.
    let issuer_host = AxumTestRequest::get(&authorize_uri(&client_id, Some(ISSUER)))
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(
        param(&redirect_query(&issuer_host), "error"),
        Some("invalid_target")
    );
}

#[tokio::test]
async fn a_resource_the_server_does_not_serve_is_invalid_target_at_token() {
    let resources = resources_with(split_hosts()).await;
    let (client_id, client_secret) = register(&resources).await;
    let (_user, session) = athlete(&resources, "token-target@example.test").await;
    let code = authorization_code(&resources, &client_id, &session, None).await;

    let exchange = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", REDIRECT),
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("code_verifier", VERIFIER),
    ];

    let mut elsewhere = exchange.to_vec();
    elsewhere.push(("resource", OTHER_RESOURCE));
    let refused = token_request(&resources, &elsewhere).await;
    assert_eq!(refused.status(), 400);
    let error: Value = refused.json();
    assert_eq!(error["error"], "invalid_target");
    assert!(error.get("access_token").is_none());

    // The refusal came before the code was spent, and a request that names
    // no resource keeps today's platform audience.
    let exchanged = token_request(&resources, &exchange).await;
    assert_eq!(exchanged.status(), 200);
    let tokens: Value = exchanged.json();
    let claims = resources
        .auth
        .auth_manager
        .validate_token(
            tokens["access_token"].as_str().unwrap(),
            &resources.auth.jwks_manager,
        )
        .unwrap();
    assert_eq!(
        claims.aud, MCP,
        "without a resource the audience is unchanged"
    );
}

#[tokio::test]
async fn a_token_bound_to_another_resource_is_refused_at_mcp() {
    let resources = resources_with(split_hosts()).await;
    let (user, _session) = athlete(&resources, "elsewhere@example.test").await;
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .map(|tenant| tenant.id.to_string());

    let mint = |audience: &str| {
        resources
            .auth
            .auth_manager
            .generate_oauth_access_token(
                &resources.auth.jwks_manager,
                &DelegatedGrant {
                    client_id: "mcp_client_test",
                    scopes: &["fitness:read".to_owned()],
                    audience: Some(audience),
                },
                &user.id,
                &[],
                tenant.clone(),
            )
            .unwrap()
    };

    let elsewhere = tools_list(&resources, Some(&mint(OTHER_RESOURCE))).await;
    assert_eq!(
        elsewhere.status(),
        401,
        "a token for another resource is refused"
    );
    assert_eq!(
        elsewhere.header("www-authenticate"),
        Some(metadata_challenge(Some("invalid_token")).as_str()),
        "and sent to re-authorize for this one"
    );

    // The same grant bound to this resource is served: the refusal is the
    // audience, nothing else about the token.
    let here = tools_list(&resources, Some(&mint(MCP_RESOURCE))).await;
    assert_eq!(here.status(), 200);
}

// ── Satellite resources (carnet#540) ─────────────────────────────────────────

/// The split-host deployment, also minting for [`SATELLITE_RESOURCE`].
fn with_satellite() -> OAuth2ServerConfig {
    OAuth2ServerConfig {
        satellite_resources: vec![SATELLITE_RESOURCE.to_owned()],
        ..split_hosts()
    }
}

/// The claims of `token`, read without checking its audience: a satellite
/// checks the signature itself, and these tests ask only what was minted.
fn minted_claims(token: &str) -> Claims {
    insecure_decode::<Claims>(token).unwrap().claims
}

/// Remove `user`'s membership of `tenant`, as leaving a tenant does.
async fn leave_tenant(resources: &Arc<ServerContext>, user: &User, tenant: TenantId) {
    const LEAVE: &str = "DELETE FROM tenant_users WHERE user_id = $1 AND tenant_id = $2";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(LEAVE)
                .bind(user.id.to_string())
                .bind(tenant.to_string())
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(LEAVE)
                .bind(user.id)
                .bind(tenant.as_uuid())
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// The first tenant `user` belongs to.
async fn first_tenant(resources: &Arc<ServerContext>, user: &User) -> TenantId {
    resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .map(|tenant| tenant.id)
        .unwrap()
}

/// Exchange `code` for `client_id`, naming `resource` when given.
async fn exchange(
    resources: &Arc<ServerContext>,
    (client_id, client_secret): (&str, &str),
    code: &str,
    resource: Option<&str>,
) -> AxumTestResponse {
    let mut fields = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", REDIRECT),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code_verifier", VERIFIER),
    ];
    if let Some(resource) = resource {
        fields.push(("resource", resource));
    }
    token_request(resources, &fields).await
}

/// Refresh `refresh_token` for `client_id`, naming `resource` when given.
async fn refresh(
    resources: &Arc<ServerContext>,
    (client_id, client_secret): (&str, &str),
    refresh_token: &str,
    resource: Option<&str>,
) -> AxumTestResponse {
    let mut fields = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ];
    if let Some(resource) = resource {
        fields.push(("resource", resource));
    }
    token_request(resources, &fields).await
}

/// The OAuth error a refused token request answers with.
fn token_error(response: AxumTestResponse) -> String {
    let status = response.status();
    let body = response.text();
    assert_eq!(status, 400, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    body["error"].as_str().unwrap().to_owned()
}

/// `(access_token, refresh_token)` of a 200 token response.
fn issued(response: AxumTestResponse) -> (String, String) {
    let status = response.status();
    let body = response.text();
    assert_eq!(status, 200, "{body}");
    let tokens: Value = serde_json::from_str(&body).unwrap();
    (
        tokens["access_token"].as_str().unwrap().to_owned(),
        tokens["refresh_token"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn a_satellite_resource_mints_a_tenant_bound_token_the_platform_refuses() {
    let resources = resources_with(with_satellite()).await;
    let (client_id, client_secret) = register(&resources).await;
    let client = (client_id.as_str(), client_secret.as_str());
    let (user, session) = athlete(&resources, "satellite@example.test").await;
    let tenant = first_tenant(&resources, &user).await;

    let code = authorization_code(&resources, &client_id, &session, Some(SATELLITE_RESOURCE)).await;
    let (access_token, refresh_token) = issued(exchange(&resources, client, &code, None).await);

    let claims = minted_claims(&access_token);
    assert_eq!(claims.aud, SATELLITE_RESOURCE, "bound to the satellite");
    assert_eq!(claims.sub, user.id.to_string());
    assert_eq!(
        claims.active_tenant_id,
        Some(tenant.to_string()),
        "the satellite reads the tenant off the token, so the token names it"
    );
    assert_eq!(claims.client_id.as_deref(), Some(client_id.as_str()));

    let refused = tools_list(&resources, Some(&access_token)).await;
    assert_eq!(
        refused.status(),
        401,
        "a token minted for a satellite is never the platform's"
    );

    // A refresh keeps the binding and the tenant.
    let (refreshed_access, refreshed_refresh) =
        issued(refresh(&resources, client, &refresh_token, None).await);
    let refreshed_claims = minted_claims(&refreshed_access);
    assert_eq!(refreshed_claims.aud, SATELLITE_RESOURCE);
    assert_eq!(refreshed_claims.active_tenant_id, Some(tenant.to_string()));

    // Once the athlete has left the tenant the grant is over: no token is
    // minted, since there is no tenant the athlete consented to act in, and
    // the whole refresh family is revoked so it cannot be retried.
    leave_tenant(&resources, &user, tenant).await;
    let after_leaving = refresh(&resources, client, &refreshed_refresh, None).await;
    assert_eq!(token_error(after_leaving), "invalid_grant");
    let stored = resources
        .common
        .repos
        .oauth2_server
        .get_refresh_token_by_value(&refreshed_refresh)
        .await
        .unwrap()
        .unwrap();
    assert!(stored.revoked, "the grant's refresh family is revoked");
}

/// One authorization in a fresh deployment minting for the satellite: the
/// deployment, the client and the code the athlete approved for `resource`.
/// Fresh each time because the flow's `state` is single-use.
async fn authorized(resource: Option<&str>) -> (Arc<ServerContext>, (String, String), String) {
    let resources = resources_with(with_satellite()).await;
    let client = register(&resources).await;
    let (_user, session) = athlete(&resources, "crossing@example.test").await;
    let code = authorization_code(&resources, &client.0, &session, resource).await;
    (resources, client, code)
}

#[tokio::test]
async fn a_grant_never_crosses_between_the_platform_and_a_satellite() {
    // Code exchange: a grant bound to one resource server never mints for the
    // other, and an unbound grant never narrows to the satellite.
    for (bound, asked) in [
        (Some(SATELLITE_RESOURCE), MCP_RESOURCE),
        (Some(MCP_RESOURCE), SATELLITE_RESOURCE),
        (None, SATELLITE_RESOURCE),
    ] {
        let (resources, (id, secret), code) = authorized(bound).await;
        let crossed = exchange(&resources, (&id, &secret), &code, Some(asked)).await;
        assert_eq!(
            token_error(crossed),
            "invalid_target",
            "a code bound to {bound:?} exchanged for {asked}"
        );
    }

    // Refresh: the same, refused before the refresh token is rotated away.
    for (bound, asked) in [
        (SATELLITE_RESOURCE, MCP_RESOURCE),
        (MCP_RESOURCE, SATELLITE_RESOURCE),
    ] {
        let (resources, (id, secret), code) = authorized(Some(bound)).await;
        let client = (id.as_str(), secret.as_str());
        let (_, refresh_token) = issued(exchange(&resources, client, &code, None).await);
        let crossed = refresh(&resources, client, &refresh_token, Some(asked)).await;
        assert_eq!(
            token_error(crossed),
            "invalid_target",
            "a refresh bound to {bound} asking for {asked}"
        );
        let (still_bound, _) = issued(refresh(&resources, client, &refresh_token, None).await);
        assert_eq!(
            minted_claims(&still_bound).aud,
            bound,
            "the refused refresh token still exchanges, for its own resource server"
        );
    }
}

#[tokio::test]
async fn a_platform_grant_ends_when_the_athlete_leaves_its_tenant() {
    let resources = resources_with(split_hosts()).await;
    let (client_id, client_secret) = register(&resources).await;
    let client = (client_id.as_str(), client_secret.as_str());
    let (user, session) = athlete(&resources, "leaving@example.test").await;
    let tenant = first_tenant(&resources, &user).await;

    let code = authorization_code(&resources, &client_id, &session, Some(MCP_RESOURCE)).await;
    let (access_token, refresh_token) = issued(exchange(&resources, client, &code, None).await);
    assert_eq!(
        tools_list(&resources, Some(&access_token)).await.status(),
        200
    );

    leave_tenant(&resources, &user, tenant).await;

    // The access token still in its hour names the tenant, and `/mcp` checks
    // the membership on every request: it is refused, never re-pointed at
    // another of the athlete's tenants.
    let live = tools_list(&resources, Some(&access_token)).await.status();
    assert_ne!(live, 200, "a token naming a tenant left is refused");
    assert_eq!(
        token_error(refresh(&resources, client, &refresh_token, None).await),
        "invalid_grant"
    );
}

#[tokio::test]
async fn a_grant_acts_in_the_tenant_it_was_authorized_in() {
    let resources = resources_with(split_hosts()).await;
    let (client_id, client_secret) = register(&resources).await;
    let client = (client_id.as_str(), client_secret.as_str());
    let (user, _first_session) = athlete(&resources, "two-tenants@example.test").await;
    let first = first_tenant(&resources, &user).await;

    let second = TenantId::from_uuid(Uuid::new_v4());
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: second,
            name: "Second club".to_owned(),
            slug: format!("second-{second}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    assert_eq!(first_tenant(&resources, &user).await, first);
    assert_ne!(first, second);

    // The athlete signs in to the authorization server acting in the second.
    let session = resources
        .auth
        .auth_manager
        .generate_authorization_session_token(
            &user,
            &resources.auth.jwks_manager,
            Some(second.to_string()),
        )
        .unwrap();
    let code = authorization_code(&resources, &client_id, &session, Some(MCP_RESOURCE)).await;
    let (access_token, _) = issued(exchange(&resources, client, &code, None).await);
    assert_eq!(
        minted_claims(&access_token).active_tenant_id,
        Some(second.to_string())
    );

    // `/mcp` resolves the call's tenant from the token exactly so: the
    // scoped authentication its auth hook runs, then the tenant isolation it
    // hands the result to.
    let admitted = resources
        .auth
        .auth_middleware
        .authenticate_scoped_request(
            Some(&format!("Bearer {access_token}")),
            &resources.common.config.oauth2_server.mcp_resources(),
        )
        .await
        .unwrap();
    let context = extract_tenant_context_internal(
        &resources.common.repos,
        admitted.user_id,
        admitted.active_tenant_id.map(TenantId::from_uuid),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        context.tenant_id, second,
        "the authorized tenant, not the first"
    );
    assert_eq!(
        tools_list(&resources, Some(&access_token)).await.status(),
        200
    );
}

#[tokio::test]
async fn validate_and_refresh_keeps_a_satellite_grant_on_its_satellite() {
    let resources = resources_with(with_satellite()).await;
    let (client_id, client_secret) = register(&resources).await;
    let client = (client_id.as_str(), client_secret.as_str());
    let (user, session) = athlete(&resources, "satellite-refresh@example.test").await;
    let tenant = first_tenant(&resources, &user).await;
    let code = authorization_code(&resources, &client_id, &session, Some(SATELLITE_RESOURCE)).await;
    let (access_token, refresh_token) = issued(exchange(&resources, client, &code, None).await);

    let stale = expired(&resources, &access_token);
    let refreshed = validate_and_refresh(&resources, &stale, Some(&refresh_token)).await;
    assert_eq!(refreshed["status"], "refreshed", "{refreshed}");
    let claims = minted_claims(refreshed["access_token"].as_str().unwrap());
    assert_eq!(claims.aud, SATELLITE_RESOURCE);
    assert_eq!(claims.active_tenant_id, Some(tenant.to_string()));

    // After leaving the tenant, validate-and-refresh mints nothing either.
    leave_tenant(&resources, &user, tenant).await;
    let rotated = refreshed["refresh_token"].as_str().unwrap().to_owned();
    let refused = validate_and_refresh(&resources, &stale, Some(&rotated)).await;
    assert_eq!(refused["status"], "invalid", "{refused}");
}

#[tokio::test]
async fn a_satellite_resource_is_refused_where_none_is_configured() {
    let resources = resources_with(split_hosts()).await;
    let (client_id, _secret) = register(&resources).await;
    let (_user, session) = athlete(&resources, "no-satellite@example.test").await;
    let answer = AxumTestRequest::get(&authorize_uri(&client_id, Some(SATELLITE_RESOURCE)))
        .header("cookie", &format!("__Host-pierre_session={session}"))
        .send(oauth2_routes(&resources))
        .await;
    let query = redirect_query(&answer);
    assert_eq!(param(&query, "error"), Some("invalid_target"));
}

// ── Both hosts serve /mcp (carnet#639) ───────────────────────────────────────

/// A deployment publishing `MCP_RESOURCE` while `ISSUER`, its `BASE_URL`,
/// still serves `/mcp` for the clients that were configured against it.
fn both_hosts() -> OAuth2ServerConfig {
    OAuth2ServerConfig {
        mcp_resource_aliases: vec![ISSUER.to_owned()],
        ..split_hosts()
    }
}

/// `tools/list` at `/mcp` as the frontend proxy forwards a client that
/// dialed `host`.
async fn tools_list_via(
    resources: &Arc<ServerContext>,
    host: &str,
    bearer: Option<&str>,
) -> AxumTestResponse {
    let mut request = AxumTestRequest::post("/mcp")
        .header("x-forwarded-host", host)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }));
    if let Some(token) = bearer {
        request = request.header("authorization", &format!("Bearer {token}"));
    }
    request.send(McpRoutes::routes(resources.clone())).await
}

/// The protected-resource document as a client that dialed `host` reads it.
async fn metadata_via(resources: &Arc<ServerContext>, host: Option<&str>) -> Value {
    let mut request = AxumTestRequest::get("/.well-known/oauth-protected-resource");
    if let Some(host) = host {
        request = request.header("x-forwarded-host", host);
    }
    request.send(oauth2_routes(resources)).await.json()
}

#[tokio::test]
async fn each_host_is_told_its_own_resource() {
    let resources = resources_with(both_hosts()).await;

    let via_app = metadata_via(&resources, Some("app.example.test")).await;
    assert_eq!(
        via_app["resource"], ISSUER,
        "RFC 9728 §3.3: the resource is the origin the client dialed"
    );
    assert_eq!(via_app["authorization_servers"], json!([ISSUER]));

    let via_mcp = metadata_via(&resources, Some("mcp.example.test")).await;
    assert_eq!(via_mcp["resource"], MCP_RESOURCE);

    let unforwarded = metadata_via(&resources, None).await;
    assert_eq!(unforwarded["resource"], MCP_RESOURCE);

    let forged = metadata_via(&resources, Some("evil.example.test")).await;
    assert_eq!(
        forged["resource"], MCP_RESOURCE,
        "a host the server does not answer to gets the published resource"
    );
}

#[tokio::test]
async fn the_mcp_401_challenge_names_the_dialed_host() {
    let resources = resources_with(both_hosts()).await;

    let via_app = tools_list_via(&resources, "app.example.test", None).await;
    assert_eq!(via_app.status(), 401);
    assert_eq!(
        via_app.header("www-authenticate"),
        Some(
            format!("Bearer resource_metadata=\"{ISSUER}/.well-known/oauth-protected-resource\"")
                .as_str()
        ),
        "a client of app.example.test/mcp is sent to that host's metadata"
    );

    let via_mcp = tools_list_via(&resources, "mcp.example.test", None).await;
    assert_eq!(
        via_mcp.header("www-authenticate"),
        Some(metadata_challenge(None).as_str())
    );
}

#[tokio::test]
async fn a_token_bound_to_either_host_is_served_at_mcp() {
    let resources = resources_with(both_hosts()).await;
    let (user, _session) = athlete(&resources, "both-hosts@example.test").await;
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .map(|tenant| tenant.id.to_string());
    let mint = |audience: &str| {
        resources
            .auth
            .auth_manager
            .generate_oauth_access_token(
                &resources.auth.jwks_manager,
                &DelegatedGrant {
                    client_id: "mcp_client_test",
                    scopes: &["fitness:read".to_owned()],
                    audience: Some(audience),
                },
                &user.id,
                &[],
                tenant.clone(),
            )
            .unwrap()
    };

    for (host, audience) in [
        ("app.example.test", ISSUER),
        ("mcp.example.test", MCP_RESOURCE),
        ("mcp.example.test", ISSUER),
    ] {
        let served = tools_list_via(&resources, host, Some(&mint(audience))).await;
        assert_eq!(
            served.status(),
            200,
            "a token bound to {audience} is served when {host} is dialed"
        );
    }
    let elsewhere =
        tools_list_via(&resources, "app.example.test", Some(&mint(OTHER_RESOURCE))).await;
    assert_eq!(elsewhere.status(), 401, "a third resource is still refused");
}

/// `POST /oauth2/validate-and-refresh` with `bearer`, presenting
/// `refresh_token` when given; the JSON verdict.
async fn validate_and_refresh(
    resources: &Arc<ServerContext>,
    bearer: &str,
    refresh_token: Option<&str>,
) -> Value {
    let response = AxumTestRequest::post("/oauth2/validate-and-refresh")
        .header("authorization", &format!("Bearer {bearer}"))
        .json(&json!({ "refresh_token": refresh_token }))
        .send(oauth2_routes(resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "validate-and-refresh answers every verdict in the body"
    );
    response.json()
}

/// `token` with its claims unchanged but for an expiry an hour past,
/// re-signed with the server's active key.
fn expired(resources: &Arc<ServerContext>, token: &str) -> String {
    let mut claims = insecure_decode::<Claims>(token).unwrap().claims;
    claims.iat -= 7200;
    claims.exp = Utc::now().timestamp() - 3600;
    let key = resources.auth.jwks_manager.get_active_key().unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(&header, &claims, &key.encoding_key().unwrap()).unwrap()
}

#[tokio::test]
async fn validate_and_refresh_rotates_a_bound_grant_into_a_token_still_bound() {
    let resources = resources_with(split_hosts()).await;
    let (client_id, client_secret) = register(&resources).await;
    let (user, session) = athlete(&resources, "validate-refresh@example.test").await;
    let code = authorization_code(&resources, &client_id, &session, Some(MCP_RESOURCE)).await;
    let exchanged = token_request(
        &resources,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", REDIRECT),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
            ("code_verifier", VERIFIER),
            ("resource", MCP_RESOURCE),
        ],
    )
    .await;
    assert_eq!(exchanged.status(), 200);
    let tokens: Value = exchanged.json();
    let access_token = tokens["access_token"].as_str().unwrap().to_owned();
    let refresh_token = tokens["refresh_token"].as_str().unwrap().to_owned();

    // A live token bound to this resource is valid here.
    let live = validate_and_refresh(&resources, &access_token, None).await;
    assert_eq!(live["status"], "valid");
    assert!(live["expires_in"].as_i64().unwrap() > 0, "{live}");
    assert!(live.get("access_token").is_none_or(Value::is_null));

    // One bound to a resource this server does not serve is not.
    let foreign = resources
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &resources.auth.jwks_manager,
            &DelegatedGrant {
                client_id: "mcp_client_test",
                scopes: &["fitness:read".to_owned()],
                audience: Some(OTHER_RESOURCE),
            },
            &user.id,
            &[],
            None,
        )
        .unwrap();
    let refused = validate_and_refresh(&resources, &foreign, None).await;
    assert_eq!(refused["status"], "invalid");
    assert_eq!(refused["reason"], "invalid_signature");
    assert_eq!(refused["requires_full_reauth"], true);

    let stale = expired(&resources, &access_token);
    let unrefreshable = validate_and_refresh(&resources, &stale, None).await;
    assert_eq!(unrefreshable["status"], "invalid");
    assert_eq!(unrefreshable["reason"], "token_expired");

    // Another athlete's expired token cannot spend this athlete's refresh
    // token, and the refusal does not consume it.
    let (intruder, _) = athlete(&resources, "validate-refresh-intruder@example.test").await;
    let intruder_token = resources
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &resources.auth.jwks_manager,
            &DelegatedGrant {
                client_id: "mcp_client_test",
                scopes: &["fitness:read".to_owned()],
                audience: Some(MCP_RESOURCE),
            },
            &intruder.id,
            &[],
            None,
        )
        .unwrap();
    let stolen = validate_and_refresh(
        &resources,
        &expired(&resources, &intruder_token),
        Some(&refresh_token),
    )
    .await;
    assert_eq!(stolen["status"], "invalid");
    assert_eq!(stolen["reason"], "invalid_refresh_token");

    let refreshed = validate_and_refresh(&resources, &stale, Some(&refresh_token)).await;
    assert_eq!(refreshed["status"], "refreshed", "{refreshed}");
    assert_eq!(refreshed["token_type"], "Bearer");
    assert_eq!(refreshed["expires_in"], 3600);
    let rotated = refreshed["refresh_token"].as_str().unwrap().to_owned();
    assert_ne!(rotated, refresh_token, "the refresh token is rotated");
    let claims = resources
        .auth
        .auth_manager
        .validate_resource_token(
            refreshed["access_token"].as_str().unwrap(),
            &resources.auth.jwks_manager,
            &[MCP_RESOURCE],
        )
        .unwrap();
    assert_eq!(claims.aud, MCP_RESOURCE, "the grant's binding carries over");
    assert_eq!(claims.sub, user.id.to_string());

    // The consumed refresh token is spent; its replacement is not.
    let replayed = validate_and_refresh(&resources, &stale, Some(&refresh_token)).await;
    assert_eq!(replayed["status"], "invalid");
    assert_eq!(replayed["reason"], "invalid_refresh_token");
    let again = validate_and_refresh(&resources, &stale, Some(&rotated)).await;
    assert_eq!(again["status"], "refreshed", "{again}");
}

#[tokio::test]
async fn a_refused_login_links_back_with_every_oauth_parameter_and_the_resource() {
    let resources = resources_with(split_hosts()).await;
    let (user, _session) = athlete(&resources, "refused-login@example.test").await;
    let challenge = pkce_challenge();
    let form = [
        ("email", user.email.as_str()),
        ("password", "not-this-athlete's-password"),
        ("client_id", "client \"quoted\" <id>"),
        ("redirect_uri", REDIRECT),
        ("response_type", "code"),
        ("state", STATE),
        ("scope", "fitness:read"),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("resource", MCP_RESOURCE),
    ];

    let refused = AxumTestRequest::post("/oauth2/login")
        .header("accept-language", "en")
        .form(&form)
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(refused.status(), 401);
    let page = refused.text();
    assert!(page.contains("Invalid email or password"), "{page}");
    assert!(!page.contains("{{"), "every placeholder is filled: {page}");
    let retry = format!(
        "href=\"/oauth2/login?client_id=client%20%22quoted%22%20%3Cid%3E&redirect_uri={}&response_type=code&state={}&scope=fitness%3Aread&code_challenge={challenge}&code_challenge_method=S256&resource={}&ui_locales=en\"",
        urlencoding::encode(REDIRECT),
        urlencoding::encode(STATE),
        urlencoding::encode(MCP_RESOURCE),
    );
    assert!(
        page.contains(&retry),
        "the retry link carries the form back: {page}"
    );
}
