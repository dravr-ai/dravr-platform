// ABOUTME: OAuth service handling provider authorization, token exchange, and connection management
// ABOUTME: Implements OAuth callback processing, PKCE, provider disconnect, and connection status
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::{
    collections::{HashMap, HashSet},
    env,
    fmt::Write,
    sync::Arc,
    time::Duration as StdDuration,
};

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde_json::{json, Value as JsonValue};
use tracing::{debug, error, field::Empty, info, warn};
use urlencoding::encode;

use crate::{
    config::environment::get_oauth_config,
    context::{ConfigContext, DataContext, NotificationContext, ServerContext},
    errors::{AppError, AppResult, ErrorCode},
    mcp::{
        oauth_flow_manager::OAuthTemplateRenderer, resources::ServerResources,
        schema::OAuthCompletedNotification,
    },
    models::{ConnectionType, TenantId, User, UserOAuthToken},
    providers::ProviderDescriptor,
    services::oauth_flow as oauth_flow_service,
    types::OAuthCallbackResponse,
    utils::http_client::{get_oauth_callback_notification_timeout_secs, shared_client},
};
use pierre_auth::oauth2_client::{
    OAuth2Client, OAuth2Config, OAuth2Token, OAuthClientState, PkceParams,
};
use pierre_auth::tenant::{TenantContext, TenantRole};
use pierre_database::database::repositories::UserRepository;

use super::types::{
    ConnectionStatus, OAuthAuthorizationResponse, OAuthStatus, ProviderStatus,
    ProvidersStatusResponse,
};

/// OAuth service for OAuth flow business logic
#[derive(Clone)]
pub struct OAuthService {
    data: DataContext,
    config: ConfigContext,
    notifications: NotificationContext,
}

/// Parsed OAuth state containing user ID and optional mobile redirect URL
struct ParsedOAuthState {
    user_id: uuid::Uuid,
    /// Optional redirect URL for mobile OAuth flows (base64 encoded in state)
    mobile_redirect_url: Option<String>,
    /// PKCE code verifier recovered from server-side state storage
    pkce_code_verifier: Option<String>,
    /// Tenant ID from the OAuth state, used for tenant-specific credential lookup
    tenant_id: Option<uuid::Uuid>,
}

impl OAuthService {
    /// Creates a new OAuth service instance
    #[must_use]
    pub const fn new(
        data_context: DataContext,
        config_context: ConfigContext,
        notification_context: NotificationContext,
    ) -> Self {
        Self {
            data: data_context,
            config: config_context,
            notifications: notification_context,
        }
    }

    /// Get configuration context
    #[must_use]
    pub const fn config(&self) -> &ConfigContext {
        &self.config
    }

    /// Handle OAuth callback
    ///
    /// Validates the state parameter against server-side storage to prevent CSRF attacks,
    /// then exchanges the authorization code for tokens. Uses PKCE when the code verifier
    /// was stored with the state during authorization URL generation.
    ///
    /// # Errors
    /// Returns error if OAuth state is invalid/expired/reused or callback processing fails
    pub async fn handle_callback(
        &self,
        code: &str,
        state: &str,
        provider: &str,
    ) -> AppResult<OAuthCallbackResponse> {
        // Validate provider is supported before consuming state
        self.validate_provider(provider)?;

        // Consume state atomically from database (verifies it was server-issued,
        // not expired, not reused, and matches the expected provider)
        let parsed_state = self.consume_and_validate_state(state, provider).await?;
        let user_id = parsed_state.user_id;
        let mobile_redirect_url = parsed_state.mobile_redirect_url;
        let pkce_code_verifier = parsed_state.pkce_code_verifier;
        let state_tenant_id = parsed_state.tenant_id;

        info!(
            "Processing OAuth callback for user {} provider {}{}",
            user_id,
            provider,
            if mobile_redirect_url.is_some() {
                " (mobile flow)"
            } else {
                ""
            }
        );

        // Get user and tenant from database
        let (.., tenant_id) = self.get_user_and_tenant(user_id, provider).await?;

        // Exchange OAuth code for access token (with PKCE if verifier was stored)
        // Pass tenant_id from state so exchange uses tenant-specific credentials if available
        let token = self
            .exchange_oauth_code(
                code,
                provider,
                user_id,
                pkce_code_verifier.as_deref(),
                state_tenant_id,
            )
            .await?;

        info!(
            "Successfully exchanged OAuth code for user {} provider {}",
            user_id, provider
        );

        // Store token and send notifications
        let expires_at = self
            .store_oauth_token(user_id, tenant_id, provider, &token)
            .await?;
        self.send_oauth_notifications(user_id, provider, &expires_at)
            .await?;
        self.notify_bridge_oauth_success(provider, &token).await;

        Ok(OAuthCallbackResponse {
            user_id: user_id.to_string(),
            provider: provider.to_owned(),
            expires_at: expires_at.to_rfc3339(),
            scopes: token.scope.unwrap_or_else(|| "read".to_owned()),
            mobile_redirect_url,
        })
    }

    /// Consume and validate OAuth state from server-side storage
    ///
    /// Atomically verifies the state was issued by this server, has not expired,
    /// and has not been used before (one-time use). Uses the provider name as the
    /// `client_id` for additional validation that the callback matches the initiated flow.
    ///
    /// State format: `{user_id}:{random}` or `{user_id}:{random}:{base64_redirect_url}`
    /// The redirect URL allows mobile apps to specify where to redirect after OAuth completes.
    async fn consume_and_validate_state(
        &self,
        state: &str,
        provider: &str,
    ) -> AppResult<ParsedOAuthState> {
        // Atomically consume the state from database (marks as used, checks expiry)
        let consumed = self
            .data
            .repos()
            .oauth_client_state
            .consume_oauth_client_state(state, provider, Utc::now())
            .await
            .map_err(|e| {
                warn!("Failed to consume OAuth state from database: {}", e);
                AppError::auth_invalid("OAuth state validation failed")
            })?;

        let client_state = consumed.ok_or_else(|| {
            warn!(
                "OAuth state not found, expired, or already used for provider {}",
                provider
            );
            AppError::auth_invalid("Invalid, expired, or already used OAuth state parameter")
        })?;

        let user_id = client_state.user_id.ok_or_else(|| {
            error!("OAuth state missing user_id for provider {}", provider);
            AppError::auth_invalid("OAuth state missing user identity")
        })?;

        // Extract optional mobile redirect URL from the state string
        // (embedded as base64 in the third segment of the state format)
        let mobile_redirect_url = self.extract_mobile_redirect_from_state_str(state);

        // PKCE code verifier stored server-side during authorization URL generation
        let pkce_code_verifier = client_state.pkce_code_verifier;

        // Parse tenant_id from the stored OAuth client state for credential lookup
        let tenant_id = client_state
            .tenant_id
            .as_deref()
            .and_then(|tid| uuid::Uuid::parse_str(tid).ok());

        Ok(ParsedOAuthState {
            user_id,
            mobile_redirect_url,
            pkce_code_verifier,
            tenant_id,
        })
    }

