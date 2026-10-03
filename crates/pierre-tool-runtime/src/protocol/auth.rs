// ABOUTME: Authentication service for universal protocol handlers
// ABOUTME: Handles OAuth token management and provider creation with tenant support
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::protocol::token_writeback::persist_refreshed_token;
use crate::protocol::types::{UniversalResponse, META_AUTH_REQUIRED_PROVIDER};
use crate::runtime::ToolRuntime;
use chrono::{DateTime, Utc};
use pierre_auth::tenant::oauth_manager::{issuing_client, IssuingLookup};
use pierre_config::environment::get_oauth_config;
use pierre_core::errors::AppError;
use pierre_core::models::{refresh_due, TenantId, UserOAuthToken};
use pierre_providers::ai_scope::AiGovernedProvider;
use pierre_providers::backend_resolver;
use pierre_providers::{CoreFitnessProvider, CredentialKind, OAuth2Credentials};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};
use uuid::Uuid;

/// One refresh at a time per connection, shared by its concurrent callers
mod single_flight;
/// The refresh of a stored token: descriptor-driven, single-flight, compare-and-swap
mod token_refresh;

/// OAuth token data structure
#[derive(Debug, Clone)]
pub struct TokenData {
    /// OAuth access token
    pub access_token: String,
    /// OAuth refresh token
    pub refresh_token: String,
    /// When the access token expires; `None` for a token issued without an
    /// expiry (an Intervals.icu OAuth token never expires)
    pub expires_at: Option<DateTime<Utc>>,
    /// OAuth scopes as comma-separated string
    pub scopes: String,
    /// Provider name (e.g., "strava", "whoop")
    pub provider: String,
    /// Provider-side user id for API-key providers (e.g. Intervals.icu
    /// `athlete_id`, used as the HTTP Basic username). `None` for OAuth
    /// bearer providers, which carry identity inside the access token.
    pub provider_user_id: Option<String>,
    /// The Strava shared-pool app that issued the token (`None` for the env
    /// app and every other provider): the client a refresh must use.
    pub oauth_app_client_id: Option<String>,
    /// The `id` of the stored row the token was read from. A refreshed pair
    /// is written back only while that row stands: a reconnect that stored a
    /// new token meanwhile wrote a fresh `id`.
    pub row_id: String,
    /// What `access_token` is: an OAuth grant, or a personal API key the
    /// athlete pasted (an intervals.icu link), read from the row's `token_type`.
    pub kind: CredentialKind,
}

/// OAuth error types
///
/// `Clone`, because one refresh serves every caller waiting on it, a failed
/// one included.
#[derive(Debug, Clone, thiserror::Error)]
pub enum OAuthError {
    /// Failed to exchange authorization code for tokens
    #[error("Token exchange failed: {0}")]
    TokenExchangeFailed(String),

    /// Failed to refresh expired access token
    #[error("Token refresh failed: {0}")]
    TokenRefreshFailed(String),

    /// The refresh of the named provider's token failed without the provider
    /// refusing the grant (a rate limit, a 5xx, a transport failure), over a
    /// connection no earlier refusal flagged. Its text reaches the model and is
    /// stored with the conversation, so it names the provider and nothing it
    /// answered; the answer itself is only logged.
    #[error("The {0} token could not be refreshed just now; the connection itself is intact, and a later request retries the refresh")]
    RefreshUnavailable(String),

    /// The named provider's stored token, or its connection's status, could
    /// not be read or written: a failed query, or a row that does not decrypt.
    /// Its text reaches the model and is stored with the conversation, so it
    /// names the provider and nothing the store answered; the store's error is
    /// only logged. It says nothing about the grant.
    #[error("The {0} token could not be loaded just now because of a temporary failure on our side; a later request retries it")]
    TokenStoreUnavailable(String),

    /// Database operation failed
    #[error("Database error: {0}")]
    DatabaseError(String),
}

