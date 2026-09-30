// ABOUTME: Pins the web session cookie's name: __Host-auth_token on an HTTPS deployment, auth_token over plain HTTP
// ABOUTME: A plain auth_token planted by a sibling host is refused on HTTPS by the middleware, the extractor and /oauth2/authorize
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The web app's session cookie used to be a plain `auth_token`. Any host
//! under the parent domain can set a plain-named cookie for its siblings
//! (`Domain=dravr.ai`), and `/oauth2/authorize` accepts the web app's cookie
//! as a session, so a sibling host could plant its own account's session and
//! have the victim approve an MCP connector as that account (carnet#669).
//!
//! On an HTTPS deployment the cookie is now `__Host-auth_token`: browsers
//! accept that name only from the host itself, `Secure`, with `Path=/` and no
//! `Domain`, so no sibling can plant it. Every reader matches that exact
//! name, so a plain `auth_token` is ignored there. Over plain HTTP (local
//! development) the prefix would make browsers drop the cookie, so the bare
//! name is kept.
//!
//! The name follows `BASE_URL`, the same predicate as the cookie's `Secure`
//! flag. Each test sets it and restores it on drop; the tests are serial
//! because the variable is process-wide, and this file is its own binary so
//! no other test sees the change.

mod common;
mod helpers;

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::http::{header, HeaderMap, HeaderValue};
use common::{create_test_server_resources_with_config, create_test_tenant};
use helpers::axum_test::AxumTestRequest;
use pierre_auth::config::OAuth2ServerConfig;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::security::cookies::{auth_cookie_name, clear_auth_cookie, set_auth_cookie};
use pierre_config::environment::ServerConfig;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use serial_test::serial;

const HTTPS_BASE: &str = "https://app.example.test";
const HTTP_BASE: &str = "http://127.0.0.1:8081";
const REDIRECT: &str = "https://client.example.test/callback";
const PKCE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

/// Sets `BASE_URL` for one test and puts back whatever was there before.
struct BaseUrl {
    previous: Option<String>,
}

impl BaseUrl {
    fn set(value: &str) -> Self {
        let previous = env::var("BASE_URL").ok();
        env::set_var("BASE_URL", value);
        Self { previous }
    }
}

impl Drop for BaseUrl {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => env::set_var("BASE_URL", value),
            None => env::remove_var("BASE_URL"),
        }
    }
}

/// The one `Set-Cookie` value a helper wrote.
fn set_cookie(headers: &HeaderMap) -> String {
    headers
        .get(header::SET_COOKIE)
        .expect("a Set-Cookie header is written")
        .to_str()
        .unwrap()
        .to_owned()
}

fn cookie_headers(cookie: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
    headers
}

/// A deployment whose authorization server is `issuer`.
async fn resources_with_issuer(issuer: &str) -> Arc<ServerContext> {
    Box::pin(create_test_server_resources_with_config(ServerConfig {
        oauth2_server: OAuth2ServerConfig {
            issuer_url: issuer.to_owned(),
            ..OAuth2ServerConfig::default()
        },
        activity_fetch_limit: 100,
        ..ServerConfig::default()
    }))
    .await
    .unwrap()
}

/// The OAuth routes over the context's own `oauth2_server` config slice.
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
        accounts: resources.oauth2_accounts(),
        google_sign_in: None,
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_669))))
}

/// `/oauth2/authorize` for a freshly registered client.
async fn authorize_uri(resources: &Arc<ServerContext>) -> String {
    let client_id = ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![REDIRECT.to_owned()],
                client_name: Some("Host cookie client".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap()
        .client_id;
    format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&state=host-cookie-state&code_challenge={PKCE_CHALLENGE}&code_challenge_method=S256",
        urlencoding::encode(&client_id),
        urlencoding::encode(REDIRECT),
    )
}