    /// Extract mobile redirect URL from state string format
    ///
    /// State format: `{user_id}:{random}:{base64_redirect_url}`
    /// Delegates to `services::oauth_flow::extract_mobile_redirect_from_state`.
    fn extract_mobile_redirect_from_state_str(&self, state: &str) -> Option<String> {
        let config = self.config.config();
        oauth_flow_service::extract_mobile_redirect_from_state(
            state,
            &config.base_url,
            &config.security.allowed_mobile_redirect_origins,
        )
    }

    /// Validate that provider is supported by checking the provider registry
    fn validate_provider(&self, provider: &str) -> AppResult<()> {
        if self.data.provider_registry().is_supported(provider) {
            Ok(())
        } else {
            Err(AppError::invalid_input(format!(
                "Unsupported provider: {provider}"
            )))
        }
    }

    /// Get user and tenant from database
    ///
    /// Tenant is determined from the `tenant_users` junction table.
    async fn get_user_and_tenant(
        &self,
        user_id: uuid::Uuid,
        provider: &str,
    ) -> AppResult<(User, String)> {
        let repos = self.data.repos();
        let user = repos
            .users
            .get_global(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user: {e}")))?
            .ok_or_else(|| {
                error!(
                    "OAuth callback failed: User not found - user_id: {}, provider: {}",
                    user_id, provider
                );
                AppError::not_found("User")
            })?;

        // Get tenant from tenant_users table (user's default/first tenant).
        // NOTE: We use tenants.first() here intentionally because this is an OAuth callback
        // where the user has not yet established a JWT session with active_tenant_id.
        // The resulting token will carry this tenant_id as the default active_tenant_id.
        let tenants = repos
            .tenants
            .list_for_user(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;

        let tenant_id = tenants.first().map(|t| t.id.to_string()).ok_or_else(|| {
            error!(
                user_id = %user.id,
                provider = %provider,
                "OAuth callback failed: user has no tenant"
            );
            AppError::invalid_input("User has no tenant")
        })?;

        Ok((user, tenant_id))
    }

    /// Exchange OAuth code for access token, using PKCE when a code verifier is available
    ///
    /// When `tenant_id` is provided, attempts to use tenant-specific OAuth credentials
    /// (`client_id`, `client_secret`) before falling back to environment configuration.
    async fn exchange_oauth_code(
        &self,
        code: &str,
        provider: &str,
        user_id: uuid::Uuid,
        pkce_code_verifier: Option<&str>,
        tenant_id: Option<uuid::Uuid>,
    ) -> AppResult<OAuth2Token> {
        let oauth_config = self
            .create_oauth_config_with_user(provider, user_id, tenant_id)
            .await?;
        let oauth_client = OAuth2Client::new(oauth_config)?;

        let token = if let Some(verifier) = pkce_code_verifier {
            // Use PKCE-enhanced token exchange when verifier was stored with the state
            let pkce = PkceParams {
                code_verifier: verifier.to_owned(),
                code_challenge: String::new(),
                code_challenge_method: "S256".to_owned(),
            };
            oauth_client
                .exchange_code_with_pkce(code, &pkce)
                .await
                .map_err(|e| {
                    error!(
                        "OAuth PKCE token exchange failed for {provider} - user_id: {user_id}, error: {e}",
                    );
                    AppError::internal(format!("Failed to exchange OAuth code for token: {e}"))
                })?
        } else {
            oauth_client.exchange_code(code).await.map_err(|e| {
                error!(
                    "OAuth token exchange failed for {provider} - user_id: {user_id}, error: {e}",
                );
                AppError::internal(format!("Failed to exchange OAuth code for token: {e}"))
            })?
        };

        Ok(token)
    }

    /// Create `OAuth2` config for provider using descriptor and configuration
    ///
    /// # Errors
    /// Returns error if provider is unsupported or required credentials are not configured
    fn create_oauth_config(&self, provider: &str) -> AppResult<OAuth2Config> {
        // Get provider descriptor from registry
        let descriptor = self
            .data
            .provider_registry()
            .get_descriptor(provider)
            .ok_or_else(|| AppError::invalid_input(format!("Unsupported provider: {provider}")))?;

        // Get OAuth endpoints from descriptor
        let endpoints = descriptor.oauth_endpoints().ok_or_else(|| {
            AppError::invalid_input(format!("Provider {provider} does not support OAuth"))
        })?;

        // Get OAuth params from descriptor
        let params = descriptor.oauth_params().ok_or_else(|| {
            AppError::invalid_input(format!("Provider {provider} OAuth params not configured"))
        })?;

        // Get credentials from environment/config
        let env_config = get_oauth_config(provider);
        let client_id = env_config.client_id.ok_or_else(|| {
            AppError::invalid_input(format!(
                "{provider} client_id not configured for token exchange"
            ))
        })?;
        let client_secret = env_config.client_secret.ok_or_else(|| {
            AppError::invalid_input(format!(
                "{provider} client_secret not configured for token exchange"
            ))
        })?;

        // Build redirect URI - use BASE_URL if set for tunnel/external access
        let server_config = self.config.config();
        let redirect_uri = env_config.redirect_uri.unwrap_or_else(|| {
            let base_url = env::var("BASE_URL")
                .unwrap_or_else(|_| format!("http://localhost:{}", server_config.http_port));
            format!("{base_url}/api/oauth/callback/{provider}")
        });

        // Get default scopes and join with provider's separator
        let scopes = descriptor
            .default_scopes()
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
            .join(params.scope_separator);

        Ok(OAuth2Config {
            client_id,
            client_secret,
            auth_url: endpoints.auth_url.to_owned(),
            token_url: endpoints.token_url.to_owned(),
            redirect_uri,
            scopes: vec![scopes],
            use_pkce: params.use_pkce,
        })
    }

    /// Create `OAuth2` config with user-specific credential priority
    ///
    /// Resolution order (matching `TenantOAuthManager::get_credentials_for_user`):
    /// 1. User-specific credentials (from `user_oauth_app_credentials` table)
    /// 2. Tenant-specific credentials (from `tenant_oauth_credentials` table)
    /// 3. Server-level OAuth configuration (environment variables)
    ///
    /// This ensures the token exchange uses the same credentials as the authorization
    /// URL generation, preventing `client_id` mismatches that cause "invalid code" errors.
    ///
    /// # Errors
    /// Returns error if provider is unsupported or no credentials are configured
    async fn create_oauth_config_with_user(
        &self,
        provider: &str,
        user_id: uuid::Uuid,
        tenant_id: Option<uuid::Uuid>,
    ) -> AppResult<OAuth2Config> {
        // Priority 1: Try user-specific credentials (per-user OAuth app)
        if let Ok(Some(user_app)) = self
            .data
            .repos()
            .oauth_tokens
            .get_user_oauth_app(user_id, provider)
            .await
        {
            info!(
                "Token exchange using user-specific {} credentials for user {} (client_id={})",
                provider, user_id, user_app.client_id
            );

            let descriptor = self
                .data
                .provider_registry()
                .get_descriptor(provider)
                .ok_or_else(|| {
                    AppError::invalid_input(format!("Unsupported provider: {provider}"))
                })?;

            let endpoints = descriptor.oauth_endpoints().ok_or_else(|| {
                AppError::invalid_input(format!("Provider {provider} does not support OAuth"))
            })?;

            let params = descriptor.oauth_params().ok_or_else(|| {
                AppError::invalid_input(format!("Provider {provider} OAuth params not configured"))
            })?;

            let scopes = descriptor
                .default_scopes()
                .iter()
                .map(|s| (*s).to_owned())
                .collect::<Vec<_>>()
                .join(params.scope_separator);

            return Ok(OAuth2Config {
                client_id: user_app.client_id,
                client_secret: user_app.client_secret,
                auth_url: endpoints.auth_url.to_owned(),
                token_url: endpoints.token_url.to_owned(),
                redirect_uri: user_app.redirect_uri,
                scopes: vec![scopes],
                use_pkce: params.use_pkce,
            });
        }

        // Priority 2+3: Tenant-specific, then server-level
        self.create_oauth_config_with_tenant(provider, tenant_id)
            .await
    }

    /// Create `OAuth2` config using tenant-specific credentials when available
    ///
    /// Looks up tenant credentials from the database when `tenant_id` is provided.
    /// Falls back to environment-based configuration if no tenant credentials are found
    /// or if `tenant_id` is None.
    ///
    /// # Errors
    /// Returns error if provider is unsupported or no credentials are configured
    async fn create_oauth_config_with_tenant(
        &self,
        provider: &str,
        tenant_id: Option<uuid::Uuid>,
    ) -> AppResult<OAuth2Config> {
        // Try tenant-specific credentials first
        if let Some(tid) = tenant_id {
            let tid = TenantId::from(tid);
            let tenant_creds = self
                .data
                .repos().tenants
                .get_oauth_credentials(tid, provider)
                .await
                .map_err(|e| {
                    warn!(
                        "Failed to fetch tenant OAuth credentials for tenant {tid}, provider {provider}: {e}"
                    );
                    AppError::database(format!(
                        "Failed to fetch tenant OAuth credentials: {e}"
                    ))
                })?;

            if let Some(creds) = tenant_creds {
                debug!(
                    "Using tenant-specific OAuth credentials for tenant {tid}, provider {provider}"
                );

                // Get provider descriptor for endpoints and params
                let descriptor = self
                    .data
                    .provider_registry()
                    .get_descriptor(provider)
                    .ok_or_else(|| {
                        AppError::invalid_input(format!("Unsupported provider: {provider}"))
                    })?;

                let endpoints = descriptor.oauth_endpoints().ok_or_else(|| {
                    AppError::invalid_input(format!("Provider {provider} does not support OAuth"))
                })?;

                let params = descriptor.oauth_params().ok_or_else(|| {
                    AppError::invalid_input(format!(
                        "Provider {provider} OAuth params not configured"
                    ))
                })?;

                let scopes = creds.scopes.join(params.scope_separator);

                return Ok(OAuth2Config {
                    client_id: creds.client_id,
                    client_secret: creds.client_secret,
                    auth_url: endpoints.auth_url.to_owned(),
                    token_url: endpoints.token_url.to_owned(),
                    redirect_uri: creds.redirect_uri,
                    scopes: vec![scopes],
                    use_pkce: params.use_pkce,
                });
            }
        }

        // Fall back to environment-based configuration
        self.create_oauth_config(provider)
    }

    /// Store OAuth token in database
    async fn store_oauth_token(
        &self,
        user_id: uuid::Uuid,
        tenant_id: String,
        provider: &str,
        token: &OAuth2Token,
    ) -> AppResult<chrono::DateTime<chrono::Utc>> {
        let expires_at = token
            .expires_at
            .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(1));

        let user_oauth_token = UserOAuthToken {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            tenant_id,
            provider: provider.to_owned(),
            access_token: token.access_token.clone(),
            refresh_token: token.refresh_token.clone(),
            token_type: token.token_type.clone(),
            expires_at: Some(expires_at),
            scope: token.scope.clone(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.data
            .repos()
            .oauth_tokens
            .upsert_token(&user_oauth_token)
            .await
            .map_err(|e| AppError::database(format!("Failed to upsert OAuth token: {e}")))?;

        // Register provider connection alongside the OAuth token
        let connection_tenant_id: TenantId = user_oauth_token.tenant_id.parse().map_err(|_| {
            AppError::internal(format!(
                "Invalid tenant_id in OAuth token: {}",
                user_oauth_token.tenant_id
            ))
        })?;
        self.data
            .repos()
            .provider_connections
            .register_connection(
                user_id,
                connection_tenant_id,
                provider,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to register provider connection: {e}"))
            })?;

        Ok(expires_at)
    }

    /// Send OAuth completion notifications
    async fn send_oauth_notifications(
        &self,
        user_id: uuid::Uuid,
        provider: &str,
        expires_at: &chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        let notification_id = self
            .store_oauth_notification(user_id, provider, expires_at)
            .await?;
        self.broadcast_oauth_notification(&notification_id, user_id, provider);
        Ok(())
    }

    /// Store OAuth notification in database
    async fn store_oauth_notification(
        &self,
        user_id: uuid::Uuid,
        provider: &str,
        expires_at: &chrono::DateTime<chrono::Utc>,
    ) -> AppResult<String> {
        let notification_id = self
            .data
            .repos()
            .notifications
            .store(
                user_id,
                provider,
                true,
                "OAuth authorization completed successfully",
                Some(&expires_at.to_rfc3339()),
            )
            .await
            .map_err(|e| AppError::database(format!("Failed to store OAuth notification: {e}")))?;

        info!(
            "Created OAuth completion notification {} for user {} provider {}",
            notification_id, user_id, provider
        );

        Ok(notification_id)
    }

    /// Broadcast OAuth completion notification via WebSocket/SSE
    fn broadcast_oauth_notification(
        &self,
        notification_id: &str,
        user_id: uuid::Uuid,
        provider: &str,
    ) {
        let Some(sender) = self.notifications.oauth_notification_sender() else {
            debug!(
                notification_id = %notification_id,
                user_id = %user_id,
                provider = %provider,
                "OAuth notification sender not configured"
            );
            return;
        };

        let notification = OAuthCompletedNotification::new(
            provider.to_owned(),
            true,
            format!("{provider} connected successfully"),
            Some(user_id.to_string()),
        );

        match sender.send(notification) {
            Ok(receiver_count) => {
                info!(
                    notification_id = %notification_id,
                    user_id = %user_id,
                    provider = %provider,
                    receiver_count = %receiver_count,
                    "OAuth notification broadcast to {} receivers",
                    receiver_count
                );
            }
            Err(e) => {
                debug!(
                    notification_id = %notification_id,
                    user_id = %user_id,
                    provider = %provider,
                    error = %e,
                    "No active receivers for OAuth notification"
                );
            }
        }
    }

    /// Build OAuth token data for bridge notification
    fn build_bridge_token_data(token: &OAuth2Token) -> JsonValue {
        // Calculate expires_in from expires_at if available
        let expires_in = token.expires_at.map(|expires_at| {
            let duration = expires_at - chrono::Utc::now();
            duration.num_seconds().max(0)
        });

        json!({
            "access_token": token.access_token,
            "refresh_token": token.refresh_token,
            "expires_in": expires_in,
            "token_type": token.token_type,
            "scope": token.scope
        })
    }

    /// Log bridge notification response
    fn log_bridge_notification_result(
        result: Result<reqwest::Response, reqwest::Error>,
        provider: &str,
    ) {
        match result {
            Ok(response) if response.status().is_success() => {
                info!(
                    "✅ Successfully notified bridge about {} OAuth completion",
                    provider
                );
            }
            Ok(response) => {
                warn!(
                    "Bridge notification responded with status {} for provider {}",
                    response.status(),
                    provider
                );
            }
            Err(e) => {
                warn!(
                    "Failed to notify bridge about {} OAuth (bridge may not be running): {}",
                    provider, e
                );
            }
        }
    }

    /// Notify bridge about successful OAuth (for client-side token storage and focus recovery)
    async fn notify_bridge_oauth_success(&self, provider: &str, token: &OAuth2Token) {
        let oauth_callback_port = self.config.config().oauth_callback_port;
        let callback_url =
            format!("http://localhost:{oauth_callback_port}/oauth/provider-callback/{provider}");

        let token_data = Self::build_bridge_token_data(token);

        debug!(
            "Notifying bridge about {} OAuth success at {}",
            provider, callback_url
        );

        // Best-effort notification with configured timeout - don't fail OAuth flow if bridge notification fails
        // Configuration must be initialized via initialize_http_clients() at server startup
        let timeout_secs = get_oauth_callback_notification_timeout_secs();
        let result = shared_client()
            .post(&callback_url)
            .json(&token_data)
            .timeout(StdDuration::from_secs(timeout_secs))
            .send()
            .await;

        Self::log_bridge_notification_result(result, provider);
    }

    /// Disconnect OAuth provider for user
    ///
    /// # Errors
    /// Returns error if provider is unsupported or disconnection fails
    pub async fn disconnect_provider(
        &self,
        user_id: uuid::Uuid,
        provider: &str,
        active_tenant_id: Option<uuid::Uuid>,
    ) -> AppResult<()> {
        debug!(
            "Processing OAuth provider disconnect for user {} provider {}",
            user_id, provider
        );

        // Validate provider is supported
        self.validate_provider(provider)?;

        // Use active_tenant_id from JWT claims (user's selected tenant)
        let tenant_id: TenantId = active_tenant_id.map(TenantId::from).ok_or_else(|| {
            AppError::auth_invalid("No active tenant in session — cannot disconnect provider")
        })?;

        // Delete OAuth tokens from database
        self.data
            .repos()
            .oauth_tokens
            .delete_token(user_id, tenant_id, provider)
            .await
            .map_err(|e| AppError::database(format!("Failed to delete OAuth token: {e}")))?;

        // Remove provider connection record
        self.data
            .repos()
            .provider_connections
            .remove_connection(user_id, tenant_id, provider)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to remove provider connection: {e}"))
            })?;

        info!("Disconnected {} for user {}", provider, user_id);

        Ok(())
    }

