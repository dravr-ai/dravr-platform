// ABOUTME: Authentication service for universal protocol handlers
// ABOUTME: Handles OAuth token management and provider creation with tenant support
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::protocol::types::{UniversalResponse, META_AUTH_REQUIRED_PROVIDER};
use crate::runtime::ToolRuntime;
use chrono::{DateTime, Utc};
use pierre_auth::oauth2_client::client::fitbit::refresh_fitbit_token;
use pierre_auth::oauth2_client::client::strava::refresh_strava_token;
use pierre_auth::oauth2_client::client::whoop::refresh_whoop_token;
use pierre_auth::tenant::{TenantContext, TenantRole};
use pierre_config::environment::get_oauth_config;
use pierre_core::errors::AppError;
use pierre_core::http_client::api_client;
use pierre_core::models::{TenantId, UserOAuthToken};
#[cfg(feature = "client-notifications")]
use pierre_notifications::models::NotificationCategory;
#[cfg(feature = "client-notifications")]
use pierre_notifications::{DispatchRequest, TenantId as CommTenantId};
use pierre_providers::backend_resolver;
use pierre_providers::{CoreFitnessProvider, OAuth2Credentials};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
#[cfg(not(feature = "client-notifications"))]
use std::future::ready;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// OAuth token data structure
#[derive(Debug, Clone)]
pub struct TokenData {
    /// OAuth access token
    pub access_token: String,
    /// OAuth refresh token
    pub refresh_token: String,
    /// When the access token expires
    pub expires_at: DateTime<Utc>,
    /// OAuth scopes as comma-separated string
    pub scopes: String,
    /// Provider name (e.g., "strava", "fitbit")
    pub provider: String,
    /// Provider-side user id for API-key providers (e.g. Intervals.icu
    /// `athlete_id`, used as the HTTP Basic username). `None` for OAuth
    /// bearer providers, which carry identity inside the access token.
    pub provider_user_id: Option<String>,
}

/// OAuth error types
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    /// Failed to exchange authorization code for tokens
    #[error("Token exchange failed: {0}")]
    TokenExchangeFailed(String),

    /// Failed to refresh expired access token
    #[error("Token refresh failed: {0}")]
    TokenRefreshFailed(String),

    /// Database operation failed
    #[error("Database error: {0}")]
    DatabaseError(String),
}

/// Classify a token-refresh failure as either "the user must re-authorize" or transient.
///
/// Returns `Some(error_code)` when the provider's token endpoint definitively rejected the
/// refresh — a dead/rotated refresh token, a revoked grant, or a rejected client. These do
/// not recover on retry: the connection must be flipped to `needs_reauth` and the user
/// prompted to reconnect. Returns `None` for transient failures (network, timeout, 5xx),
/// which must NOT disconnect a healthy connection — a flake is not a disconnect.
///
/// Conservative by design: transient transport/server signatures are checked first and
/// short-circuit to `None`, so an auth marker that merely appears in a 5xx body can't
/// trip a false disconnect. WHOOP returns `invalid_request` for a consumed/rotated refresh
/// token; RFC 6749's code for a dead refresh token is `invalid_grant`.
fn classify_refresh_failure(reason: &str) -> Option<&'static str> {
    // Transient transport/server failures recover on retry — never disconnect on these.
    const TRANSIENT_MARKERS: [&str; 8] = [
        "timed out",
        "timeout",
        "connection",
        "dns",
        "http 500",
        "http 502",
        "http 503",
        "http 504",
    ];
    // Definitive OAuth-layer rejections of the refresh grant or client → require reconnect.
    const REAUTH_MARKERS: [&str; 8] = [
        "invalid_grant",
        "invalid_request",
        "invalid_client",
        "unauthorized_client",
        "unauthorized",
        "forbidden",
        "http 401",
        "http 403",
    ];

    let r = reason.to_ascii_lowercase();
    if TRANSIENT_MARKERS.into_iter().any(|m| r.contains(m)) {
        return None;
    }
    REAUTH_MARKERS.into_iter().find(|m| r.contains(*m))
}

/// Service responsible for authentication and provider creation
/// Centralizes OAuth token management and reduces duplication across handlers
pub struct AuthService {
    resources: Arc<dyn ToolRuntime>,
}

