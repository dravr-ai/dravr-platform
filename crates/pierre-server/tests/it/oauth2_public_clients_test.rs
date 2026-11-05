// ABOUTME: Public OAuth clients (none), client_secret_basic, client metadata documents and registration hygiene
// ABOUTME: Pins the token endpoint's three client authentication methods and every SSRF refusal of the metadata fetch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Who a token request can be from
//!
//! A directory-listed MCP client is a public client: it holds no secret, names
//! itself with an `https` URL (a client metadata document) or registers
//! dynamically as `none`, and proves possession of its code with PKCE alone.
//! A confidential client authenticates with `client_secret_basic` or
//! `client_secret_post`. Each test below pins one of those behaviours, or one
//! refusal that keeps the metadata fetch from reaching somewhere it should not.

use crate::common::{self, create_test_server_resources};
use crate::helpers::axum_test::AxumTestRequest;
use async_trait::async_trait;
use axum::extract::connect_info::MockConnectInfo;
use axum::Router;
use base64::{engine::general_purpose, Engine as _};
use chrono::{Duration, Utc};
use pierre_auth::auth::AuthManager;
use pierre_auth::config::OAuth2ServerConfig;
use pierre_auth::oauth2_server::cimd::{client_from_document, CimdError, MetadataFetcher};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::endpoints::OAuth2AuthorizationServer;
use pierre_auth::oauth2_server::models::{
    AuthorizeRejection, AuthorizeRequest, ClientRegistrationRequest, OAuth2Client, TokenRequest,
};
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::{factory::Database, DatabaseProvider};
use pierre_database::database::generate_encryption_key;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use pierre_test_support::db::create_test_db_with_key;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const REDIRECT: &str = "https://app.example.com/callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

fn challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

struct Env {
    database: Arc<Database>,
    server: OAuth2AuthorizationServer,
    registration: ClientRegistrationManager,
}

/// A server whose metadata documents are served by `fetcher`.
async fn env_with(fetcher: Arc<dyn MetadataFetcher>) -> Env {
    let database = Arc::new(
        create_test_db_with_key(generate_encryption_key().to_vec())
            .await
            .unwrap(),
    );
    database.migrate().await.unwrap();
    let repos = database.repositories();
    let server = OAuth2AuthorizationServer::new(
        repos.oauth2_server.clone(),
        repos.tenants.clone(),
        repos.users.clone(),
        Arc::new(AuthManager::new(24)),
        common::get_shared_test_jwks(),
        30,
        vec![OAuth2ServerConfig::default().mcp_resource_url],
    )
    .with_metadata_fetcher(fetcher);
    let registration = ClientRegistrationManager::new(repos.oauth2_server.clone());
    Env {
        database,
        server,
        registration,
    }
}

async fn env() -> Env {
    env_with(Arc::new(StaticFetcher::refusing())).await
}