    /// Generate OAuth authorization URL for provider
    ///
    /// This function supports both multi-tenant and single-tenant modes:
    /// - Multi-tenant: Uses tenant-specific OAuth credentials from database
    /// - Single-tenant: Falls back to server-level configuration
    ///
    /// Stores the OAuth state server-side with TTL for CSRF protection, and generates
    /// PKCE parameters when the provider declares `use_pkce=true`.
    ///
    /// # Errors
    /// Returns error if provider is unsupported or OAuth credentials not configured
    pub async fn get_auth_url(
        &self,
        user_id: uuid::Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<OAuthAuthorizationResponse> {
        // Get provider descriptor from registry
        let descriptor = self
            .data
            .provider_registry()
            .get_descriptor(provider)
            .ok_or_else(|| AppError::invalid_input(format!("Unsupported provider: {provider}")))?;

        // Get OAuth endpoints and params from descriptor
        let endpoints = descriptor.oauth_endpoints().ok_or_else(|| {
            AppError::invalid_input(format!("Provider {provider} does not support OAuth"))
        })?;
        let params = descriptor.oauth_params().ok_or_else(|| {
            AppError::invalid_input(format!("Provider {provider} OAuth params not configured"))
        })?;

        let use_pkce = params.use_pkce;

        // Check for tenant-specific OAuth credentials first (multi-tenant mode)
        let tenant_creds = self
            .data
            .repos()
            .tenants
            .get_oauth_credentials(tenant_id, provider)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to get tenant OAuth credentials: {e}"))
            })?;