#[test]
#[serial]
fn an_https_deployment_sets_the_host_prefixed_cookie() {
    let _base = BaseUrl::set(HTTPS_BASE);
    assert_eq!(auth_cookie_name(), "__Host-auth_token");

    let mut headers = HeaderMap::new();
    set_auth_cookie(&mut headers, "a-session-jwt", 3600);
    let cookie = set_cookie(&headers);

    assert!(
        cookie.starts_with("__Host-auth_token=a-session-jwt;"),
        "{cookie}"
    );
    let attributes: Vec<&str> = cookie.split(';').map(str::trim).collect();
    assert!(attributes.contains(&"Secure"), "{cookie}");
    assert!(attributes.contains(&"Path=/"), "{cookie}");
    assert!(attributes.contains(&"HttpOnly"), "{cookie}");
    assert!(
        !cookie.to_ascii_lowercase().contains("domain="),
        "a __Host- cookie carries no Domain: {cookie}"
    );
}

#[test]
#[serial]
fn an_https_logout_clears_the_host_prefixed_cookie() {
    let _base = BaseUrl::set(HTTPS_BASE);

    let mut headers = HeaderMap::new();
    clear_auth_cookie(&mut headers);
    let cookie = set_cookie(&headers);

    assert!(
        cookie.starts_with("__Host-auth_token=; Max-Age=0;"),
        "{cookie}"
    );
    let attributes: Vec<&str> = cookie.split(';').map(str::trim).collect();
    // A browser ignores a __Host- clear that lacks Secure or Path=/.
    assert!(attributes.contains(&"Secure"), "{cookie}");
    assert!(attributes.contains(&"Path=/"), "{cookie}");
}

#[test]
#[serial]
fn plain_http_keeps_the_bare_name() {
    let _base = BaseUrl::set(HTTP_BASE);
    assert_eq!(auth_cookie_name(), "auth_token");

    let mut headers = HeaderMap::new();
    set_auth_cookie(&mut headers, "a-session-jwt", 3600);
    let cookie = set_cookie(&headers);
    assert!(cookie.starts_with("auth_token=a-session-jwt;"), "{cookie}");
    assert!(
        !cookie.split(';').any(|a| a.trim() == "Secure"),
        "no Secure over plain HTTP: {cookie}"
    );

    let mut cleared = HeaderMap::new();
    clear_auth_cookie(&mut cleared);
    let cleared = set_cookie(&cleared);
    assert!(cleared.starts_with("auth_token=; Max-Age=0;"), "{cleared}");
}

#[tokio::test]
#[serial]
async fn the_auth_middleware_ignores_a_plain_cookie_on_https() {
    let _base = BaseUrl::set(HTTPS_BASE);
    let resources = resources_with_issuer(HTTPS_BASE).await;
    let (user, session) = create_test_tenant(&resources, "middleware-host@example.test")
        .await
        .unwrap();
    let middleware = &resources.auth.auth_middleware;

    let planted = middleware
        .authenticate_request_with_headers(&cookie_headers(&format!("auth_token={session}")))
        .await;
    assert!(
        planted.is_err(),
        "a plain auth_token authenticates nobody on HTTPS: {:?}",
        planted.map(|auth| auth.user_id)
    );

    let own = middleware
        .authenticate_request_with_headers(&cookie_headers(&format!("__Host-auth_token={session}")))
        .await
        .unwrap();
    assert_eq!(own.user_id, user.id, "the host-only cookie is the session");
}

#[tokio::test]
#[serial]
async fn an_https_route_behind_the_extractor_ignores_a_plain_cookie() {
    let _base = BaseUrl::set(HTTPS_BASE);
    let resources = resources_with_issuer(HTTPS_BASE).await;
    let (_, session) = create_test_tenant(&resources, "extractor-host@example.test")
        .await
        .unwrap();
    let app = ProviderToolRouter::build_http_app(&resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_670))));

    let planted = AxumTestRequest::get("/api/usage/status")
        .header("cookie", &format!("auth_token={session}"))
        .send(app.clone())
        .await;
    assert_eq!(planted.status(), 401, "{}", planted.body_text());

    let own = AxumTestRequest::get("/api/usage/status")
        .header("cookie", &format!("__Host-auth_token={session}"))
        .send(app)
        .await;
    assert_eq!(own.status(), 200, "{}", own.body_text());
}

