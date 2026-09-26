// ABOUTME: Pins where GET /oauth2/authorize and POST /oauth2/consent send an error (RFC 6749 section 4.1.2.1)
// ABOUTME: A verified client and redirect_uri get a redirect with error + state; an unknown one gets a page, never a redirect

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! RFC 6749 section 4.1.2.1: once the `client_id` is valid and the
//! `redirect_uri` is registered for it, an authorization error (an unknown or
//! undelegable scope, a denied consent, a missing parameter) MUST reach the
//! client as a redirect to that `redirect_uri` carrying `error`, an optional
//! `error_description` and the request's `state`. Only an unknown client or an
//! unregistered `redirect_uri` is shown to the user instead, and never
//! redirected, since nothing vouches for where the redirect would go.
//!
//! The authorization server used to render every such error as an HTML page,
//! so an MCP client asking for a scope it may not have (`admin` after
//! carnet#550) waited on a callback that never came.

mod common;
mod helpers;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use common::{create_test_server_resources, create_test_user_with_email, generate_test_token};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use url::Url;

const REDIRECT: &str = "https://app.example.com/callback";
/// A registered `redirect_uri` that carries its own query, which an error
/// redirect must keep (RFC 6749 section 3.1.2).
const REDIRECT_WITH_QUERY: &str = "https://app.example.com/callback?tenant=acme";
const STATE: &str = "state with spaces & symbols=1";
const PKCE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

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
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_001))))
}

async fn register(resources: &Arc<ServerContext>, redirect_uri: &str) -> String {
    ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![redirect_uri.to_owned()],
                client_name: Some("Error redirect client".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap()
        .client_id
}

/// `/oauth2/authorize` with every parameter percent-encoded; `scope` is
/// omitted when `None`.
fn authorize_uri(client_id: &str, redirect_uri: &str, scope: Option<&str>) -> String {
    let mut uri = format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={PKCE_CHALLENGE}&code_challenge_method=S256",
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(STATE),
    );
    if let Some(scope) = scope {
        uri.push_str("&scope=");
        uri.push_str(&urlencoding::encode(scope));
    }
    uri
}

/// The redirect a response carries, parsed, with its query as pairs.
fn redirect_target(response: &AxumTestResponse) -> (Url, Vec<(String, String)>) {
    assert_eq!(
        response.status(),
        303,
        "an error for a verified client is a redirect"
    );
    let location = response
        .header("location")
        .expect("a redirect carries a Location");
    let url = Url::parse(location).unwrap_or_else(|e| panic!("{location}: {e}"));
    let query = url.query_pairs().into_owned().collect();
    (url, query)
}

fn param<'a>(query: &'a [(String, String)], name: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// The error page, and no redirect anywhere. The page names no error detail
/// by design, so what is pinned is that it is the page and not a redirect.
fn assert_shown_to_user(response: AxumTestResponse) {
    assert_eq!(response.status(), 400, "an untrusted request is a page");
    assert!(
        response.header("location").is_none(),
        "never redirected: {:?}",
        response.header("location")
    );
    let page = response.text();
    assert!(
        page.starts_with("<!DOCTYPE html>") && !page.contains("attacker.example.net"),
        "the error page, carrying nothing of the request: {page}"
    );
}

#[tokio::test]
async fn an_unknown_scope_is_redirected_to_the_client_with_its_state() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = register(&resources, REDIRECT).await;

    let response = AxumTestRequest::get(&authorize_uri(
        &client_id,
        REDIRECT,
        Some("fitness:read read:everything"),
    ))
    .send(oauth2_routes(&resources))
    .await;

    let (url, query) = redirect_target(&response);
    assert_eq!(
        format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().unwrap(),
            url.path()
        ),
        REDIRECT,
        "the error goes to the registered redirect_uri"
    );
    assert_eq!(param(&query, "error"), Some("invalid_scope"));
    assert!(
        param(&query, "error_description").is_some_and(|d| d.contains("read:everything")),
        "the description names the refused scope: {query:?}"
    );
    assert_eq!(
        param(&query, "state"),
        Some(STATE),
        "the original state, decoded"
    );
    assert_eq!(param(&query, "code"), None, "no code is issued");
}

