// ABOUTME: The OAuth client a grant belongs to: one resolution order for authorize, exchange, refresh and revoke
// ABOUTME: Strava pool app, the user's own app, the tenant's credentials, then the server-level app
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::oauth::OAuthProviderConfig;
use crate::strava_pool::select_strava_app;
use pierre_core::constants::oauth_providers;
use pierre_core::constants::rate_limits::{
    GARMIN_DEFAULT_DAILY_RATE_LIMIT, STRAVA_RATE_LIMIT_DAILY, TERRA_DEFAULT_DAILY_RATE_LIMIT,
    WHOOP_DEFAULT_DAILY_RATE_LIMIT,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, TenantOAuthCredentials, UserOAuthApp};
use pierre_database::backends::{OAuthTokenRepository, TenantRepository};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Where [`issuing_client`] found the client a token belongs to.
#[derive(Debug, Clone)]
pub enum IssuingClient {
    /// The Strava shared-pool app the token names.
    StravaPool {
        /// The pool app's client identifier.
        client_id: String,
        /// The pool app's client secret.
        client_secret: String,
    },
    /// The user's own OAuth app for the provider.
    UserApp(UserOAuthApp),
    /// The tenant's OAuth credentials for the provider.
    Tenant(TenantOAuthCredentials),
    /// The server-level app the environment configures.
    ServerLevel {
        /// The server-level client identifier.
        client_id: String,
        /// The server-level client secret.
        client_secret: String,
    },
}

impl IssuingClient {
    /// The client identifier, whichever source it came from.
    #[must_use]
    pub fn client_id(&self) -> &str {
        match self {
            Self::StravaPool { client_id, .. } | Self::ServerLevel { client_id, .. } => client_id,
            Self::UserApp(app) => &app.client_id,
            Self::Tenant(credentials) => &credentials.client_id,
        }
    }

    /// The client secret, whichever source it came from.
    #[must_use]
    pub fn client_secret(&self) -> &str {
        match self {
            Self::StravaPool { client_secret, .. } | Self::ServerLevel { client_secret, .. } => {
                client_secret
            }
            Self::UserApp(app) => &app.client_secret,
            Self::Tenant(credentials) => &credentials.client_secret,
        }
    }
}

/// What [`issuing_client`] resolves.
#[derive(Debug, Clone, Copy)]
pub struct IssuingLookup<'a> {
    /// The user the token belongs to, when there is one.
    pub user_id: Option<Uuid>,
    /// The tenant the token was issued in, when known.
    pub tenant_id: Option<TenantId>,
    /// The provider slug.
    pub provider: &'a str,
    /// The Strava shared-pool app that issued the token, as the stored token
    /// or the pinned authorization state names it. `None` for the env app
    /// and every other provider.
    pub issuing_app: Option<&'a str>,
    /// The server-level credentials for `provider`.
    pub server_level: &'a OAuthProviderConfig,
}

/// The client an issued token belongs to: the one a refresh, a code exchange
/// or a revocation must present, since a grant is bound to the client that
/// issued it.
///
/// Resolution order:
/// 1. Strava only: the shared-pool app `issuing_app` names, while its secret
///    is stored, whatever user or tenant credentials were configured since
/// 2. The user's own OAuth app (`user_oauth_app_credentials`)
/// 3. The tenant's credentials (the `tenants` repository)
/// 4. The server-level credentials
///
/// This is the one place the order is written; the token refresh, the code
/// exchange and the revocation all resolve through it, and a new
/// authorization resolves through [`authorizing_client`], which keeps it.
///
/// # Errors
///
/// Returns an error when the pool app's or the tenant's credentials cannot
/// be read (a failed read is not a missing app: reading it as one would
/// present another client), or when no source holds credentials for the
/// provider.
pub async fn issuing_client(
    lookup: IssuingLookup<'_>,
    tenants: &dyn TenantRepository,
    oauth_tokens: &dyn OAuthTokenRepository,
) -> AppResult<IssuingClient> {
    if let Some(pool_app) = issuing_pool_app(&lookup, oauth_tokens).await? {
        return Ok(pool_app);
    }
    if let Some(user_id) = lookup.user_id {
        if let Some(app) = user_app(user_id, lookup.provider, oauth_tokens).await {
            return Ok(IssuingClient::UserApp(app));
        }
    }
    if let Some(credentials) = tenant_credentials(&lookup, tenants).await? {
        return Ok(IssuingClient::Tenant(credentials));
    }
    server_level_app(&lookup).ok_or_else(|| no_credentials(lookup.provider))
}

/// The client a new authorization for `user_id` runs under, and the Strava
/// shared-pool app it belongs to (`None` for every other source).
///
/// Resolution order, [`issuing_client`]'s with the seat rules where a token
/// would name its app, since no grant exists yet:
/// 1. The user's own OAuth app (`user_oauth_app_credentials`)
/// 2. The tenant's credentials (the `tenants` repository)
/// 3. Strava only: the app [`select_strava_app`] picks — the one the athlete's
///    grant holds a seat on, else the env app, then a pool app with a free seat
/// 4. The server-level credentials
///
/// The caller pins the pool app on the state it stores, so the code exchange
/// resolves the same client through [`issuing_client`].
///
/// # Errors
///
/// Returns an error when a credential read fails, when every Strava app is at
/// capacity, or when no source holds credentials for the provider.
pub async fn authorizing_client(
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    server_level: &OAuthProviderConfig,
    tenants: &dyn TenantRepository,
    oauth_tokens: &dyn OAuthTokenRepository,
) -> AppResult<(IssuingClient, Option<String>)> {
    find_authorizing_client(
        user_id,
        tenant_id,
        provider,
        server_level,
        tenants,
        oauth_tokens,
    )
    .await?
    .ok_or_else(|| no_credentials(provider))
}

