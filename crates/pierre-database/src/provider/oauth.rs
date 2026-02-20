// ABOUTME: OAuth database operations trait covering tokens, apps, OAuth2 server, client state, password reset
// ABOUTME: Enables 5 OAuth-related repository blanket impls with focused trait bound
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{
    AuthorizationCode, ConnectionType, OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken,
    OAuth2State, OAuthClientState, ProviderConnection, TenantId, UserOAuthApp, UserOAuthToken,
};
use uuid::Uuid;

/// OAuth and authentication database operations
#[async_trait]
pub trait OAuthDbOps: Send + Sync + Clone {
    // --- User OAuth Tokens (Multi-Tenant) ---

    /// Store or update user OAuth token for a tenant-provider combination
    async fn upsert_user_oauth_token(&self, token: &UserOAuthToken) -> AppResult<()>;

    /// Get user OAuth token for a specific tenant-provider combination
    async fn get_user_oauth_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<UserOAuthToken>>;

    /// Get all OAuth tokens for a user, optionally scoped to a specific tenant
    async fn get_user_oauth_tokens(
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

    /// Delete user OAuth token for a tenant-provider combination
    async fn delete_user_oauth_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;

    /// Delete all OAuth tokens for a user within a tenant scope
    async fn delete_user_oauth_tokens(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()>;

    /// Update OAuth token expiration and refresh info
    async fn refresh_user_oauth_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> AppResult<()>;

    // --- User OAuth App Credentials ---

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

    // --- Provider Sync Tracking ---

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

    // --- OAuth 2.0 Server (RFC 7591) ---

    /// Store OAuth 2.0 client registration
    async fn store_oauth2_client(&self, client: &OAuth2Client) -> AppResult<()>;

    /// Get OAuth 2.0 client by `client_id`
    async fn get_oauth2_client(&self, client_id: &str) -> AppResult<Option<OAuth2Client>>;

    /// Store OAuth 2.0 authorization code
    async fn store_oauth2_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()>;

    /// Get OAuth 2.0 authorization code
    async fn get_oauth2_auth_code(&self, code: &str) -> AppResult<Option<OAuth2AuthCode>>;

    /// Update OAuth 2.0 authorization code (mark as used)
    async fn update_oauth2_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()>;

    /// Store OAuth 2.0 refresh token
    async fn store_oauth2_refresh_token(&self, refresh_token: &OAuth2RefreshToken)
        -> AppResult<()>;

    /// Get OAuth 2.0 refresh token
    async fn get_oauth2_refresh_token(&self, token: &str) -> AppResult<Option<OAuth2RefreshToken>>;

    /// Revoke OAuth 2.0 refresh token
    async fn revoke_oauth2_refresh_token(&self, token: &str) -> AppResult<()>;

    /// Atomically consume OAuth 2.0 authorization code (check-and-set in single operation)
    ///
    /// Returns `Some(auth_code)` if the code was valid, unused, and successfully consumed.
    /// Returns `None` if the code is invalid, already used, expired, or validation failed.
    async fn consume_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2AuthCode>>;

    /// Atomically consume OAuth 2.0 refresh token (check-and-revoke in single operation)
    ///
    /// Returns `Some(refresh_token)` if the token was valid and successfully consumed.
    /// Returns `None` if the token is invalid, already revoked, or validation failed.
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
    async fn store_oauth2_state(&self, state: &OAuth2State) -> AppResult<()>;

    /// Consume `OAuth2` state (atomically check and mark as used)
    async fn consume_oauth2_state(
        &self,
        state_value: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2State>>;

    // --- OAuth Client State (CSRF + PKCE) ---

    /// Store OAuth client-side state for CSRF protection and PKCE verifier storage
    async fn store_oauth_client_state(&self, state: &OAuthClientState) -> AppResult<()>;

    /// Consume OAuth client state atomically (verify and mark as used)
    async fn consume_oauth_client_state(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuthClientState>>;

    // --- Password Reset Tokens ---

    /// Store a password reset token (hashed) for a user
    async fn store_password_reset_token(
        &self,
        user_id: Uuid,
        token_hash: &str,
        created_by: &str,
    ) -> AppResult<Uuid>;

    /// Consume a password reset token by its hash
    async fn consume_password_reset_token(&self, token_hash: &str) -> AppResult<Uuid>;

    /// Invalidate all unused reset tokens for a user
    async fn invalidate_user_reset_tokens(&self, user_id: Uuid) -> AppResult<()>;

    // --- Provider Connections ---

    /// Register a provider connection (upsert)
    async fn register_provider_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        connection_type: &ConnectionType,
        metadata: Option<&str>,
    ) -> AppResult<()>;

    /// Remove a provider connection
    async fn remove_provider_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;

    /// Get all provider connections for a user
    async fn get_user_provider_connections(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<ProviderConnection>>;

    /// Check if a specific provider is connected for a user (cross-tenant)
    async fn is_provider_connected(&self, user_id: Uuid, provider: &str) -> AppResult<bool>;
}
