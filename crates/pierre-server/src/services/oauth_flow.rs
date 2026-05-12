// ABOUTME: OAuth flow business logic extracted from route handlers
// ABOUTME: State parsing, redirect URL validation, PKCE, token exchange, provider disconnect, connection status
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::{
    collections::{HashMap, HashSet},
    env,
    fmt::Write,
    time::Duration as StdDuration,
};

use chrono::Utc;
use serde_json::{json, Value as JsonValue};
use tracing::{debug, error, info, warn};
use urlencoding::encode;

use crate::{
    config::environment::get_oauth_config,
    context::{ConfigContext, DataContext, NotificationContext},
    errors::{AppError, AppResult},
    mcp::schema::OAuthCompletedNotification,
    models::{ConnectionType, TenantId, User, UserOAuthToken},
    types::OAuthCallbackResponse,
    utils::http_client::{get_oauth_callback_notification_timeout_secs, shared_client},
};
use pierre_auth::oauth2_client::{
    OAuth2Client, OAuth2Config, OAuth2Token, OAuthClientState, PkceParams,
};
use pierre_database::database::repositories::UserRepository;

use crate::routes::auth::types::{ConnectionStatus, OAuthAuthorizationResponse};

/// App-specific URL schemes that are always allowed for mobile OAuth redirects.
/// These are deep-link schemes that cannot be intercepted by external websites.
const APP_SCHEMES: &[&str] = &["pierre://", "exp://", "http://localhost"];

/// Validate a mobile OAuth redirect URL against the allowlist.
///
/// Allowed redirect targets:
/// - `pierre://` deep links (mobile app)
/// - `exp://` deep links (Expo development)
/// - `http://localhost` (local development)
/// - `https://` URLs whose origin matches `base_url` or an entry in
///   `allowed_redirect_origins` (prevents open-redirect to arbitrary sites)
///
/// The `base_url` is the server's own origin (e.g. `https://api.dravr.ai`).
/// `extra_origins` are additional HTTPS origins configured via
/// `ALLOWED_MOBILE_REDIRECT_ORIGINS` (e.g. Cloudflare tunnel URLs).
#[must_use]
pub fn is_allowed_redirect_url(url: &str, base_url: &str, extra_origins: &[String]) -> bool {
    // App-specific schemes are always safe
    if APP_SCHEMES.iter().any(|scheme| url.starts_with(scheme)) {
        return true;
    }

    // For https:// URLs, verify the origin matches an allowlisted host
    if url.starts_with("https://") {
        return is_origin_allowed(url, base_url, extra_origins);
    }

    false
}

/// Extract the host portion from a URL string (`scheme://host/path` -> host)
fn extract_host(url: &str) -> Option<&str> {
    // Strip scheme
    let after_scheme = url.split("://").nth(1)?;
    // Take host (before first / or ? or :port)
    let host = after_scheme
        .split('/')
        .next()?
        .split('?')
        .next()?
        .split(':')
        .next()?;
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// Check whether an HTTPS URL's origin matches the server `base_url` or an extra allowed origin.
fn is_origin_allowed(url: &str, base_url: &str, extra_origins: &[String]) -> bool {
    let Some(redirect_host) = extract_host(url) else {
        warn!("Failed to extract host from redirect URL: {url}");
        return false;
    };

    // Check against server's own base_url
    if let Some(base_host) = extract_host(base_url) {
        if base_host == redirect_host {
            return true;
        }
    }

    // Check against extra allowed origins
    for origin in extra_origins {
        if let Some(allowed_host) = extract_host(origin) {
            if allowed_host == redirect_host {
                return true;
            }
        }
    }

    warn!(
        "Redirect URL host '{}' not in allowlist (base_url: {}, extra: {:?})",
        redirect_host, base_url, extra_origins
    );
    false
}

/// Extract mobile redirect URL from the OAuth state string
///
/// State format: `{user_id}:{random}:{base64_redirect_url}`
/// The redirect URL is embedded as base64-encoded data in the third segment.
///
/// Returns `None` if the state doesn't contain a redirect URL or decoding fails.
/// The `base_url` and `extra_origins` are used to validate HTTPS redirect targets.
#[must_use]
pub fn extract_mobile_redirect_from_state(
    state: &str,
    base_url: &str,
    extra_origins: &[String],
) -> Option<String> {
    let parts: Vec<&str> = state.splitn(3, ':').collect();
    parts
        .get(2)
        .filter(|s| !s.is_empty())
        .and_then(|encoded| decode_and_validate_redirect_url(encoded, base_url, extra_origins))
}

/// Decode a base64-encoded redirect URL and validate against the allowlist
///
/// Only URLs with allowed schemes/origins are accepted to prevent open redirect attacks.
///
/// Returns `None` if decoding fails or the URL is not allowed.
#[must_use]
pub fn decode_and_validate_redirect_url(
    encoded: &str,
    base_url: &str,
    extra_origins: &[String],
) -> Option<String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|e| {
            warn!("Failed to decode base64 redirect URL: {}", e);
            e
        })
        .ok()
        .and_then(|bytes| {
            String::from_utf8(bytes)
                .map_err(|e| {
                    warn!("Failed to decode redirect URL as UTF-8: {}", e);
                    e
                })
                .ok()
        })
        .and_then(|url| {
            if is_allowed_redirect_url(&url, base_url, extra_origins) {
                Some(url)
            } else {
                warn!("Rejected redirect URL (not in allowlist): {}", url);
                None
            }
        })
}