impl AuthService {
    /// Create new authentication service
    #[must_use]
    pub const fn new(resources: Arc<dyn ToolRuntime>) -> Self {
        Self { resources }
    }

    /// The runtime backing this service.
    ///
    /// Exposed for callers that need repository access alongside provider
    /// authentication — notably activity-cache write-through, which persists
    /// freshly fetched activities keyed by the connection provider name.
    #[must_use]
    pub const fn runtime(&self) -> &Arc<dyn ToolRuntime> {
        &self.resources
    }

    /// Get valid token for a provider, automatically refreshing if needed
    /// Returns None if no token exists or is expired, Error if token operations fail
    ///
    /// # Errors
    /// Returns `OAuthError` if token refresh fails or database operations fail
    pub async fn get_valid_token(
        &self,
        user_id: Uuid,
        provider: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<TokenData>, OAuthError> {
        debug!(
            "get_valid_token called for user={}, provider={}, tenant={:?}",
            user_id, provider, tenant_id
        );

        // If we have tenant context, initialize tenant-specific OAuth credentials
        if let Some(tenant_id_str) = tenant_id {
            self.initialize_tenant_oauth_context(user_id, tenant_id_str, provider)
                .await;
        }

        // Look up token from database with tenant context
        let Some(tenant_id_str) = tenant_id else {
            debug!("No tenant_id provided, returning Ok(None)");
            return Ok(None);
        };

        // Direct database lookup with tenant_id
        let tenant_id_parsed: TenantId = tenant_id_str.parse().map_err(|_| {
            OAuthError::DatabaseError(format!("Invalid tenant_id format: {tenant_id_str}"))
        })?;
        let token_result = self
            .resources
            .repos()
            .oauth_tokens
            .get_token(user_id, tenant_id_parsed, provider)
            .await;

        Self::log_token_lookup_result(&token_result, user_id, tenant_id_str, provider);

        let Ok(Some(oauth_token)) = token_result else {
            return Ok(None);
        };

        // Process the token - validate expiration and refresh if needed
        self.process_oauth_token(user_id, tenant_id_str, provider, oauth_token)
            .await
    }

    /// Initialize tenant-specific OAuth context if available
    async fn initialize_tenant_oauth_context(
        &self,
        user_id: Uuid,
        tenant_id_str: &str,
        provider: &str,
    ) {
        let Ok(tenant_uuid) = tenant_id_str.parse::<TenantId>() else {
            return;
        };

        let Ok(tenant) = self.resources.repos().tenants.get_by_id(tenant_uuid).await else {
            return;
        };

        let tenant_context = TenantContext {
            tenant_id: tenant_uuid,
            tenant_name: tenant.name.clone(), // Safe: String ownership needed for tenant context
            user_id,
            user_role: TenantRole::Member,
        };

        // Get tenant-specific OAuth credentials - result is unused but initializes context
        let _ = self
            .resources
            .tenant_oauth_client()
            .get_oauth_client(
                &tenant_context,
                provider,
                self.resources.repos().tenants.as_ref(),
                self.resources.repos().oauth_tokens.as_ref(),
            )
            .await;
    }

    /// Process OAuth token - validate expiration and refresh if needed
    async fn process_oauth_token(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
        oauth_token: UserOAuthToken,
    ) -> Result<Option<TokenData>, OAuthError> {
        // Check if token is expired (with 5-minute buffer)
        if let Some(expires_at) = oauth_token.expires_at {
            if Self::is_token_expired(expires_at) {
                return self
                    .handle_expired_token(user_id, tenant_id, provider, &oauth_token)
                    .await;
            }
        }

        // Token is valid, return it
        Ok(Some(TokenData {
            provider: provider.to_owned(),
            access_token: oauth_token.access_token,
            refresh_token: oauth_token.refresh_token.unwrap_or_default(),
            expires_at: oauth_token.expires_at.unwrap_or_else(chrono::Utc::now),
            scopes: oauth_token.scope.unwrap_or_default(),
            provider_user_id: oauth_token.provider_user_id,
        }))
    }

    /// Check if token is expired or expiring within 5 minutes
    fn is_token_expired(expires_at: DateTime<Utc>) -> bool {
        let now = chrono::Utc::now();
        expires_at <= now + chrono::Duration::minutes(5)
    }

    /// Log the result of a token lookup operation
    fn log_token_lookup_result(
        token_result: &Result<Option<UserOAuthToken>, AppError>,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
    ) {
        match token_result {
            Ok(Some(token)) => debug!(
                "Found OAuth token for user={}, provider={}, expires_at={:?}",
                user_id, provider, token.expires_at
            ),
            Ok(None) => debug!(
                "No OAuth token found for user={}, tenant={}, provider={}",
                user_id, tenant_id, provider
            ),
            Err(e) => warn!(
                "Error retrieving OAuth token for user={}, tenant={}, provider={}: {}",
                user_id, tenant_id, provider, e
            ),
        }
    }

    /// Handle expired token by attempting refresh
    async fn handle_expired_token(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
        oauth_token: &UserOAuthToken,
    ) -> Result<Option<TokenData>, OAuthError> {
        // Check if we have a valid refresh token
        let Some(ref refresh_token) = oauth_token.refresh_token else {
            return Ok(None);
        };

        if refresh_token.is_empty() {
            return Ok(None);
        }

        info!(
            "Token expired for user {} provider {}, attempting refresh",
            user_id, provider
        );

        // Attempt to refresh the token
        match self
            .refresh_provider_token(user_id, tenant_id, provider, refresh_token)
            .await
        {
            Ok(refreshed_token) => {
                info!(
                    "Token refreshed successfully for user {} provider {}",
                    user_id, provider
                );
                // A successful refresh proves the token works — re-arm a connection that a
                // prior transient/raced failure may have flipped to needs_reauth. No-op when
                // already active.
                self.mark_connection_active(user_id, tenant_id, provider)
                    .await;
                Ok(Some(refreshed_token))
            }
            Err(e) => {
                self.handle_refresh_failure(user_id, tenant_id, provider, &e)
                    .await;
                Ok(None)
            }
        }
    }

    /// Log a token refresh failure and, when it is a non-recoverable auth rejection,
    /// flip the connection to `needs_reauth`.
    ///
    /// A dead/rotated refresh token, a revoked grant, or a rejected client means the user
    /// must reconnect — flip the connection so the status tool, group context, and app
    /// stop treating it as healthy. Transient failures (network, 5xx) are logged only: a
    /// flake is not a disconnect.
    async fn handle_refresh_failure(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
        error: &OAuthError,
    ) {
        warn!(
            "Token refresh failed for user {} provider {}: {}",
            user_id, provider, error
        );
        if let Some(error_code) = classify_refresh_failure(&error.to_string()) {
            self.mark_connection_needs_reauth(user_id, tenant_id, provider, error_code)
                .await;
        }
    }

    /// Persist a non-recoverable refresh failure as `needs_reauth` on the provider
    /// connection (best-effort).
    ///
    /// The connection row stays in the DB so the user/tenant mapping and history survive;
    /// only its `status` flips. A write failure here must not abort the user's request —
    /// log and continue.
    async fn mark_connection_needs_reauth(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
        error_code: &str,
    ) {
        let Ok(tenant) = tenant_id.parse::<TenantId>() else {
            warn!("Cannot mark {provider} needs_reauth for user {user_id}: invalid tenant_id");
            return;
        };
        match self
            .resources
            .repos()
            .provider_connections
            .mark_needs_reauth(user_id, tenant, provider, Some(error_code))
            .await
        {
            Ok(()) => {
                info!(
                    "Provider {provider} flipped to needs_reauth for user {user_id} ({error_code})"
                );
                self.notify_provider_disconnected(user_id, tenant, provider)
                    .await;
            }
            Err(e) => {
                warn!("Failed to persist needs_reauth for user {user_id} provider {provider}: {e}");
            }
        }
    }

    /// Send a one-time out-of-band push telling the user a provider disconnected and to
    /// reconnect.
    ///
    /// Deduped via the `notified_at` claim so a user is nudged exactly once per
    /// `active`→`needs_reauth` transition (re-armed on reconnect). Best-effort; cfg-gated on
    /// `client-notifications` so builds without the notification backend compile to a no-op.
    #[cfg(feature = "client-notifications")]
    async fn notify_provider_disconnected(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) {
        let claimed = self
            .resources
            .repos()
            .provider_connections
            .claim_reauth_notification(user_id, tenant_id, provider)
            .await
            .unwrap_or(false);
        if !claimed {
            return;
        }
        let Some(service) = self.resources.notification_service() else {
            return;
        };
        let data = [
            ("type", "provider_needs_reauth"),
            ("provider", provider),
            ("screen", "connections"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), JsonValue::String(v.to_owned())))
        .collect();
        let request = DispatchRequest {
            user_id,
            tenant_id: CommTenantId(tenant_id.0),
            category: NotificationCategory::System,
            notification_type: "provider_needs_reauth".to_owned(),
            title: "Reconnect needed".to_owned(),
            body: format!(
                "Your {provider} connection expired. Reconnect it so I can keep using your data."
            ),
            data: Some(JsonValue::Object(data)),
            image_url: None,
            actions: None,
            bypass_frequency_cap: false,
        };
        if let Err(e) = service.dispatch(&request).await {
            warn!("Failed to dispatch reauth push for user {user_id} provider {provider}: {e}");
        }
    }