#[tokio::test]
#[serial]
async fn authorize_on_https_refuses_a_planted_plain_cookie() {
    let _base = BaseUrl::set(HTTPS_BASE);
    let resources = resources_with_issuer(HTTPS_BASE).await;
    let (_, session) = create_test_tenant(&resources, "authorize-host@example.test")
        .await
        .unwrap();
    let uri = authorize_uri(&resources).await;

    let planted = AxumTestRequest::get(&uri)
        .header("cookie", &format!("auth_token={session}"))
        .send(oauth2_routes(&resources))
        .await;
    assert!(
        planted.status_code().is_redirection(),
        "a plain auth_token is no session: got {}",
        planted.status()
    );
    let location = planted.header("location").unwrap_or_default();
    assert!(
        location.starts_with("/oauth2/login"),
        "sent to log in, never to consent: {location}"
    );

    let own = AxumTestRequest::get(&uri)
        .header("cookie", &format!("__Host-auth_token={session}"))
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(
        own.status(),
        200,
        "the host-only web session reaches consent"
    );
    assert!(own.text().contains("fitness:read"), "the consent screen");
}

/// `OAUTH2_ISSUER_URL` can name an HTTPS issuer while `BASE_URL` stays plain
/// HTTP. The authorize endpoint is then served over HTTPS, so it reads the web
/// session only under the host-prefixed name, never a plantable bare one.
#[tokio::test]
#[serial]
async fn an_https_issuer_refuses_a_plain_cookie_whatever_base_url_says() {
    let _base = BaseUrl::set(HTTP_BASE);
    let resources = resources_with_issuer(HTTPS_BASE).await;
    let (_, session) = create_test_tenant(&resources, "authorize-split@example.test")
        .await
        .unwrap();
    let uri = authorize_uri(&resources).await;

    let planted = AxumTestRequest::get(&uri)
        .header("cookie", &format!("auth_token={session}"))
        .send(oauth2_routes(&resources))
        .await;
    let location = planted.header("location").unwrap_or_default();
    assert!(
        planted.status_code().is_redirection() && location.starts_with("/oauth2/login"),
        "a plain auth_token is no session at an HTTPS issuer: {} {location}",
        planted.status()
    );

    let own = AxumTestRequest::get(&uri)
        .header("cookie", &format!("__Host-auth_token={session}"))
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(own.status(), 200, "the host-only name still bridges");
    assert!(own.text().contains("fitness:read"), "the consent screen");
}

#[tokio::test]
#[serial]
async fn authorize_over_plain_http_bridges_the_bare_cookie() {
    let _base = BaseUrl::set(HTTP_BASE);
    let resources = resources_with_issuer(HTTP_BASE).await;
    let (_, session) = create_test_tenant(&resources, "authorize-http@example.test")
        .await
        .unwrap();
    let uri = authorize_uri(&resources).await;

    let response = AxumTestRequest::get(&uri)
        .header("cookie", &format!("auth_token={session}"))
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "local development keeps the bare name"
    );
}

#[tokio::test]
#[serial]
async fn the_logout_route_clears_the_host_prefixed_cookie_on_https() {
    let _base = BaseUrl::set(HTTPS_BASE);
    let resources = resources_with_issuer(HTTPS_BASE).await;
    let app = ProviderToolRouter::build_http_app(&resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_671))));

    let response = AxumTestRequest::post("/api/auth/logout").send(app).await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let cleared = response.header_all("set-cookie");
    assert!(
        cleared
            .iter()
            .any(|c| c.starts_with("__Host-auth_token=; Max-Age=0;") && c.contains("Secure")),
        "logout clears the name the session was set under: {cleared:?}"
    );
    assert!(
        !cleared.iter().any(|c| c.starts_with("auth_token=")),
        "and not the retired plain name: {cleared:?}"
    );
}