        let state = format!("{}:{}", user_id, uuid::Uuid::new_v4());
        // Use BASE_URL environment variable if set, otherwise fall back to localhost.
        // This allows dynamic OAuth callbacks when using tunnels for local development.
        let base_url = env::var("BASE_URL")
            .unwrap_or_else(|_| format!("http://localhost:{}", self.config.config().http_port));
        let redirect_uri = format!("{base_url}/api/oauth/callback/{provider}");

        // Generate PKCE parameters when provider supports it
        let pkce = if use_pkce {
            Some(PkceParams::generate())
        } else {
            None
        };

        // URL-encode parameters for OAuth URLs
        let encoded_state = encode(&state);
        let encoded_redirect_uri = encode(&redirect_uri);

        // Determine client_id and scopes (tenant-specific or environment)
        let (client_id, scope) = if let Some(creds) = tenant_creds {
            // Multi-tenant: use tenant-specific credentials
            let scope = creds.scopes.join(params.scope_separator);
            (creds.client_id, scope)
        } else {
            // Single-tenant: use environment configuration
            let env_config = get_oauth_config(provider);
            let client_id = env_config.client_id.ok_or_else(|| {
                AppError::invalid_input(format!(
                    "{provider} client_id not configured (set in environment or database)"
                ))
            })?;
            let scope = descriptor.default_scopes().join(params.scope_separator);
            (client_id, scope)
        };