    /// No-op variant when the notification backend is not compiled in.
    #[cfg(not(feature = "client-notifications"))]
    async fn notify_provider_disconnected(
        &self,
        _user_id: Uuid,
        _tenant_id: TenantId,
        _provider: &str,
    ) {
        // Nothing to dispatch without a backend; await a ready future so the
        // signature stays `async` for the unconditional caller.
        ready(()).await;
    }

    /// Re-arm a provider connection to `active` after a successful refresh (best-effort).
    ///
    /// No-op when the connection is already active or the row does not exist. A write
    /// failure must not abort the user's request — log and continue.
    async fn mark_connection_active(&self, user_id: Uuid, tenant_id: &str, provider: &str) {
        let Ok(tenant) = tenant_id.parse::<TenantId>() else {
            return;
        };
        if let Err(e) = self
            .resources
            .repos()
            .provider_connections
            .mark_active(user_id, tenant, provider)
            .await
        {
            warn!("Failed to re-arm connection for user {user_id} provider {provider}: {e}");
        }
    }

    /// Whether the user's connection for `provider` is flipped to `needs_reauth`/`revoked`.
    ///
    /// Reads the persisted `provider_connections.status`. Best-effort: a read failure
    /// returns `false` so a DB blip never spuriously short-circuits a turn.
    async fn connection_needs_reauth(
        &self,
        user_id: Uuid,
        tenant_id: Option<&str>,
        provider: &str,
    ) -> bool {
        let tenant = tenant_id.and_then(|t| t.parse::<TenantId>().ok());
        self.resources
            .repos()
            .provider_connections
            .get_for_user(user_id, tenant)
            .await
            .unwrap_or_default()
            .iter()
            .any(|c| c.provider == provider && c.status.requires_reauth())
    }

