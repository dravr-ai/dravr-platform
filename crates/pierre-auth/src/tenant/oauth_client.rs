// ABOUTME: Tenant-aware OAuth client for multi-tenant fitness platform authentication
// ABOUTME: Provides OAuth flow integration with tenant-specific credentials and rate limiting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - OAuth credential string ownership transfers (client_id, client_secret, redirect_uri)
// - Tenant context ownership for multi-tenant OAuth flows

use super::oauth_manager::TenantOAuthManager;
use super::TenantContext;
use crate::oauth2_client::{OAuth2Client, OAuth2Config, OAuth2Token, PkceParams};
use pierre_core::constants::oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, TenantOAuthCredentials};
use pierre_database::backends::{OAuthTokenRepository, TenantRepository};
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// An authorization URL and the Strava shared-pool app it names.
///
/// The app is `None` for the env app and for every other credential source.
/// The caller pins it on the OAuth state so the code exchange uses that
/// client.
#[derive(Debug, Clone)]
pub struct ConnectAuthorization {
    /// The provider's authorization URL.
    pub url: String,
    /// The pool app's `client_id` when the URL names one.
    pub oauth_app_client_id: Option<String>,
}

/// Tenant-aware OAuth client with credential isolation and rate limiting
pub struct TenantOAuthClient {
    /// Shared OAuth manager instance for handling tenant-specific OAuth operations
    pub oauth_manager: Arc<Mutex<TenantOAuthManager>>,
}

impl TenantOAuthClient {
    /// Create new tenant OAuth client with provided manager
    #[must_use]
    pub fn new(oauth_manager: TenantOAuthManager) -> Self {
        Self {
            oauth_manager: Arc::new(Mutex::new(oauth_manager)),
        }
    }