        let encoded_scope = encode(&scope);

        // Build authorization URL with provider-specific parameters
        let mut auth_url = format!(
            "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&state={}",
            endpoints.auth_url, client_id, encoded_redirect_uri, encoded_scope, encoded_state
        );

        // Add PKCE code_challenge to authorization URL when enabled
        if let Some(ref pkce_params) = pkce {
            use Write;
            let _ = write!(
                &mut auth_url,
                "&code_challenge={}&code_challenge_method={}",
                encode(&pkce_params.code_challenge),
                encode(&pkce_params.code_challenge_method)
            );
        }

        // Add provider-specific additional parameters
        for (key, value) in params.additional_auth_params {
            use Write;
            // Writing to String cannot fail
            let _ = write!(&mut auth_url, "&{}={}", encode(key), encode(value));
        }

        let authorization_url = auth_url;

        // Store state server-side for CSRF protection with 10-minute TTL.
        // The code_challenge field stores the PKCE code_verifier (needed during
        // token exchange to prove we initiated the authorization request).
        let now = Utc::now();
        let client_state = OAuthClientState {
            state: state.clone(),
            provider: provider.to_owned(),
            user_id: Some(user_id),
            tenant_id: Some(tenant_id.to_string()),
            redirect_uri,
            scope: Some(scope),
            pkce_code_verifier: pkce.as_ref().map(|p| p.code_verifier.clone()),
            created_at: now,
            expires_at: now + chrono::Duration::minutes(10),
            used: false,
        };

        self.data
            .repos()
            .oauth_client_state
            .store_oauth_client_state(&client_state)
            .await
            .map_err(|e| {
                error!("Failed to store OAuth state for CSRF protection: {}", e);
                AppError::internal(format!("Failed to initiate OAuth flow: {e}"))
            })?;

        debug!(
            "Generated OAuth authorization URL for user {} tenant {} provider {}",
            user_id, tenant_id, provider
        );

        Ok(OAuthAuthorizationResponse {
            authorization_url,
            state,
            instructions: format!("Click the link to authorize {provider} access"),
            expires_in_minutes: 10,
        })
    }

    /// Get connection status for all providers for a user
    ///
    /// Uses `provider_connections` table as the single source of truth.
    /// For OAuth connections, also looks up token expiry and scope info.
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn get_connection_status(
        &self,
        user_id: uuid::Uuid,
    ) -> AppResult<Vec<ConnectionStatus>> {
        debug!("Getting provider connection status for user {}", user_id);

        // Get all provider connections (cross-tenant view)
        let connections = self
            .data
            .repos()
            .provider_connections
            .get_for_user(user_id, None)
            .await
            .map_err(|e| AppError::database(format!("Failed to get provider connections: {e}")))?;

        // For OAuth connections, look up token expiry/scope info
        let oauth_tokens = self
            .data
            .repos()
            .oauth_tokens
            .get_tokens(user_id, None)
            .await
            .unwrap_or_default();

        let token_map: HashMap<String, &UserOAuthToken> = oauth_tokens
            .iter()
            .map(|t| (t.provider.clone(), t))
            .collect();

        let mut providers_seen = HashSet::new();
        let mut statuses = Vec::new();

        // Build status for each connected provider
        for conn in &connections {
            if providers_seen.insert(conn.provider.clone()) {
                let (expires_at, scopes) = if conn.connection_type == ConnectionType::OAuth {
                    // Look up OAuth token details for expiry/scope info
                    token_map.get(&conn.provider).map_or((None, None), |t| {
                        (t.expires_at.map(|dt| dt.to_rfc3339()), t.scope.clone())
                    })
                } else {
                    (None, None)
                };

                statuses.push(ConnectionStatus {
                    provider: conn.provider.clone(),
                    connected: true,
                    connection_type: Some(conn.connection_type.as_str().to_owned()),
                    expires_at,
                    scopes,
                });
            }
        }

        // Add default disconnected status for all registered OAuth providers not in connections
        for provider_name in self.data.provider_registry().oauth_providers() {
            if !providers_seen.contains(provider_name) {
                statuses.push(ConnectionStatus {
                    provider: provider_name.to_owned(),
                    connected: false,
                    connection_type: None,
                    expires_at: None,
                    scopes: None,
                });
            }
        }

        Ok(statuses)
    }
}