    /// Create authenticated provider with proper tenant-aware credentials
    /// Returns configured provider ready for API calls
    ///
    /// The `requested_provider` is the name the caller asked for (typically a
    /// user-facing name like "strava" or "garmin"). When the user has a
    /// sciotte* row in the database the resolver swaps that for the mirror
    /// backend — callers never need to know about that distinction.
    ///
    /// # Errors
    /// Returns `UniversalResponse` error if provider is unsupported or authentication fails
    pub async fn create_authenticated_provider(
        &self,
        requested_provider: &str,
        user_id: Uuid,
        tenant_id: Option<&str>,
    ) -> Result<Box<dyn CoreFitnessProvider>, UniversalResponse> {
        // Resolve the user-facing provider to the backend that actually
        // serves the request (OAuth or sciotte mirror). A sciotte row in
        // the DB wins over OAuth, even when its session is stale — the
        // user's stated preference is to keep using the mirror.
        let tenant_id_parsed = tenant_id.and_then(|t| t.parse::<TenantId>().ok());
        let effective_provider = backend_resolver::resolve_backend(
            &self.resources.repos().auth_repos(),
            user_id,
            tenant_id_parsed,
            requested_provider,
        )
        .await;
        let provider_name = effective_provider.as_str();

        // Check if provider is supported by the registry
        if !self
            .resources
            .provider_registry()
            .is_supported(provider_name)
        {
            return Err(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Unsupported provider: {requested_provider}")),
                metadata: None,
            });
        }

