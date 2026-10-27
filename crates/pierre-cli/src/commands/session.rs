// ABOUTME: `pierre-cli user mint-session` — a mobile app session for an account, minted as the hosted sign-in's code exchange mints it
// ABOUTME: For the mobile E2E lanes, whose flows that do not test sign-in start signed in without opening the sign-in sheet
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A first-party mobile session without the hosted login page (carnet#1201).
//!
//! The mobile app signs in by redeeming the authorization code the hosted
//! login page issued (`POST /oauth/token`, `grant_type=authorization_code`,
//! `scope=offline_access`). On the CI iOS simulator the sheet that page opens
//! in can stay blank for minutes, so the Maestro flows whose subject is not
//! sign-in start signed in instead: the e2e control process runs this command
//! and hands the pair to the app's E2E-only session link.
//!
//! The pair is the one the code exchange issues, minted by the same two calls
//! that exchange makes once the code is redeemed —
//! [`AuthService::sign_in_with_code`] for the JWT and
//! [`AuthService::issue_refresh_token`] for the refresh token — for the
//! `dravr-mobile` client, on the tenant the authorization endpoint would have
//! put in the code, with the expiry the server's configuration sets. Nothing
//! here signs a token itself. The sign-in is logged and raises `user.login`
//! as any other does, its `source` naming this command.
//!
//! It signs an account in with no credential, so it is kept out of every
//! `pierre-cli` but the E2E lanes' one: the module exists only with the
//! `e2e-seeded-sessions` feature, which the server image's build never names,
//! and it refuses any database that is not a local `SQLite` file
//! ([`require_local_database`]), so even a build with the feature cannot reach
//! a deployed environment's accounts.

use std::sync::Arc;

use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::AuthManager;
use pierre_auth::oauth2_server::first_party::{MOBILE_CLIENT_ID, OFFLINE_ACCESS_SCOPE};
use pierre_cache::{Cache, CacheConfig};
use pierre_config::environment::ServerConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::factory::Database;
use pierre_middleware::mask_email;
use pierre_providers::ProviderRegistry;
use pierre_runtime_context::DataContext;
use pierre_services::auth::{AuthService, SessionOrigin};
use secrecy::ExposeSecret;
use serde::Serialize;

/// The minted session, printed as one JSON line: what `POST /oauth/token`
/// answers the mobile app's code exchange with, less the cookies and CSRF
/// token a browser needs.
#[derive(Serialize)]
struct MintedSession<'session> {
    access_token: &'session str,
    refresh_token: &'session str,
    token_type: &'static str,
    expires_at: &'session str,
    scope: &'static str,
    client_id: &'static str,
    user_id: &'session str,
}

/// Refuse `database_url` unless it names a local `SQLite` database.
///
/// The seeded session exists for the E2E lanes and the dev stack, which serve
/// a `SQLite` file; a deployed environment is Postgres. Checked before any key
/// or connection is opened.
///
/// # Errors
/// Returns an error naming the policy when the URL is not `sqlite:`.
pub(crate) fn require_local_database(database_url: &str) -> AppResult<()> {
    if database_url.starts_with("sqlite:") {
        Ok(())
    } else {
        Err(AppError::invalid_input(
            "user mint-session runs only against a local SQLite database (DATABASE_URL=sqlite:...)",
        ))
    }
}

/// Mint a `dravr-mobile` session for `email` and print it to stdout as JSON.
///
/// The caller has already passed the URL through [`require_local_database`].
///
/// # Errors
/// Returns an error when no account has that email, the account belongs to
/// no tenant or is suspended, or the session cannot be stored.
pub(crate) async fn mint_session(
    database: Database,
    jwks_manager: JwksManager,
    email: &str,
) -> AppResult<()> {
    // The server's own configuration readers, so JWT_EXPIRY_HOURS and
    // REFRESH_TOKEN_EXPIRY_DAYS set the expiry here as they do in the server.
    let config = ServerConfig::from_env()?;
    let jwt_expiry_hours = i64::try_from(config.auth.jwt_expiry_hours)
        .map_err(|_| AppError::config("JWT_EXPIRY_HOURS does not fit a session expiry"))?;
    let auth_manager = AuthManager::new(jwt_expiry_hours);
    // Arc: AuthService holds its collaborators shared, as every server handler does.
    let repos = Arc::clone(database.repositories());
    let cache = Cache::new(CacheConfig {
        enable_background_cleanup: false,
        ..CacheConfig::default()
    })
    .await?;
    let data = DataContext::new(
        Arc::new(database),
        Arc::clone(&repos),
        Arc::new(cache),
        Arc::new(ProviderRegistry::new()),
    );
    let auth_service = AuthService::new(
        Arc::new(auth_manager),
        Arc::new(jwks_manager),
        Arc::new(config),
        data,
    );

    let user = repos
        .users
        .get_by_email(email)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User with email {}", mask_email(email))))?;
    // The tenant the authorization endpoint puts in a code when the sign-in
    // names none.
    let tenant_id = repos
        .tenants
        .first_for_user(user.id)
        .await?
        .ok_or_else(|| {
            AppError::invalid_input("The account belongs to no tenant, so it cannot sign in")
        })?;

    let login = auth_service
        .sign_in_with_code(
            user.id,
            &tenant_id.to_string(),
            MOBILE_CLIENT_ID,
            SessionOrigin::CliMintedSession,
        )
        .await?;
    let refresh_token = auth_service
        .issue_refresh_token(
            user.id,
            login.user.tenant_id.clone(),
            MOBILE_CLIENT_ID.to_owned(),
        )
        .await?;
    let access_token = login
        .jwt_token
        .as_ref()
        .ok_or_else(|| AppError::internal("JWT token missing from the minted session"))?;

    let minted = MintedSession {
        access_token: access_token.expose_secret(),
        refresh_token: &refresh_token,
        token_type: "Bearer",
        expires_at: &login.expires_at,
        scope: OFFLINE_ACCESS_SCOPE,
        client_id: MOBILE_CLIENT_ID,
        user_id: &login.user.user_id,
    };
    let line = serde_json::to_string(&minted)
        .map_err(|e| AppError::internal(format!("Failed to encode the minted session: {e}")))?;
    println!("{line}");
    Ok(())
}
