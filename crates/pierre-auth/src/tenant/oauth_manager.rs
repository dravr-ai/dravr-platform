// ABOUTME: Per-tenant OAuth credential management for isolated multi-tenant operation
// ABOUTME: Handles secure storage, encryption, and retrieval of tenant-specific OAuth applications
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::oauth::{OAuthConfig, OAuthProviderConfig};
use crate::strava_pool::select_strava_app;
use chrono::Utc;
use pierre_core::constants::oauth_providers;
use pierre_core::constants::rate_limits::{
    GARMIN_DEFAULT_DAILY_RATE_LIMIT, STRAVA_DEFAULT_DAILY_RATE_LIMIT,
    TERRA_DEFAULT_DAILY_RATE_LIMIT, WHOOP_DEFAULT_DAILY_RATE_LIMIT,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, TenantOAuthCredentials, UserOAuthApp};
use pierre_database::backends::{OAuthTokenRepository, TenantRepository};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Credential configuration for storing OAuth credentials
#[derive(Debug, Clone)]
pub struct CredentialConfig {
    /// OAuth client ID (public)
    pub client_id: String,
    /// OAuth client secret (to be encrypted)
    pub client_secret: String,
    /// OAuth redirect URI
    pub redirect_uri: String,
    /// OAuth scopes
    pub scopes: Vec<String>,
    /// User who configured these credentials
    pub configured_by: Uuid,
}

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
    /// Tenant credentials held in memory, read before the database: the
    /// tenant OAuth manager's cache. `None` reads the database alone.
    pub cached_tenant: Option<&'a TenantOAuthCredentials>,
}

/// The client an issued token belongs to: the one a refresh, a code exchange
/// or a revocation must present, since a grant is bound to the client that
/// issued it.
///
/// Resolution order:
/// 1. Strava only: the shared-pool app `issuing_app` names, while its secret
///    is stored, whatever user or tenant credentials were configured since
/// 2. The user's own OAuth app (`user_oauth_app_credentials`)
/// 3. The tenant's credentials (`cached_tenant`, then the database)
/// 4. The server-level credentials
///
/// This is the one place the order is written; the tenant OAuth manager, the
/// token refresh and the code exchange all resolve through it.
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
    server_level_client(&lookup)
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

