// ABOUTME: Repository trait definitions for the OAuth tokens, OAuth2 server, client state, provider connections domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;

use pierre_core::models::TenantId;
use pierre_core::models::{
    AuthorizationCode, ConnectionType, DeviceAuthorization, OAuth2AuthCode, OAuth2Client,
    OAuth2RefreshToken, OAuth2State, OAuthClientGrant, OAuthClientState, ProviderConnection,
    StravaPoolApp, UserOAuthApp, UserOAuthToken,
};
use uuid::Uuid;

/// OAuth token storage repository (tenant-scoped, includes OAuth apps and sync tracking)
#[async_trait]
pub trait OAuthTokenRepository: Send + Sync {
    /// Store or update user OAuth token for a tenant-provider combination
    async fn upsert_token(&self, token: &UserOAuthToken) -> AppResult<()>;
    /// Get user OAuth token for a specific tenant-provider combination
    async fn get_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<UserOAuthToken>>;
    /// Get all OAuth tokens for a user, optionally scoped to a specific tenant
    async fn get_tokens(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<UserOAuthToken>>;
    /// Get all OAuth tokens for a tenant-provider combination
    async fn get_tenant_provider_tokens(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Vec<UserOAuthToken>>;
    /// Resolve the single user who owns a provider-side account id.
    ///
    /// Maps a provider's own user identifier (e.g. a Strava athlete id delivered
    /// in a webhook `owner_id`) back to the `(user_id, tenant_id)` of the matching
    /// stored token. Returns `None` when no token carries that `provider_user_id`,
    /// which callers must treat as "unknown owner" — never as a signal to fan out
    /// to every connected user.
    async fn find_user_by_provider_user_id(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> AppResult<Option<(Uuid, String)>>;
    /// Count distinct users occupying a shared-app OAuth seat for `provider`.
    ///
    /// A "seat" is one athlete connected through the platform's shared OAuth
    /// application. Users who registered their own (BYO) OAuth app run on their
    /// own athlete quota and are excluded. The count is intentionally
    /// cross-tenant: the shared app's athlete cap is a single global limit
    /// enforced upstream (Strava) across every tenant that uses it.
    async fn count_shared_app_seat_usage(&self, provider: &str) -> AppResult<u32>;

    /// List Strava shared-app pool apps — the extra DB-configured apps beside
    /// the env `STRAVA_CLIENT_ID` app. Secrets are never included. When
    /// `only_enabled` is true, disabled apps are omitted (the connect-selection
    /// path); otherwise all rows are returned (admin listing).
    async fn list_strava_pool_apps(&self, only_enabled: bool) -> AppResult<Vec<StravaPoolApp>>;

    /// Decrypt and return a pool app's `client_secret`, or `None` when the
    /// `client_id` is not in the pool. Used at token exchange and refresh to use
    /// the same app that minted the token.
    async fn get_strava_pool_app_secret(&self, client_id: &str) -> AppResult<Option<String>>;

    /// Distinct-user seat usage grouped by the issuing Strava app, excluding
    /// BYO-app users. Each entry is `(oauth_app_client_id, distinct_user_count)`;
    /// the `None` key is the env-default app (NULL attribution + legacy tokens).
    async fn count_strava_seat_usage_by_app(&self) -> AppResult<Vec<(Option<String>, u32)>>;

    /// Insert or update a pool app, encrypting `client_secret` at rest with the
    /// same AES-256-GCM envelope used for user tokens.
    async fn upsert_strava_pool_app(
        &self,
        client_id: &str,
        client_secret: &str,
        seat_cap: u32,
        label: Option<&str>,
    ) -> AppResult<()>;

    /// Enable or disable a pool app; disabled apps are skipped for new connects.
    async fn set_strava_pool_app_enabled(&self, client_id: &str, enabled: bool) -> AppResult<()>;

    /// Remove a pool app. Tokens it already issued keep their attribution and
    /// their refresh will fail once the secret is gone — only delete an app
    /// whose athletes have migrated or disconnected.
    async fn delete_strava_pool_app(&self, client_id: &str) -> AppResult<()>;

    /// Delete user OAuth token for a tenant-provider combination
    async fn delete_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;
    /// Delete all OAuth tokens for a user within a tenant scope
    async fn delete_tokens(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()>;
    /// Update OAuth token expiration and refresh info
    async fn refresh_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> AppResult<()>;
    /// Store user OAuth app credentials (`client_id`, `client_secret`)
    async fn store_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> AppResult<()>;
    /// Get user OAuth app credentials for a provider
    async fn get_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> AppResult<Option<UserOAuthApp>>;
    /// List all OAuth app providers configured for a user
    async fn list_user_oauth_apps(&self, user_id: Uuid) -> AppResult<Vec<UserOAuthApp>>;
    /// Remove user OAuth app credentials for a provider
    async fn remove_user_oauth_app(&self, user_id: Uuid, provider: &str) -> AppResult<()>;
    /// Get last sync timestamp for a provider within a specific tenant
    async fn get_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<DateTime<Utc>>>;
    /// Update last sync timestamp for a provider within a specific tenant
    async fn update_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        sync_time: DateTime<Utc>,
    ) -> AppResult<()>;
}

/// OAuth 2.0 server repository (RFC 7591)
#[async_trait]
pub trait OAuth2ServerRepository: Send + Sync {
    /// Store OAuth 2.0 client registration
    async fn store_client(&self, client: &OAuth2Client) -> AppResult<()>;
    /// Get OAuth 2.0 client by `client_id`
    async fn get_client(&self, client_id: &str) -> AppResult<Option<OAuth2Client>>;
    /// Store OAuth 2.0 authorization code
    async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()>;
    /// Get OAuth 2.0 authorization code
    async fn get_auth_code(&self, code: &str) -> AppResult<Option<OAuth2AuthCode>>;
    /// Update OAuth 2.0 authorization code (mark as used)
    async fn update_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()>;
    /// Store OAuth 2.0 refresh token
    async fn store_refresh_token(&self, refresh_token: &OAuth2RefreshToken) -> AppResult<()>;
    /// Get OAuth 2.0 refresh token
    async fn get_refresh_token(&self, token: &str) -> AppResult<Option<OAuth2RefreshToken>>;
    /// Revoke OAuth 2.0 refresh token
    async fn revoke_refresh_token(&self, token: &str) -> AppResult<()>;
    /// Atomically consume OAuth 2.0 authorization code (check-and-set in single operation)
    async fn consume_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2AuthCode>>;
    /// Atomically consume OAuth 2.0 refresh token (check-and-revoke in single operation)
    async fn consume_refresh_token(
        &self,
        token: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2RefreshToken>>;
    /// Look up a refresh token by its value (without `client_id` constraint)
    async fn get_refresh_token_by_value(
        &self,
        token: &str,
    ) -> AppResult<Option<OAuth2RefreshToken>>;
    /// Store authorization code
    async fn store_authorization_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        scope: &str,
        user_id: Uuid,
    ) -> AppResult<()>;
    /// Get authorization code data
    async fn get_authorization_code(&self, code: &str) -> AppResult<AuthorizationCode>;
    /// Delete authorization code (after use)
    async fn delete_authorization_code(&self, code: &str) -> AppResult<()>;
    /// Store `OAuth2` state for CSRF protection
    async fn store_state(&self, state: &OAuth2State) -> AppResult<()>;
    /// Consume `OAuth2` state (atomically check and mark as used)
    async fn consume_state(
        &self,
        state_value: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2State>>;
    /// Persist a user's consent to an MCP OAuth client.
    ///
    /// Inserts a new active grant with `granted_at = now` and `revoked_at = NULL`.
    /// The caller supplies `grant.id` (a uuid string). If an active grant for the
    /// same `(user_id, tenant_id, client_id, scope)` already exists, the insert is
    /// a no-op (the active partial-unique index makes re-consent idempotent).
    async fn store_client_grant(&self, grant: &OAuthClientGrant) -> AppResult<()>;
    /// Find the active grant for a `(user, tenant, client, scope)` tuple.
    ///
    /// Returns the un-revoked grant matching all four fields, or `None` when no
    /// active grant exists — the authorize path treats `None` as "show the
    /// consent screen".
    async fn find_active_client_grant(
        &self,
        user_id: &str,
        tenant_id: &str,
        client_id: &str,
        scope: &str,
    ) -> AppResult<Option<OAuthClientGrant>>;
    /// List a user's active (un-revoked) client grants within a tenant.
    ///
    /// Ordered by `granted_at` descending (most recent first). Backs the user's
    /// "connected apps" view.
    async fn list_client_grants(
        &self,
        user_id: &str,
        tenant_id: &str,
    ) -> AppResult<Vec<OAuthClientGrant>>;
    /// Revoke a client grant, verifying ownership via `user_id` + `tenant_id`.
    ///
    /// Soft-deletes by setting `revoked_at = now` only when the grant is owned by
    /// the caller and still active. Returns `Ok(true)` when a row changed,
    /// `Ok(false)` when nothing matched (unknown id, wrong owner, or already
    /// revoked).
    async fn revoke_client_grant(
        &self,
        id: &str,
        user_id: &str,
        tenant_id: &str,
    ) -> AppResult<bool>;

    /// Store a new pending device authorization (RFC 8628).
    async fn create_device_authorization(&self, da: &DeviceAuthorization) -> AppResult<()>;
    /// Look up a device authorization by the SHA-256 hash of its `device_code`.
    async fn get_device_authorization_by_code_hash(
        &self,
        device_code_hash: &str,
    ) -> AppResult<Option<DeviceAuthorization>>;
    /// Look up a device authorization by its `user_code` (operator-entered code).
    async fn get_device_authorization_by_user_code(
        &self,
        user_code: &str,
    ) -> AppResult<Option<DeviceAuthorization>>;
    /// Mark a pending device authorization approved by a super-admin.
    ///
    /// Returns `true` when a still-pending row was updated; `false` when nothing
    /// matched (unknown `user_code`, or already approved/denied).
    async fn approve_device_authorization(
        &self,
        user_code: &str,
        approved_by: &str,
    ) -> AppResult<bool>;
    /// Mark a pending device authorization denied. Returns `true` if a pending
    /// row was updated.
    async fn deny_device_authorization(&self, user_code: &str) -> AppResult<bool>;
    /// Delete a device authorization by `device_code` hash (single-use consume).
    ///
    /// Returns `true` if a row was deleted — the token endpoint mints the admin
    /// token only when this returns `true`, so a duplicate poll can never mint
    /// twice.
    async fn delete_device_authorization(&self, device_code_hash: &str) -> AppResult<bool>;
}

/// OAuth client-side state management repository
#[async_trait]
pub trait OAuthClientStateRepository: Send + Sync {
    /// Store OAuth client-side state for CSRF protection and PKCE verifier storage
    async fn store_oauth_client_state(&self, state: &OAuthClientState) -> AppResult<()>;
    /// Consume OAuth client state atomically (verify and mark as used)
    async fn consume_oauth_client_state(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuthClientState>>;
}

/// Provider connection management repository
#[async_trait]
pub trait ProviderConnectionRepository: Send + Sync {
    /// Register a provider connection (upsert)
    async fn register_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        connection_type: &ConnectionType,
        metadata: Option<&str>,
    ) -> AppResult<()>;
    /// Remove a provider connection
    async fn remove_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;
    /// Get all provider connections for a user
    async fn get_for_user(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<ProviderConnection>>;
    /// Check if a specific provider is connected for a user (cross-tenant)
    async fn is_connected(&self, user_id: Uuid, provider: &str) -> AppResult<bool>;
    /// Mark a provider connection as just-used, updating `last_used_at = now()`.
    ///
    /// Called from the read path (chat tool execution, REST activity fetches) so the
    /// resolver can pick the most-recently-active backend when a subsequent tool call
    /// omits the provider argument. Best-effort: a failure to write should not abort
    /// the user's request — log and continue. No-op when the (user, tenant, provider)
    /// row does not exist.
    async fn touch_last_used(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;
    /// Resolve the user's most-recently-used provider connection.
    ///
    /// Returns the connection with the freshest `last_used_at` (NULLs last), falling
    /// back to the freshest `connected_at` when no row has been touched yet. Tenant
    /// scope is honored when `tenant_id` is provided; otherwise the lookup is
    /// cross-tenant. Returns `None` when the user has no provider connections at all.
    async fn resolve_most_recent(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<ProviderConnection>>;
    /// Mark a connection as needing re-authentication after a non-recoverable token
    /// refresh failure.
    ///
    /// Transitions `status` to `needs_reauth` and records the token-free error class in
    /// `last_error`. Guarded so the transition timestamp reflects the first failure, not
    /// every retry. `error_code` is a short OAuth error class (e.g. `invalid_request`) —
    /// NEVER token material. No-op when the row does not exist.
    async fn mark_needs_reauth(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        error_code: Option<&str>,
    ) -> AppResult<()>;
    /// Re-arm a connection after a successful (re)connect or token refresh.
    ///
    /// Transitions `status` back to `active` and clears the disconnect notification
    /// marker so a future disconnect notifies again. No-op when already active or when
    /// the row does not exist.
    async fn mark_active(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;
    /// Atomically claim the one-time disconnect notification for a `needs_reauth`
    /// connection.
    ///
    /// Sets `notified_at` only when the connection is `needs_reauth` and not yet notified,
    /// returning whether this call won the claim. Drives a single out-of-band reconnect
    /// nudge per disconnect; the marker is cleared on reconnect via [`Self::mark_active`].
    async fn claim_reauth_notification(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<bool>;
}