#[tokio::test]
async fn admin_is_refused_to_the_client_even_with_a_session() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = register(&resources, REDIRECT).await;
    let (_, user) =
        create_test_user_with_email(&resources.agent.database, "redirect-admin@example.test")
            .await
            .unwrap();
    let session = generate_test_token(&resources, &user).await;

    let response = AxumTestRequest::get(&authorize_uri(
        &client_id,
        REDIRECT,
        Some("fitness:read admin"),
    ))
    .header("cookie", &format!("auth_token={session}"))
    .send(oauth2_routes(&resources))
    .await;

    let (_, query) = redirect_target(&response);
    assert_eq!(
        param(&query, "error"),
        Some("invalid_scope"),
        "no consent screen for a grant that cannot be issued"
    );
    assert_eq!(param(&query, "state"), Some(STATE));
}

#[tokio::test]
async fn an_error_keeps_the_query_the_redirect_uri_was_registered_with() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = register(&resources, REDIRECT_WITH_QUERY).await;

    let response = AxumTestRequest::get(&authorize_uri(
        &client_id,
        REDIRECT_WITH_QUERY,
        Some("unknown:scope"),
    ))
    .send(oauth2_routes(&resources))
    .await;

    let location = response.header("location").unwrap_or_default().to_owned();
    assert!(
        location.starts_with("https://app.example.com/callback?tenant=acme&error=invalid_scope&"),
        "{location}"
    );
}

#[tokio::test]
async fn a_missing_response_type_is_redirected_once_the_client_is_verified() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = register(&resources, REDIRECT).await;
    let uri = authorize_uri(&client_id, REDIRECT, None).replace("response_type=code&", "");

    let response = AxumTestRequest::get(&uri)
        .send(oauth2_routes(&resources))
        .await;

    let (_, query) = redirect_target(&response);
    assert_eq!(param(&query, "error"), Some("invalid_request"));
    assert_eq!(
        param(&query, "error_description"),
        Some("Missing response_type parameter")
    );
    assert_eq!(param(&query, "state"), Some(STATE));
}

#[tokio::test]
async fn an_unregistered_redirect_uri_is_shown_and_never_redirected() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = register(&resources, REDIRECT).await;

    let response = AxumTestRequest::get(&authorize_uri(
        &client_id,
        "https://attacker.example.net/steal",
        Some("unknown:scope"),
    ))
    .send(oauth2_routes(&resources))
    .await;

    assert_shown_to_user(response);
}

#[tokio::test]
async fn an_unknown_client_is_shown_and_never_redirected() {
    let resources = create_test_server_resources().await.unwrap();

    let response = AxumTestRequest::get(&authorize_uri(
        "no-such-client",
        REDIRECT,
        Some("unknown:scope"),
    ))
    .send(oauth2_routes(&resources))
    .await;

    assert_shown_to_user(response);
}

#[tokio::test]
async fn a_denied_consent_is_redirected_to_the_client_as_access_denied() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = register(&resources, REDIRECT).await;
    let (_, user) =
        create_test_user_with_email(&resources.agent.database, "redirect-deny@example.test")
            .await
            .unwrap();
    let session = generate_test_token(&resources, &user).await;

    let response = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &format!("auth_token={session}"))
        .form(&[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", REDIRECT),
            ("state", STATE),
            ("scope", "fitness:read"),
            ("code_challenge", PKCE_CHALLENGE),
            ("code_challenge_method", "S256"),
            ("decision", "deny"),
        ])
        .send(oauth2_routes(&resources))
        .await;

    let (_, query) = redirect_target(&response);
    assert_eq!(param(&query, "error"), Some("access_denied"));
    assert_eq!(param(&query, "state"), Some(STATE));
    assert_eq!(param(&query, "code"), None);
}

#[tokio::test]
async fn a_consent_for_an_unregistered_redirect_uri_is_never_redirected() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = register(&resources, REDIRECT).await;
    let (_, user) =
        create_test_user_with_email(&resources.agent.database, "redirect-forged@example.test")
            .await
            .unwrap();
    let session = generate_test_token(&resources, &user).await;

    let response = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &format!("auth_token={session}"))
        .form(&[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", "https://attacker.example.net/steal"),
            ("state", STATE),
            ("code_challenge", PKCE_CHALLENGE),
            ("code_challenge_method", "S256"),
            ("decision", "deny"),
        ])
        .send(oauth2_routes(&resources))
        .await;

    assert_shown_to_user(response);
}
