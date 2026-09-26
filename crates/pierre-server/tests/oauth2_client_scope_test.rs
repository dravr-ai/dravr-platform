// ABOUTME: Pins the scope a dynamically registered OAuth client may be granted, from registration to minted token
// ABOUTME: The defaulted scope is persisted, names are checked against the vocabulary, and admin is never delegated
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # What an anonymously registered client can be granted
//!
//! Registration used to store `scope = NULL` while telling the client its
//! grant was the default, and the authorization endpoint read NULL as "no
//! restriction" — so any client registered anonymously could be authorized
//! for `fitness:write profile:write admin`, although `admin` is documented as
//! never delegated. Every test below asserts the granted scope itself: on the
//! old code each one reads a wider grant than it pins.

mod common;

use base64::{engine::general_purpose, Engine as _};
use chrono::{Duration, Utc};
use pierre_auth::auth::AuthManager;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::endpoints::OAuth2AuthorizationServer;
use pierre_auth::oauth2_server::models::{
    AuthorizeRequest, ClientRegistrationRequest, ClientRegistrationResponse, OAuth2Client,
    OAuth2RefreshToken, TokenRequest,
};
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::backends::{factory::Database, DatabaseProvider};
use pierre_database::database::generate_encryption_key;
use pierre_database::database::test_utils::create_test_db_with_key;
use sha2::{Digest, Sha256};
use std::sync::Arc;

const REDIRECT: &str = "https://app.example.com/callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

struct Env {
    database: Arc<Database>,
    auth_manager: Arc<AuthManager>,
    server: OAuth2AuthorizationServer,
    registration: ClientRegistrationManager,
}

async fn env() -> Env {
    let database = Arc::new(
        create_test_db_with_key(generate_encryption_key().to_vec())
            .await
            .unwrap(),
    );
    database.migrate().await.unwrap();
    let auth_manager = Arc::new(AuthManager::new(24));
    let repos = database.repositories();
    let server = OAuth2AuthorizationServer::new(
        repos.oauth2_server.clone(),
        repos.tenants.clone(),
        repos.users.clone(),
        auth_manager.clone(),
        common::get_shared_test_jwks(),
    );
    let registration = ClientRegistrationManager::new(repos.oauth2_server.clone());
    Env {
        database,
        auth_manager,
        server,
        registration,
    }
}

fn registration_request(scope: Option<&str>) -> ClientRegistrationRequest {
    ClientRegistrationRequest {
        redirect_uris: vec![REDIRECT.to_owned()],
        client_name: Some("Scope Test Client".to_owned()),
        client_uri: None,
        grant_types: None,
        response_types: None,
        scope: scope.map(str::to_owned),
    }
}