/// OAuth routes - alias for OAuth service to match test expectations
pub type OAuthRoutes = OAuthService;

// ---------------------------------------------------------------------------
// Axum handler functions — called from AuthRoutes::routes() in mod.rs
// ---------------------------------------------------------------------------

/// Handle OAuth callback
#[tracing::instrument(
    skip(resources, params),
    fields(
        route = "oauth_callback",
        provider = %provider,
        user_id = Empty,
        success = Empty,
    )
)]
pub(super) async fn handle_oauth_callback(
    State(resources): State<Arc<ServerResources>>,
    Path(provider): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let server_context = ServerContext::from(resources.as_ref());
    let oauth_routes = OAuthService::new(
        server_context.data().clone(),
        server_context.config().clone(),
        server_context.notification().clone(),
    );

    let code = params
        .get("code")
        .ok_or_else(|| AppError::auth_invalid("Missing OAuth code parameter"))?;

    let state = params
        .get("state")
        .ok_or_else(|| AppError::auth_invalid("Missing OAuth state parameter"))?;

    // Check if we should redirect to a separate frontend URL
    let frontend_url = server_context.config().config().frontend_url.clone();

    match oauth_routes.handle_callback(code, state, &provider).await {
        Ok(response) => {
            // Priority: mobile redirect URL > frontend URL > render template
            // Mobile apps pass redirect URL through OAuth state for deep linking
            if let Some(mobile_url) = &response.mobile_redirect_url {
                let redirect_url = format!(
                    "{}?provider={}&success=true",
                    mobile_url.trim_end_matches('/'),
                    encode(&provider)
                );
                info!("Redirecting OAuth success to mobile app: {}", redirect_url);
                return Ok(
                    (StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response()
                );
            }

            // If frontend URL is configured, redirect to frontend with success params
            if let Some(url) = frontend_url {
                let redirect_url = format!(
                    "{}/oauth-callback?provider={}&success=true",
                    url.trim_end_matches('/'),
                    encode(&provider)
                );
                info!("Redirecting OAuth success to frontend: {}", redirect_url);
                return Ok(
                    (StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response()
                );
            }

            // Otherwise serve the success page directly (same-origin production)
            let html = OAuthTemplateRenderer::render_success_template(&provider, &response);

            Ok((StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html).into_response())
        }
        Err(e) => {
            error!("OAuth callback failed: {}", e);

            // Determine error message and description based on error type
            let (error_msg, description) = categorize_oauth_error(&e);

            // For errors, we need to parse the state to check for mobile redirect URL
            // since handle_callback failed and didn't return the parsed state
            let config = server_context.config().config();
            let mobile_redirect_url = oauth_flow_service::extract_mobile_redirect_from_state(
                state,
                &config.base_url,
                &config.security.allowed_mobile_redirect_origins,
            );

            // Priority: mobile redirect URL > frontend URL > render template
            if let Some(mobile_url) = mobile_redirect_url {
                let redirect_url = format!(
                    "{}?provider={}&success=false&error={}",
                    mobile_url.trim_end_matches('/'),
                    encode(&provider),
                    encode(error_msg)
                );
                info!("Redirecting OAuth error to mobile app: {}", redirect_url);
                return Ok(
                    (StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response()
                );
            }

            // If frontend URL is configured, redirect to frontend with error params
            if let Some(url) = frontend_url {
                let redirect_url = format!(
                    "{}/oauth-callback?provider={}&success=false&error={}",
                    url.trim_end_matches('/'),
                    encode(&provider),
                    encode(error_msg)
                );
                info!("Redirecting OAuth error to frontend: {}", redirect_url);
                return Ok(
                    (StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response()
                );
            }

            let html =
                OAuthTemplateRenderer::render_error_template(&provider, error_msg, description);

            Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/html")],
                html,
            )
                .into_response())
        }
    }
}

/// Handle OAuth status check
#[tracing::instrument(
    skip(resources, headers),
    fields(
        route = "oauth_status",
        user_id = Empty,
    )
)]
pub(super) async fn handle_oauth_status(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate using middleware (supports both cookies and Authorization header)
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;

    // Check OAuth provider connection status for the user (cross-tenant view)
    let provider_statuses = resources
        .repos
        .oauth_tokens
        .get_tokens(user_id, None)
        .await
        .map_or_else(
            |_| {
                vec![
                    OAuthStatus {
                        provider: "strava".to_owned(),
                        connected: false,
                        last_sync: None,
                    },
                    OAuthStatus {
                        provider: "fitbit".to_owned(),
                        connected: false,
                        last_sync: None,
                    },
                ]
            },
            |tokens| {
                // Convert tokens to status objects
                let mut statuses = vec![];
                let mut providers_seen = HashSet::new();

                for token in tokens {
                    if providers_seen.insert(token.provider.clone()) {
                        statuses.push(OAuthStatus {
                            provider: token.provider,
                            connected: true,
                            last_sync: Some(token.created_at.to_rfc3339()),
                        });
                    }
                }

                // Add default providers if not connected
                for provider in ["strava", "fitbit"] {
                    if !providers_seen.contains(provider) {
                        statuses.push(OAuthStatus {
                            provider: provider.to_owned(),
                            connected: false,
                            last_sync: None,
                        });
                    }
                }

                statuses
            },
        );

    Ok((StatusCode::OK, Json(provider_statuses)).into_response())
}