impl OAuthError {
    /// The text a tool result carries for this error, which the model reads
    /// and the conversation stores: `label` (an authentication-error prefix)
    /// and the error. A transient failure, which says nothing about the grant
    /// because the provider or the store could not be reached just now, is
    /// written for that reader already and is handed over as it is: labelled
    /// an authentication error, it reads as a reason to reconnect.
    #[must_use]
    pub fn tool_error_text(&self, label: &str) -> String {
        if matches!(
            self,
            Self::RefreshUnavailable(_) | Self::TokenStoreUnavailable(_)
        ) {
            self.to_string()
        } else {
            format!("{label}: {self}")
        }
    }
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
    ///
    /// Returns `None` when there is no token, or none a refresh can renew: no
    /// refresh token is stored, the provider's descriptor declares no refresh
    /// grant, or the provider refused the refresh (the connection is then
    /// `needs_reauth`), now or on an earlier refresh whose flag still stands.
    /// Reconnecting is the remedy for each.
    ///
    /// # Errors
    /// Returns `OAuthError::TokenStoreUnavailable` when the token cannot be
    /// read (a failed query, or a row that does not decrypt), and
    /// `OAuthError::RefreshUnavailable` when a refresh failed without the
    /// provider refusing it (a rate limit, a 5xx, a transport failure) over a
    /// connection no earlier refusal flagged. Neither says the grant is dead,
    /// and a later call can succeed.
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

        // Look up token from database with tenant context
        let Some(tenant_id_str) = tenant_id else {
            debug!("No tenant_id provided, returning Ok(None)");
            return Ok(None);
        };

        // Direct database lookup with tenant_id
        let tenant_id_parsed = TenantId::parse_str(tenant_id_str).map_err(|_| {
            OAuthError::DatabaseError(format!("Invalid tenant_id format: {tenant_id_str}"))
        })?;
        let token_result = self
            .resources
            .repos()
            .oauth_tokens
            .get_token(user_id, tenant_id_parsed, provider)
            .await;

        Self::log_token_lookup_result(&token_result, user_id, tenant_id_str, provider);

        // A read that failed is not a missing token: reported as one, every
        // caller that tags a missing token as auth-required would flag a live
        // connection and free its seat over a database hiccup, or over a key
        // misconfiguration that leaves every row undecryptable at once. The
        // store's error was logged above; the caller's names only the provider.
        let Some(oauth_token) =
            token_result.map_err(|_| OAuthError::TokenStoreUnavailable(provider.to_owned()))?
        else {
            return Ok(None);
        };

