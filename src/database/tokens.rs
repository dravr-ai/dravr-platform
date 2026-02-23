// ABOUTME: OAuth token management database operations
// ABOUTME: Handles encryption, storage, and retrieval of OAuth tokens
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - Option<String> ownership for OAuth token scope fields

use super::Database;
use crate::errors::{AppError, AppResult};
use crate::models::{AuthorizationCode, DecryptedToken, EncryptedToken};
use crate::oauth2_client::OAuthClientState;
use crate::oauth2_server::models::{OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_database::repositories::{OAuth2ServerRepository, OAuthClientStateRepository};
use sqlx::Row;
use uuid::Uuid;

/// OAuth provider types
#[derive(Debug, Clone, Copy)]
pub enum OAuthProvider {
    /// Strava fitness platform
    Strava,
    /// Fitbit health tracking platform
    Fitbit,
}

impl OAuthProvider {
    /// Get the column prefix for this provider
    const fn column_prefix(self) -> &'static str {
        match self {
            Self::Strava => "strava",
            Self::Fitbit => "fitbit",
        }
    }
}

impl Database {
    /// Generic function to update OAuth token for any provider
    ///
    /// # Errors
    /// Returns an error if encryption fails or database update fails
    pub async fn update_oauth_token(
        &self,
        user_id: Uuid,
        provider: OAuthProvider,
        token: &DecryptedToken,
    ) -> AppResult<()> {
        let encrypted = EncryptedToken::new(
            &token.access_token,
            &token.refresh_token,
            token.expires_at,
            token.scope.clone(),
            &self.encryption_key,
        )?;

        let prefix = provider.column_prefix();
        let query = format!(
            r"
            UPDATE users SET
                {prefix}_access_token = $2,
                {prefix}_refresh_token = $3,
                {prefix}_expires_at = $4,
                {prefix}_scope = $5
            WHERE id = $1
            "
        );

        sqlx::query(&query)
            .bind(user_id)
            .bind(&encrypted.access_token)
            .bind(&encrypted.refresh_token)
            .bind(encrypted.expires_at.timestamp())
            .bind(&encrypted.scope)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to update OAuth token: {e}")))?;

        Ok(())
    }

    /// Generic function to get OAuth token for any provider
    ///
    /// # Errors
    /// Returns an error if database query fails or decryption fails
    pub async fn get_oauth_token(
        &self,
        user_id: Uuid,
        provider: OAuthProvider,
    ) -> AppResult<Option<DecryptedToken>> {
        let prefix = provider.column_prefix();
        let query = format!(
            r"
            SELECT {prefix}_access_token, {prefix}_refresh_token, {prefix}_expires_at,
                   {prefix}_scope
            FROM users WHERE id = $1
            "
        );

        let row = sqlx::query(&query)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to query OAuth token: {e}")))?;

        if let Some(row) = row {
            let access_col = format!("{prefix}_access_token");
            let refresh_col = format!("{prefix}_refresh_token");
            let expires_col = format!("{prefix}_expires_at");
            let scope_col = format!("{prefix}_scope");

            if let (Some(access), Some(refresh), Some(expires_at)) = (
                row.get::<Option<String>, _>(access_col.as_str()),
                row.get::<Option<String>, _>(refresh_col.as_str()),
                row.get::<Option<i64>, _>(expires_col.as_str()),
            ) {
                let scope: Option<String> = row.get(scope_col.as_str());

                let encrypted = EncryptedToken {
                    access_token: access,
                    refresh_token: refresh,
                    expires_at: chrono::DateTime::from_timestamp(expires_at, 0).ok_or_else(
                        || AppError::internal(format!("Invalid timestamp: {expires_at}")),
                    )?,
                    scope: scope.unwrap_or_default(),
                };

                let decrypted = encrypted.decrypt(&self.encryption_key)?;
                Ok(Some(decrypted))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    /// Generic function to clear OAuth token for any provider
    ///
    /// # Errors
    /// Returns an error if database update fails
    pub async fn clear_oauth_token(&self, user_id: Uuid, provider: OAuthProvider) -> AppResult<()> {
        let prefix = provider.column_prefix();
        let query = format!(
            r"
            UPDATE users SET
                {prefix}_access_token = NULL,
                {prefix}_refresh_token = NULL,
                {prefix}_expires_at = NULL,
                {prefix}_scope = NULL,
                {prefix}_nonce = NULL
            WHERE id = $1
            "
        );

        sqlx::query(&query)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to clear OAuth token: {e}")))?;

        Ok(())
    }
}

#[async_trait]
impl OAuth2ServerRepository for Database {
    async fn store_client(&self, client: &OAuth2Client) -> AppResult<()> {
        Self::store_oauth2_client_impl(self, client).await
    }
    async fn get_client(&self, client_id: &str) -> AppResult<Option<OAuth2Client>> {
        Self::get_oauth2_client_impl(self, client_id).await
    }
    async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()> {
        Self::store_oauth2_auth_code_impl(self, auth_code).await
    }
    async fn get_auth_code(&self, code: &str) -> AppResult<Option<OAuth2AuthCode>> {
        Self::get_oauth2_auth_code_impl(self, code).await
    }
    async fn update_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()> {
        Self::update_oauth2_auth_code_impl(self, auth_code).await
    }
    async fn store_refresh_token(&self, refresh_token: &OAuth2RefreshToken) -> AppResult<()> {
        Self::store_oauth2_refresh_token_impl(self, refresh_token).await
    }
    async fn get_refresh_token(&self, token: &str) -> AppResult<Option<OAuth2RefreshToken>> {
        Self::get_oauth2_refresh_token_impl(self, token).await
    }
    async fn revoke_refresh_token(&self, token: &str) -> AppResult<()> {
        Self::revoke_oauth2_refresh_token_impl(self, token).await
    }
    async fn consume_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2AuthCode>> {
        Self::consume_auth_code_impl(self, code, client_id, redirect_uri, now).await
    }
    async fn consume_refresh_token(
        &self,
        token: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2RefreshToken>> {
        Self::consume_refresh_token_impl(self, token, client_id, now).await
    }
    async fn get_refresh_token_by_value(
        &self,
        token: &str,
    ) -> AppResult<Option<OAuth2RefreshToken>> {
        // Delegate to get_oauth2_refresh_token_impl which handles token hashing
        Self::get_oauth2_refresh_token_impl(self, token).await
    }
    async fn store_authorization_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        scope: &str,
        user_id: Uuid,
    ) -> AppResult<()> {
        Self::store_authorization_code(self, code, client_id, redirect_uri, scope, user_id).await
    }
    async fn get_authorization_code(&self, code: &str) -> AppResult<AuthorizationCode> {
        Self::get_authorization_code_impl(self, code).await
    }
    async fn delete_authorization_code(&self, code: &str) -> AppResult<()> {
        Self::delete_authorization_code_impl(self, code).await
    }
    async fn store_state(&self, state: &OAuth2State) -> AppResult<()> {
        Self::store_oauth2_state_impl(self, state).await
    }
    async fn consume_state(
        &self,
        state_value: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2State>> {
        Self::consume_oauth2_state_impl(self, state_value, client_id, now).await
    }
}

#[async_trait]
impl OAuthClientStateRepository for Database {
    async fn store_oauth_client_state(&self, state: &OAuthClientState) -> AppResult<()> {
        Self::store_oauth_client_state_impl(self, state).await
    }

    async fn consume_oauth_client_state(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuthClientState>> {
        Self::consume_oauth_client_state_impl(self, state_value, provider, now).await
    }
}