/// [`authorizing_client`]'s resolution, `None` when no source holds credentials.
///
/// The athlete can then connect only through an OAuth app of their own,
/// which the provider status says before a connect is tried.
///
/// # Errors
///
/// Returns an error when a credential read fails or when every Strava app is
/// at capacity.
pub async fn find_authorizing_client(
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    server_level: &OAuthProviderConfig,
    tenants: &dyn TenantRepository,
    oauth_tokens: &dyn OAuthTokenRepository,
) -> AppResult<Option<(IssuingClient, Option<String>)>> {
    let lookup = IssuingLookup {
        user_id: Some(user_id),
        tenant_id: Some(tenant_id),
        provider,
        issuing_app: None,
        server_level,
    };
    if let Some(app) = user_app(user_id, provider, oauth_tokens).await {
        return Ok(Some((IssuingClient::UserApp(app), None)));
    }
    if let Some(credentials) = tenant_credentials(&lookup, tenants).await? {
        return Ok(Some((IssuingClient::Tenant(credentials), None)));
    }
    if provider.eq_ignore_ascii_case(oauth_providers::STRAVA) {
        let selected = select_strava_app(oauth_tokens, user_id, tenant_id).await?;
        let client = if selected.attribution.is_some() {
            IssuingClient::StravaPool {
                client_id: selected.client_id,
                client_secret: selected.client_secret,
            }
        } else {
            IssuingClient::ServerLevel {
                client_id: selected.client_id,
                client_secret: selected.client_secret,
            }
        };
        return Ok(Some((client, selected.attribution)));
    }
    Ok(server_level_app(&lookup).map(|client| (client, None)))
}

/// The Strava shared-pool app the lookup names, while its secret is stored.
async fn issuing_pool_app(
    lookup: &IssuingLookup<'_>,
    oauth_tokens: &dyn OAuthTokenRepository,
) -> AppResult<Option<IssuingClient>> {
    let Some(app) = lookup.issuing_app.filter(|_| {
        lookup
            .provider
            .eq_ignore_ascii_case(oauth_providers::STRAVA)
    }) else {
        return Ok(None);
    };
    let Some(client_secret) = oauth_tokens.get_strava_pool_app_secret(app).await? else {
        warn!(
            client_id = %app,
            "The Strava pool app that issued the token is no longer registered"
        );
        return Ok(None);
    };
    Ok(Some(IssuingClient::StravaPool {
        client_id: app.to_owned(),
        client_secret,
    }))
}

/// The user's own OAuth app for `provider`. A lookup that fails is logged
/// and read as no app, as every credential path has read it.
async fn user_app(
    user_id: Uuid,
    provider: &str,
    oauth_tokens: &dyn OAuthTokenRepository,
) -> Option<UserOAuthApp> {
    match oauth_tokens.get_user_oauth_app(user_id, provider).await {
        Ok(Some(app)) => {
            info!(
                "Using user-specific {} OAuth credentials for user {} (client_id={})",
                provider, user_id, app.client_id
            );
            Some(app)
        }
        Ok(None) => {
            debug!(
                "No user-specific {} OAuth credentials found for user {}",
                provider, user_id
            );
            None
        }
        Err(e) => {
            warn!(
                "Error fetching user-specific {} OAuth credentials for user {}: {}",
                provider, user_id, e
            );
            None
        }
    }
}

/// The tenant's credentials for the lookup's provider, from the `tenants`
/// repository. `None` without a tenant.
async fn tenant_credentials(
    lookup: &IssuingLookup<'_>,
    tenants: &dyn TenantRepository,
) -> AppResult<Option<TenantOAuthCredentials>> {
    let Some(tenant_id) = lookup.tenant_id else {
        return Ok(None);
    };
    let credentials = tenants
        .get_oauth_credentials(tenant_id, lookup.provider)
        .await?;
    if credentials.is_some() {
        debug!(
            "Using tenant-specific {} OAuth credentials for tenant {}",
            lookup.provider, tenant_id
        );
    }
    Ok(credentials)
}

/// The server-level app for the lookup's provider, when the environment
/// configures one.
fn server_level_app(lookup: &IssuingLookup<'_>) -> Option<IssuingClient> {
    let server_level = lookup.server_level;
    Some(IssuingClient::ServerLevel {
        client_id: server_level.client_id.clone()?,
        client_secret: server_level.client_secret.clone()?,
    })
}

/// The error naming what to configure when no source holds credentials for
/// `provider`.
fn no_credentials(provider: &str) -> AppError {
    let upper = provider.to_uppercase();
    AppError::not_found(format!(
        "No OAuth credentials configured for provider {provider}: set {upper}_CLIENT_ID and {upper}_CLIENT_SECRET, or configure the tenant's or the user's own OAuth app"
    ))
}

/// The daily request budget a tenant's OAuth app for `provider` is registered
/// with when its operator names none (`pierre-cli tenant set-oauth-app`).
#[must_use]
pub fn default_rate_limit_for_provider(provider: &str) -> u32 {
    match provider.to_lowercase().as_str() {
        "strava" => STRAVA_RATE_LIMIT_DAILY,
        "garmin" => GARMIN_DEFAULT_DAILY_RATE_LIMIT,
        "whoop" => WHOOP_DEFAULT_DAILY_RATE_LIMIT,
        "terra" => TERRA_DEFAULT_DAILY_RATE_LIMIT,
        _ => 1000, // Default fallback
    }
}