        // Process the token - validate expiration and refresh if needed
        self.process_oauth_token(user_id, tenant_id_str, provider, oauth_token)
            .await
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
                    .refresh_stored_token(user_id, tenant_id, provider, &oauth_token)
                    .await;
            }
        }

        // Token is valid, return it
        Ok(Some(Self::token_data(provider, oauth_token)))
    }

    /// A stored token as the caller uses it.
    fn token_data(provider: &str, oauth_token: UserOAuthToken) -> TokenData {
        TokenData {
            provider: provider.to_owned(),
            access_token: oauth_token.access_token,
            refresh_token: oauth_token.refresh_token.unwrap_or_default(),
            expires_at: oauth_token.expires_at,
            scopes: oauth_token.scope.unwrap_or_default(),
            provider_user_id: oauth_token.provider_user_id,
            oauth_app_client_id: oauth_token.oauth_app_client_id,
            row_id: oauth_token.id,
            kind: CredentialKind::from_token_type(&oauth_token.token_type),
        }
    }

    /// Check if token is expired or due for refresh ([`refresh_due`])
    fn is_token_expired(expires_at: DateTime<Utc>) -> bool {
        refresh_due(expires_at)
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

    /// Log a store failure on `provider`'s token path, where `what` names the
    /// operation, and return the error the caller gets for it, which names
    /// only the provider: the store's text reaches no model.
    fn store_failed(user_id: Uuid, provider: &str, what: &str, error: &AppError) -> OAuthError {
        warn!(
            user_id = %user_id,
            provider = %provider,
            error = %error,
            "Could not {what} on the OAuth token path"
        );
        OAuthError::TokenStoreUnavailable(provider.to_owned())
    }

    /// Create authenticated provider with proper tenant-aware credentials
    /// Returns configured provider ready for API calls
    ///
    /// The `requested_provider` is the name the caller asked for (typically a
    /// user-facing name like "strava" or "garmin"). When the user has a
    /// sciotte* row in the database the resolver swaps that for the mirror
    /// backend — callers never need to know about that distinction.
    ///
    /// Every provider it returns is an [`AiGovernedProvider`]: inside a read
    /// for a model its reads are filtered by each provider's AI policy, and
    /// everywhere else they pass through (carnet#723). This is the one place
    /// a tool's live reads are governed.
    ///
    /// # Errors
    /// Returns a boxed `UniversalResponse` error if provider is unsupported or authentication fails
    pub async fn create_authenticated_provider(
        &self,
        requested_provider: &str,
        user_id: Uuid,
        tenant_id: Option<&str>,
    ) -> Result<Box<dyn CoreFitnessProvider>, Box<UniversalResponse>> {
        let registry = Arc::clone(self.resources.provider_registry());
        self.authenticate_provider(requested_provider, user_id, tenant_id)
            .await
            .map(|provider| AiGovernedProvider::wrap(provider, registry))
    }

    async fn authenticate_provider(
        &self,
        requested_provider: &str,
        user_id: Uuid,
        tenant_id: Option<&str>,
    ) -> Result<Box<dyn CoreFitnessProvider>, Box<UniversalResponse>> {
        // Resolve the user-facing provider to the backend that actually
        // serves the request (OAuth or sciotte mirror). A sciotte row in
        // the DB wins over OAuth, even when its session is stale — the
        // user's stated preference is to keep using the mirror.
        let tenant_id_parsed = tenant_id.and_then(|t| TenantId::parse_str(t).ok());
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
            return Err(Box::new(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Unsupported provider: {requested_provider}")),
                metadata: None,
            }));
        }

        // A TrainingPeaks read whose subject is decided before the user's own
        // token is (a coach account's own calendar is refused in words).
        if let Some(served) = self
            .trainingpeaks_subject(provider_name, user_id, tenant_id_parsed)
            .await
        {
            return served;
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
                // The error STRING reports the user-facing provider so the
                // LLM never mentions "sciotte" in text it forwards to the
                // user.
                let user_facing = backend_resolver::user_facing_name(provider_name);
                // No usable token and nothing left to refresh: whatever the
                // connection row's status says, (re)connecting is the only
                // action that can fix this. The tag is unconditional because
                // gating it on a `needs_reauth` status missed the drift case
                // where the row still reads Active but the token/session is
                // gone — sciotte sessions never refresh, so no refresh
                // failure ever flips their status (live incident 2026-08-11:
                // the athlete's dead scrape session produced a generic error
                // and never the reconnect link). The tag makes the chat tool
                // loop short-circuit so auth_recovery renders a localized
                // reconnect message + minted login link instead of letting
                // the LLM rephrase a generic error.
                //
                // The tag carries the BACKEND slug, never the user-facing
                // name: auth_recovery branches on `sciotte`/`sciotte_garmin`
                // to pick the hosted-login mint over the OAuth mint, and the
                // metadata never reaches the LLM (`FunctionResponse` drops
                // it), so there is nothing to sanitise. Tagging "strava" for
                // a dead sciotte session sent the athlete to a Strava OAuth
                // flow their tenant may not even have credentials for.
                let mut map = HashMap::new();
                map.insert(
                    META_AUTH_REQUIRED_PROVIDER.to_owned(),
                    JsonValue::String(provider_name.to_owned()),
                );
                Err(Box::new(UniversalResponse {
                    success: false,
                    result: None,
                    error: Some(format!(
                        "No valid {user_facing} token found. Please reconnect your {user_facing} account."
                    )),
                    metadata: Some(map),
                }))
            }
            // A transient failure is no authentication error: nothing says the
            // grant is dead, and the text says so rather than prompting a
            // reconnect.
            Err(e) => Err(Box::new(UniversalResponse {
                success: false,
                result: None,
                error: Some(e.tool_error_text("Authentication error")),
                metadata: None,
            })),
        }
    }

    /// Create provider with token and tenant-aware credentials
    async fn create_provider_with_token(
        &self,
        provider_name: &str,
        token_data: TokenData,
        user_id: Uuid,
        tenant_id: Option<&str>,
    ) -> Result<Box<dyn CoreFitnessProvider>, Box<UniversalResponse>> {
        // Get tenant-aware OAuth credentials or fall back to environment.
        // Non-OAuth providers (sciotte, synthetic) skip credential lookup
        // entirely, and so does a pasted API key on a provider that also
        // links by OAuth (intervals.icu): no client issued it.
        let needs_client = token_data.kind == CredentialKind::OAuthBearer
            && self
                .resources
                .provider_registry()
                .requires_oauth(provider_name);

        let (client_id, client_secret) = if needs_client {
            // The provider refreshes on its own when a call is refused, so it
            // gets the client that issued the token, as the expiry refresh does.
            self.issuing_client_credentials(
                user_id,
                tenant_id,
                provider_name,
                token_data.oauth_app_client_id.as_deref(),
            )
            .await
            .map_err(|error| {
                Box::new(UniversalResponse {
                    success: false,
                    result: None,
                    error: Some(error),
                    metadata: None,
                })
            })?
        } else {
            // An API key (intervals.icu) carries its provider-side user id
            // here so it reaches the provider as `client_id`, the athlete the
            // API path addresses. Synthetic providers have no id → empty string.
            (
                token_data.provider_user_id.clone().unwrap_or_default(),
                String::new(),
            )
        };

        let row_id = token_data.row_id.clone();

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
                    expires_at: token_data.expires_at,
                    scopes,
                    kind: token_data.kind,
                };

                // Set credentials asynchronously
                match provider.set_credentials(credentials).await {
                    Ok(()) => {
                        // Wire up callback so provider-level token refresh persists to DB,
                        // over the row these credentials were read from
                        let resources = Arc::clone(&self.resources);
                        let provider_name_owned = provider_name.to_owned();
                        let tenant_id_owned = tenant_id.map(str::to_owned);
                        provider.set_token_refresh_callback(Arc::new(
                            move |creds: OAuth2Credentials| {
                                let resources = Arc::clone(&resources);
                                let provider_name = provider_name_owned.clone();
                                let tenant_id = tenant_id_owned.clone();
                                let row_id = row_id.clone();
                                Box::pin(async move {
                                    persist_refreshed_token(
                                        &*resources,
                                        user_id,
                                        tenant_id.as_deref(),
                                        &provider_name,
                                        &row_id,
                                        &creds,
                                    )
                                    .await;
                                })
                            },
                        ));
                        Ok(provider)
                    }
                    Err(e) => Err(Box::new(UniversalResponse {
                        success: false,
                        result: None,
                        error: Some(format!("Failed to set provider credentials: {e}")),
                        metadata: None,
                    })),
                }
            }
            Err(e) => Err(Box::new(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to create provider: {e}")),
                metadata: None,
            })),
        }
    }

    /// The client credentials a stored token refreshes under: the client
    /// that issued it, as [`issuing_client`] resolves it from the Strava pool
    /// app the token names, the user's own app, the tenant's credentials and
    /// the environment.
    async fn issuing_client_credentials(
        &self,
        user_id: Uuid,
        tenant_id: Option<&str>,
        provider: &str,
        issuing_app: Option<&str>,
    ) -> Result<(String, String), String> {
        let tenant_id = tenant_id
            .filter(|t| !t.is_empty())
            .map(|t| TenantId::parse_str(t).map_err(|_| format!("Invalid tenant_id: {t}")))
            .transpose()?;
        let repos = self.resources.repos();
        let server_level = get_oauth_config(provider);
        let client = issuing_client(
            IssuingLookup {
                user_id: Some(user_id),
                tenant_id,
                provider,
                issuing_app,
                server_level: &server_level,
            },
            repos.tenants.as_ref(),
            repos.oauth_tokens.as_ref(),
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok((
            client.client_id().to_owned(),
            client.client_secret().to_owned(),
        ))
    }

    /// Refresh the stored token for `provider` regardless of its recorded expiry.
    ///
    /// Reactive path for provider-rejected tokens (e.g. a 401 despite a
    /// DB-valid `expires_at` — revoked grant, rotated secret, clock skew).
    /// Persists the refreshed token and re-arms / flips the connection status
    /// exactly like the expiry-driven path. Returns `Ok(None)` when no token
    /// row exists, no refresh token is stored, the provider's descriptor
    /// declares no refresh grant, or the provider refused the refresh.
    ///
    /// # Errors
    /// Returns `OAuthError` if the tenant id is malformed, the token cannot be
    /// read or stored ([`OAuthError::TokenStoreUnavailable`]), or the refresh
    /// failed without the provider refusing it
    /// ([`OAuthError::RefreshUnavailable`]).
    pub async fn force_refresh_token(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
    ) -> Result<Option<TokenData>, OAuthError> {
        let tenant_id_parsed = TenantId::parse_str(tenant_id).map_err(|_| {
            OAuthError::DatabaseError(format!("Invalid tenant_id format: {tenant_id}"))
        })?;
        let token = self
            .resources
            .repos()
            .oauth_tokens
            .get_token(user_id, tenant_id_parsed, provider)
            .await
            .map_err(|e| Self::store_failed(user_id, provider, "read the token", &e))?;

        let Some(oauth_token) = token else {
            return Ok(None);
        };

        self.refresh_stored_token(user_id, tenant_id, provider, &oauth_token)
            .await
    }
}