        // Get valid token for the (resolved) provider with automatic refresh.
        match self
            .get_valid_token(user_id, provider_name, tenant_id)
            .await
        {
            Ok(Some(token_data)) => {
                self.create_provider_with_token(provider_name, token_data, user_id, tenant_id)
                    .await
            }
            Ok(None) => {
                // Report the failure in terms of the user-facing provider
                // so the LLM never mentions "sciotte" in an error it
                // forwards to the user.
                let user_facing = backend_resolver::user_facing_name(provider_name);
                // When the connection died non-recoverably (needs_reauth), tag the response
                // with the auth-required provider so the chat tool loop short-circuits the
                // turn and the auth_recovery stage renders a localized reconnect message +
                // minted OAuth link — instead of letting the LLM rephrase a generic error.
                let metadata = if self
                    .connection_needs_reauth(user_id, tenant_id, provider_name)
                    .await
                {
                    let mut map = HashMap::new();
                    map.insert(
                        META_AUTH_REQUIRED_PROVIDER.to_owned(),
                        JsonValue::String(user_facing.to_owned()),
                    );
                    Some(map)
                } else {
                    None
                };
                Err(UniversalResponse {
                    success: false,
                    result: None,
                    error: Some(format!(
                        "No valid {user_facing} token found. Please reconnect your {user_facing} account."
                    )),
                    metadata,
                })
            }
            Err(e) => Err(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Authentication error: {e}")),
                metadata: None,
            }),
        }
    }

    /// Create provider with token and tenant-aware credentials
    async fn create_provider_with_token(
        &self,
        provider_name: &str,
        token_data: TokenData,
        user_id: Uuid,
        tenant_id: Option<&str>,
    ) -> Result<Box<dyn CoreFitnessProvider>, UniversalResponse> {
        // Get tenant-aware OAuth credentials or fall back to environment.
        // Non-OAuth providers (sciotte, synthetic) skip credential lookup entirely.
        let requires_oauth = self
            .resources
            .provider_registry()
            .requires_oauth(provider_name);

        let (client_id, client_secret) = if !requires_oauth {
            // API-key providers (e.g. Intervals.icu) carry their provider-side
            // user id here so it reaches the provider as `client_id` (the HTTP
            // Basic username). Synthetic providers have no id → empty string.
            (
                token_data.provider_user_id.clone().unwrap_or_default(),
                String::new(),
            )
        } else if let Some(tenant_id_str) = tenant_id {
            // Check user-specific OAuth app credentials first
            if let Ok(Some(user_app)) = self
                .resources
                .repos()
                .oauth_tokens
                .get_user_oauth_app(user_id, provider_name)
                .await
            {
                (user_app.client_id, user_app.client_secret)
            } else {
                // Fall back to tenant-level credentials
                let tid: TenantId = tenant_id_str.parse().map_err(|_| UniversalResponse {
                    success: false,
                    result: None,
                    error: Some(format!("Invalid tenant_id: {tenant_id_str}")),
                    metadata: None,
                })?;
                let creds = self
                    .resources
                    .repos()
                    .tenants
                    .get_oauth_credentials(tid, provider_name)
                    .await
                    .map_err(|e| UniversalResponse {
                        success: false,
                        result: None,
                        error: Some(format!("Failed to get OAuth credentials: {e}")),
                        metadata: None,
                    })?;
                match creds {
                    Some(c) => (c.client_id, c.client_secret),
                    None => Self::get_default_oauth_credentials(provider_name).map_err(|e| *e)?,
                }
            }
        } else {
            Self::get_default_oauth_credentials(provider_name).map_err(|e| *e)?
        };

        // Get provider-specific scopes
        let scopes = token_data
            .scopes
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();

        // Create provider using the factory function
        match self
            .resources
            .provider_registry()
            .create_provider(provider_name)
        {
            Ok(provider) => {
                // Prepare credentials in the correct format
                let credentials = OAuth2Credentials {
                    client_id,
                    client_secret,
                    access_token: Some(token_data.access_token),
                    refresh_token: Some(token_data.refresh_token),
                    expires_at: Some(token_data.expires_at),
                    scopes,
                };

                // Set credentials asynchronously
                match provider.set_credentials(credentials).await {
                    Ok(()) => {
                        // Wire up callback so provider-level token refresh persists to DB
                        let resources = Arc::clone(&self.resources);
                        let provider_name_owned = provider_name.to_owned();
                        let tenant_id_owned = tenant_id.map(str::to_owned);
                        provider.set_token_refresh_callback(Arc::new(
                            move |creds: OAuth2Credentials| {
                                let resources = Arc::clone(&resources);
                                let provider_name = provider_name_owned.clone();
                                let tenant_id = tenant_id_owned.clone();
                                Box::pin(async move {
                                    persist_refreshed_token(
                                        &*resources,
                                        user_id,
                                        tenant_id.as_deref(),
                                        &provider_name,
                                        &creds,
                                    )
                                    .await;
                                })
                            },
                        ));
                        Ok(provider)
                    }
                    Err(e) => Err(UniversalResponse {
                        success: false,
                        result: None,
                        error: Some(format!("Failed to set provider credentials: {e}")),
                        metadata: None,
                    }),
                }
            }
            Err(e) => Err(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to create provider: {e}")),
                metadata: None,
            }),
        }
    }

    /// Get default OAuth credentials from `ServerConfig` or environment for a provider
    ///
    /// # Errors
    /// Returns boxed `UniversalResponse` error if credentials are not configured
    fn get_default_oauth_credentials(
        provider_name: &str,
    ) -> Result<(String, String), Box<UniversalResponse>> {
        // Get OAuth config from environment (PIERRE_<PROVIDER>_* env vars)
        let oauth_config = get_oauth_config(provider_name);

        let client_id = oauth_config.client_id.as_ref().ok_or_else(|| {
            Box::new(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!(
                    "{}_CLIENT_ID not configured for provider {}",
                    provider_name.to_uppercase(),
                    provider_name
                )),
                metadata: None,
            })
        })?;

        let client_secret = oauth_config.client_secret.as_ref().ok_or_else(|| {
            Box::new(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!(
                    "{}_CLIENT_SECRET not configured for provider {}",
                    provider_name.to_uppercase(),
                    provider_name
                )),
                metadata: None,
            })
        })?;

        Ok((client_id.clone(), client_secret.clone()))
    }

    /// Refresh an expired OAuth token for a provider
    ///
    /// Calls the provider's token refresh endpoint and stores the new token in the database.
    ///
    /// # Errors
    /// Returns `OAuthError` if token refresh or database operations fail
    async fn refresh_provider_token(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
        refresh_token: &str,
    ) -> Result<TokenData, OAuthError> {
        // Get OAuth credentials: user-specific → tenant-level → env var defaults
        let (client_id, client_secret) = if tenant_id.is_empty() {
            Self::get_default_oauth_credentials(provider)
                .map_err(|e| OAuthError::TokenRefreshFailed(e.error.unwrap_or_default()))?
        } else {
            // Check user-specific OAuth app credentials first
            if let Ok(Some(user_app)) = self
                .resources
                .repos()
                .oauth_tokens
                .get_user_oauth_app(user_id, provider)
                .await
            {
                info!(
                    "Using user-specific {} credentials for token refresh (user={})",
                    provider, user_id
                );
                (user_app.client_id, user_app.client_secret)
            } else {
                // Fall back to tenant-level credentials
                let tid: TenantId = tenant_id.parse().map_err(|_| {
                    OAuthError::TokenRefreshFailed(format!("Invalid tenant_id: {tenant_id}"))
                })?;
                let creds = self
                    .resources
                    .repos()
                    .tenants
                    .get_oauth_credentials(tid, provider)
                    .await
                    .map_err(|e| OAuthError::TokenRefreshFailed(e.to_string()))?;
                match creds {
                    Some(c) => (c.client_id, c.client_secret),
                    None => Self::get_default_oauth_credentials(provider)
                        .map_err(|e| OAuthError::TokenRefreshFailed(e.error.unwrap_or_default()))?,
                }
            }
        };

        // Call provider-specific token refresh
        let new_token = match provider.to_lowercase().as_str() {
            "strava" => {
                let http_client = api_client();
                refresh_strava_token(http_client, &client_id, &client_secret, refresh_token)
                    .await
                    .map_err(|e| OAuthError::TokenRefreshFailed(e.to_string()))?
            }
            "fitbit" => {
                let http_client = api_client();
                refresh_fitbit_token(http_client, &client_id, &client_secret, refresh_token)
                    .await
                    .map_err(|e| OAuthError::TokenRefreshFailed(e.to_string()))?
            }
            "whoop" => {
                let http_client = api_client();
                refresh_whoop_token(http_client, &client_id, &client_secret, refresh_token)
                    .await
                    .map_err(|e| OAuthError::TokenRefreshFailed(e.to_string()))?
            }
            other => {
                return Err(OAuthError::TokenRefreshFailed(format!(
                    "Token refresh not supported for provider: {other}"
                )));
            }
        };

        // Prepare token data for database update
        let new_access_token = new_token.access_token.clone();
        let new_refresh_token = new_token.refresh_token.clone();
        let new_expires_at = new_token.expires_at;

        // Update the token in the database
        let tenant_id_parsed: TenantId = tenant_id.parse().map_err(|_| {
            OAuthError::DatabaseError(format!("Invalid tenant_id format: {tenant_id}"))
        })?;
        self.resources
            .repos()
            .oauth_tokens
            .refresh_token(
                user_id,
                tenant_id_parsed,
                provider,
                &new_access_token,
                new_refresh_token.as_deref(),
                new_expires_at,
            )
            .await
            .map_err(|e| OAuthError::DatabaseError(e.to_string()))?;

        // Return the refreshed token data. Refresh only runs for OAuth bearer
        // providers (strava/fitbit/whoop), which have no provider-side user id.
        Ok(TokenData {
            provider: provider.to_owned(),
            access_token: new_access_token,
            refresh_token: new_refresh_token.unwrap_or_default(),
            expires_at: new_expires_at.unwrap_or_else(chrono::Utc::now),
            scopes: new_token.scope.unwrap_or_default(),
            provider_user_id: None,
        })
    }

    /// Check if user has valid authentication for a provider
    pub async fn has_valid_auth(
        &self,
        user_id: Uuid,
        provider: &str,
        tenant_id: Option<&str>,
    ) -> bool {
        matches!(
            self.get_valid_token(user_id, provider, tenant_id).await,
            Ok(Some(_))
        )
    }

    /// Disconnect user from a provider
    ///
    /// # Errors
    /// Returns `OAuthError` if database operations fail
    pub async fn disconnect_provider(
        &self,
        user_id: Uuid,
        provider: &str,
        tenant_id: Option<&str>,
    ) -> Result<(), OAuthError> {
        // Use database to delete tokens directly (like original implementation)
        let tenant_id_str = tenant_id.ok_or_else(|| {
            OAuthError::DatabaseError("tenant_id is required to disconnect a provider".to_owned())
        })?;
        let tenant_id_parsed: TenantId = tenant_id_str.parse().map_err(|_| {
            OAuthError::DatabaseError(format!("Invalid tenant_id format: {tenant_id_str}"))
        })?;
        self.resources
            .repos()
            .oauth_tokens
            .delete_token(user_id, tenant_id_parsed, provider)
            .await
            .map_err(|e| OAuthError::DatabaseError(format!("Failed to delete token: {e}")))
    }
}

/// Persist a provider-refreshed token to the database.
/// Called from the token refresh callback wired into providers.
/// Errors are logged but not propagated — the in-memory token is still valid for the current request.
async fn persist_refreshed_token(
    resources: &dyn ToolRuntime,
    user_id: Uuid,
    tenant_id: Option<&str>,
    provider: &str,
    creds: &OAuth2Credentials,
) {
    let Some(tenant_id_str) = tenant_id else {
        return;
    };
    let Ok(tid) = tenant_id_str.parse::<TenantId>() else {
        return;
    };

    let result = resources
        .repos()
        .oauth_tokens
        .refresh_token(
            user_id,
            tid,
            provider,
            creds.access_token.as_deref().unwrap_or_default(),
            creds.refresh_token.as_deref(),
            creds.expires_at,
        )
        .await;

    match result {
        Ok(()) => info!(
            "Persisted provider-refreshed {} token for user {}",
            provider, user_id
        ),
        Err(e) => warn!(
            "Failed to persist provider-refreshed {} token for user {}: {}",
            provider, user_id, e
        ),
    }
}