    /// Get `OAuth2Client` configured for specific tenant and provider
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Tenant exceeds daily rate limit for the provider
    /// - No OAuth credentials configured for tenant and provider
    /// - OAuth configuration creation fails
    pub async fn get_oauth_client(
        &self,
        tenant_context: &TenantContext,
        provider: &str,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<OAuth2Client> {
        // Check rate limit first
        let manager = self.oauth_manager.lock().await;
        let (current_usage, daily_limit) = manager
            .check_rate_limit(tenant_context.tenant_id, provider, tenants)
            .await?;

        if current_usage >= daily_limit {
            return Err(AppError::invalid_input(format!(
                "Tenant {} has exceeded daily rate limit for provider {}: {}/{}",
                tenant_context.tenant_id, provider, current_usage, daily_limit
            )));
        }

        // Get credentials: user-specific → tenant-specific → server-level
        let credentials = manager
            .get_credentials_for_user(
                Some(tenant_context.user_id),
                tenant_context.tenant_id,
                provider,
                tenants,
                oauth_tokens,
            )
            .await?;
        drop(manager);

        // Build OAuth2Config from tenant credentials
        let oauth_config = Self::build_oauth_config(&credentials, provider)?;

        info!(
            "Created OAuth client for tenant={}, provider={}, client_id={}",
            tenant_context.tenant_id, provider, credentials.client_id
        );

        OAuth2Client::new(oauth_config)
    }

    /// The client a new authorization for this tenant context runs under, and
    /// the Strava shared-pool app it belongs to.
    ///
    /// Resolves credentials the way the code exchange resolves a pinned state
    /// ([`TenantOAuthManager::get_connect_credentials_for_user`]), so the
    /// authorize URL and the exchange cannot name different clients.
    async fn connect_client(
        &self,
        tenant_context: &TenantContext,
        provider: &str,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<(OAuth2Client, Option<String>)> {
        let manager = self.oauth_manager.lock().await;
        let (current_usage, daily_limit) = manager
            .check_rate_limit(tenant_context.tenant_id, provider, tenants)
            .await?;
        if current_usage >= daily_limit {
            return Err(AppError::invalid_input(format!(
                "Tenant {} has exceeded daily rate limit for provider {}: {}/{}",
                tenant_context.tenant_id, provider, current_usage, daily_limit
            )));
        }
        let (credentials, attribution) = manager
            .get_connect_credentials_for_user(
                tenant_context.user_id,
                tenant_context.tenant_id,
                provider,
                tenants,
                oauth_tokens,
            )
            .await?;
        drop(manager);

        let oauth_config = Self::build_oauth_config(&credentials, provider)?;
        info!(
            "Created OAuth connect client for tenant={}, provider={}, client_id={}",
            tenant_context.tenant_id, provider, credentials.client_id
        );
        Ok((OAuth2Client::new(oauth_config)?, attribution))
    }

    /// Get authorization URL for tenant-specific OAuth flow
    ///
    /// The result names the Strava shared-pool app the URL sends the athlete
    /// to, which the caller pins on the state it stores so the exchange spends
    /// the code under the same client.
    ///
    /// # Errors
    ///
    /// Returns an error if OAuth client creation or authorization URL generation fails
    pub async fn get_authorization_url(
        &self,
        tenant_context: &TenantContext,
        provider: &str,
        state: &str,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<ConnectAuthorization> {
        let (oauth_client, oauth_app_client_id) = self
            .connect_client(tenant_context, provider, tenants, oauth_tokens)
            .await?;
        let url = oauth_client.get_authorization_url(state).map_err(|e| {
            AppError::external_service(
                "oauth2",
                format!("OAuth authorization URL generation failed: {e}"),
            )
        })?;
        Ok(ConnectAuthorization {
            url,
            oauth_app_client_id,
        })
    }

    /// Get authorization URL with PKCE for tenant-specific OAuth flow
    ///
    /// Names the Strava shared-pool app as [`Self::get_authorization_url`] does.
    ///
    /// # Errors
    ///
    /// Returns an error if OAuth client creation or authorization URL generation fails
    pub async fn get_authorization_url_with_pkce(
        &self,
        tenant_context: &TenantContext,
        provider: &str,
        state: &str,
        pkce: &PkceParams,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<ConnectAuthorization> {
        let (oauth_client, oauth_app_client_id) = self
            .connect_client(tenant_context, provider, tenants, oauth_tokens)
            .await?;
        let url = oauth_client
            .get_authorization_url_with_pkce(state, pkce)
            .map_err(|e| {
                AppError::external_service(
                    "oauth2",
                    format!("OAuth authorization URL with PKCE generation failed: {e}"),
                )
            })?;
        Ok(ConnectAuthorization {
            url,
            oauth_app_client_id,
        })
    }

    /// Exchange authorization code for access token
    ///
    /// # Errors
    ///
    /// Returns an error if OAuth client creation or token exchange fails
    pub async fn exchange_code(
        &self,
        tenant_context: &TenantContext,
        provider: &str,
        code: &str,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<OAuth2Token> {
        let oauth_client = self
            .get_oauth_client(tenant_context, provider, tenants, oauth_tokens)
            .await?;
        let token = oauth_client.exchange_code(code).await.map_err(|e| {
            AppError::external_service("oauth2", format!("OAuth code exchange failed: {e}"))
        })?;

        // Increment usage counter
        self.oauth_manager.lock().await.increment_usage(
            tenant_context.tenant_id,
            provider,
            1,
            0,
        )?;

        info!(
            "Successfully exchanged OAuth code for tenant={}, provider={}",
            tenant_context.tenant_id, provider
        );

        Ok(token)
    }

    /// Exchange authorization code with PKCE for access token
    ///
    /// # Errors
    ///
    /// Returns an error if OAuth client creation or token exchange fails
    pub async fn exchange_code_with_pkce(
        &self,
        tenant_context: &TenantContext,
        provider: &str,
        code: &str,
        pkce: &PkceParams,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<OAuth2Token> {
        let oauth_client = self
            .get_oauth_client(tenant_context, provider, tenants, oauth_tokens)
            .await?;
        let token = oauth_client
            .exchange_code_with_pkce(code, pkce)
            .await
            .map_err(|e| {
                AppError::external_service(
                    "oauth2",
                    format!("OAuth code exchange with PKCE failed: {e}"),
                )
            })?;

        // Increment usage counter
        self.oauth_manager.lock().await.increment_usage(
            tenant_context.tenant_id,
            provider,
            1,
            0,
        )?;

        info!(
            "Successfully exchanged OAuth code with PKCE for tenant={}, provider={}",
            tenant_context.tenant_id, provider
        );

        Ok(token)
    }

    /// Refresh access token
    ///
    /// # Errors
    ///
    /// Returns an error if OAuth client creation or token refresh fails
    pub async fn refresh_token(
        &self,
        tenant_context: &TenantContext,
        provider: &str,
        refresh_token: &str,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<OAuth2Token> {
        let oauth_client = self
            .get_oauth_client(tenant_context, provider, tenants, oauth_tokens)
            .await?;
        let token = oauth_client
            .refresh_token(refresh_token)
            .await
            .map_err(|e| {
                AppError::external_service("oauth2", format!("OAuth token refresh failed: {e}"))
            })?;

        // Increment usage counter
        self.oauth_manager.lock().await.increment_usage(
            tenant_context.tenant_id,
            provider,
            1,
            0,
        )?;

        info!(
            "Successfully refreshed OAuth token for tenant={}, provider={}",
            tenant_context.tenant_id, provider
        );

        Ok(token)
    }

    /// Check if tenant can make OAuth requests (rate limit check)
    ///
    /// # Errors
    ///
    /// Returns an error if rate limit checking fails
    pub async fn check_rate_limit(
        &self,
        tenant_id: TenantId,
        provider: &str,
        tenants: &dyn TenantRepository,
    ) -> AppResult<(u32, u32)> {
        let manager = self.oauth_manager.lock().await;
        manager.check_rate_limit(tenant_id, provider, tenants).await
    }

    /// Get tenant's OAuth credentials (without decrypted secret)
    ///
    /// # Errors
    ///
    /// Returns an error if credential retrieval fails
    pub async fn get_tenant_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
        tenants: &dyn TenantRepository,
        oauth_tokens: &dyn OAuthTokenRepository,
    ) -> AppResult<Option<TenantOAuthCredentials>> {
        let manager = self.oauth_manager.lock().await;
        manager
            .get_credentials(tenant_id, provider, tenants, oauth_tokens)
            .await
            .map(Some)
    }

    /// Build `OAuth2Config` from tenant credentials
    fn build_oauth_config(
        credentials: &TenantOAuthCredentials,
        provider: &str,
    ) -> AppResult<OAuth2Config> {
        let (auth_url, token_url, use_pkce) = match provider {
            "strava" => (
                "https://www.strava.com/oauth/authorize".to_owned(),
                "https://www.strava.com/oauth/token".to_owned(),
                true,
            ),
            // Garmin's OAuth2 PKCE flow; the descriptor reads the same constants.
            "garmin" => (
                oauth_providers::GARMIN_AUTH_URL.to_owned(),
                oauth_providers::GARMIN_TOKEN_URL.to_owned(),
                true,
            ),
            "whoop" => (
                "https://api.prod.whoop.com/oauth/oauth2/auth".to_owned(),
                "https://api.prod.whoop.com/oauth/oauth2/token".to_owned(),
                true,
            ),
            "terra" => (
                "https://widget.tryterra.co/session".to_owned(),
                "https://api.tryterra.co/v2/auth/token".to_owned(),
                false, // Terra uses API key auth, not standard OAuth
            ),
            // Synthetic/COROS providers don't use OAuth — reject early
            "synthetic" | "synthetic_sleep" | "coros" => {
                return Err(AppError::invalid_input(format!(
                    "Provider {provider} does not use OAuth authentication"
                )));
            }
            _ => {
                warn!("Unknown provider {}, using generic OAuth URLs", provider);
                return Err(AppError::invalid_input(format!(
                    "Unsupported OAuth provider: {provider}"
                )));
            }
        };

        // Use BASE_URL for redirect_uri when set (tunnel/external access),
        // otherwise fall back to stored credentials redirect_uri
        let redirect_uri = env::var("BASE_URL").map_or_else(
            |_| credentials.redirect_uri.clone(),
            |base_url| format!("{base_url}/api/oauth/callback/{provider}"),
        );

        Ok(OAuth2Config {
            client_id: credentials.client_id.clone(),
            client_secret: credentials.client_secret.clone(),
            auth_url,
            token_url,
            redirect_uri,
            scopes: credentials.scopes.clone(), // Safe: Option<String> ownership for OAuth config
            use_pkce,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::TenantOAuthClient;
    use crate::oauth2_client::{OAuth2Client, PkceParams};
    use pierre_core::models::{TenantId, TenantOAuthCredentials};
    use url::Url;

    /// The connect route builds Garmin's authorization from this table, and
    /// sends the PKCE challenge only when it says `use_pkce`. Garmin's `OAuth2`
    /// PKCE specification requires `code_challenge` with
    /// `code_challenge_method=S256` at `connect.garmin.com/oauth2Confirm`, and
    /// the code is exchanged at `diauth.garmin.com`.
    #[test]
    fn garmin_connect_authorization_is_oauth2_pkce() {
        let credentials = TenantOAuthCredentials {
            tenant_id: TenantId::generate(),
            provider: "garmin".to_owned(),
            client_id: "garmin-client".to_owned(),
            client_secret: "garmin-secret".to_owned(),
            redirect_uri: "https://dravr.example/api/oauth/callback/garmin".to_owned(),
            scopes: Vec::new(),
            rate_limit_per_day: 1000,
        };
        let config = TenantOAuthClient::build_oauth_config(&credentials, "garmin").unwrap();
        assert_eq!(
            config.token_url,
            "https://diauth.garmin.com/di-oauth2-service/oauth/token"
        );
        assert!(config.use_pkce, "Garmin requires PKCE");

        let pkce = PkceParams::generate();
        let url = OAuth2Client::new(config)
            .unwrap()
            .get_authorization_url_with_pkce("state-1", &pkce)
            .unwrap();
        let url = Url::parse(&url).unwrap();
        assert_eq!(
            format!("{}{}", url.origin().ascii_serialization(), url.path()),
            "https://connect.garmin.com/oauth2Confirm"
        );
        let query = |key: &str| {
            url.query_pairs()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.into_owned())
        };
        assert_eq!(query("response_type").as_deref(), Some("code"));
        assert_eq!(query("client_id").as_deref(), Some("garmin-client"));
        assert_eq!(query("code_challenge"), Some(pkce.code_challenge));
        assert_eq!(query("code_challenge_method").as_deref(), Some("S256"));
        // Garmin's scope is fixed server-side: no `scope` parameter at all,
        // not an empty one.
        assert_eq!(query("scope"), None);
    }

    /// A provider that requests scopes still sends them.
    #[test]
    fn whoop_connect_authorization_carries_its_scope() {
        let credentials = TenantOAuthCredentials {
            tenant_id: TenantId::generate(),
            provider: "whoop".to_owned(),
            client_id: "whoop-client".to_owned(),
            client_secret: "whoop-secret".to_owned(),
            redirect_uri: "https://dravr.example/api/oauth/callback/whoop".to_owned(),
            scopes: vec!["offline".to_owned(), "read:sleep".to_owned()],
            rate_limit_per_day: 1000,
        };
        let config = TenantOAuthClient::build_oauth_config(&credentials, "whoop").unwrap();
        let url = OAuth2Client::new(config)
            .unwrap()
            .get_authorization_url_with_pkce("state-1", &PkceParams::generate())
            .unwrap();
        let url = Url::parse(&url).unwrap();
        let scope = url
            .query_pairs()
            .find(|(name, _)| name == "scope")
            .map(|(_, value)| value.into_owned());
        assert_eq!(scope.as_deref(), Some("offline read:sleep"));
    }
}
