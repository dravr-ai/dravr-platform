// ABOUTME: PostgreSQL OAuth token and authorization repository implementations
// ABOUTME: Manages OAuth tokens, OAuth2 server, client state, provider connections, and password resets
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::{
    OAuth2ServerRepository, OAuthClientStateRepository, OAuthTokenRepository,
    PasswordResetRepository, ProviderConnectionRepository,
};
use super::PostgresDatabase;
use crate::plugins::shared;
use crate::plugins::shared::encryption::HasEncryption;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuthClientState;
use pierre_core::models::TenantId;
use pierre_core::models::{
    AuthorizationCode, ConnectionType, ProviderConnection, UserOAuthApp, UserOAuthToken,
};
use pierre_core::models::{OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State};
use sqlx::Row;
use tracing::warn;
use uuid::Uuid;

#[async_trait]
impl OAuthTokenRepository for PostgresDatabase {
    async fn get_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<DateTime<Utc>>> {
        let last_sync: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT last_sync FROM user_oauth_tokens WHERE user_id = $1 AND tenant_id = $2 AND provider = $3",
        )
        .bind(user_id)
        .bind(tenant_id.to_string())
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get provider last sync: {e}")))?;

        Ok(last_sync)
    }

    async fn update_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        sync_time: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE user_oauth_tokens SET last_sync = $1 WHERE user_id = $2 AND tenant_id = $3 AND provider = $4",
        )
        .bind(sync_time)
        .bind(user_id)
        .bind(tenant_id.to_string())
        .bind(provider)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update provider last sync: {e}")))?;

        Ok(())
    }

    // UserOAuthToken Methods - PostgreSQL implementations
    // ================================

    async fn upsert_token(&self, token: &UserOAuthToken) -> AppResult<()> {
        // SECURITY: Encrypt OAuth tokens at rest with AAD binding (AES-256-GCM)
        let encrypted_access_token = shared::encryption::encrypt_oauth_token(
            self,
            &token.access_token,
            &token.tenant_id,
            token.user_id,
            &token.provider,
        )?;

        let encrypted_refresh_token = token
            .refresh_token
            .as_ref()
            .map(|rt| {
                shared::encryption::encrypt_oauth_token(
                    self,
                    rt,
                    &token.tenant_id,
                    token.user_id,
                    &token.provider,
                )
            })
            .transpose()?;

        sqlx::query(
            r"
            INSERT INTO user_oauth_tokens (
                id, user_id, tenant_id, provider, access_token, refresh_token,
                token_type, expires_at, scope, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (user_id, tenant_id, provider)
            DO UPDATE SET
                id = EXCLUDED.id,
                access_token = EXCLUDED.access_token,
                refresh_token = EXCLUDED.refresh_token,
                token_type = EXCLUDED.token_type,
                expires_at = EXCLUDED.expires_at,
                scope = EXCLUDED.scope,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(&token.id)
        .bind(token.user_id)
        .bind(&token.tenant_id)
        .bind(&token.provider)
        .bind(&encrypted_access_token)
        .bind(encrypted_refresh_token.as_deref())
        .bind(&token.token_type)
        .bind(token.expires_at)
        .bind(token.scope.as_deref().unwrap_or(""))
        .bind(token.created_at)
        .bind(token.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    async fn get_token(
        &self,
        user_id: uuid::Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<UserOAuthToken>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, provider, access_token, refresh_token,
                   token_type, expires_at, scope, created_at, updated_at
            FROM user_oauth_tokens
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ",
        )
        .bind(user_id)
        .bind(tenant_id.to_string())
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| Ok(Some(self.row_to_user_oauth_token(&row)?)),
        )
    }

    async fn get_tokens(
        &self,
        user_id: uuid::Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<UserOAuthToken>> {
        let rows = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, access_token, refresh_token,
                       token_type, expires_at, scope, created_at, updated_at
                FROM user_oauth_tokens
                WHERE user_id = $1 AND tenant_id = $2
                ORDER BY created_at DESC
                ",
            )
            .bind(user_id)
            .bind(tid.to_string())
            .fetch_all(&self.pool)
            .await
        } else {
            // Intentional cross-tenant view for OAuth status checks (e.g. admin views)
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, access_token, refresh_token,
                       token_type, expires_at, scope, created_at, updated_at
                FROM user_oauth_tokens
                WHERE user_id = $1
                ORDER BY created_at DESC
                ",
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let mut tokens = Vec::with_capacity(rows.len());
        for row in rows {
            tokens.push(self.row_to_user_oauth_token(&row)?);
        }
        Ok(tokens)
    }

    async fn get_tenant_provider_tokens(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Vec<UserOAuthToken>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, provider, access_token, refresh_token,
                   token_type, expires_at, scope, created_at, updated_at
            FROM user_oauth_tokens
            WHERE tenant_id = $1 AND provider = $2
            ORDER BY created_at DESC
            ",
        )
        .bind(tenant_id.to_string())
        .bind(provider)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let mut tokens = Vec::with_capacity(rows.len());
        for row in rows {
            tokens.push(self.row_to_user_oauth_token(&row)?);
        }
        Ok(tokens)
    }

    async fn delete_token(
        &self,
        user_id: uuid::Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            DELETE FROM user_oauth_tokens
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ",
        )
        .bind(user_id)
        .bind(tenant_id.to_string())
        .bind(provider)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    async fn delete_tokens(&self, user_id: uuid::Uuid, tenant_id: TenantId) -> AppResult<()> {
        sqlx::query(
            r"
            DELETE FROM user_oauth_tokens
            WHERE user_id = $1 AND tenant_id = $2
            ",
        )
        .bind(user_id)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    async fn refresh_token(
        &self,
        user_id: uuid::Uuid,
        tenant_id: TenantId,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        // SECURITY: Encrypt OAuth tokens at rest with AAD binding (AES-256-GCM)
        let tid = tenant_id.to_string();
        let encrypted_access_token =
            shared::encryption::encrypt_oauth_token(self, access_token, &tid, user_id, provider)?;

        let encrypted_refresh_token = refresh_token
            .map(|rt| shared::encryption::encrypt_oauth_token(self, rt, &tid, user_id, provider))
            .transpose()?;

        sqlx::query(
            r"
            UPDATE user_oauth_tokens
            SET access_token = $4,
                refresh_token = $5,
                expires_at = $6,
                updated_at = CURRENT_TIMESTAMP
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ",
        )
        .bind(user_id)
        .bind(tenant_id.to_string())
        .bind(provider)
        .bind(&encrypted_access_token)
        .bind(encrypted_refresh_token.as_deref())
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    // ================================
    // User OAuth App Credentials Implementation
    // ================================

    /// Store user OAuth app credentials (`client_id`, `client_secret`)
    async fn store_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> AppResult<()> {
        // Create user_oauth_apps table if it doesn't exist
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS user_oauth_apps (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                provider TEXT NOT NULL,
                client_id TEXT NOT NULL,
                client_secret TEXT NOT NULL,
                redirect_uri TEXT NOT NULL,
                created_at TIMESTAMPTZ DEFAULT NOW(),
                updated_at TIMESTAMPTZ DEFAULT NOW(),
                UNIQUE(user_id, provider)
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        // Insert or update OAuth app credentials
        sqlx::query(
            r"
            INSERT INTO user_oauth_apps (user_id, provider, client_id, client_secret, redirect_uri)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (user_id, provider)
            DO UPDATE SET 
                client_id = EXCLUDED.client_id,
                client_secret = EXCLUDED.client_secret,
                redirect_uri = EXCLUDED.redirect_uri,
                updated_at = NOW()
            ",
        )
        .bind(user_id)
        .bind(provider)
        .bind(client_id)
        .bind(client_secret)
        .bind(redirect_uri)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    /// Get user OAuth app credentials for a provider
    async fn get_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> AppResult<Option<UserOAuthApp>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, provider, client_id, client_secret, redirect_uri, created_at, updated_at
            FROM user_oauth_apps
            WHERE user_id = $1 AND provider = $2
            "
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.pool).await.map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(UserOAuthApp {
                    id: row.get("id"),
                    user_id: row.get("user_id"),
                    provider: row.get("provider"),
                    client_id: row.get("client_id"),
                    client_secret: row.get("client_secret"),
                    redirect_uri: row.get("redirect_uri"),
                    created_at: row.get("created_at"),
                    updated_at: row.get("updated_at"),
                }))
            },
        )
    }

    /// List all OAuth app providers configured for a user
    async fn list_user_oauth_apps(&self, user_id: Uuid) -> AppResult<Vec<UserOAuthApp>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, provider, client_id, client_secret, redirect_uri, created_at, updated_at
            FROM user_oauth_apps
            WHERE user_id = $1
            ORDER BY provider
            "
        )
        .bind(user_id)
        .fetch_all(&self.pool).await.map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let mut apps = Vec::new();
        for row in rows {
            apps.push(UserOAuthApp {
                id: row.get("id"),
                user_id: row.get("user_id"),
                provider: row.get("provider"),
                client_id: row.get("client_id"),
                client_secret: row.get("client_secret"),
                redirect_uri: row.get("redirect_uri"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            });
        }

        Ok(apps)
    }

    /// Remove user OAuth app credentials for a provider
    async fn remove_user_oauth_app(&self, user_id: Uuid, provider: &str) -> AppResult<()> {
        sqlx::query(
            r"
            DELETE FROM user_oauth_apps
            WHERE user_id = $1 AND provider = $2
            ",
        )
        .bind(user_id)
        .bind(provider)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }
}

