// ABOUTME: OAuth repository dispatch for the database factory
// ABOUTME: Delegates OAuth token, server, client state, provider connection, and password reset calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::database_plugins::{
    OAuth2ServerRepository, OAuthClientStateRepository, OAuthTokenRepository,
    PasswordResetRepository, ProviderConnectionRepository,
};
use crate::errors::AppResult;
use crate::models::{
    AuthorizationCode, ConnectionType, ProviderConnection, UserOAuthApp, UserOAuthToken,
};
use crate::oauth2_client::OAuthClientState;
use crate::oauth2_server::models::{OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::models::TenantId;
use uuid::Uuid;

#[async_trait]
impl OAuthTokenRepository for Database {
    async fn upsert_token(&self, token: &UserOAuthToken) -> AppResult<()> {
        match self {
            Self::SQLite(db) => OAuthTokenRepository::upsert_token(db, token).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.upsert_token(token).await,
        }
    }
    async fn get_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<UserOAuthToken>> {
        match self {
            Self::SQLite(db) => {
                OAuthTokenRepository::get_token(db, user_id, tenant_id, provider).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                OAuthTokenRepository::get_token(db, user_id, tenant_id, provider).await
            }
        }
    }
    async fn get_tokens(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<UserOAuthToken>> {
        match self {
            Self::SQLite(db) => db.get_tokens(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tokens(user_id, tenant_id).await,
        }
    }
    async fn get_tenant_provider_tokens(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Vec<UserOAuthToken>> {
        match self {
            Self::SQLite(db) => db.get_tenant_provider_tokens(tenant_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tenant_provider_tokens(tenant_id, provider).await,
        }
    }
    async fn delete_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.delete_token(user_id, tenant_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_token(user_id, tenant_id, provider).await,
        }
    }
    async fn delete_tokens(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.delete_tokens(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_tokens(user_id, tenant_id).await,
        }
    }
    async fn refresh_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.refresh_token(
                    user_id,
                    tenant_id,
                    provider,
                    access_token,
                    refresh_token,
                    expires_at,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.refresh_token(
                    user_id,
                    tenant_id,
                    provider,
                    access_token,
                    refresh_token,
                    expires_at,
                )
                .await
            }
        }
    }
    async fn store_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.store_user_oauth_app(user_id, provider, client_id, client_secret, redirect_uri)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.store_user_oauth_app(user_id, provider, client_id, client_secret, redirect_uri)
                    .await
            }
        }
    }
    async fn get_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> AppResult<Option<UserOAuthApp>> {
        match self {
            Self::SQLite(db) => db.get_user_oauth_app(user_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_user_oauth_app(user_id, provider).await,
        }
    }
    async fn list_user_oauth_apps(&self, user_id: Uuid) -> AppResult<Vec<UserOAuthApp>> {
        match self {
            Self::SQLite(db) => db.list_user_oauth_apps(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_user_oauth_apps(user_id).await,
        }
    }
    async fn remove_user_oauth_app(&self, user_id: Uuid, provider: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.remove_user_oauth_app(user_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.remove_user_oauth_app(user_id, provider).await,
        }
    }
    async fn get_provider_last_sync(
        &self,
        user_id: uuid::Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<chrono::DateTime<chrono::Utc>>> {
        match self {
            Self::SQLite(db) => {
                db.get_provider_last_sync(user_id, tenant_id, provider)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_provider_last_sync(user_id, tenant_id, provider)
                    .await
            }
        }
    }
    async fn update_provider_last_sync(
        &self,
        user_id: uuid::Uuid,
        tenant_id: TenantId,
        provider: &str,
        sync_time: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.update_provider_last_sync(user_id, tenant_id, provider, sync_time)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.update_provider_last_sync(user_id, tenant_id, provider, sync_time)
                    .await
            }
        }
    }
}

