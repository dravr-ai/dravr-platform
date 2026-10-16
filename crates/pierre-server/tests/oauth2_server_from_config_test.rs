// ABOUTME: Pins that the authorization server built from the deployment's config carries its whole policy
// ABOUTME: First-party redirects and satellite resources reach the server both the issuer and the redeemer build
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # One authorization server per configuration
//!
//! `/oauth2/authorize` issues a first-party code and `/oauth/token` redeems
//! it, and each used to assemble its own authorization server by hand. Both
//! now build it through [`OAuth2AuthorizationServer::from_config`], so a
//! policy the configuration carries reaches both. These tests pin that the
//! constructor applies every policy the configuration names.

mod common;

use base64::{engine::general_purpose, Engine as _};
use pierre_auth::auth::AuthManager;
use pierre_auth::config::OAuth2ServerConfig;
use pierre_auth::oauth2_server::endpoints::OAuth2AuthorizationServer;
use pierre_auth::oauth2_server::first_party::{FirstPartyRedirects, CALLBACK_PATH, WEB_CLIENT_ID};
use pierre_auth::oauth2_server::models::{AuthorizeRejection, AuthorizeRequest};
use pierre_database::backends::DatabaseProvider;
use pierre_database::database::generate_encryption_key;
use pierre_test_support::db::create_test_db_with_key;
use sha2::{Digest, Sha256};
use std::sync::Arc;

const WEB_ORIGIN: &str = "https://web.from-config.example";
const SATELLITE: &str = "https://satellite.from-config.example";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

async fn server() -> OAuth2AuthorizationServer {
    let database = create_test_db_with_key(generate_encryption_key().to_vec())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repos = database.repositories();
    let config = OAuth2ServerConfig {
        satellite_resources: vec![SATELLITE.to_owned()],
        first_party_redirects: FirstPartyRedirects {
            web_origins: vec![WEB_ORIGIN.to_owned()],
            allow_expo_go: false,
        },
        ..OAuth2ServerConfig::default()
    };
    OAuth2AuthorizationServer::from_config(
        &config,
        repos.oauth2_server.clone(),
        repos.tenants.clone(),
        repos.users.clone(),
        Arc::new(AuthManager::new(24)),
        common::get_shared_test_jwks(),
        30,
    )
}

fn web_request(redirect_uri: String, resource: Option<&str>) -> AuthorizeRequest {
    AuthorizeRequest {
        response_type: "code".to_owned(),
        client_id: WEB_CLIENT_ID.to_owned(),
        redirect_uri,
        scope: None,
        state: None,
        code_challenge: Some(
            general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes())),
        ),
        code_challenge_method: Some("S256".to_owned()),
        resource: resource.map(str::to_owned),
    }
}

/// The web app's callback on the configured origin is accepted; the same
/// callback on another origin is refused before anything is redirected.
#[tokio::test]
async fn the_configured_first_party_redirect_is_accepted_and_no_other() {
    let server = server().await;

    let accepted = server
        .check_authorize_request(&web_request(format!("{WEB_ORIGIN}{CALLBACK_PATH}"), None))
        .await
        .expect("the configured web origin's callback is accepted");
    assert_eq!(accepted.resource, None, "no resource was asked for");

    let refused = server
        .check_authorize_request(&web_request(
            format!("https://elsewhere.example{CALLBACK_PATH}"),
            None,
        ))
        .await
        .expect_err("a callback outside the configured origins is refused");
    let AuthorizeRejection::ShownToUser(error) = refused else {
        panic!("an unaccepted redirect is never redirected to: {refused:?}");
    };
    assert_eq!(error.error, "invalid_request");
}

/// A satellite the configuration names is a resource the server mints for.
#[tokio::test]
async fn the_configured_satellite_is_a_served_resource() {
    let server = server().await;

    let checked = server
        .check_authorize_request(&web_request(
            format!("{WEB_ORIGIN}{CALLBACK_PATH}"),
            Some(SATELLITE),
        ))
        .await
        .expect("the configured satellite is served");
    assert_eq!(checked.resource.as_deref(), Some(SATELLITE));
}