// ---------------------------------------------------------------------------
// OAuthService — core business logic for OAuth flows
// ---------------------------------------------------------------------------

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
    #[tracing::instrument(
        skip(self, code, state),
        fields(
            provider = %provider,
            user_id = tracing::field::Empty,
            tenant_id = tracing::field::Empty,
        )
    )]
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
        let flow_label = if mobile_redirect_url.is_some() {
            " (mobile flow)"
        } else {
            ""
        };

        info!("Processing OAuth callback for user {user_id} provider {provider}{flow_label}");

        // Get user and tenant from database
        let (user, tenant_id) = self.get_user_and_tenant(user_id, provider).await?;

        // Record IDs on the current span so the NotifyLayer can attribute the
        // provider.connected event without re-passing tenant/user fields.
        let span = tracing::Span::current();
        span.record("user_id", tracing::field::display(&user_id));
        span.record("tenant_id", tracing::field::display(&tenant_id));

        // Exchange OAuth code for access token (with PKCE if verifier was stored)
        // Pass tenant_id from state so exchange uses tenant-specific credentials if available
        let token = self
            .exchange_oauth_code(
                code,
                provider,
                user_id,
                parsed_state.pkce_code_verifier.as_deref(),
                parsed_state.tenant_id,
            )
            .await?;

        info!("Successfully exchanged OAuth code for user {user_id} provider {provider}");

        // Persist token and dispatch all post-connection side effects
        let expires_at = self
            .finalize_oauth_connection(user_id, tenant_id, provider, &user.email, &token)
            .await?;

        // notify: provider successfully linked. Fires after token persist +
        // notifications dispatch so a Slack ping only goes out for a usable link.
        info!(
            target: "notify",
            event = "provider.connected",
            provider = %provider,
            "user connected fitness provider"
        );

        Ok(OAuthCallbackResponse {
            user_id: user_id.to_string(),
            provider: provider.to_owned(),
            expires_at: expires_at.to_rfc3339(),
            scopes: token.scope.unwrap_or_else(|| "read".to_owned()),
            mobile_redirect_url,
        })
    }

    /// Persist the OAuth token and dispatch all post-connection side effects.
    ///
    /// Stores the token, sends UI/bridge notifications, and logs the ops event.
    async fn finalize_oauth_connection(
        &self,
        user_id: uuid::Uuid,
        tenant_id: String,
        provider: &str,
        user_email: &str,
        token: &OAuth2Token,
    ) -> AppResult<chrono::DateTime<chrono::Utc>> {
        let expires_at = self
            .store_oauth_token(user_id, tenant_id, provider, token)
            .await?;
        self.send_oauth_notifications(user_id, provider, &expires_at)
            .await?;
        self.notify_bridge_oauth_success(provider, token).await;

        // Send ops notification for provider connection
        crate::ops_notifier().notify_oauth_connected(user_email, provider);

        // Health data backfill is triggered by the callback handler after this returns.
        // The scheduler will also auto-detect this user on subsequent cycles.
        #[cfg(feature = "health-sync")]
        {
            tracing::info!(
                user_id = %user_id,
                provider = provider,
                "Health data sync: provider connected, backfill triggered from callback handler"
            );
        }

        Ok(expires_at)
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
    /// Delegates to `extract_mobile_redirect_from_state`.
    fn extract_mobile_redirect_from_state_str(&self, state: &str) -> Option<String> {
        let config = self.config.config();
        extract_mobile_redirect_from_state(
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
                    "Successfully notified bridge about {} OAuth completion",
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

        // Send ops notification for provider disconnection
        let disconnect_email = self
            .data
            .repos()
            .users
            .get_global(user_id)
            .await
            .ok()
            .flatten()
            .map_or_else(|| user_id.to_string(), |u| u.email);
        crate::ops_notifier().notify_oauth_disconnected(&disconnect_email, provider);

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

// ---------------------------------------------------------------------------
// Helper functions used by OAuth handlers
// ---------------------------------------------------------------------------

/// Parse a user ID string to UUID
pub(crate) fn parse_user_id(user_id_str: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(user_id_str).map_err(|_| {
        error!("Invalid user_id format: {}", user_id_str);
        AppError::invalid_input("Invalid user ID format")
    })
}

/// Retrieve user from database with proper error handling
pub(crate) async fn get_user_for_oauth(
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
pub(crate) fn extract_tenant_id(active_tenant_id: Option<TenantId>) -> Result<TenantId, AppError> {
    active_tenant_id.ok_or_else(|| AppError::auth_invalid("No active tenant in session"))
}

/// Categorize OAuth errors for better user messaging
pub(crate) fn categorize_oauth_error(error: &AppError) -> (&'static str, Option<&'static str>) {
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