async fn athlete(database: &Database, email: &str) -> User {
    let user = User::new(email.to_owned(), "hash".to_owned(), Some("Pub".to_owned()));
    let repos = database.repositories();
    repos.users.create(&user).await.unwrap();
    repos
        .tenants
        .create(&Tenant {
            id: TenantId::generate(),
            name: "Public Tenant".to_owned(),
            slug: format!("tenant-{}", user.id),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    user
}

fn registration(redirect: &str, scope: Option<&str>) -> ClientRegistrationRequest {
    ClientRegistrationRequest {
        redirect_uris: vec![redirect.to_owned()],
        client_name: Some("Public Test Client".to_owned()),
        client_uri: None,
        grant_types: None,
        response_types: None,
        scope: scope.map(str::to_owned),
    }
}

fn authorize_request(client_id: &str, redirect: &str, scope: Option<&str>) -> AuthorizeRequest {
    AuthorizeRequest {
        response_type: "code".to_owned(),
        client_id: client_id.to_owned(),
        redirect_uri: redirect.to_owned(),
        scope: scope.map(str::to_owned),
        state: None,
        code_challenge: Some(challenge()),
        code_challenge_method: Some("S256".to_owned()),
        resource: None,
    }
}

fn exchange(client_id: &str, secret: &str, code: String, redirect: &str) -> TokenRequest {
    TokenRequest {
        grant_type: "authorization_code".to_owned(),
        code: Some(code),
        redirect_uri: Some(redirect.to_owned()),
        client_id: client_id.to_owned(),
        client_secret: SecretString::from(secret),
        scope: None,
        refresh_token: None,
        code_verifier: Some(VERIFIER.to_owned()),
        resource: None,
    }
}

fn refresh(client_id: &str, secret: &str, token: SecretString) -> TokenRequest {
    TokenRequest {
        grant_type: "refresh_token".to_owned(),
        code: None,
        redirect_uri: None,
        client_id: client_id.to_owned(),
        client_secret: SecretString::from(secret),
        scope: None,
        refresh_token: Some(token),
        code_verifier: None,
        resource: None,
    }
}

// ============================================================================
// Client metadata documents, served by a fake fetcher
// ============================================================================

/// Serves one document, counting the fetches, or refuses with an error.
struct StaticFetcher {
    outcome: Mutex<Result<Vec<u8>, CimdError>>,
    fetches: AtomicUsize,
}

impl StaticFetcher {
    fn serving(document: &Value) -> Self {
        Self {
            outcome: Mutex::new(Ok(serde_json::to_vec(document).unwrap())),
            fetches: AtomicUsize::new(0),
        }
    }

    fn refusing() -> Self {
        Self {
            outcome: Mutex::new(Err(CimdError::Status(404))),
            fetches: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl MetadataFetcher for StaticFetcher {
    async fn fetch(&self, _url: &str) -> Result<Vec<u8>, CimdError> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        self.outcome.lock().unwrap().clone()
    }
}

/// A fresh metadata document URL, since the freshness cache is per process.
fn document_url() -> String {
    format!("https://client.example/oauth/{}.json", uuid::Uuid::new_v4())
}

fn document(url: &str, redirect: &str) -> Value {
    json!({
        "client_id": url,
        "client_name": "Directory Client",
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_method": "none",
    })
}

async fn rejection_of(env: &Env, request: AuthorizeRequest) -> AuthorizeRejection {
    env.server
        .check_authorize_request(&request)
        .await
        .expect_err("the request must be refused")
}

#[tokio::test]
async fn a_metadata_document_client_runs_the_whole_flow_without_a_secret() {
    let url = document_url();
    let fetcher = Arc::new(StaticFetcher::serving(&document(&url, REDIRECT)));
    let env = env_with(fetcher.clone()).await;
    let user = athlete(&env.database, "cimd-flow@example.com").await;

    // No registration happened; the first authorization reads the document.
    assert!(env
        .database
        .repositories()
        .oauth2_server
        .get_client(&url)
        .await
        .unwrap()
        .is_none());
    let code = env
        .server
        .authorize(
            authorize_request(&url, REDIRECT, Some("fitness:read fitness:write")),
            Some(user.id),
            None,
        )
        .await
        .unwrap()
        .code;

    let stored = env
        .database
        .repositories()
        .oauth2_server
        .get_client(&url)
        .await
        .unwrap()
        .expect("the document became a client row");
    assert!(
        stored.client_secret_hash.is_empty(),
        "a public client has no secret"
    );
    assert_eq!(stored.client_name.as_deref(), Some("Directory Client"));

    let token = env
        .server
        .token(exchange(&url, "", code, REDIRECT))
        .await
        .unwrap();
    assert_eq!(token.scope.as_deref(), Some("fitness:read fitness:write"));
    let rotated = env
        .server
        .token(refresh(&url, "", token.refresh_token.unwrap()))
        .await
        .unwrap();
    assert!(rotated.refresh_token.is_some());
}

#[tokio::test]
async fn the_document_is_fetched_once_while_it_is_fresh() {
    let url = document_url();
    let fetcher = Arc::new(StaticFetcher::serving(&document(&url, REDIRECT)));
    let env = env_with(fetcher.clone()).await;
    let user = athlete(&env.database, "cimd-fresh@example.com").await;
    env.server
        .authorize(authorize_request(&url, REDIRECT, None), Some(user.id), None)
        .await
        .unwrap();
    let after_enrolling = fetcher.fetches.load(Ordering::SeqCst);

    for _ in 0..3 {
        env.server
            .check_authorize_request(&authorize_request(&url, REDIRECT, None))
            .await
            .unwrap();
    }
    assert_eq!(fetcher.fetches.load(Ordering::SeqCst), after_enrolling);
}

/// Anyone can send an authorize request; a row per request would fill the
/// registration ceiling every real connection shares.
#[tokio::test]
async fn an_authorize_request_nobody_has_signed_in_to_stores_nothing() {
    let url = document_url();
    let env = env_with(Arc::new(StaticFetcher::serving(&document(&url, REDIRECT)))).await;

    let checked = env
        .server
        .check_authorize_request(&authorize_request(&url, REDIRECT, None))
        .await
        .unwrap();

    assert!(checked.unstored_client.is_some());
    assert!(env
        .database
        .repositories()
        .oauth2_server
        .get_client(&url)
        .await
        .unwrap()
        .is_none());
}

/// Answers every fetch with one failure.
struct FailingFetcher(CimdError);

#[async_trait]
impl MetadataFetcher for FailingFetcher {
    async fn fetch(&self, _url: &str) -> Result<Vec<u8>, CimdError> {
        Err(self.0.clone())
    }
}

/// A server whose fetcher fails with `error`, holding a stored row for a client
/// this process has not fetched, so the row is stale and a fetch is attempted.
async fn stale_client_failing_with(error: CimdError) -> (Env, String) {
    let url = document_url();
    let env = env_with(Arc::new(FailingFetcher(error))).await;
    let body = serde_json::to_vec(&document(&url, REDIRECT)).unwrap();
    let client = client_from_document(&url, &body, Utc::now()).unwrap();
    env.database
        .repositories()
        .oauth2_server
        .store_client(&client)
        .await
        .unwrap();
    (env, url)
}

#[tokio::test]
async fn a_known_client_keeps_working_while_its_documents_host_is_down() {
    let (env, url) = stale_client_failing_with(CimdError::Status(503)).await;
    let checked = env
        .server
        .check_authorize_request(&authorize_request(&url, REDIRECT, None))
        .await
        .unwrap();
    assert!(checked.unstored_client.is_none());
}

#[tokio::test]
async fn a_known_client_whose_document_is_gone_is_refused() {
    let (env, url) = stale_client_failing_with(CimdError::Status(404)).await;
    let AuthorizeRejection::ShownToUser(error) =
        rejection_of(&env, authorize_request(&url, REDIRECT, None)).await
    else {
        panic!("shown to the user");
    };
    assert_eq!(error.error, "invalid_client");
}

#[tokio::test]
async fn a_new_client_whose_documents_host_is_down_is_told_to_retry() {
    let url = document_url();
    let env = env_with(Arc::new(FailingFetcher(CimdError::Status(503)))).await;
    let AuthorizeRejection::ShownToUser(error) =
        rejection_of(&env, authorize_request(&url, REDIRECT, None)).await
    else {
        panic!("shown to the user");
    };
    assert_eq!(error.error, "temporarily_unavailable");
}

/// RFC 8252 §8.6: anything on the machine can listen on a loopback port and
/// name a public client, so a remembered approval is not enough for it.
#[tokio::test]
async fn a_client_returning_to_a_local_program_asks_for_consent_every_time() {
    let url = document_url();
    let loopback = "http://127.0.0.1/callback";
    let env = env_with(Arc::new(StaticFetcher::serving(&document(&url, loopback)))).await;
    let checked = env
        .server
        .check_authorize_request(&authorize_request(
            &url,
            "http://127.0.0.1:53682/callback",
            None,
        ))
        .await
        .unwrap();
    assert!(checked.consent_every_time);

    let web = document_url();
    let env = env_with(Arc::new(StaticFetcher::serving(&document(&web, REDIRECT)))).await;
    let checked = env
        .server
        .check_authorize_request(&authorize_request(&web, REDIRECT, None))
        .await
        .unwrap();
    assert!(!checked.consent_every_time);
}

#[tokio::test]
async fn a_document_naming_another_client_id_registers_nothing() {
    let url = document_url();
    let other = document_url();
    let env = env_with(Arc::new(StaticFetcher::serving(&document(
        &other, REDIRECT,
    ))))
    .await;

    let AuthorizeRejection::ShownToUser(error) =
        rejection_of(&env, authorize_request(&url, REDIRECT, None)).await
    else {
        panic!("an unverified client's error is shown, never redirected");
    };
    assert_eq!(error.error, "invalid_client");
    assert!(env
        .database
        .repositories()
        .oauth2_server
        .get_client(&url)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_document_that_cannot_be_fetched_registers_nothing() {
    let url = document_url();
    let env = env().await;
    let AuthorizeRejection::ShownToUser(error) =
        rejection_of(&env, authorize_request(&url, REDIRECT, None)).await
    else {
        panic!("shown to the user");
    };
    assert_eq!(error.error, "invalid_client");
    assert!(env
        .database
        .repositories()
        .oauth2_server
        .get_client(&url)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_document_asking_for_a_shared_secret_is_refused() {
    let url = document_url();
    let mut body = document(&url, REDIRECT);
    body["token_endpoint_auth_method"] = json!("client_secret_basic");
    let env = env_with(Arc::new(StaticFetcher::serving(&body))).await;
    let AuthorizeRejection::ShownToUser(error) =
        rejection_of(&env, authorize_request(&url, REDIRECT, None)).await
    else {
        panic!("shown to the user");
    };
    assert_eq!(error.error, "invalid_client");
}

#[tokio::test]
async fn a_request_is_checked_against_the_documents_redirect_uris() {
    let url = document_url();
    let env = env_with(Arc::new(StaticFetcher::serving(&document(&url, REDIRECT)))).await;
    let AuthorizeRejection::ShownToUser(error) = rejection_of(
        &env,
        authorize_request(&url, "https://evil.example/callback", None),
    )
    .await
    else {
        panic!("an unregistered redirect_uri is shown, never redirected to");
    };
    assert_eq!(error.error, "invalid_request");
}

#[tokio::test]
async fn the_pending_ceiling_stops_new_metadata_document_clients() {
    let url = document_url();
    let env = env_with(Arc::new(StaticFetcher::serving(&document(&url, REDIRECT)))).await;
    let repos = env.database.repositories();
    let server = OAuth2AuthorizationServer::new(
        repos.oauth2_server.clone(),
        repos.tenants.clone(),
        repos.users.clone(),
        Arc::new(AuthManager::new(24)),
        common::get_shared_test_jwks(),
        30,
        vec![OAuth2ServerConfig::default().mcp_resource_url],
    )
    .with_metadata_fetcher(Arc::new(StaticFetcher::serving(&document(&url, REDIRECT))))
    .with_max_pending_registrations(0);
    // Reading the document costs nothing against the ceiling; storing it does.
    server
        .check_authorize_request(&authorize_request(&url, REDIRECT, None))
        .await
        .unwrap();
    let user = athlete(&env.database, "cimd-ceiling@example.com").await;
    let error = server
        .authorize(authorize_request(&url, REDIRECT, None), Some(user.id), None)
        .await
        .unwrap_err();
    assert_eq!(error.error, "server_error");
}

/// The real guarded fetcher, not a fake: every URL here is refused before any
/// connection is made, so the test needs no network.
#[tokio::test]
async fn the_guarded_fetcher_refuses_every_url_that_could_reach_inside() {
    let database = Arc::new(
        create_test_db_with_key(generate_encryption_key().to_vec())
            .await
            .unwrap(),
    );
    database.migrate().await.unwrap();
    let repos = database.repositories();
    let server = OAuth2AuthorizationServer::new(
        repos.oauth2_server.clone(),
        repos.tenants.clone(),
        repos.users.clone(),
        Arc::new(AuthManager::new(24)),
        common::get_shared_test_jwks(),
        30,
        vec![OAuth2ServerConfig::default().mcp_resource_url],
    );
    let refused = [
        "https://127.0.0.1/client.json",
        "https://[::1]/client.json",
        "https://10.0.0.7/client.json",
        "https://192.168.1.10/client.json",
        "https://169.254.169.254/latest/meta-data/",
        "https://localhost/client.json",
        "https://metadata.google.internal/computeMetadata/v1/",
        "https://app.local/client.json",
        "https://user:secret@client.example/client.json",
        "https://client.example:8443/client.json",
        "https://client.example",
        "https://client.example/a/../client.json",
    ];
    for client_id in refused {
        let AuthorizeRejection::ShownToUser(error) = server
            .check_authorize_request(&authorize_request(client_id, REDIRECT, None))
            .await
            .expect_err(client_id)
        else {
            panic!("{client_id}: shown to the user");
        };
        assert_eq!(error.error, "invalid_client", "{client_id}");
        assert!(
            repos
                .oauth2_server
                .get_client(client_id)
                .await
                .unwrap()
                .is_none(),
            "{client_id} left a row"
        );
    }
}

// ============================================================================
// Loopback redirect matching
// ============================================================================

#[tokio::test]
async fn a_loopback_redirect_matches_on_any_port_but_nothing_else() {
    let env = env().await;
    let client = env
        .registration
        .register_client(
            registration("http://127.0.0.1/callback", None),
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap();

    for accepted in [
        "http://127.0.0.1/callback",
        "http://127.0.0.1:49152/callback",
        "http://127.0.0.1:1/callback",
    ] {
        env.server
            .check_authorize_request(&authorize_request(&client.client_id, accepted, None))
            .await
            .unwrap_or_else(|e| panic!("{accepted} must match: {e:?}"));
    }
    for refused in [
        "http://127.0.0.1:49152/other",
        "http://127.0.0.1:49152/callback?x=1",
        "http://localhost:49152/callback",
        "https://127.0.0.1:49152/callback",
        "http://127.0.0.2:49152/callback",
        "http://evil.example:49152/callback",
    ] {
        let AuthorizeRejection::ShownToUser(error) =
            rejection_of(&env, authorize_request(&client.client_id, refused, None)).await
        else {
            panic!("{refused}: shown to the user");
        };
        assert_eq!(error.error, "invalid_request", "{refused}");
    }
}

#[tokio::test]
async fn a_non_loopback_redirect_still_needs_an_exact_match() {
    let env = env().await;
    let client = env
        .registration
        .register_client(registration(REDIRECT, None), MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap();
    for refused in [
        "https://app.example.com:8443/callback",
        "https://app.example.com/callback/",
        "http://app.example.com/callback",
    ] {
        let AuthorizeRejection::ShownToUser(error) =
            rejection_of(&env, authorize_request(&client.client_id, refused, None)).await
        else {
            panic!("{refused}: shown to the user");
        };
        assert_eq!(error.error, "invalid_request", "{refused}");
    }
}

#[tokio::test]
async fn a_metadata_document_client_gets_port_agnostic_loopback_too() {
    let url = document_url();
    let env = env_with(Arc::new(StaticFetcher::serving(&document(
        &url,
        "http://localhost/callback",
    ))))
    .await;
    env.server
        .check_authorize_request(&authorize_request(
            &url,
            "http://localhost:53682/callback",
            None,
        ))
        .await
        .unwrap();
}

// ============================================================================
// Dynamic registration
// ============================================================================

#[tokio::test]
async fn registration_echoes_the_auth_method_and_defaults_to_basic() {
    let env = env().await;
    let default = env
        .registration
        .register_client(registration(REDIRECT, None), MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap();
    assert_eq!(default.auth_method, "client_secret_basic");
    assert_ne!(default.client_secret.expose_secret(), "");

    for method in ["client_secret_post", "client_secret_basic"] {
        let response = env
            .registration
            .register_client_with_auth_method(
                registration(REDIRECT, None),
                Some(method.to_owned()),
                MAX_PENDING_REGISTRATIONS,
            )
            .await
            .unwrap();
        assert_eq!(response.auth_method, method);
    }
}

#[tokio::test]
async fn a_public_registration_gets_no_secret_and_an_unknown_method_is_refused() {
    let env = env().await;
    let public = env
        .registration
        .register_client_with_auth_method(
            registration(REDIRECT, None),
            Some("none".to_owned()),
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap();
    assert_eq!(public.auth_method, "none");
    let body = serde_json::to_value(&public).unwrap();
    assert!(body.get("client_secret").is_none(), "{body}");
    assert!(body.get("client_secret_expires_at").is_none(), "{body}");
    let stored = env
        .database
        .repositories()
        .oauth2_server
        .get_client(&public.client_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.client_secret_hash, "");

    for method in [
        "private_key_jwt",
        "client_secret_jwt",
        "tls_client_auth",
        "",
    ] {
        let refused = env
            .registration
            .register_client_with_auth_method(
                registration(REDIRECT, None),
                Some(method.to_owned()),
                MAX_PENDING_REGISTRATIONS,
            )
            .await
            .unwrap_err();
        assert_eq!(refused.error, "invalid_client_metadata", "{method}");
    }
}

#[tokio::test]
async fn self_service_registration_refuses_client_credentials() {
    let env = env().await;
    let mut request = registration(REDIRECT, None);
    request.grant_types = Some(vec!["client_credentials".to_owned()]);
    let refused = env
        .registration
        .register_client(request, MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap_err();
    assert_eq!(refused.error, "invalid_request");
    assert!(refused
        .error_description
        .unwrap()
        .contains("Unsupported grant_type"));

    let mut mixed = registration(REDIRECT, None);
    mixed.grant_types = Some(vec![
        "authorization_code".to_owned(),
        "client_credentials".to_owned(),
    ]);
    assert!(env
        .registration
        .register_client(mixed, MAX_PENDING_REGISTRATIONS)
        .await
        .is_err());
}

#[tokio::test]
async fn a_registration_expires_after_ninety_days_without_use() {
    let env = env().await;
    let client = env
        .registration
        .register_client(registration(REDIRECT, None), MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap();
    let expires = client.client_secret_expires_at.unwrap();
    let days = (expires - Utc::now().timestamp()) / 86_400;
    assert!((89..=90).contains(&days), "expires in {days} days");
}

#[tokio::test]
async fn authorizing_through_a_client_slides_its_expiry_forward() {
    let env = env().await;
    let user = athlete(&env.database, "slide@example.com").await;
    let client = env
        .registration
        .register_client(registration(REDIRECT, None), MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap();
    // A registration close to its expiry: 3 days left.
    let repos = env.database.repositories();
    let soon = Utc::now() + Duration::days(3);
    let mut row = repos
        .oauth2_server
        .get_client(&client.client_id)
        .await
        .unwrap()
        .unwrap();
    repos
        .oauth2_server
        .delete_client(&client.client_id)
        .await
        .unwrap();
    row.expires_at = Some(soon);
    repos.oauth2_server.store_client(&row).await.unwrap();

    let code = env
        .server
        .authorize(
            authorize_request(&client.client_id, REDIRECT, None),
            Some(user.id),
            None,
        )
        .await
        .unwrap()
        .code;
    env.server
        .token(exchange(
            &client.client_id,
            client.client_secret.expose_secret(),
            code,
            REDIRECT,
        ))
        .await
        .unwrap();

    let slid = repos
        .oauth2_server
        .get_client(&client.client_id)
        .await
        .unwrap()
        .unwrap()
        .expires_at
        .unwrap();
    assert!(slid > Utc::now() + Duration::days(80), "expires {slid}");
}

#[tokio::test]
async fn the_expiry_never_moves_back_or_onto_a_row_without_one() {
    let env = env().await;
    let repos = env.database.repositories();
    let secret = env
        .registration
        .register_client_credentials_client("machine_expiry", "Machine")
        .await
        .unwrap();
    assert_ne!(secret, "");
    let far = Utc::now() + Duration::days(400);
    repos
        .oauth2_server
        .extend_client_expiry("machine_expiry", far)
        .await
        .unwrap();
    assert!(repos
        .oauth2_server
        .get_client("machine_expiry")
        .await
        .unwrap()
        .unwrap()
        .expires_at
        .is_none());

    let client = env
        .registration
        .register_client(registration(REDIRECT, None), MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap();
    let before = repos
        .oauth2_server
        .get_client(&client.client_id)
        .await
        .unwrap()
        .unwrap()
        .expires_at
        .unwrap();
    repos
        .oauth2_server
        .extend_client_expiry(&client.client_id, Utc::now() + Duration::days(1))
        .await
        .unwrap();
    let after = repos
        .oauth2_server
        .get_client(&client.client_id)
        .await
        .unwrap()
        .unwrap()
        .expires_at
        .unwrap();
    assert_eq!(before, after);
}

// ============================================================================
// Public clients at the token endpoint
// ============================================================================

async fn public_client(env: &Env) -> String {
    env.registration
        .register_client_with_auth_method(
            registration(REDIRECT, None),
            Some("none".to_owned()),
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap()
        .client_id
}

#[tokio::test]
async fn a_public_client_exchanges_and_refreshes_with_pkce_and_no_secret() {
    let env = env().await;
    let user = athlete(&env.database, "public-flow@example.com").await;
    let client_id = public_client(&env).await;
    let code = env
        .server
        .authorize(
            authorize_request(&client_id, REDIRECT, None),
            Some(user.id),
            None,
        )
        .await
        .unwrap()
        .code;
    let token = env
        .server
        .token(exchange(&client_id, "", code, REDIRECT))
        .await
        .unwrap();
    let refreshed = env
        .server
        .token(refresh(&client_id, "", token.refresh_token.unwrap()))
        .await
        .unwrap();
    assert!(refreshed.refresh_token.is_some());
}

#[tokio::test]
async fn a_public_client_cannot_exchange_without_a_verifier() {
    let env = env().await;
    let user = athlete(&env.database, "public-noverifier@example.com").await;
    let client_id = public_client(&env).await;
    let code = env
        .server
        .authorize(
            authorize_request(&client_id, REDIRECT, None),
            Some(user.id),
            None,
        )
        .await
        .unwrap()
        .code;
    let mut request = exchange(&client_id, "", code, REDIRECT);
    request.code_verifier = None;
    let refused = env.server.token(request).await.unwrap_err();
    assert_eq!(refused.error, "invalid_request");
}

#[tokio::test]
async fn a_public_client_with_the_wrong_verifier_is_refused() {
    let env = env().await;
    let user = athlete(&env.database, "public-badverifier@example.com").await;
    let client_id = public_client(&env).await;
    let code = env
        .server
        .authorize(
            authorize_request(&client_id, REDIRECT, None),
            Some(user.id),
            None,
        )
        .await
        .unwrap()
        .code;
    let mut request = exchange(&client_id, "", code, REDIRECT);
    request.code_verifier = Some("x".repeat(43));
    let refused = env.server.token(request).await.unwrap_err();
    assert_eq!(refused.error, "invalid_grant");
}

#[tokio::test]
async fn a_public_client_cannot_present_a_secret_or_use_client_credentials() {
    let env = env().await;
    let client_id = public_client(&env).await;

    let mut with_secret = exchange(&client_id, "guess", "code".to_owned(), REDIRECT);
    with_secret.grant_type = "authorization_code".to_owned();
    assert_eq!(
        env.server.token(with_secret).await.unwrap_err().error,
        "invalid_client"
    );

    let mut machine = exchange(&client_id, "", String::new(), REDIRECT);
    machine.grant_type = "client_credentials".to_owned();
    machine.code = None;
    assert_eq!(
        env.server.token(machine).await.unwrap_err().error,
        "unauthorized_client"
    );
}

#[tokio::test]
async fn pkce_s256_is_required_to_authorize_a_public_client() {
    let env = env().await;
    let client_id = public_client(&env).await;

    let mut none = authorize_request(&client_id, REDIRECT, None);
    none.code_challenge = None;
    none.code_challenge_method = None;
    let AuthorizeRejection::RedirectedToClient(error) = rejection_of(&env, none).await else {
        panic!("a verified client's error goes back to it");
    };
    assert_eq!(error.error, "invalid_request");

    let mut plain = authorize_request(&client_id, REDIRECT, None);
    plain.code_challenge_method = Some("plain".to_owned());
    let AuthorizeRejection::RedirectedToClient(error) = rejection_of(&env, plain).await else {
        panic!("a verified client's error goes back to it");
    };
    assert_eq!(error.error, "invalid_request");
}

#[tokio::test]
async fn a_confidential_client_still_needs_its_secret() {
    let env = env().await;
    let client = env
        .registration
        .register_client(registration(REDIRECT, None), MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap();
    for secret in ["", "wrong"] {
        let refused = env
            .server
            .token(exchange(
                &client.client_id,
                secret,
                "code".to_owned(),
                REDIRECT,
            ))
            .await
            .unwrap_err();
        assert_eq!(refused.error, "invalid_client", "secret {secret:?}");
    }
}

// ============================================================================
// The token endpoint over HTTP: Basic, form and none
// ============================================================================

fn oauth2_routes(resources: &Arc<ServerContext>) -> Router {
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

/// A registration over HTTP, and an authorization code for it.
async fn registered_over_http(
    resources: &Arc<ServerContext>,
    method: &str,
    email: &str,
) -> (Value, String) {
    let response = AxumTestRequest::post("/oauth2/register")
        .json(&json!({
            "redirect_uris": [REDIRECT],
            "client_name": "HTTP Client",
            "token_endpoint_auth_method": method,
        }))
        .send(oauth2_routes(resources))
        .await;
    assert_eq!(response.status(), 201);
    let client: Value = response.json();

    let repos = &resources.common.repos;
    let user = User::new(email.to_owned(), "hash".to_owned(), Some("Http".to_owned()));
    repos.users.create(&user).await.unwrap();
    repos
        .tenants
        .create(&Tenant {
            id: TenantId::generate(),
            name: "Http Tenant".to_owned(),
            slug: format!("tenant-{}", user.id),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    let server = OAuth2AuthorizationServer::from_config(
        &resources.common.config.oauth2_server,
        repos.oauth2_server.clone(),
        repos.tenants.clone(),
        repos.users.clone(),
        resources.auth.auth_manager.clone(),
        resources.auth.jwks_manager.clone(),
        resources.common.config.auth.refresh_token_expiry_days,
    );
    let code = server
        .authorize(
            authorize_request(client["client_id"].as_str().unwrap(), REDIRECT, None),
            Some(user.id),
            None,
        )
        .await
        .unwrap()
        .code;
    (client, code)
}

fn basic(id: &str, secret: &str) -> String {
    format!(
        "Basic {}",
        general_purpose::STANDARD.encode(format!("{id}:{secret}"))
    )
}

#[tokio::test]
async fn the_token_endpoint_accepts_client_secret_basic() {
    let resources = create_test_server_resources().await.unwrap();
    let (client, code) =
        registered_over_http(&resources, "client_secret_basic", "basic@example.com").await;
    assert_eq!(client["token_endpoint_auth_method"], "client_secret_basic");
    let id = client["client_id"].as_str().unwrap();
    let secret = client["client_secret"].as_str().unwrap();

    let response = AxumTestRequest::post("/oauth2/token")
        .header("authorization", &basic(id, secret))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = response.json();
    assert!(body["access_token"].as_str().is_some());
    assert!(body["refresh_token"].as_str().is_some());
}

#[tokio::test]
async fn a_wrong_basic_secret_is_a_401_naming_the_scheme() {
    let resources = create_test_server_resources().await.unwrap();
    let (client, code) =
        registered_over_http(&resources, "client_secret_basic", "basic-bad@example.com").await;
    let id = client["client_id"].as_str().unwrap();

    let response = AxumTestRequest::post("/oauth2/token")
        .header("authorization", &basic(id, "not-the-secret"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(response.status(), 401);
    assert!(response
        .header("www-authenticate")
        .is_some_and(|v| v.starts_with("Basic")));
    assert_eq!(response.json::<Value>()["error"], "invalid_client");
}

#[tokio::test]
async fn credentials_sent_in_the_header_and_the_body_are_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let (client, code) =
        registered_over_http(&resources, "client_secret_basic", "basic-both@example.com").await;
    let id = client["client_id"].as_str().unwrap();
    let secret = client["client_secret"].as_str().unwrap();

    let response = AxumTestRequest::post("/oauth2/token")
        .header("authorization", &basic(id, secret))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_secret", secret),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(response.status(), 400);
    assert_eq!(response.json::<Value>()["error"], "invalid_request");
}

#[tokio::test]
async fn the_token_endpoint_still_accepts_client_secret_post() {
    let resources = create_test_server_resources().await.unwrap();
    let (client, code) =
        registered_over_http(&resources, "client_secret_post", "post@example.com").await;
    let id = client["client_id"].as_str().unwrap();
    let secret = client["client_secret"].as_str().unwrap();

    let response = AxumTestRequest::post("/oauth2/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", id),
            ("client_secret", secret),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
}

#[tokio::test]
async fn a_public_client_registers_and_redeems_over_http_with_no_secret() {
    let resources = create_test_server_resources().await.unwrap();
    let (client, code) = registered_over_http(&resources, "none", "none@example.com").await;
    assert_eq!(client["token_endpoint_auth_method"], "none");
    assert!(client.get("client_secret").is_none(), "{client}");
    let id = client["client_id"].as_str().unwrap();

    let response = AxumTestRequest::post("/oauth2/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", id),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let refresh_token = response.json::<Value>()["refresh_token"]
        .as_str()
        .unwrap()
        .to_owned();

    let refreshed = AxumTestRequest::post("/oauth2/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", id),
            ("refresh_token", refresh_token.as_str()),
        ])
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(refreshed.status(), 200, "{}", refreshed.body_text());
}

#[tokio::test]
async fn registration_over_http_refuses_client_credentials_and_oob() {
    let resources = create_test_server_resources().await.unwrap();
    for body in [
        json!({"redirect_uris": [REDIRECT], "grant_types": ["client_credentials"]}),
        json!({"redirect_uris": ["urn:ietf:wg:oauth:2.0:oob"]}),
        json!({"redirect_uris": [REDIRECT], "token_endpoint_auth_method": "private_key_jwt"}),
    ] {
        let response = AxumTestRequest::post("/oauth2/register")
            .json(&body)
            .send(oauth2_routes(&resources))
            .await;
        assert_eq!(response.status(), 400, "{body}");
    }
}

#[tokio::test]
async fn a_token_request_with_no_client_id_anywhere_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let response = AxumTestRequest::post("/oauth2/token")
        .form(&[("grant_type", "authorization_code"), ("code", "x")])
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(response.status(), 400);
    assert_eq!(response.json::<Value>()["error"], "invalid_request");
}

// Silence the unused-import lint when a fixture type is not needed above.
const _: fn(&OAuth2Client) = |_| {};