#[async_trait]
impl OAuth2ServerRepository for Database {
    async fn store_client(&self, client: &OAuth2Client) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_client(client).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_client(client).await,
        }
    }
    async fn get_client(&self, client_id: &str) -> AppResult<Option<OAuth2Client>> {
        match self {
            Self::SQLite(db) => OAuth2ServerRepository::get_client(db, client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => OAuth2ServerRepository::get_client(db, client_id).await,
        }
    }
    async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_auth_code(auth_code).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_auth_code(auth_code).await,
        }
    }
    async fn get_auth_code(&self, code: &str) -> AppResult<Option<OAuth2AuthCode>> {
        match self {
            Self::SQLite(db) => db.get_auth_code(code).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_auth_code(code).await,
        }
    }
    async fn update_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_auth_code(auth_code).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_auth_code(auth_code).await,
        }
    }
    async fn store_refresh_token(&self, refresh_token: &OAuth2RefreshToken) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_refresh_token(refresh_token).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_refresh_token(refresh_token).await,
        }
    }
    async fn get_refresh_token(&self, token: &str) -> AppResult<Option<OAuth2RefreshToken>> {
        match self {
            Self::SQLite(db) => db.get_refresh_token(token).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_refresh_token(token).await,
        }
    }
    async fn revoke_refresh_token(&self, token: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.revoke_refresh_token(token).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.revoke_refresh_token(token).await,
        }
    }
    async fn consume_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2AuthCode>> {
        match self {
            Self::SQLite(db) => {
                db.consume_auth_code(code, client_id, redirect_uri, now)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.consume_auth_code(code, client_id, redirect_uri, now)
                    .await
            }
        }
    }
    async fn consume_refresh_token(
        &self,
        token: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2RefreshToken>> {
        match self {
            Self::SQLite(db) => db.consume_refresh_token(token, client_id, now).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.consume_refresh_token(token, client_id, now).await,
        }
    }
    async fn get_refresh_token_by_value(
        &self,
        token: &str,
    ) -> AppResult<Option<OAuth2RefreshToken>> {
        match self {
            Self::SQLite(db) => db.get_refresh_token_by_value(token).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_refresh_token_by_value(token).await,
        }
    }
    async fn store_authorization_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        scope: &str,
        user_id: Uuid,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.store_authorization_code(code, client_id, redirect_uri, scope, user_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.store_authorization_code(code, client_id, redirect_uri, scope, user_id)
                    .await
            }
        }
    }
    async fn get_authorization_code(&self, code: &str) -> AppResult<AuthorizationCode> {
        match self {
            Self::SQLite(db) => db.get_authorization_code(code).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_authorization_code(code).await,
        }
    }
    async fn delete_authorization_code(&self, code: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.delete_authorization_code(code).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_authorization_code(code).await,
        }
    }
    async fn store_state(&self, state: &OAuth2State) -> AppResult<()> {
        match self {
            Self::SQLite(db) => OAuth2ServerRepository::store_state(db, state).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => OAuth2ServerRepository::store_state(db, state).await,
        }
    }
    async fn consume_state(
        &self,
        state_value: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2State>> {
        match self {
            Self::SQLite(db) => {
                OAuth2ServerRepository::consume_state(db, state_value, client_id, now).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                OAuth2ServerRepository::consume_state(db, state_value, client_id, now).await
            }
        }
    }
}

#[async_trait]
impl OAuthClientStateRepository for Database {
    async fn store_oauth_client_state(&self, state: &OAuthClientState) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_oauth_client_state(state).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_oauth_client_state(state).await,
        }
    }

    async fn consume_oauth_client_state(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuthClientState>> {
        match self {
            Self::SQLite(db) => {
                db.consume_oauth_client_state(state_value, provider, now)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.consume_oauth_client_state(state_value, provider, now)
                    .await
            }
        }
    }
}

#[async_trait]
impl ProviderConnectionRepository for Database {
    async fn register_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        connection_type: &ConnectionType,
        metadata: Option<&str>,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.register_provider_connection_impl(
                    user_id,
                    tenant_id,
                    provider,
                    connection_type,
                    metadata,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.register_connection(user_id, tenant_id, provider, connection_type, metadata)
                    .await
            }
        }
    }
    async fn remove_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.remove_provider_connection_impl(user_id, tenant_id, provider)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.remove_connection(user_id, tenant_id, provider).await,
        }
    }
    async fn get_for_user(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<ProviderConnection>> {
        match self {
            Self::SQLite(db) => {
                db.get_user_provider_connections_impl(user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                ProviderConnectionRepository::get_for_user(db, user_id, tenant_id).await
            }
        }
    }
    async fn is_connected(&self, user_id: Uuid, provider: &str) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.is_provider_connected_impl(user_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.is_connected(user_id, provider).await,
        }
    }
}

#[async_trait]
impl PasswordResetRepository for Database {
    async fn store_token(
        &self,
        user_id: uuid::Uuid,
        token_hash: &str,
        created_by: &str,
    ) -> AppResult<uuid::Uuid> {
        match self {
            Self::SQLite(db) => {
                db.store_password_reset_token_impl(user_id, token_hash, created_by)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                PasswordResetRepository::store_token(db, user_id, token_hash, created_by).await
            }
        }
    }

    async fn consume_token(&self, token_hash: &str) -> AppResult<uuid::Uuid> {
        match self {
            Self::SQLite(db) => db.consume_password_reset_token_impl(token_hash).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => PasswordResetRepository::consume_token(db, token_hash).await,
        }
    }

    async fn invalidate_tokens(&self, user_id: uuid::Uuid) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.invalidate_user_reset_tokens_impl(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.invalidate_tokens(user_id).await,
        }
    }
}
