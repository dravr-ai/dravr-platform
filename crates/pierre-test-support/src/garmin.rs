// ABOUTME: Garmin athlete fixtures: an athlete in their own tenant, the usr/pwd session, the API grant pushing wellness
// ABOUTME: The rows the sciotte login and the Garmin OAuth callback leave, for tests of the pull standing down for the push
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::oauth::providers::TOKEN_TYPE_SESSION;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConnectionType, Tenant, TenantId, UserOAuthToken};
use pierre_database::RepositoryRegistry;
use secrecy::SecretString;
use uuid::Uuid;

use crate::server::create_test_user;

/// The token row and connection the usr/pwd session is stored under.
pub const SESSION_BACKEND: &str = "sciotte_garmin";

/// The token row and connection the Garmin API grant is stored under.
pub const PUSH_BACKEND: &str = "garmin";

/// Create an athlete owning a tenant of their own, holding nothing yet.
///
/// # Errors
///
/// Returns an error if the user or the tenant cannot be stored.
pub async fn create_athlete(repos: &RepositoryRegistry) -> AppResult<(Uuid, TenantId)> {
    let user = create_test_user(&format!("{}@example.com", Uuid::new_v4()), None);
    repos.users.create(&user).await?;
    let tenant = TenantId::generate();
    let now = Timestamp::now();
    repos
        .tenants
        .create(&Tenant {
            id: tenant,
            name: format!("Tenant {tenant}"),
            slug: format!("tenant-{tenant}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: now,
            updated_at: now,
        })
        .await?;
    Ok((user.id, tenant))
}

/// The usr/pwd session the sciotte login stores as the token row's access
/// token: a session `AuthSession` deserializes, holding no expiry of its own.
const GARMIN_SESSION: &str = r#"{"session_id":"garmin-session","cookies":[{"name":"SESSIONID",
    "value":"t","domain":".garmin.com","path":"/","secure":true,"http_only":true}],
    "created_at":"2026-09-20T00:00:00Z"}"#;

/// Store `token`, and register its live connection of `connection` type in
/// `tenant`, the tenant the row names.
async fn connect(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    token: &UserOAuthToken,
    connection: &ConnectionType,
) -> AppResult<()> {
    repos.oauth_tokens.upsert_token(token).await?;
    repos
        .provider_connections
        .register_connection(token.user_id, tenant, &token.provider, connection, None)
        .await
}

/// `backend`'s token row for `user` in `tenant`, carrying `access_token`.
fn token_row(
    user: Uuid,
    tenant: TenantId,
    backend: &str,
    access_token: String,
    token_type: &str,
    expires_at: Option<Timestamp>,
) -> UserOAuthToken {
    let now = Timestamp::now();
    UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id: user,
        tenant_id: tenant.to_string(),
        provider: backend.to_owned(),
        access_token: SecretString::from(access_token),
        refresh_token: None,
        token_type: token_type.to_owned(),
        expires_at,
        scope: None,
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// The usr/pwd Garmin session and its live connection, as the sciotte login
/// leaves them: the session itself as the access token a scraper reads, with
/// no expiry, under a manual connection.
///
/// # Errors
///
/// Returns an error if the token row or the connection cannot be stored.
pub async fn connect_garmin_session(
    repos: &RepositoryRegistry,
    user: Uuid,
    tenant: TenantId,
) -> AppResult<()> {
    let session = token_row(
        user,
        tenant,
        SESSION_BACKEND,
        GARMIN_SESSION.to_owned(),
        TOKEN_TYPE_SESSION,
        None,
    );
    connect(repos, tenant, &session, &ConnectionType::Manual).await
}

/// The Garmin API grant pushing wellness, as the OAuth callback and Garmin's
/// permission report leave it.
///
/// # Errors
///
/// Returns an error if a row cannot be stored, or the connection takes no
/// grants.
pub async fn grant_garmin_push(
    repos: &RepositoryRegistry,
    user: Uuid,
    tenant: TenantId,
) -> AppResult<()> {
    let grant = token_row(
        user,
        tenant,
        PUSH_BACKEND,
        format!("{PUSH_BACKEND}-access"),
        "Bearer",
        Some(Timestamp::now() + Elapsed::hours(1)),
    );
    connect(repos, tenant, &grant, &ConnectionType::OAuth).await?;
    let grants = vec!["ACTIVITY_EXPORT".to_owned(), "HEALTH_EXPORT".to_owned()];
    if repos
        .provider_connections
        .set_provider_grants(user, tenant, PUSH_BACKEND, &grants)
        .await?
    {
        Ok(())
    } else {
        Err(AppError::internal(
            "the Garmin connection registered above took no grants",
        ))
    }
}

/// An athlete in their own tenant holding the usr/pwd Garmin session beside
/// the API grant pushing their wellness: the push feeds it, so no pull reads
/// the session.
///
/// # Errors
///
/// Returns an error if any of the rows cannot be stored.
pub async fn create_push_fed_athlete(repos: &RepositoryRegistry) -> AppResult<(Uuid, TenantId)> {
    let (user, tenant) = create_athlete(repos).await?;
    connect_garmin_session(repos, user, tenant).await?;
    grant_garmin_push(repos, user, tenant).await?;
    Ok((user, tenant))
}
