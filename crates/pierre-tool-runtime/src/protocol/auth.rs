// ABOUTME: Authentication service for universal protocol handlers
// ABOUTME: Handles OAuth token management and provider creation with tenant support
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::protocol::refresh_failure::classify_refresh_failure;
use crate::protocol::token_writeback::persist_refreshed_token;
use crate::protocol::types::{UniversalResponse, META_AUTH_REQUIRED_PROVIDER};
use crate::runtime::ToolRuntime;
use chrono::{DateTime, Utc};
use pierre_auth::oauth2_client::client::fitbit::refresh_fitbit_token;
use pierre_auth::oauth2_client::client::strava::refresh_strava_token;
use pierre_auth::oauth2_client::client::whoop::refresh_whoop_token;
use pierre_auth::strava_pool;
use pierre_auth::tenant::TenantContext;
use pierre_config::environment::get_oauth_config;
use pierre_core::constants::oauth_providers;
use pierre_core::errors::AppError;
use pierre_core::http_client::api_client;
use pierre_core::models::{connection_needs_reauth, TenantId, UserOAuthToken};
use pierre_providers::backend_resolver;
use pierre_providers::whoop_provider::owner_id_for_access_token;
use pierre_providers::{CoreFitnessProvider, OAuth2Credentials};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
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
    /// The Strava shared-pool app that issued the token (`None` for the env
    /// app and every other provider): the client a refresh must use.
    pub oauth_app_client_id: Option<String>,
    /// The `id` of the stored row the token was read from. A refreshed pair
    /// is written back only while that row stands: a reconnect that stored a
    /// new token meanwhile wrote a fresh `id`.
    pub row_id: String,
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

/// A provider whose token endpoint this service refreshes against. A token
/// of any other provider has nothing a refresh here can renew.
#[derive(Debug, Clone, Copy)]
enum RefreshEndpoint {
    Strava,
    Fitbit,
    Whoop,
}

impl RefreshEndpoint {
    fn of(provider: &str) -> Option<Self> {
        [Self::Strava, Self::Fitbit, Self::Whoop]
            .into_iter()
            .find(|endpoint| provider.eq_ignore_ascii_case(endpoint.provider()))
    }