#[async_trait]
impl OAuth2ServerRepository for PostgresDatabase {
    /// Store authorization code
    async fn store_authorization_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        scope: &str,
        user_id: Uuid,
    ) -> AppResult<()> {
        // Use the provided user_id from auth context
        let expires_at = Utc::now() + chrono::Duration::minutes(10); // OAuth codes expire in 10 minutes

        sqlx::query(
            r"
            INSERT INTO authorization_codes
                (code, client_id, user_id, redirect_uri, scope, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, CURRENT_TIMESTAMP, $6)
            ",
        )
        .bind(code)
        .bind(client_id)
        .bind(user_id)
        .bind(redirect_uri)
        .bind(scope)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store authorization code: {e}")))?;

        Ok(())
    }

    /// Get authorization code data
    async fn get_authorization_code(&self, code: &str) -> AppResult<AuthorizationCode> {
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                Uuid,
                String,
                String,
                DateTime<Utc>,
                DateTime<Utc>,
            ),
        >(
            r"
            SELECT code, client_id, user_id, redirect_uri, scope, created_at, expires_at
            FROM authorization_codes
            WHERE code = $1 AND expires_at > CURRENT_TIMESTAMP
            ",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        match row {
            Some((code, client_id, user_id, redirect_uri, scope, created_at, expires_at)) => {
                Ok(AuthorizationCode {
                    code,
                    client_id,
                    redirect_uri,
                    scope,
                    user_id: Some(user_id),
                    expires_at,
                    created_at,
                    is_used: false, // Will be marked as used when deleted
                })
            }
            None => Err(AppError::not_found(
                "Authorization code not found or expired".to_owned(),
            )),
        }
    }

    /// Delete authorization code
    async fn delete_authorization_code(&self, code: &str) -> AppResult<()> {
        let result = sqlx::query(
            r"
            DELETE FROM authorization_codes
            WHERE code = $1
            ",
        )
        .bind(code)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete authorization code: {e}")))?;

        if result.rows_affected() == 0 {
            warn!("Authorization code not found for deletion (code redacted)");
        }

        Ok(())
    }

    async fn store_client(&self, client: &OAuth2Client) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO oauth2_clients (id, client_id, client_secret_hash, redirect_uris, grant_types, response_types, client_name, client_uri, scope, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
        )
        .bind(&client.id)
        .bind(&client.client_id)
        .bind(&client.client_secret_hash)
        .bind(serde_json::to_string(&client.redirect_uris)?)
        .bind(serde_json::to_string(&client.grant_types)?)
        .bind(serde_json::to_string(&client.response_types)?)
        .bind(&client.client_name)
        .bind(&client.client_uri)
        .bind(&client.scope)
        .bind(client.created_at)
        .bind(client.expires_at)
        .execute(&self.pool).await.map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    async fn get_client(&self, client_id: &str) -> AppResult<Option<OAuth2Client>> {
        let row = sqlx::query(
            "SELECT id, client_id, client_secret_hash, redirect_uris, grant_types, response_types, client_name, client_uri, scope, created_at, expires_at
             FROM oauth2_clients WHERE client_id = $1"
        )
        .bind(client_id)
        .fetch_optional(&self.pool).await.map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = row {
            let redirect_uris: Vec<String> =
                serde_json::from_str(&row.get::<String, _>("redirect_uris"))?;
            let grant_types: Vec<String> =
                serde_json::from_str(&row.get::<String, _>("grant_types"))?;
            let response_types: Vec<String> =
                serde_json::from_str(&row.get::<String, _>("response_types"))?;

            Ok(Some(OAuth2Client {
                id: row.get("id"),
                client_id: row.get("client_id"),
                client_secret_hash: row.get("client_secret_hash"),
                redirect_uris,
                grant_types,
                response_types,
                client_name: row.get("client_name"),
                client_uri: row.get("client_uri"),
                scope: row.get("scope"),
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
            }))
        } else {
            Ok(None)
        }
    }

    async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO oauth2_auth_codes (code, client_id, user_id, tenant_id, redirect_uri, scope, expires_at, used, state, code_challenge, code_challenge_method)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
        )
        .bind(&auth_code.code)
        .bind(&auth_code.client_id)
        .bind(auth_code.user_id)
        .bind(&auth_code.tenant_id)
        .bind(&auth_code.redirect_uri)
        .bind(&auth_code.scope)
        .bind(auth_code.expires_at)
        .bind(auth_code.used)
        .bind(&auth_code.state)
        .bind(&auth_code.code_challenge)
        .bind(&auth_code.code_challenge_method)
        .execute(&self.pool).await.map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    async fn get_auth_code(&self, code: &str) -> AppResult<Option<OAuth2AuthCode>> {
        let row = sqlx::query(
            "SELECT code, client_id, user_id, tenant_id, redirect_uri, scope, expires_at, used, state, code_challenge, code_challenge_method
             FROM oauth2_auth_codes WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&self.pool).await.map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(OAuth2AuthCode {
                    code: row.get("code"),
                    client_id: row.get("client_id"),
                    user_id: row.get("user_id"),
                    tenant_id: row.get("tenant_id"),
                    redirect_uri: row.get("redirect_uri"),
                    scope: row.get("scope"),
                    expires_at: row.get("expires_at"),
                    used: row.get("used"),
                    state: row.get("state"),
                    code_challenge: row.get("code_challenge"),
                    code_challenge_method: row.get("code_challenge_method"),
                }))
            },
        )
    }

    async fn update_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()> {
        sqlx::query("UPDATE oauth2_auth_codes SET used = $1 WHERE code = $2")
            .bind(auth_code.used)
            .bind(&auth_code.code)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    /// Store OAuth 2.0 refresh token
    ///
    /// The refresh token value is HMAC-SHA256 hashed before storage so that
    /// plaintext tokens are never persisted to disk.
    async fn store_refresh_token(&self, refresh_token: &OAuth2RefreshToken) -> AppResult<()> {
        let token_hash = HasEncryption::hash_token_for_storage(self, &refresh_token.token)?;

        sqlx::query(
            "INSERT INTO oauth2_refresh_tokens (token, client_id, user_id, tenant_id, scope, expires_at, created_at, revoked)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        )
        .bind(&token_hash)
        .bind(&refresh_token.client_id)
        .bind(refresh_token.user_id)
        .bind(&refresh_token.tenant_id)
        .bind(&refresh_token.scope)
        .bind(refresh_token.expires_at)
        .bind(refresh_token.created_at)
        .bind(refresh_token.revoked)
        .execute(&self.pool).await.map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    /// Get OAuth 2.0 refresh token
    ///
    /// The input token is HMAC-SHA256 hashed before querying.
    async fn get_refresh_token(&self, token: &str) -> AppResult<Option<OAuth2RefreshToken>> {
        let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

        let row = sqlx::query(
            "SELECT token, client_id, user_id, tenant_id, scope, expires_at, created_at, revoked
             FROM oauth2_refresh_tokens
             WHERE token = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = row {
            use sqlx::Row;
            Ok(Some(OAuth2RefreshToken {
                token: row.try_get("token").map_err(|e| {
                    AppError::database(format!("Failed to parse token column: {e}"))
                })?,
                client_id: row.try_get("client_id").map_err(|e| {
                    AppError::database(format!("Failed to parse client_id column: {e}"))
                })?,
                user_id: row.try_get("user_id").map_err(|e| {
                    AppError::database(format!("Failed to parse user_id column: {e}"))
                })?,
                tenant_id: row.try_get("tenant_id").map_err(|e| {
                    AppError::database(format!("Failed to parse tenant_id column: {e}"))
                })?,
                scope: row.try_get("scope").map_err(|e| {
                    AppError::database(format!("Failed to parse scope column: {e}"))
                })?,
                expires_at: row.try_get("expires_at").map_err(|e| {
                    AppError::database(format!("Failed to parse expires_at column: {e}"))
                })?,
                created_at: row.try_get("created_at").map_err(|e| {
                    AppError::database(format!("Failed to parse created_at column: {e}"))
                })?,
                revoked: row.try_get("revoked").map_err(|e| {
                    AppError::database(format!("Failed to parse revoked column: {e}"))
                })?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Revoke OAuth 2.0 refresh token
    ///
    /// The input token is HMAC-SHA256 hashed before querying.
    async fn revoke_refresh_token(&self, token: &str) -> AppResult<()> {
        let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

        sqlx::query("UPDATE oauth2_refresh_tokens SET revoked = true WHERE token = $1")
            .bind(&token_hash)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    /// Atomically consume OAuth 2.0 authorization code
    ///
    /// Implements atomic check-and-set using UPDATE...RETURNING
    /// to prevent TOCTOU race conditions in concurrent token exchange requests.
    async fn consume_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2AuthCode>> {
        let row = sqlx::query(
            "UPDATE oauth2_auth_codes
             SET used = true
             WHERE code = $1
               AND client_id = $2
               AND redirect_uri = $3
               AND used = false
               AND expires_at > $4
             RETURNING code, client_id, user_id, tenant_id, redirect_uri, scope, expires_at, used, state, code_challenge, code_challenge_method"
        )
        .bind(code)
        .bind(client_id)
        .bind(redirect_uri)
        .bind(now)
        .fetch_optional(&self.pool).await.map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                use sqlx::Row;
                Ok(Some(OAuth2AuthCode {
                    code: row.get("code"),
                    client_id: row.get("client_id"),
                    user_id: row.get("user_id"),
                    tenant_id: row.get("tenant_id"),
                    redirect_uri: row.get("redirect_uri"),
                    scope: row.get("scope"),
                    expires_at: row.get("expires_at"),
                    used: row.get("used"),
                    state: row.get("state"),
                    code_challenge: row.get("code_challenge"),
                    code_challenge_method: row.get("code_challenge_method"),
                }))
            },
        )
    }

    /// Atomically consume OAuth 2.0 refresh token
    ///
    /// Implements atomic check-and-revoke using UPDATE...RETURNING
    /// to prevent TOCTOU race conditions in concurrent refresh requests.
    /// The input token is HMAC-SHA256 hashed before querying.
    async fn consume_refresh_token(
        &self,
        token: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2RefreshToken>> {
        let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

        let row = sqlx::query(
            "UPDATE oauth2_refresh_tokens
             SET revoked = true
             WHERE token = $1
               AND client_id = $2
               AND revoked = false
               AND expires_at > $3
             RETURNING token, client_id, user_id, tenant_id, scope, expires_at, created_at, revoked",
        )
        .bind(&token_hash)
        .bind(client_id)
        .bind(now)
        .fetch_optional(&self.pool).await.map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = row {
            use sqlx::Row;
            Ok(Some(OAuth2RefreshToken {
                token: row.try_get("token").map_err(|e| {
                    AppError::database(format!("Failed to parse token column: {e}"))
                })?,
                client_id: row.try_get("client_id").map_err(|e| {
                    AppError::database(format!("Failed to parse client_id column: {e}"))
                })?,
                user_id: row.try_get("user_id").map_err(|e| {
                    AppError::database(format!("Failed to parse user_id column: {e}"))
                })?,
                tenant_id: row.try_get("tenant_id").map_err(|e| {
                    AppError::database(format!("Failed to parse tenant_id column: {e}"))
                })?,
                scope: row.try_get("scope").map_err(|e| {
                    AppError::database(format!("Failed to parse scope column: {e}"))
                })?,
                expires_at: row.try_get("expires_at").map_err(|e| {
                    AppError::database(format!("Failed to parse expires_at column: {e}"))
                })?,
                created_at: row.try_get("created_at").map_err(|e| {
                    AppError::database(format!("Failed to parse created_at column: {e}"))
                })?,
                revoked: row.try_get("revoked").map_err(|e| {
                    AppError::database(format!("Failed to parse revoked column: {e}"))
                })?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Look up a refresh token by value (without `client_id` constraint)
    ///
    /// The input token is HMAC-SHA256 hashed before querying.
    async fn get_refresh_token_by_value(
        &self,
        token: &str,
    ) -> AppResult<Option<OAuth2RefreshToken>> {
        let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

        let row = sqlx::query(
            "SELECT token, client_id, user_id, tenant_id, scope, expires_at, created_at, revoked
             FROM oauth2_refresh_tokens
             WHERE token = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = row {
            use sqlx::Row;
            Ok(Some(OAuth2RefreshToken {
                token: row.try_get("token").map_err(|e| {
                    AppError::database(format!("Failed to parse token column: {e}"))
                })?,
                client_id: row.try_get("client_id").map_err(|e| {
                    AppError::database(format!("Failed to parse client_id column: {e}"))
                })?,
                user_id: row.try_get("user_id").map_err(|e| {
                    AppError::database(format!("Failed to parse user_id column: {e}"))
                })?,
                tenant_id: row.try_get("tenant_id").map_err(|e| {
                    AppError::database(format!("Failed to parse tenant_id column: {e}"))
                })?,
                scope: row.try_get("scope").map_err(|e| {
                    AppError::database(format!("Failed to parse scope column: {e}"))
                })?,
                expires_at: row.try_get("expires_at").map_err(|e| {
                    AppError::database(format!("Failed to parse expires_at column: {e}"))
                })?,
                created_at: row.try_get("created_at").map_err(|e| {
                    AppError::database(format!("Failed to parse created_at column: {e}"))
                })?,
                revoked: row.try_get("revoked").map_err(|e| {
                    AppError::database(format!("Failed to parse revoked column: {e}"))
                })?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Store `OAuth2` state for CSRF protection
    async fn store_state(&self, state: &OAuth2State) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO oauth2_states (state, client_id, user_id, tenant_id, redirect_uri, scope, code_challenge, code_challenge_method, created_at, expires_at, used)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
        )
        .bind(&state.state)
        .bind(&state.client_id)
        .bind(state.user_id)
        .bind(&state.tenant_id)
        .bind(&state.redirect_uri)
        .bind(&state.scope)
        .bind(&state.code_challenge)
        .bind(&state.code_challenge_method)
        .bind(state.created_at)
        .bind(state.expires_at)
        .bind(state.used)
        .execute(&self.pool).await.map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    /// Consume `OAuth2` state (atomically check and mark as used)
    async fn consume_state(
        &self,
        state_value: &str,
        client_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuth2State>> {
        let row = sqlx::query(
            "UPDATE oauth2_states
             SET used = true
             WHERE state = $1
               AND client_id = $2
               AND used = false
               AND expires_at > $3
             RETURNING state, client_id, user_id, tenant_id, redirect_uri, scope, code_challenge, code_challenge_method, created_at, expires_at, used",
        )
        .bind(state_value)
        .bind(client_id)
        .bind(now)
        .fetch_optional(&self.pool).await.map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = row {
            use sqlx::Row;
            Ok(Some(OAuth2State {
                state: row.try_get("state").map_err(|e| {
                    AppError::database(format!("Failed to parse state column: {e}"))
                })?,
                client_id: row.try_get("client_id").map_err(|e| {
                    AppError::database(format!("Failed to parse client_id column: {e}"))
                })?,
                user_id: row.try_get("user_id").map_err(|e| {
                    AppError::database(format!("Failed to parse user_id column: {e}"))
                })?,
                tenant_id: row.try_get("tenant_id").map_err(|e| {
                    AppError::database(format!("Failed to parse tenant_id column: {e}"))
                })?,
                redirect_uri: row.try_get("redirect_uri").map_err(|e| {
                    AppError::database(format!("Failed to parse redirect_uri column: {e}"))
                })?,
                scope: row.try_get("scope").map_err(|e| {
                    AppError::database(format!("Failed to parse scope column: {e}"))
                })?,
                code_challenge: row.try_get("code_challenge").map_err(|e| {
                    AppError::database(format!("Failed to parse code_challenge column: {e}"))
                })?,
                code_challenge_method: row.try_get("code_challenge_method").map_err(|e| {
                    AppError::database(format!("Failed to parse code_challenge_method column: {e}"))
                })?,
                created_at: row.try_get("created_at").map_err(|e| {
                    AppError::database(format!("Failed to parse created_at column: {e}"))
                })?,
                expires_at: row.try_get("expires_at").map_err(|e| {
                    AppError::database(format!("Failed to parse expires_at column: {e}"))
                })?,
                used: row
                    .try_get("used")
                    .map_err(|e| AppError::database(format!("Failed to parse used column: {e}")))?,
            }))
        } else {
            Ok(None)
        }
    }

    // ================================
    // OAuth Client State (CSRF + PKCE)
    // ================================
}

#[async_trait]
impl OAuthClientStateRepository for PostgresDatabase {
    async fn store_oauth_client_state(&self, state: &OAuthClientState) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO oauth_client_states (state, provider, user_id, tenant_id, redirect_uri, scope, pkce_code_verifier, created_at, expires_at, used)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&state.state)
        .bind(&state.provider)
        .bind(state.user_id)
        .bind(&state.tenant_id)
        .bind(&state.redirect_uri)
        .bind(&state.scope)
        .bind(&state.pkce_code_verifier)
        .bind(state.created_at)
        .bind(state.expires_at)
        .bind(state.used)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store OAuth client state: {e}")))?;

        Ok(())
    }

    async fn consume_oauth_client_state(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuthClientState>> {
        let row = sqlx::query(
            "UPDATE oauth_client_states
             SET used = true
             WHERE state = $1
               AND provider = $2
               AND used = false
               AND expires_at > $3
             RETURNING state, provider, user_id, tenant_id, redirect_uri, scope, pkce_code_verifier, created_at, expires_at, used",
        )
        .bind(state_value)
        .bind(provider)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to consume OAuth client state: {e}")))?;

        if let Some(row) = row {
            use sqlx::Row;
            Ok(Some(OAuthClientState {
                state: row.try_get("state").map_err(|e| {
                    AppError::database(format!("Failed to parse state column: {e}"))
                })?,
                provider: row.try_get("provider").map_err(|e| {
                    AppError::database(format!("Failed to parse provider column: {e}"))
                })?,
                user_id: row.try_get("user_id").map_err(|e| {
                    AppError::database(format!("Failed to parse user_id column: {e}"))
                })?,
                tenant_id: row.try_get("tenant_id").map_err(|e| {
                    AppError::database(format!("Failed to parse tenant_id column: {e}"))
                })?,
                redirect_uri: row.try_get("redirect_uri").map_err(|e| {
                    AppError::database(format!("Failed to parse redirect_uri column: {e}"))
                })?,
                scope: row.try_get("scope").map_err(|e| {
                    AppError::database(format!("Failed to parse scope column: {e}"))
                })?,
                pkce_code_verifier: row.try_get("pkce_code_verifier").map_err(|e| {
                    AppError::database(format!("Failed to parse pkce_code_verifier column: {e}"))
                })?,
                created_at: row.try_get("created_at").map_err(|e| {
                    AppError::database(format!("Failed to parse created_at column: {e}"))
                })?,
                expires_at: row.try_get("expires_at").map_err(|e| {
                    AppError::database(format!("Failed to parse expires_at column: {e}"))
                })?,
                used: row
                    .try_get("used")
                    .map_err(|e| AppError::database(format!("Failed to parse used column: {e}")))?,
            }))
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl ProviderConnectionRepository for PostgresDatabase {
    // ================================
    // Provider Connections (PostgreSQL implementation)
    // ================================

    async fn register_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        connection_type: &ConnectionType,
        metadata: Option<&str>,
    ) -> AppResult<()> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let conn_type_str = connection_type.as_str();

        sqlx::query(
            r"
            INSERT INTO provider_connections (id, user_id, tenant_id, provider, connection_type, connected_at, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT(user_id, tenant_id, provider) DO UPDATE SET
                connection_type = EXCLUDED.connection_type,
                connected_at = EXCLUDED.connected_at,
                metadata = EXCLUDED.metadata
            ",
        )
        .bind(&id)
        .bind(user_id.to_string())
        .bind(tenant_id.0)
        .bind(provider)
        .bind(conn_type_str)
        .bind(now)
        .bind(metadata)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn remove_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM provider_connections WHERE user_id = $1 AND tenant_id = $2 AND provider = $3",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.0)
        .bind(provider)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_for_user(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<ProviderConnection>> {
        let rows = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, metadata
                FROM provider_connections
                WHERE user_id = $1 AND tenant_id = $2
                ORDER BY connected_at DESC
                ",
            )
            .bind(user_id.to_string())
            .bind(tid.to_string())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, metadata
                FROM provider_connections
                WHERE user_id = $1
                ORDER BY connected_at DESC
                ",
            )
            .bind(user_id.to_string())
            .fetch_all(&self.pool)
            .await?
        };

        let mut connections = Vec::with_capacity(rows.len());
        for row in rows {
            let conn_type_str: String = row.get("connection_type");
            let connected_at: DateTime<Utc> = row.get("connected_at");

            let user_id_from_db: String = row.get("user_id");
            let parsed_user_id = Uuid::parse_str(&user_id_from_db).unwrap_or_else(|_| Uuid::nil());

            connections.push(ProviderConnection {
                id: row.get("id"),
                user_id: parsed_user_id,
                tenant_id: row.get("tenant_id"),
                provider: row.get("provider"),
                connection_type: ConnectionType::from_str_value(&conn_type_str)
                    .unwrap_or(ConnectionType::Manual),
                connected_at,
                metadata: row.get("metadata"),
            });
        }

        Ok(connections)
    }

    async fn is_connected(&self, user_id: Uuid, provider: &str) -> AppResult<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM provider_connections WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id.to_string())
        .bind(provider)
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }
}

#[async_trait]
impl PasswordResetRepository for PostgresDatabase {
    async fn store_token(
        &self,
        user_id: Uuid,
        token_hash: &str,
        created_by: &str,
    ) -> AppResult<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(1);

        sqlx::query(
            r"
            INSERT INTO password_reset_tokens (id, user_id, token_hash, expires_at, created_by, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(token_hash)
        .bind(expires_at)
        .bind(created_by)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store password reset token: {e}")))?;

        Ok(id)
    }

    async fn consume_token(&self, token_hash: &str) -> AppResult<Uuid> {
        let now = Utc::now();

        let row = sqlx::query(
            r"
            UPDATE password_reset_tokens
            SET used_at = $1
            WHERE token_hash = $2
              AND used_at IS NULL
              AND expires_at > $1
            RETURNING user_id
            ",
        )
        .bind(now)
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to consume reset token: {e}")))?;

        row.map_or_else(
            || {
                Err(AppError::not_found(
                    "Password reset token is invalid, expired, or already used",
                ))
            },
            |row| {
                let user_id_str: String = row.get("user_id");
                Uuid::parse_str(&user_id_str)
                    .map_err(|e| AppError::internal(format!("Invalid user_id in reset token: {e}")))
            },
        )
    }

    async fn store_token_with_ttl(
        &self,
        user_id: Uuid,
        token_hash: &str,
        created_by: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + chrono::Duration::minutes(ttl_minutes);

        sqlx::query(
            r"
            INSERT INTO password_reset_tokens (id, user_id, token_hash, expires_at, created_by, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(token_hash)
        .bind(expires_at)
        .bind(created_by)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store password reset token: {e}")))?;

        Ok(id)
    }

    async fn invalidate_tokens(&self, user_id: Uuid) -> AppResult<()> {
        let now = Utc::now();

        sqlx::query(
            r"
            UPDATE password_reset_tokens
            SET used_at = $1
            WHERE user_id = $2
              AND used_at IS NULL
            ",
        )
        .bind(now)
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to invalidate reset tokens: {e}")))?;

        Ok(())
    }

    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as cnt
            FROM password_reset_tokens
            WHERE user_id = $1
              AND created_at >= $2
            ",
        )
        .bind(user_id.to_string())
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count recent reset tokens: {e}")))?;

        Ok(row.get::<i64, _>("cnt"))
    }
}