async fn register(env: &Env, scope: Option<&str>) -> ClientRegistrationResponse {
    env.registration
        .register_client(registration_request(scope), MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap()
}

async fn stored_scope(env: &Env, client_id: &str) -> Option<String> {
    env.database
        .repositories()
        .oauth2_server
        .get_client(client_id)
        .await
        .unwrap()
        .expect("the client row exists")
        .scope
}

async fn athlete(env: &Env, email: &str) -> User {
    let user = User::new(
        email.to_owned(),
        "hash".to_owned(),
        Some("Scoped".to_owned()),
    );
    let repos = env.database.repositories();
    repos.users.create(&user).await.unwrap();
    repos
        .tenants
        .create(&Tenant {
            id: TenantId::generate(),
            name: "Scope Tenant".to_owned(),
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

fn challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

fn authorize_request(client_id: &str, scope: Option<&str>) -> AuthorizeRequest {
    AuthorizeRequest {
        response_type: "code".to_owned(),
        client_id: client_id.to_owned(),
        redirect_uri: REDIRECT.to_owned(),
        scope: scope.map(str::to_owned),
        state: None,
        code_challenge: Some(challenge()),
        code_challenge_method: Some("S256".to_owned()),
    }
}

/// The `scope` claim of a minted access token, as the resource server reads it.
fn token_grant(env: &Env, access_token: &str) -> Vec<OAuthScope> {
    let claims = env
        .auth_manager
        .validate_token(access_token, &common::get_shared_test_jwks())
        .unwrap();
    OAuthScope::parse_granted(&claims.scope)
}

// ============================================================================
// Registration
// ============================================================================

#[tokio::test]
async fn registration_without_a_scope_persists_the_default_it_advertises() {
    let env = env().await;
    let client = register(&env, None).await;

    assert_eq!(client.scope.as_deref(), Some("fitness:read profile:read"));
    assert_eq!(
        stored_scope(&env, &client.client_id).await.as_deref(),
        Some("fitness:read profile:read"),
        "the row holds what the response promised, not NULL"
    );
}

#[tokio::test]
async fn registration_stores_a_requested_scope_in_canonical_form() {
    let env = env().await;
    let client = register(&env, Some("profile:write fitness:read fitness:read")).await;

    assert_eq!(client.scope.as_deref(), Some("fitness:read profile:write"));
    assert_eq!(
        stored_scope(&env, &client.client_id).await.as_deref(),
        Some("fitness:read profile:write")
    );
}

#[tokio::test]
async fn registration_refuses_admin_and_names_outside_the_vocabulary() {
    let env = env().await;

    for (scope, named) in [
        ("fitness:read admin", "'admin' scope is never delegated"),
        (
            "read:fitness write:fitness",
            "unknown OAuth scope 'read:fitness'",
        ),
    ] {
        let refused = env
            .registration
            .register_client(registration_request(Some(scope)), MAX_PENDING_REGISTRATIONS)
            .await
            .expect_err("the registration must be refused");
        assert_eq!(refused.error, "invalid_client_metadata", "for {scope}");
        let description = refused.error_description.unwrap_or_default();
        assert!(description.contains(named), "for {scope}: {description}");
    }
}

// ============================================================================
// Authorization
// ============================================================================

#[tokio::test]
async fn a_default_client_is_refused_a_grant_it_never_registered() {
    let env = env().await;
    let user = athlete(&env, "scope-default@example.com").await;
    let client = register(&env, None).await;

    let refused = env
        .server
        .authorize(
            authorize_request(&client.client_id, Some("fitness:read fitness:write")),
            Some(user.id),
            None,
        )
        .await
        .expect_err("fitness:write is outside the default grant");
    assert_eq!(refused.error, "invalid_scope");
    assert!(refused
        .error_description
        .unwrap_or_default()
        .contains("'fitness:write'"));
}

#[tokio::test]
async fn admin_is_never_authorized_even_for_a_row_registered_before_the_check() {
    let env = env().await;
    let user = athlete(&env, "scope-legacy@example.com").await;
    let repos = env.database.repositories();

    // Rows the old registration wrote: NULL, and every published scope
    // including `admin` (what a spec-following MCP client asked for).
    for (client_id, scope) in [
        ("legacy_null_client", None),
        (
            "legacy_admin_client",
            Some("fitness:read fitness:write profile:read profile:write admin"),
        ),
    ] {
        let stored = repos
            .oauth2_server
            .store_client_within_ceiling(
                &OAuth2Client {
                    id: uuid::Uuid::new_v4().to_string(),
                    client_id: client_id.to_owned(),
                    client_secret_hash: "unused".to_owned(),
                    redirect_uris: vec![REDIRECT.to_owned()],
                    grant_types: vec!["authorization_code".to_owned()],
                    response_types: vec!["code".to_owned()],
                    client_name: None,
                    client_uri: None,
                    scope: scope.map(str::to_owned),
                    created_at: Utc::now(),
                    expires_at: None,
                },
                u64::MAX,
            )
            .await
            .unwrap();
        assert!(stored, "no ceiling can refuse a client under u64::MAX");

        let refused = env
            .server
            .authorize(
                authorize_request(client_id, Some("fitness:read admin")),
                Some(user.id),
                None,
            )
            .await
            .expect_err("admin is never delegated");
        assert_eq!(refused.error, "invalid_scope", "for {client_id}");
    }

    // NULL reads as the default the client was told it had, never as "no
    // restriction".
    let refused = env
        .server
        .authorize(
            authorize_request("legacy_null_client", Some("fitness:write")),
            Some(user.id),
            None,
        )
        .await
        .expect_err("a NULL row is the default grant");
    assert_eq!(refused.error, "invalid_scope");
}

#[tokio::test]
async fn an_authorization_without_a_scope_carries_the_default_grant_into_the_token() {
    let env = env().await;
    let user = athlete(&env, "scope-omitted@example.com").await;
    let client = register(&env, None).await;

    let code = env
        .server
        .authorize(
            authorize_request(&client.client_id, None),
            Some(user.id),
            None,
        )
        .await
        .unwrap()
        .code;
    let token = env
        .server
        .token(TokenRequest {
            grant_type: "authorization_code".to_owned(),
            code: Some(code),
            redirect_uri: Some(REDIRECT.to_owned()),
            client_id: client.client_id,
            client_secret: client.client_secret,
            scope: None,
            refresh_token: None,
            code_verifier: Some(VERIFIER.to_owned()),
        })
        .await
        .unwrap();

    assert_eq!(token.scope.as_deref(), Some("fitness:read profile:read"));
    assert_eq!(
        token_grant(&env, &token.access_token),
        vec![OAuthScope::FitnessRead, OAuthScope::ProfileRead],
        "the token carries the grant the consent screen showed, not an empty one"
    );
}

#[tokio::test]
async fn a_refresh_token_holding_admin_mints_a_token_without_it() {
    let env = env().await;
    let user = athlete(&env, "scope-refresh@example.com").await;
    let client = register(&env, None).await;
    let tenant = env
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap();

    // A refresh token issued before `admin` was refused at authorization.
    env.database
        .repositories()
        .oauth2_server
        .store_refresh_token(&OAuth2RefreshToken {
            token: "legacy-refresh-token-with-admin".to_owned(),
            client_id: client.client_id.clone(),
            user_id: user.id,
            tenant_id: tenant[0].id.to_string(),
            scope: Some("fitness:read admin".to_owned()),
            expires_at: Utc::now() + Duration::days(30),
            created_at: Utc::now(),
            revoked: false,
        })
        .await
        .unwrap();

    let token = env
        .server
        .token(TokenRequest {
            grant_type: "refresh_token".to_owned(),
            code: None,
            redirect_uri: None,
            client_id: client.client_id,
            client_secret: client.client_secret,
            scope: None,
            refresh_token: Some("legacy-refresh-token-with-admin".to_owned()),
            code_verifier: None,
        })
        .await
        .unwrap();

    assert_eq!(token.scope.as_deref(), Some("fitness:read"));
    let grant = token_grant(&env, &token.access_token);
    assert_eq!(grant, vec![OAuthScope::FitnessRead]);
    assert!(
        !OAuthScope::is_self_grant(&grant),
        "every delegated token stays narrower than the athlete's own grant"
    );
}

// ============================================================================
// Metadata
// ============================================================================

#[test]
fn scopes_supported_publishes_only_what_a_client_can_be_granted() {
    // A spec-following MCP client registers and authorizes with exactly this
    // list, so a name the server refuses here would fail every connection.
    assert_eq!(
        OAuth2AuthorizationServer::supported_scopes(),
        vec![
            "fitness:read",
            "fitness:write",
            "profile:read",
            "profile:write"
        ]
    );
}