    const fn provider(self) -> &'static str {
        match self {
            Self::Strava => oauth_providers::STRAVA,
            Self::Fitbit => oauth_providers::FITBIT,
            Self::Whoop => oauth_providers::WHOOP,
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
    /// refresh token is stored, this service cannot refresh the provider's
    /// tokens, or the provider refused the refresh (the connection is then
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

    /// Initialize tenant-specific OAuth context if available
    async fn initialize_tenant_oauth_context(
        &self,
        user_id: Uuid,
        tenant_id_str: &str,
        provider: &str,
    ) {
        let Ok(tenant_uuid) = TenantId::parse_str(tenant_id_str) else {
            return;
        };

        let Ok(tenant) = self.resources.repos().tenants.get_by_id(tenant_uuid).await else {
            return;
        };

        // Resolving per-tenant OAuth credentials — no membership was looked up
        // here, so this context asserts a tenant and user, not a role.
        let tenant_context = TenantContext::for_tenant_scoped_operation(
            tenant_uuid,
            tenant.name.clone(), // Safe: String ownership needed for tenant context
            user_id,
        );

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
        Ok(Some(Self::token_data(provider, oauth_token)))
    }

    /// A stored token as the caller uses it.
    fn token_data(provider: &str, oauth_token: UserOAuthToken) -> TokenData {
        TokenData {
            provider: provider.to_owned(),
            access_token: oauth_token.access_token,
            refresh_token: oauth_token.refresh_token.unwrap_or_default(),
            expires_at: oauth_token.expires_at.unwrap_or_else(chrono::Utc::now),
            scopes: oauth_token.scope.unwrap_or_default(),
            provider_user_id: oauth_token.provider_user_id,
            oauth_app_client_id: oauth_token.oauth_app_client_id,
            row_id: oauth_token.id,
        }
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

    /// Handle expired token by attempting refresh
    async fn handle_expired_token(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
        oauth_token: &UserOAuthToken,
    ) -> Result<Option<TokenData>, OAuthError> {
        let Some((endpoint, refresh_token)) = Self::refresh_inputs(provider, oauth_token) else {
            return Ok(None);
        };

        info!(
            "Token expired for user {} provider {}, attempting refresh",
            user_id, provider
        );

        // Attempt to refresh the token, under the app that issued it
        match self
            .refresh_provider_token(user_id, tenant_id, endpoint, oauth_token, refresh_token)
            .await
        {
            Ok(None) => {
                self.superseding_token(user_id, tenant_id, provider, oauth_token)
                    .await
            }
            Ok(Some(mut refreshed_token)) => {
                info!(
                    "Token refreshed successfully for user {} provider {}",
                    user_id, provider
                );
                // A successful refresh proves the token works — re-arm a connection that a
                // prior transient/raced failure may have flipped to needs_reauth. No-op when
                // already active.
                self.mark_connection_active(user_id, tenant_id, provider)
                    .await;
                refreshed_token.provider_user_id = self
                    .owner_id_after_refresh(oauth_token, &refreshed_token)
                    .await;
                Ok(Some(refreshed_token))
            }
            Err(e) => {
                self.refresh_failed(user_id, tenant_id, provider, oauth_token, e)
                    .await
            }
        }
    }

    /// The endpoint and refresh token an expired `token` of `provider`
    /// refreshes with, or `None` when a refresh here cannot renew it: no
    /// refresh token is stored, or the provider's refresh endpoint is not one
    /// this service calls. Reconnecting is then the remedy.
    fn refresh_inputs<'a>(
        provider: &str,
        token: &'a UserOAuthToken,
    ) -> Option<(RefreshEndpoint, &'a str)> {
        let refresh_token = token.refresh_token.as_deref().filter(|t| !t.is_empty())?;
        Some((RefreshEndpoint::of(provider)?, refresh_token))
    }

    /// The token that replaced the row `read` a refresh started from while the
    /// refresh was in flight: a reconnect's (a fresh `id`), or the pair another
    /// refresh of the same row wrote (a new `updated_at`). It is returned while
    /// it is unexpired; whatever this refresh brought back is dropped. `None`
    /// while the row stands as it was read.
    async fn superseding_token(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
        read: &UserOAuthToken,
    ) -> Result<Option<TokenData>, OAuthError> {
        let tenant = TenantId::parse_str(tenant_id).map_err(|_| {
            OAuthError::DatabaseError(format!("Invalid tenant_id format: {tenant_id}"))
        })?;
        let current = self
            .resources
            .repos()
            .oauth_tokens
            .get_token(user_id, tenant, provider)
            .await
            .map_err(|e| Self::store_failed(user_id, provider, "re-read the token", &e))?;
        let replacement = current.filter(|token| {
            (token.id != read.id || token.updated_at != read.updated_at)
                && token
                    .expires_at
                    .is_none_or(|expires_at| !Self::is_token_expired(expires_at))
        });
        if replacement.is_some() {
            info!(
                user_id = %user_id,
                provider = %provider,
                "The token was replaced while its refresh was in flight; the replacement stands"
            );
        }
        Ok(replacement.map(|token| Self::token_data(provider, token)))
    }

    /// The provider-side owner id a refreshed token carries.
    ///
    /// The bearer providers' refresh endpoints return no owner id and the row
    /// update leaves the stored one untouched, so a refreshed token reports the
    /// id the row already holds. A WHOOP row that never captured one (its token
    /// response carries none, and connections made before the OAuth flow read
    /// the profile have `None`) is filled here: the profile is read with the
    /// fresh access token and the id is written back to the row, so the next
    /// webhook naming this athlete routes to them. Best-effort — a failed read
    /// leaves the row as it was and the next refresh tries again.
    async fn owner_id_after_refresh(
        &self,
        stored: &UserOAuthToken,
        refreshed: &TokenData,
    ) -> Option<String> {
        if stored.provider_user_id.is_some() || stored.provider != oauth_providers::WHOOP {
            return stored.provider_user_id.clone();
        }
        match owner_id_for_access_token(self.resources.provider_registry(), &refreshed.access_token)
            .await
        {
            Ok(owner_id) => {
                self.persist_owner_id(stored, refreshed, &owner_id).await;
                Some(owner_id)
            }
            Err(e) => {
                warn!(
                    user_id = %stored.user_id,
                    provider = %stored.provider,
                    error = %e,
                    "provider user id lookup failed after refresh; push events for this connection route only once a later refresh captures it"
                );
                None
            }
        }
    }

    /// Write a captured owner id onto the token row the refresh just updated.
    ///
    /// The row is re-written whole — the refreshed tokens plus the id — so
    /// nothing the refresh stored is lost, and only while it is still the row
    /// the refresh read: a reconnect that replaced it since stands. A write
    /// failure is logged: the in-memory token still carries the id for this
    /// request, and the next refresh captures it again.
    async fn persist_owner_id(
        &self,
        stored: &UserOAuthToken,
        refreshed: &TokenData,
        owner_id: &str,
    ) {
        let row = UserOAuthToken {
            access_token: refreshed.access_token.clone(),
            refresh_token: Some(refreshed.refresh_token.clone()).filter(|t| !t.is_empty()),
            expires_at: Some(refreshed.expires_at),
            provider_user_id: Some(owner_id.to_owned()),
            updated_at: Utc::now(),
            ..stored.clone()
        };
        match self
            .resources
            .repos()
            .oauth_tokens
            .replace_token_if_current(&row, Some(&stored.id))
            .await
        {
            Ok(true) => info!(
                user_id = %stored.user_id,
                provider = %stored.provider,
                "captured provider user id on token refresh"
            ),
            Ok(false) => info!(
                user_id = %stored.user_id,
                provider = %stored.provider,
                "the token was replaced since its refresh; the captured provider user id is not written over the replacement"
            ),
            Err(e) => warn!(
                user_id = %stored.user_id,
                provider = %stored.provider,
                error = %e,
                "failed to persist the provider user id captured on refresh"
            ),
        }
    }

    /// What a failed refresh of the stored row `stored` leaves the caller.
    ///
    /// Only the provider's answer is judged: a database failure storing the
    /// pair is ours, and is returned as it is. A failure the provider did not
    /// word as a refusal of the grant or of the client (a rate limit, a 5xx, a
    /// transport failure) says nothing about the grant, so the connection is
    /// left as it is and the grant's standing is the connection's (see
    /// [`Self::unrefused_refresh_failed`]). The provider's answer is logged
    /// here and nowhere else, since the error's text reaches the model and the
    /// stored conversation. A refusal — a dead or rotated refresh token, a
    /// revoked grant, a rejected client — means the user must reconnect: the
    /// connection flips to `needs_reauth` while `stored` is still its token as
    /// read, and a token a reconnect or another refresh stored meanwhile is
    /// returned instead of `None`.
    async fn refresh_failed(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
        stored: &UserOAuthToken,
        error: OAuthError,
    ) -> Result<Option<TokenData>, OAuthError> {
        warn!(
            "Token refresh failed for user {} provider {}: {}",
            user_id, provider, error
        );
        let OAuthError::TokenRefreshFailed(answer) = &error else {
            return Err(error);
        };
        let Some(error_code) = classify_refresh_failure(answer) else {
            return self
                .unrefused_refresh_failed(user_id, tenant_id, provider)
                .await;
        };
        self.mark_connection_needs_reauth(user_id, tenant_id, provider, stored, error_code)
            .await;
        self.superseding_token(user_id, tenant_id, provider, stored)
            .await
    }

    /// What a refresh the provider failed without refusing leaves the caller.
    ///
    /// The failure revives nothing: a connection an earlier refusal left
    /// `needs_reauth` (or that was revoked) still holds a grant only a
    /// reconnect restores, and the athlete was already told so, so the caller
    /// gets `None`, the reconnect every other surface shows. Any other
    /// connection's grant stands, and the caller gets
    /// [`OAuthError::RefreshUnavailable`]: read as "no token", it would send
    /// every caller that tags a missing token as auth-required (the capture
    /// sweep, the backfill) to flag a live connection and free its seat. A
    /// status that cannot be read confirms neither, and is the store's
    /// failure.
    async fn unrefused_refresh_failed(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
    ) -> Result<Option<TokenData>, OAuthError> {
        let tenant = TenantId::parse_str(tenant_id).map_err(|_| {
            OAuthError::DatabaseError(format!("Invalid tenant_id format: {tenant_id}"))
        })?;
        let connections = self
            .resources
            .repos()
            .provider_connections
            .get_for_user(user_id, Some(tenant))
            .await
            .map_err(|e| Self::store_failed(user_id, provider, "read the connection status", &e))?;
        if connection_needs_reauth(&connections, provider) {
            info!(
                user_id = %user_id,
                provider = %provider,
                "The refresh failed transiently over a connection an earlier refusal flagged; it still needs reconnecting"
            );
            return Ok(None);
        }
        Err(OAuthError::RefreshUnavailable(provider.to_owned()))
    }

    /// Persist a refused refresh of the token row `stored` as `needs_reauth`
    /// on the provider connection (best-effort), and nudge the user once when
    /// it flipped.
    ///
    /// The connection row stays in the DB so the user/tenant mapping and history
    /// survive; only its `status` flips, and only while `stored` is still its
    /// token as the refresh read it: a reconnect that replaced it meanwhile
    /// re-armed the connection, and a concurrent refresh that landed proved the
    /// grant alive, so the refusal of what they replaced leaves either alone.
    /// A write failure here must not abort the user's request — log and continue.
    async fn mark_connection_needs_reauth(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
        stored: &UserOAuthToken,
        error_code: &str,
    ) {
        let Ok(tenant) = TenantId::parse_str(tenant_id) else {
            warn!("Cannot mark {provider} needs_reauth for user {user_id}: invalid tenant_id");
            return;
        };
        if self.flip_to_needs_reauth(stored, error_code).await {
            self.notify_provider_disconnected(user_id, tenant, provider)
                .await;
        }
    }

    /// Write the `needs_reauth` flip guarded on the token row `stored` as it
    /// was read, and report whether the connection flipped. Every outcome is
    /// logged; a failed write reads as not flipped, so it nudges nobody.
    async fn flip_to_needs_reauth(&self, stored: &UserOAuthToken, error_code: &str) -> bool {
        let (user_id, provider) = (stored.user_id, &stored.provider);
        match self
            .resources
            .repos()
            .provider_connections
            .mark_needs_reauth_if_token_current(stored, error_code)
            .await
        {
            Ok(true) => {
                info!(
                    "Provider {provider} flipped to needs_reauth for user {user_id} ({error_code})"
                );
                true
            }
            Ok(false) => {
                info!(
                    "Provider {provider} left as it was for user {user_id} ({error_code}): already needs_reauth, or its token changed since the refresh read it"
                );
                false
            }
            Err(e) => {
                warn!("Failed to persist needs_reauth for user {user_id} provider {provider}: {e}");
                false
            }
        }
    }

    /// Re-arm a provider connection to `active` after a successful refresh (best-effort).
    ///
    /// No-op when the connection is already active or the row does not exist. A write
    /// failure must not abort the user's request — log and continue.
    async fn mark_connection_active(&self, user_id: Uuid, tenant_id: &str, provider: &str) {
        let Ok(tenant) = TenantId::parse_str(tenant_id) else {
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

    /// Create authenticated provider with proper tenant-aware credentials
    /// Returns configured provider ready for API calls
    ///
    /// The `requested_provider` is the name the caller asked for (typically a
    /// user-facing name like "strava" or "garmin"). When the user has a
    /// sciotte* row in the database the resolver swaps that for the mirror
    /// backend — callers never need to know about that distinction.
    ///
    /// # Errors
    /// Returns a boxed `UniversalResponse` error if provider is unsupported or authentication fails
    pub async fn create_authenticated_provider(
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
        // Non-OAuth providers (sciotte, synthetic) skip credential lookup entirely.
        let requires_oauth = self
            .resources
            .provider_registry()
            .requires_oauth(provider_name);

        let (client_id, client_secret) = if requires_oauth {
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
            // API-key providers (e.g. Intervals.icu) carry their provider-side
            // user id here so it reaches the provider as `client_id` (the HTTP
            // Basic username). Synthetic providers have no id → empty string.
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
                    expires_at: Some(token_data.expires_at),
                    scopes,
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
    /// Calls the provider's token refresh endpoint and stores the new token
    /// over `stored`, the row it was read from.
    ///
    /// A refresh token only refreshes under the client that issued it. A Strava
    /// token a shared-pool app issued names that app, and is refreshed under
    /// its credentials whatever the user or tenant has configured since; any
    /// other token resolves user-specific → tenant-level → env var defaults,
    /// the chain that issued it.
    ///
    /// Returns `None` when the refreshed pair was not stored because `stored`
    /// is no longer the row in place: a reconnect replaced it while the
    /// refresh was in flight, and its token stands.
    ///
    /// # Errors
    /// Returns `OAuthError` if token refresh or database operations fail
    async fn refresh_provider_token(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        endpoint: RefreshEndpoint,
        stored: &UserOAuthToken,
        refresh_token: &str,
    ) -> Result<Option<TokenData>, OAuthError> {
        let provider = endpoint.provider();
        let issuing_app = stored.oauth_app_client_id.as_deref();
        let (client_id, client_secret) = self
            .issuing_client_credentials(user_id, Some(tenant_id), provider, issuing_app)
            .await
            .map_err(OAuthError::TokenRefreshFailed)?;

        // Call provider-specific token refresh
        let http_client = api_client();
        let new_token = match endpoint {
            RefreshEndpoint::Strava => {
                refresh_strava_token(http_client, &client_id, &client_secret, refresh_token).await
            }
            RefreshEndpoint::Fitbit => {
                refresh_fitbit_token(http_client, &client_id, &client_secret, refresh_token).await
            }
            RefreshEndpoint::Whoop => {
                refresh_whoop_token(http_client, &client_id, &client_secret, refresh_token).await
            }
        }
        .map_err(|e| OAuthError::TokenRefreshFailed(e.to_string()))?;

        // Prepare token data for database update
        let new_access_token = new_token.access_token.clone();
        let new_refresh_token = new_token.refresh_token.clone();
        let new_expires_at = new_token.expires_at;

        // Update the token in the database, over the row this refresh read
        let landed = self
            .resources
            .repos()
            .oauth_tokens
            .refresh_token(
                stored,
                &new_access_token,
                new_refresh_token.as_deref(),
                new_expires_at,
            )
            .await
            .map_err(|e| Self::store_failed(user_id, provider, "store the refreshed token", &e))?;
        if !landed {
            return Ok(None);
        }

        // Return the refreshed token data. The refresh endpoints of the bearer
        // providers (strava/fitbit/whoop) return no owner id; the caller fills
        // it from the stored row (`owner_id_after_refresh`).
        Ok(Some(TokenData {
            provider: provider.to_owned(),
            access_token: new_access_token,
            refresh_token: new_refresh_token.unwrap_or_default(),
            expires_at: new_expires_at.unwrap_or_else(chrono::Utc::now),
            scopes: new_token.scope.unwrap_or_default(),
            provider_user_id: None,
            oauth_app_client_id: stored.oauth_app_client_id.clone(),
            row_id: stored.id.clone(),
        }))
    }

    /// The client credentials a stored token refreshes under: the client
    /// that issued it.
    ///
    /// A Strava token a shared-pool app issued names that app, and resolves
    /// to its credentials whatever the user or tenant has configured since.
    /// Any other token resolves user-specific → tenant-level → env var
    /// defaults, the chain that issued it; with no tenant, the env defaults.
    async fn issuing_client_credentials(
        &self,
        user_id: Uuid,
        tenant_id: Option<&str>,
        provider: &str,
        issuing_app: Option<&str>,
    ) -> Result<(String, String), String> {
        let repos = self.resources.repos();
        if let Some(app) =
            issuing_app.filter(|_| provider.eq_ignore_ascii_case(oauth_providers::STRAVA))
        {
            return strava_pool::resolve_strava_credentials(repos.oauth_tokens.as_ref(), Some(app))
                .await
                .map_err(|e| e.to_string());
        }
        let Some(tenant_id) = tenant_id.filter(|t| !t.is_empty()) else {
            return Self::get_default_oauth_credentials(provider)
                .map_err(|e| e.error.unwrap_or_default());
        };
        if let Ok(Some(user_app)) = repos
            .oauth_tokens
            .get_user_oauth_app(user_id, provider)
            .await
        {
            info!("Using user-specific {provider} credentials for user {user_id}");
            return Ok((user_app.client_id, user_app.client_secret));
        }
        let tid = TenantId::parse_str(tenant_id)
            .map_err(|_| format!("Invalid tenant_id: {tenant_id}"))?;
        match repos
            .tenants
            .get_oauth_credentials(tid, provider)
            .await
            .map_err(|e| format!("Failed to get OAuth credentials: {e}"))?
        {
            Some(c) => Ok((c.client_id, c.client_secret)),
            None => Self::get_default_oauth_credentials(provider)
                .map_err(|e| e.error.unwrap_or_default()),
        }
    }

    /// Refresh the stored token for `provider` regardless of its recorded expiry.
    ///
    /// Reactive path for provider-rejected tokens (e.g. a 401 despite a
    /// DB-valid `expires_at` — revoked grant, rotated secret, clock skew).
    /// Persists the refreshed token and re-arms / flips the connection status
    /// exactly like the expiry-driven path. Returns `Ok(None)` when no token
    /// row exists, no refresh token is stored, this service cannot refresh the
    /// provider's tokens, or the provider refused the refresh.
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

        self.handle_expired_token(user_id, tenant_id, provider, &oauth_token)
            .await
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
}