/// The tenant's credentials for the lookup's provider: the cached entry,
/// else the database's. `None` without a tenant.
async fn tenant_credentials(
    lookup: &IssuingLookup<'_>,
    tenants: &dyn TenantRepository,
) -> AppResult<Option<TenantOAuthCredentials>> {
    let Some(tenant_id) = lookup.tenant_id else {
        return Ok(None);
    };
    if let Some(cached) = lookup.cached_tenant {
        return Ok(Some(cached.clone())); // Safe: the cache keeps its entry
    }
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

/// The server-level app for the lookup's provider, or the error naming what
/// to configure when there is none.
fn server_level_client(lookup: &IssuingLookup<'_>) -> AppResult<IssuingClient> {
    if let (Some(client_id), Some(client_secret)) = (
        &lookup.server_level.client_id,
        &lookup.server_level.client_secret,
    ) {
        return Ok(IssuingClient::ServerLevel {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
        });
    }
    let provider = lookup.provider;
    let upper = provider.to_uppercase();
    Err(AppError::not_found(format!(
        "No OAuth credentials configured for provider {provider}: set {upper}_CLIENT_ID and {upper}_CLIENT_SECRET, or configure the tenant's or the user's own OAuth app"
    )))
}

/// Manager for tenant-specific OAuth credentials.
///
/// Maintains a per-process cache of tenant OAuth credentials plus per-process
/// daily usage counters for rate limiting. Durable credential storage is the
/// `tenants` repository (the source of truth); the MCP credential-store path
/// persists there in addition to populating this cache.
pub struct TenantOAuthManager {
    /// Per-process cache of tenant credentials; the durable copy lives in the tenants repository.
    credentials: HashMap<(TenantId, String), TenantOAuthCredentials>,
    /// Per-process daily usage counters used for rate limiting.
    usage_tracking: HashMap<(TenantId, String, chrono::NaiveDate), u32>,
    // Server-level OAuth configuration (read once at startup)
    oauth_config: Arc<OAuthConfig>,
}

impl TenantOAuthManager {
    /// Create new OAuth manager with server-level configuration
    #[must_use]
    pub fn new(oauth_config: Arc<OAuthConfig>) -> Self {
        Self {
            credentials: HashMap::new(),
            usage_tracking: HashMap::new(),
            oauth_config,
        }
    }

    /// Load OAuth credentials for a specific tenant and provider
    ///
    /// # Errors
    ///
    /// Returns an error if no credentials are found for the tenant/provider combination
    pub async fn get_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<TenantOAuthCredentials> {
        self.get_credentials_for_user(None, tenant_id, provider, tenants, oauth_tokens)
            .await
    }

    /// Load OAuth credentials with user-specific priority: the client of the
    /// user's stored token, resolved by [`issuing_client`] with the Strava
    /// pool app that token names, this manager's server-level configuration
    /// and its tenant cache.
    ///
    /// # Errors
    ///
    /// Returns an error if a lookup fails or no credentials are found for the
    /// user/tenant/provider combination
    pub async fn get_credentials_for_user(
        &self,
        user_id: Option<Uuid>,
        tenant_id: TenantId,
        provider: &str,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<TenantOAuthCredentials> {
        // The Strava pool app that issued the user's stored token, if any.
        let issuing_app = match user_id {
            Some(uid) if provider.eq_ignore_ascii_case(oauth_providers::STRAVA) => oauth_tokens
                .get_token(uid, tenant_id, oauth_providers::STRAVA)
                .await
                .ok()
                .flatten()
                .and_then(|t| t.oauth_app_client_id),
            _ => None,
        };
        let server_level = self
            .oauth_config
            .provider(provider)
            .cloned()
            .unwrap_or_default();
        let client = issuing_client(
            IssuingLookup {
                user_id,
                tenant_id: Some(tenant_id),
                provider,
                issuing_app: issuing_app.as_deref(),
                server_level: &server_level,
                cached_tenant: self.credentials.get(&(tenant_id, provider.to_owned())),
            },
            tenants,
            oauth_tokens,
        )
        .await?;

        match client {
            IssuingClient::StravaPool {
                client_id,
                client_secret,
            } => Ok(self.strava_credentials(tenant_id, client_id, client_secret)),
            IssuingClient::UserApp(app) => Ok(Self::user_app_credentials(tenant_id, provider, app)),
            IssuingClient::Tenant(credentials) => Ok(credentials),
            IssuingClient::ServerLevel { .. } => self
                .try_server_level_credentials(tenant_id, provider)
                .ok_or_else(|| {
                    AppError::not_found(format!(
                        "No server-level OAuth credentials for provider {provider}"
                    ))
                }),
        }
    }

    /// The credentials a new authorization for `user_id` runs under, with the
    /// Strava shared-pool app they belong to.
    ///
    /// Resolution order, the one the code exchange resolves the pinned state
    /// by (`OAuthService::create_oauth_config_with_user`):
    /// 1. User-specific credentials (from `user_oauth_app_credentials` table)
    /// 2. Tenant-specific credentials (in-memory cache, then database)
    /// 3. Strava only: the app [`select_strava_app`] picks — the one the
    ///    athlete's grant holds a seat on, else their pool app while it has
    ///    room, else the env app, then a pool app with a free seat
    /// 4. Server-level OAuth configuration (environment variables)
    ///
    /// The second value is the pool app's `client_id` when step 3 chose one,
    /// else `None`. The caller pins it on the state it stores, so the exchange
    /// spends the code under the client the URL named. Unlike
    /// [`Self::get_credentials_for_user`], which serves a token already issued,
    /// this chooses where a new grant goes, so it applies the seat rules.
    ///
    /// # Errors
    ///
    /// Returns an error if no credentials are found for the user/tenant/provider
    /// combination, or when every Strava app is at capacity.
    pub async fn get_connect_credentials_for_user(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<(TenantOAuthCredentials, Option<String>)> {
        if let Some(credentials) = self
            .try_user_specific_credentials(user_id, tenant_id, provider, oauth_tokens)
            .await
        {
            return Ok((credentials, None));
        }
        if let Some(credentials) = self
            .try_tenant_specific_credentials(tenant_id, provider, tenants)
            .await
        {
            return Ok((credentials, None));
        }
        if provider.eq_ignore_ascii_case("strava") {
            let selected = select_strava_app(oauth_tokens, user_id, tenant_id).await?;
            return Ok((
                self.strava_credentials(tenant_id, selected.client_id, selected.client_secret),
                selected.attribution,
            ));
        }
        self.try_server_level_credentials(tenant_id, provider)
            .map(|credentials| (credentials, None))
            .ok_or_else(|| {
                AppError::not_found(format!(
                    "No OAuth credentials configured for tenant {tenant_id} and provider {provider}"
                ))
            })
    }

    /// Store OAuth credentials for a tenant
    ///
    /// # Errors
    ///
    /// Returns an error if credential storage fails
    pub fn store_credentials(
        &mut self,
        tenant_id: TenantId,
        provider: &str,
        config: CredentialConfig,
    ) -> AppResult<()> {
        let credentials = TenantOAuthCredentials {
            tenant_id,
            provider: provider.to_owned(),
            client_id: config.client_id,
            client_secret: config.client_secret,
            redirect_uri: config.redirect_uri,
            scopes: config.scopes,
            rate_limit_per_day: Self::default_rate_limit_for_provider(provider),
        };

        self.credentials
            .insert((tenant_id, provider.to_owned()), credentials);
        Ok(())
    }

    /// Check tenant's daily rate limit usage
    ///
    /// # Errors
    ///
    /// Returns an error if rate limit check fails
    pub fn check_rate_limit(&self, tenant_id: TenantId, provider: &str) -> AppResult<(u32, u32)> {
        let today = Utc::now().date_naive();
        let usage = self
            .usage_tracking
            .get(&(tenant_id, provider.to_owned(), today))
            .copied()
            .unwrap_or(0);

        // Get tenant's rate limit
        let daily_limit = self
            .credentials
            .get(&(tenant_id, provider.to_owned()))
            .map_or_else(
                || Self::default_rate_limit_for_provider(provider),
                |c| c.rate_limit_per_day,
            );

        Ok((usage, daily_limit))
    }

    /// Increment tenant's usage counter
    ///
    /// # Errors
    ///
    /// Returns an error if usage increment fails
    pub fn increment_usage(
        &mut self,
        tenant_id: TenantId,
        provider: &str,
        successful_requests: u32,
        _failed_requests: u32,
    ) -> AppResult<()> {
        let today = Utc::now().date_naive();
        let key = (tenant_id, provider.to_owned(), today);
        let current = self.usage_tracking.get(&key).copied().unwrap_or(0);
        self.usage_tracking
            .insert(key, current + successful_requests);
        Ok(())
    }

    /// Try to load server-level OAuth credentials from `ServerConfig`
    fn try_server_level_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> Option<TenantOAuthCredentials> {
        match provider.to_lowercase().as_str() {
            "strava" => self.try_strava_config_credentials(tenant_id),
            "garmin" => self.try_garmin_config_credentials(tenant_id),
            "whoop" => self.try_whoop_config_credentials(tenant_id),
            "terra" => self.try_terra_config_credentials(tenant_id),
            // Synthetic providers generate data locally, COROS OAuth not yet configured
            // Sciotte uses browser session cookies, not OAuth credentials
            "synthetic"
            | "synthetic_sleep"
            | "coros"
            | "sciotte"
            | "sciotte_garmin"
            | "sciotte_trainingpeaks"
            | "sciotte_coros" => None,
            _ => {
                warn!("Unsupported OAuth provider: {}", provider);
                None
            }
        }
    }

    /// Build Strava [`TenantOAuthCredentials`] for a resolved `client_id`/secret.
    ///
    /// Reuses the configured redirect URI and scopes. Shared by the env-config
    /// path and the shared-app pool resolution so both produce identical
    /// redirect/scope/rate-limit settings.
    fn strava_credentials(
        &self,
        tenant_id: TenantId,
        client_id: String,
        client_secret: String,
    ) -> TenantOAuthCredentials {
        let strava_config = &self.oauth_config.strava;
        let redirect_uri = strava_config
            .redirect_uri
            .clone()
            .unwrap_or_else(|| Self::default_redirect_uri("strava"));
        TenantOAuthCredentials {
            tenant_id,
            provider: "strava".to_owned(),
            client_id,
            client_secret,
            redirect_uri,
            scopes: if strava_config.scopes.is_empty() {
                "activity:read_all".split(',').map(str::to_owned).collect()
            } else {
                strava_config.scopes.clone()
            },
            rate_limit_per_day: STRAVA_DEFAULT_DAILY_RATE_LIMIT,
        }
    }

    /// Try to load Strava credentials from `ServerConfig` (env-default app).
    fn try_strava_config_credentials(&self, tenant_id: TenantId) -> Option<TenantOAuthCredentials> {
        let strava_config = &self.oauth_config.strava;
        if let (Some(client_id), Some(client_secret)) =
            (&strava_config.client_id, &strava_config.client_secret)
        {
            info!(
                "Using server-level Strava OAuth credentials for tenant {} (client_id={})",
                tenant_id, client_id
            );
            return Some(self.strava_credentials(
                tenant_id,
                client_id.clone(),
                client_secret.clone(),
            ));
        }
        warn!(
            "No Strava OAuth credentials in ServerConfig for tenant {}. MCP client should provide these credentials via OAuth configuration tool.",
            tenant_id
        );
        None
    }

    /// Try to load Garmin credentials from `ServerConfig`
    fn try_garmin_config_credentials(&self, tenant_id: TenantId) -> Option<TenantOAuthCredentials> {
        let garmin_config = &self.oauth_config.garmin;

        if let (Some(client_id), Some(client_secret)) =
            (&garmin_config.client_id, &garmin_config.client_secret)
        {
            let redirect_uri = garmin_config
                .redirect_uri
                .clone()
                .unwrap_or_else(|| Self::default_redirect_uri("garmin"));
            info!(
                "Using server-level Garmin OAuth credentials for tenant {}",
                tenant_id
            );
            return Some(TenantOAuthCredentials {
                tenant_id,
                provider: "garmin".to_owned(),
                client_id: client_id.clone(),
                client_secret: client_secret.clone(),
                redirect_uri,
                scopes: if garmin_config.scopes.is_empty() {
                    vec!["wellness:read".to_owned(), "activities:read".to_owned()]
                } else {
                    garmin_config.scopes.clone()
                },
                rate_limit_per_day: GARMIN_DEFAULT_DAILY_RATE_LIMIT,
            });
        }
        warn!(
            "No Garmin OAuth credentials in ServerConfig for tenant {}. MCP client should provide these credentials via OAuth configuration tool.",
            tenant_id
        );
        None
    }

    /// Try to load WHOOP credentials from `ServerConfig`
    fn try_whoop_config_credentials(&self, tenant_id: TenantId) -> Option<TenantOAuthCredentials> {
        let whoop_config = &self.oauth_config.whoop;

        if let (Some(client_id), Some(client_secret)) =
            (&whoop_config.client_id, &whoop_config.client_secret)
        {
            let redirect_uri = whoop_config
                .redirect_uri
                .clone()
                .unwrap_or_else(|| Self::default_redirect_uri("whoop"));
            info!(
                "Using server-level WHOOP OAuth credentials for tenant {}",
                tenant_id
            );
            return Some(TenantOAuthCredentials {
                tenant_id,
                provider: "whoop".to_owned(),
                client_id: client_id.clone(),
                client_secret: client_secret.clone(),
                redirect_uri,
                scopes: if whoop_config.scopes.is_empty() {
                    vec![
                        "offline".to_owned(),
                        "read:profile".to_owned(),
                        "read:body_measurement".to_owned(),
                        "read:workout".to_owned(),
                        "read:sleep".to_owned(),
                        "read:recovery".to_owned(),
                        "read:cycles".to_owned(),
                    ]
                } else {
                    whoop_config.scopes.clone()
                },
                rate_limit_per_day: WHOOP_DEFAULT_DAILY_RATE_LIMIT,
            });
        }
        warn!(
            "No WHOOP OAuth credentials in ServerConfig for tenant {}. MCP client should provide these credentials via OAuth configuration tool.",
            tenant_id
        );
        None
    }

    /// Try to load Terra credentials from `ServerConfig`
    fn try_terra_config_credentials(&self, tenant_id: TenantId) -> Option<TenantOAuthCredentials> {
        let terra_config = &self.oauth_config.terra;

        if let (Some(client_id), Some(client_secret)) =
            (&terra_config.client_id, &terra_config.client_secret)
        {
            let redirect_uri = terra_config
                .redirect_uri
                .clone()
                .unwrap_or_else(|| Self::default_redirect_uri("terra"));
            info!(
                "Using server-level Terra OAuth credentials for tenant {}",
                tenant_id
            );
            return Some(TenantOAuthCredentials {
                tenant_id,
                provider: "terra".to_owned(),
                client_id: client_id.clone(),
                client_secret: client_secret.clone(),
                redirect_uri,
                scopes: if terra_config.scopes.is_empty() {
                    vec![
                        "activity".to_owned(),
                        "sleep".to_owned(),
                        "body".to_owned(),
                        "daily".to_owned(),
                        "nutrition".to_owned(),
                    ]
                } else {
                    terra_config.scopes.clone()
                },
                rate_limit_per_day: TERRA_DEFAULT_DAILY_RATE_LIMIT,
            });
        }
        warn!(
            "No Terra OAuth credentials in ServerConfig for tenant {}. MCP client should provide these credentials via OAuth configuration tool.",
            tenant_id
        );
        None
    }

    /// A user's own OAuth app as the credentials this manager serves: the
    /// app's client and redirect, the provider's default scopes and rate limit.
    fn user_app_credentials(
        tenant_id: TenantId,
        provider: &str,
        app: UserOAuthApp,
    ) -> TenantOAuthCredentials {
        TenantOAuthCredentials {
            tenant_id,
            provider: provider.to_owned(),
            client_id: app.client_id,
            client_secret: app.client_secret,
            redirect_uri: app.redirect_uri,
            scopes: Self::default_scopes_for_provider(provider),
            rate_limit_per_day: Self::default_rate_limit_for_provider(provider),
        }
    }

    /// Try to load user-specific OAuth credentials from database
    ///
    /// This allows individual users to configure their own OAuth application
    /// credentials for a provider, avoiding rate limits on shared apps.
    async fn try_user_specific_credentials(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> Option<TenantOAuthCredentials> {
        user_app(user_id, provider, oauth_tokens)
            .await
            .map(|app| Self::user_app_credentials(tenant_id, provider, app))
    }

    /// Get default scopes for a provider
    fn default_scopes_for_provider(provider: &str) -> Vec<String> {
        match provider.to_lowercase().as_str() {
            "strava" => "activity:read_all".split(',').map(str::to_owned).collect(),
            "garmin" => vec!["wellness:read".to_owned(), "activities:read".to_owned()],
            "whoop" => vec![
                "offline".to_owned(),
                "read:profile".to_owned(),
                "read:body_measurement".to_owned(),
                "read:workout".to_owned(),
                "read:sleep".to_owned(),
                "read:recovery".to_owned(),
                "read:cycles".to_owned(),
            ],
            "terra" => vec![
                "activity".to_owned(),
                "sleep".to_owned(),
                "body".to_owned(),
                "daily".to_owned(),
                "nutrition".to_owned(),
            ],
            _ => vec![],
        }
    }

    /// Get default rate limit for a provider
    #[must_use]
    pub fn default_rate_limit_for_provider(provider: &str) -> u32 {
        match provider.to_lowercase().as_str() {
            "strava" => STRAVA_DEFAULT_DAILY_RATE_LIMIT,
            "garmin" => GARMIN_DEFAULT_DAILY_RATE_LIMIT,
            "whoop" => WHOOP_DEFAULT_DAILY_RATE_LIMIT,
            "terra" => TERRA_DEFAULT_DAILY_RATE_LIMIT,
            _ => 1000, // Default fallback
        }
    }

    /// Build the default OAuth callback redirect URI for a provider.
    ///
    /// Uses the `BASE_URL` environment variable (falling back to `http://localhost:8081`)
    /// to construct the redirect URI, matching the server's configured external address.
    fn default_redirect_uri(provider: &str) -> String {
        let base_url = env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8081".to_owned());
        format!("{base_url}/api/oauth/callback/{provider}")
    }

    /// Try to load tenant-specific OAuth credentials from memory cache and database
    async fn try_tenant_specific_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
        tenants: &dyn TenantRepository,
    ) -> Option<TenantOAuthCredentials> {
        // First check in-memory cache
        if let Some(credentials) = self
            .credentials
            .get(&(tenant_id, provider.to_owned()))
            .cloned()
        {
            info!(
                "Using cached tenant-specific {} OAuth credentials for tenant {}",
                provider, tenant_id
            );
            return Some(credentials);
        }

        // Then check database
        if let Ok(Some(db_credentials)) = tenants.get_oauth_credentials(tenant_id, provider).await {
            info!(
                "Using database-stored tenant-specific {} OAuth credentials for tenant {}",
                provider, tenant_id
            );
            return Some(db_credentials);
        }

        debug!(
            "No tenant-specific {} OAuth credentials found for tenant {}",
            provider, tenant_id
        );
        None
    }
}