/// Get all providers with connection status
///
/// Returns all available providers from the registry with their connection status.
/// Uses `provider_connections` table as the single source of truth for connectivity.
pub(super) async fn handle_providers_status(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    use crate::providers::registry::global_registry;

    // Authenticate using middleware
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;

    // Get all supported providers from the registry
    let registry = global_registry();
    let supported_providers = registry.supported_providers();

    // Get user's provider connections (cross-tenant view, single source of truth)
    let connections = resources
        .repos
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap_or_default();

    let connected_providers: HashSet<String> =
        connections.into_iter().map(|c| c.provider).collect();

    // Build provider status list
    let mut provider_statuses = Vec::new();

    for provider_name in supported_providers {
        // Get provider descriptor from registry
        if let Some(descriptor) = registry.get_descriptor(provider_name) {
            let caps = descriptor.capabilities();
            let requires_oauth = caps.requires_oauth();

            // Determine connection status from the provider_connections table
            let connected = connected_providers.contains(provider_name);

            // Always show all providers regardless of connection status.
            // Non-OAuth providers (like synthetic) appear with connected=false
            // so users can activate them from the provider modal.

            // Build capabilities list from bitflags
            let mut capabilities = Vec::new();
            if caps.supports_activities() {
                capabilities.push("activities".to_owned());
            }
            if caps.supports_sleep() {
                capabilities.push("sleep".to_owned());
            }
            if caps.supports_recovery() {
                capabilities.push("recovery".to_owned());
            }
            if caps.supports_health() {
                capabilities.push("health".to_owned());
            }

            provider_statuses.push(ProviderStatus {
                provider: provider_name.to_owned(),
                display_name: descriptor.display_name().to_owned(),
                requires_oauth,
                connected,
                capabilities,
            });
        }
    }

    let response = ProvidersStatusResponse {
        providers: provider_statuses,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle OAuth authorization initiation
///
/// Requires authentication and verifies that the authenticated user matches
/// the `user_id` in the path to prevent unauthorized OAuth flow initiation.
#[tracing::instrument(
    skip(resources, headers),
    fields(
        route = "oauth_auth_initiate",
        provider = %provider,
        user_id = %user_id_str,
        tenant_id = Empty,
    )
)]
pub(super) async fn handle_oauth_auth_initiate(
    State(resources): State<Arc<ServerResources>>,
    Path((provider, user_id_str)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate the request before proceeding
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = parse_user_id(&user_id_str)?;

    // Verify authenticated user matches the requested user_id
    if auth_result.user_id != user_id {
        warn!(
            "OAuth auth initiate: authenticated user {} does not match path user_id {}",
            auth_result.user_id, user_id
        );
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Cannot initiate OAuth flow for a different user",
        ));
    }

    info!(
        "OAuth authorization initiation for provider: {} user: {}",
        provider, user_id_str
    );

    // Verify user exists
    get_user_for_oauth(resources.repos.users.as_ref(), user_id).await?;
    let tenant_id = extract_tenant_id(auth_result.active_tenant_id.map(TenantId::from))?;

    let server_context = ServerContext::from(resources.as_ref());
    let oauth_service = OAuthService::new(
        server_context.data().clone(),
        server_context.config().clone(),
        server_context.notification().clone(),
    );

    let auth_response = oauth_service
        .get_auth_url(user_id, tenant_id, &provider)
        .await
        .map_err(|e| {
            error!(
                "Failed to generate OAuth URL for {} user {}: {}",
                provider, user_id, e
            );
            AppError::internal(format!("Failed to generate OAuth URL for {provider}: {e}"))
        })?;

    info!(
        "Generated OAuth URL for {} user {} (state issued)",
        provider, user_id
    );

    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, auth_response.authorization_url)],
    )
        .into_response())
}

/// Handle mobile OAuth initiation
///
/// Returns OAuth URL in JSON format for mobile apps to use with in-app browsers.
/// Accepts optional `redirect_url` query parameter for deep linking back to the app.
#[tracing::instrument(
    skip(resources, headers, query),
    fields(
        route = "mobile_oauth_init",
        provider = %provider,
        user_id = Empty,
    )
)]
pub(super) async fn handle_mobile_oauth_init(
    State(resources): State<Arc<ServerResources>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    // Authenticate using middleware
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;
    info!(
        "Mobile OAuth initiation for provider: {} user: {}",
        provider, user_id
    );

    // Get optional redirect_uri from query parameters (mobile app's deep link)
    let redirect_url = query.get("redirect_uri");

    // Validate redirect URL against allowlist to prevent open-redirect attacks
    if let Some(url) = redirect_url {
        let base_url = &resources.config.base_url;
        let extra_origins = &resources.config.security.allowed_mobile_redirect_origins;
        if !oauth_flow_service::is_allowed_redirect_url(url, base_url, extra_origins) {
            return Err(AppError::invalid_input(
                "Invalid redirect_url. Must use pierre://, exp://, http://localhost, or an HTTPS origin matching the server's base_url.",
            ));
        }
    }

    // Verify user exists
    get_user_for_oauth(resources.repos.users.as_ref(), user_id).await?;
    let tenant_id = extract_tenant_id(auth_result.active_tenant_id.map(TenantId::from))?;

    // Build OAuth state with optional redirect URL
    let state = redirect_url.map_or_else(
        || format!("{}:{}", user_id, uuid::Uuid::new_v4()),
        |url| {
            let encoded_url = URL_SAFE_NO_PAD.encode(url.as_bytes());
            format!("{}:{}:{}", user_id, uuid::Uuid::new_v4(), encoded_url)
        },
    );

    // Generate OAuth URL using the state with embedded redirect URL
    let tenant_name = resources
        .repos
        .tenants
        .get_by_id(tenant_id)
        .await
        .map_or_else(|_| "Unknown Tenant".to_owned(), |t| t.name);

    let ctx = TenantContext {
        tenant_id,
        user_id,
        tenant_name,
        user_role: TenantRole::Member,
    };

    // Check if the provider supports PKCE for enhanced security
    let use_pkce = resources
        .provider_registry
        .get_descriptor(&provider)
        .and_then(ProviderDescriptor::oauth_params)
        .is_some_and(|p| p.use_pkce);

    let pkce = if use_pkce {
        Some(PkceParams::generate())
    } else {
        None
    };

    let authorization_url = if let Some(ref pkce_params) = pkce {
        resources
            .tenant_oauth_client
            .get_authorization_url_with_pkce(
                &ctx,
                &provider,
                &state,
                pkce_params,
                resources.repos.tenants.as_ref(),
                resources.repos.oauth_tokens.as_ref(),
            )
            .await
    } else {
        resources
            .tenant_oauth_client
            .get_authorization_url(
                &ctx,
                &provider,
                &state,
                resources.repos.tenants.as_ref(),
                resources.repos.oauth_tokens.as_ref(),
            )
            .await
    }
    .map_err(|e| {
        error!(
            "Failed to generate OAuth URL for {} user {}: {}",
            provider, user_id, e
        );
        AppError::internal(format!("Failed to generate OAuth URL for {provider}: {e}"))
    })?;

    // Build redirect URI for state storage
    let base_url = env::var("BASE_URL")
        .unwrap_or_else(|_| format!("http://localhost:{}", resources.config.http_port));
    let oauth_redirect_uri = format!("{base_url}/api/oauth/callback/{provider}");

    // Store state server-side for CSRF protection with 10-minute TTL.
    // The pkce_code_verifier is stored alongside the state for PKCE token exchange.
    let now = Utc::now();
    let client_state = OAuthClientState {
        state: state.clone(),
        provider: provider.clone(),
        user_id: Some(user_id),
        tenant_id: Some(tenant_id.to_string()),
        redirect_uri: oauth_redirect_uri,
        scope: None,
        pkce_code_verifier: pkce.as_ref().map(|p| p.code_verifier.clone()),
        created_at: now,
        expires_at: now + chrono::Duration::minutes(10),
        used: false,
    };

    resources
        .repos
        .oauth_client_state
        .store_oauth_client_state(&client_state)
        .await
        .map_err(|e| {
            error!("Failed to store OAuth state for CSRF protection: {}", e);
            AppError::internal("Failed to initiate OAuth flow")
        })?;

    info!(
        "Generated mobile OAuth URL for {} user {} (state issued){}",
        provider,
        user_id,
        if redirect_url.is_some() {
            " (with redirect)"
        } else {
            ""
        }
    );

    // Return JSON response with OAuth URL (mobile apps need this for in-app browsers)
    // State is returned so mobile apps can correlate the callback
    Ok((
        StatusCode::OK,
        Json(json!({
            "authorization_url": authorization_url,
            "provider": provider,
            "state": state,
            "message": format!("Visit the authorization URL to connect your {} account", provider)
        })),
    )
        .into_response())
}

/// REST endpoint to disconnect a provider
///
/// DELETE /api/oauth/providers/:provider/disconnect
///
/// Disconnects a fitness provider (e.g., Strava, Fitbit) by deleting the stored OAuth tokens.
/// Requires valid JWT authentication via cookie or Authorization header.
pub(super) async fn handle_disconnect_provider_rest(
    State(resources): State<Arc<ServerResources>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate using middleware (supports both cookies and Authorization header)
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;
    info!("Disconnecting provider {} for user {}", provider, user_id);

    // Create OAuthService instance and call existing disconnect logic
    let server_context = ServerContext::from(resources.as_ref());
    let oauth_service = OAuthService::new(
        server_context.data().clone(),
        server_context.config().clone(),
        server_context.notification().clone(),
    );
    oauth_service
        .disconnect_provider(user_id, &provider, auth_result.active_tenant_id)
        .await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Helper functions used by OAuth handlers
// ---------------------------------------------------------------------------

/// Parse a user ID string to UUID
fn parse_user_id(user_id_str: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(user_id_str).map_err(|_| {
        error!("Invalid user_id format: {}", user_id_str);
        AppError::invalid_input("Invalid user ID format")
    })
}

/// Retrieve user from database with proper error handling
async fn get_user_for_oauth(
    users: &dyn UserRepository,
    user_id: uuid::Uuid,
) -> Result<User, AppError> {
    match users.get_global(user_id).await {
        Ok(Some(user)) => Ok(user),
        Ok(None) => {
            error!("User {} not found in database", user_id);
            Err(AppError::not_found("User account not found"))
        }
        Err(e) => {
            error!("Failed to get user {} for OAuth: {}", user_id, e);
            Err(AppError::database(format!(
                "Failed to retrieve user information: {e}"
            )))
        }
    }
}

/// Extract tenant ID for OAuth operations from JWT claims
///
/// Returns the `active_tenant_id` from the user's JWT session.
/// Returns an error if no active tenant is set in the session.
fn extract_tenant_id(active_tenant_id: Option<TenantId>) -> Result<TenantId, AppError> {
    active_tenant_id.ok_or_else(|| AppError::auth_invalid("No active tenant in session"))
}

/// Categorize OAuth errors for better user messaging
fn categorize_oauth_error(error: &AppError) -> (&'static str, Option<&'static str>) {
    let error_str = error.to_string().to_lowercase();

    if error_str.contains("jwt") && error_str.contains("expired") {
        (
            "Your session has expired",
            Some("Please log in again to continue with OAuth authorization"),
        )
    } else if error_str.contains("jwt") && error_str.contains("invalid signature") {
        (
            "Invalid authentication token",
            Some("The authentication token signature is invalid. This may happen if the server's secret key has changed. Please log in again."),
        )
    } else if error_str.contains("jwt") && error_str.contains("malformed") {
        (
            "Malformed authentication token",
            Some("The authentication token format is invalid. Please log in again."),
        )
    } else if error_str.contains("jwt") {
        (
            "Authentication token validation failed",
            Some("There was an issue validating your authentication token. Please log in again."),
        )
    } else if error_str.contains("user not found") {
        (
            "User account not found",
            Some("The user account associated with this OAuth request could not be found."),
        )
    } else if error_str.contains("tenant") {
        (
            "Tenant configuration error",
            Some("There was an issue with your account's tenant configuration. Please contact support."),
        )
    } else if error_str.contains("oauth code") || error_str.contains("token exchange") {
        (
            "OAuth token exchange failed",
            Some("Failed to exchange the authorization code for an access token. The provider may have rejected the request."),
        )
    } else if error_str.contains("state parameter") {
        (
            "Invalid OAuth state",
            Some("The OAuth state parameter is invalid or has been tampered with. This is a security measure to prevent CSRF attacks."),
        )
    } else {
        (
            "OAuth authorization failed",
            Some("An unexpected error occurred during the OAuth authorization process."),
        )
    }
}
