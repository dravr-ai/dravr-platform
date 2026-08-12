// ABOUTME: UserOAuthToken database operations for per-user, per-tenant OAuth credential storage
// ABOUTME: Handles tenant-aware OAuth token management for multi-tenant architecture
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::repositories::OAuthTokenRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::models::{StravaPoolApp, UserOAuthApp, UserOAuthToken};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

/// Build a [`StravaPoolApp`] from a `strava_oauth_app_pool` row (`SQLite` stores
/// `enabled` as 0/1 and timestamps as epoch integers).
fn row_to_strava_pool_app(row: &SqliteRow) -> StravaPoolApp {
    let seat_cap: i64 = row.get("seat_cap");
    let enabled: i64 = row.get("enabled");
    StravaPoolApp {
        client_id: row.get("client_id"),
        seat_cap: u32::try_from(seat_cap).unwrap_or(0),
        enabled: enabled != 0,
        label: row.get("label"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// OAuth token data for database operations
pub struct OAuthTokenData<'a> {
    /// Unique token identifier
    pub id: &'a str,
    /// User ID this token belongs to
    pub user_id: Uuid,
    /// Tenant ID for multi-tenant isolation
    pub tenant_id: TenantId,
    /// OAuth provider (e.g., "strava", "fitbit")
    pub provider: &'a str,
    /// OAuth access token
    pub access_token: &'a str,
    /// Optional OAuth refresh token
    pub refresh_token: Option<&'a str>,
    /// Token type (usually "Bearer")
    pub token_type: &'a str,
    /// When the access token expires
    pub expires_at: Option<DateTime<Utc>>,
    /// OAuth scope string
    pub scope: &'a str,
    /// Provider-side user identifier (e.g. Intervals.icu `athlete_id`), stored
    /// plaintext. `None` for pure OAuth bearer providers.
    pub provider_user_id: Option<&'a str>,
    /// Shared-app pool `client_id` that issued this Strava token (so refresh
    /// resolves the matching secret). `None` = the env-default app.
    pub oauth_app_client_id: Option<&'a str>,
}

impl Database {
    /// Upsert a user OAuth token using structured data
    ///
    /// Provider tokens are encrypted at rest using AES-256-GCM with AAD binding
    /// to prevent cross-tenant or cross-user token reuse.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Encryption fails
    /// - Database operation fails
    pub async fn upsert_user_oauth_token(&self, token_data: &OAuthTokenData<'_>) -> AppResult<()> {
        // Create AAD context: tenant_id|user_id|provider|table
        let aad_context = format!(
            "{}|{}|{}|user_oauth_tokens",
            token_data.tenant_id, token_data.user_id, token_data.provider
        );

        // Encrypt access token with AAD binding
        let encrypted_access_token =
            self.encrypt_data_with_aad(token_data.access_token, &aad_context)?;

        // Encrypt refresh token if present
        let encrypted_refresh_token = token_data
            .refresh_token
            .map(|rt| self.encrypt_data_with_aad(rt, &aad_context))
            .transpose()?;

        sqlx::query(
            r"
            INSERT INTO user_oauth_tokens (
                id, user_id, tenant_id, provider, access_token, refresh_token,
                token_type, expires_at, scope, created_at, updated_at, provider_user_id,
                oauth_app_client_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (user_id, tenant_id, provider)
            DO UPDATE SET
                id = EXCLUDED.id,
                access_token = EXCLUDED.access_token,
                refresh_token = EXCLUDED.refresh_token,
                token_type = EXCLUDED.token_type,
                expires_at = EXCLUDED.expires_at,
                scope = EXCLUDED.scope,
                provider_user_id = EXCLUDED.provider_user_id,
                oauth_app_client_id = EXCLUDED.oauth_app_client_id,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(token_data.id)
        .bind(token_data.user_id.to_string())
        .bind(token_data.tenant_id)
        .bind(token_data.provider)
        .bind(&encrypted_access_token)
        .bind(encrypted_refresh_token.as_deref())
        .bind(token_data.token_type)
        .bind(token_data.expires_at)
        .bind(token_data.scope)
        .bind(Utc::now())
        .bind(Utc::now())
        .bind(token_data.provider_user_id)
        .bind(token_data.oauth_app_client_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert user OAuth token: {e}")))?;

        Ok(())
    }

    /// Get a user OAuth token
    ///
    /// Decrypts provider tokens using AAD binding for security.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database query fails
    /// - Decryption fails (possibly due to tampered data or AAD mismatch)
    pub async fn get_user_oauth_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<UserOAuthToken>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, provider, access_token, refresh_token,
                   token_type, expires_at, scope, provider_user_id, created_at, updated_at, oauth_app_client_id
            FROM user_oauth_tokens
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query user OAuth token: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| Ok(Some(self.row_to_user_oauth_token(&row)?)),
        )
    }

    /// Get all OAuth tokens for a user, optionally scoped to a specific tenant
    ///
    /// When `tenant_id` is `Some`, only tokens for that tenant are returned.
    /// When `None`, returns tokens across all tenants (intentional cross-tenant view
    /// for OAuth status checks, e.g. admin dashboards).
    ///
    /// Decrypts provider tokens using AAD binding for security.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database query fails
    /// - Decryption fails for any token
    pub async fn get_user_oauth_tokens_impl(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<UserOAuthToken>> {
        let rows = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, access_token, refresh_token,
                       token_type, expires_at, scope, provider_user_id, created_at, updated_at, oauth_app_client_id
                FROM user_oauth_tokens
                WHERE user_id = $1 AND tenant_id = $2
                ORDER BY created_at DESC
                ",
            )
            .bind(user_id.to_string())
            .bind(tid.to_string())
            .fetch_all(&self.pool)
            .await
        } else {
            // Intentional cross-tenant view for OAuth status checks (e.g. admin dashboards)
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, access_token, refresh_token,
                       token_type, expires_at, scope, provider_user_id, created_at, updated_at, oauth_app_client_id
                FROM user_oauth_tokens
                WHERE user_id = $1
                ORDER BY created_at DESC
                ",
            )
            .bind(user_id.to_string())
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to query user OAuth tokens: {e}")))?;

        let mut tokens = Vec::with_capacity(rows.len());
        for row in rows {
            tokens.push(self.row_to_user_oauth_token(&row)?);
        }
        Ok(tokens)
    }

    /// Get OAuth tokens for a tenant and provider
    ///
    /// Decrypts provider tokens using AAD binding for security.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database query fails
    /// - Decryption fails for any token
    pub async fn get_tenant_provider_tokens(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Vec<UserOAuthToken>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, provider, access_token, refresh_token,
                   token_type, expires_at, scope, provider_user_id, created_at, updated_at, oauth_app_client_id
            FROM user_oauth_tokens
            WHERE tenant_id = $1 AND provider = $2
            ORDER BY created_at DESC
            ",
        )
        .bind(tenant_id)
        .bind(provider)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query tenant provider tokens: {e}")))?;

        let mut tokens = Vec::with_capacity(rows.len());
        for row in rows {
            tokens.push(self.row_to_user_oauth_token(&row)?);
        }
        Ok(tokens)
    }

    /// Resolve the `(user_id, tenant_id)` owning a provider-side account id.
    ///
    /// Looks up the single active token whose `provider_user_id` matches (e.g. a
    /// Strava athlete id from a webhook `owner_id`). Returns `None` when no token
    /// carries that id.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails or the stored `user_id` is not
    /// a valid UUID.
    pub async fn find_user_by_provider_user_id(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> AppResult<Option<(Uuid, String)>> {
        let row = sqlx::query(
            r"
            SELECT user_id, tenant_id
            FROM user_oauth_tokens
            WHERE provider = $1 AND provider_user_id = $2
            LIMIT 1
            ",
        )
        .bind(provider)
        .bind(provider_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to look up user by provider_user_id: {e}"))
        })?;

        match row {
            Some(row) => {
                let user_id_str: String = row.get("user_id");
                let user_id = Uuid::parse_str(&user_id_str)?;
                let tenant_id: String = row.get("tenant_id");
                Ok(Some((user_id, tenant_id)))
            }
            None => Ok(None),
        }
    }

    /// Count distinct users occupying a shared-app OAuth seat for `provider`.
    ///
    /// A "seat" is one athlete connected through the platform's shared OAuth
    /// application. Users who registered their own (BYO) OAuth app — a row in
    /// `user_oauth_app_credentials` for the same provider — run on their own
    /// athlete quota and never consume a shared seat, so they are excluded.
    ///
    /// The count is intentionally cross-tenant: the shared app's athlete cap is
    /// a single global limit enforced by the upstream provider across every
    /// tenant that uses it.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn count_shared_app_seat_usage(&self, provider: &str) -> AppResult<u32> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(DISTINCT t.user_id)
            FROM user_oauth_tokens t
            WHERE t.provider = $1
              AND NOT EXISTS (
                  SELECT 1 FROM user_oauth_app_credentials a
                  WHERE a.user_id = t.user_id AND a.provider = $2
              )
            ",
        )
        .bind(provider)
        .bind(provider)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count shared-app OAuth seats: {e}")))?;

        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// Delete a specific user OAuth token
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn delete_user_oauth_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            DELETE FROM user_oauth_tokens
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(provider)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete user OAuth token: {e}")))?;

        Ok(())
    }

    /// Delete all OAuth tokens for a user within a specific tenant scope
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn delete_user_oauth_tokens_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            DELETE FROM user_oauth_tokens
            WHERE user_id = $1 AND tenant_id = $2
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete user OAuth tokens: {e}")))?;

        Ok(())
    }

    /// Refresh a user OAuth token
    ///
    /// Encrypts new tokens using AES-256-GCM with AAD binding.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Encryption fails
    /// - Database query fails
    pub async fn refresh_user_oauth_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        // Create AAD context: tenant_id|user_id|provider|table
        let aad_context = format!("{tenant_id}|{user_id}|{provider}|user_oauth_tokens");

        // Encrypt new access token
        let encrypted_access_token = self.encrypt_data_with_aad(access_token, &aad_context)?;

        // Encrypt new refresh token if present
        let encrypted_refresh_token = refresh_token
            .map(|rt| self.encrypt_data_with_aad(rt, &aad_context))
            .transpose()?;

        sqlx::query(
            r"
            UPDATE user_oauth_tokens
            SET access_token = $4,
                refresh_token = $5,
                expires_at = $6,
                updated_at = $7
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(provider)
        .bind(&encrypted_access_token)
        .bind(encrypted_refresh_token.as_deref())
        .bind(expires_at)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to refresh user OAuth token: {e}")))?;

        Ok(())
    }

    /// Convert a database row to a `UserOAuthToken`
    ///
    /// Decrypts provider tokens using AAD binding.
    ///
    /// # Errors
    ///
    /// Returns an error if decryption fails (possibly due to tampered data or AAD mismatch)
    fn row_to_user_oauth_token(&self, row: &SqliteRow) -> AppResult<UserOAuthToken> {
        let user_id_str: String = row.get("user_id");
        let user_id = Uuid::parse_str(&user_id_str)?;
        let tenant_id: String = row.get("tenant_id");
        let provider: String = row.get("provider");

        // Create AAD context: tenant_id|user_id|provider|table
        let aad_context = format!("{tenant_id}|{user_id}|{provider}|user_oauth_tokens");

        // Decrypt access token
        let encrypted_access_token: String = row.get("access_token");
        let access_token = self.decrypt_data_with_aad(&encrypted_access_token, &aad_context)?;

        // Decrypt refresh token if present
        let encrypted_refresh_token: Option<String> = row.get("refresh_token");
        let refresh_token = encrypted_refresh_token
            .as_deref()
            .map(|ert| self.decrypt_data_with_aad(ert, &aad_context))
            .transpose()?;

        Ok(UserOAuthToken {
            id: row.get("id"),
            user_id,
            tenant_id,
            provider,
            access_token,
            refresh_token,
            token_type: row.get("token_type"),
            expires_at: row.get("expires_at"),
            scope: row.get::<Option<String>, _>("scope"),
            provider_user_id: row.get::<Option<String>, _>("provider_user_id"),
            oauth_app_client_id: row.get::<Option<String>, _>("oauth_app_client_id"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
    }
    // Public wrapper methods (delegate to _impl versions)

    /// Get user OAuth tokens, optionally scoped to a tenant (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn get_user_oauth_tokens(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<UserOAuthToken>> {
        self.get_user_oauth_tokens_impl(user_id, tenant_id).await
    }

    /// Delete user OAuth tokens within a tenant scope (public API)
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn delete_user_oauth_tokens(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<()> {
        self.delete_user_oauth_tokens_impl(user_id, tenant_id).await
    }
}

#[async_trait]
impl OAuthTokenRepository for Database {
    async fn upsert_token(&self, token: &UserOAuthToken) -> AppResult<()> {
        use super::user_oauth_tokens::OAuthTokenData;

        let tenant_id = TenantId::parse_str(&token.tenant_id)
            .map_err(|e| AppError::internal(format!("Invalid tenant_id in OAuth token: {e}")))?;

        let token_data = OAuthTokenData {
            id: &token.id,
            user_id: token.user_id,
            tenant_id,
            provider: &token.provider,
            access_token: &token.access_token,
            refresh_token: token.refresh_token.as_deref(),
            token_type: &token.token_type,
            expires_at: token.expires_at,
            scope: token.scope.as_deref().unwrap_or(""),
            provider_user_id: token.provider_user_id.as_deref(),
            oauth_app_client_id: token.oauth_app_client_id.as_deref(),
        };

        Self::upsert_user_oauth_token(self, &token_data).await
    }
    async fn get_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<UserOAuthToken>> {
        Self::get_user_oauth_token(self, user_id, tenant_id, provider).await
    }
    async fn get_tokens(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<UserOAuthToken>> {
        Self::get_user_oauth_tokens_impl(self, user_id, tenant_id).await
    }
    async fn get_tenant_provider_tokens(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Vec<UserOAuthToken>> {
        Self::get_tenant_provider_tokens(self, tenant_id, provider).await
    }
    async fn find_user_by_provider_user_id(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> AppResult<Option<(Uuid, String)>> {
        Self::find_user_by_provider_user_id(self, provider, provider_user_id).await
    }
    async fn count_shared_app_seat_usage(&self, provider: &str) -> AppResult<u32> {
        Self::count_shared_app_seat_usage(self, provider).await
    }

    async fn list_strava_pool_apps(&self, only_enabled: bool) -> AppResult<Vec<StravaPoolApp>> {
        let sql = if only_enabled {
            "SELECT client_id, seat_cap, enabled, label, created_at, updated_at \
             FROM strava_oauth_app_pool WHERE enabled = 1 ORDER BY created_at"
        } else {
            "SELECT client_id, seat_cap, enabled, label, created_at, updated_at \
             FROM strava_oauth_app_pool ORDER BY created_at"
        };
        let rows = sqlx::query(sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to list Strava pool apps: {e}")))?;
        Ok(rows.iter().map(row_to_strava_pool_app).collect())
    }

    async fn get_strava_pool_app_secret(&self, client_id: &str) -> AppResult<Option<String>> {
        let row = sqlx::query(
            "SELECT client_secret_encrypted FROM strava_oauth_app_pool WHERE client_id = ?1",
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to load Strava pool app secret: {e}")))?;
        match row {
            Some(row) => {
                let enc: String = row.get("client_secret_encrypted");
                let aad = format!("strava_oauth_app_pool|{client_id}");
                Ok(Some(self.decrypt_data_with_aad(&enc, &aad)?))
            }
            None => Ok(None),
        }
    }

    async fn count_strava_seat_usage_by_app(&self) -> AppResult<Vec<(Option<String>, u32)>> {
        let rows = sqlx::query(
            r"
            SELECT t.oauth_app_client_id AS app, COUNT(DISTINCT t.user_id) AS n
            FROM user_oauth_tokens t
            WHERE t.provider = 'strava'
              AND NOT EXISTS (
                  SELECT 1 FROM user_oauth_app_credentials a
                  WHERE a.user_id = t.user_id AND a.provider = 'strava'
              )
            GROUP BY t.oauth_app_client_id
            ",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to count Strava seat usage by app: {e}"))
        })?;
        Ok(rows
            .iter()
            .map(|row| {
                let app: Option<String> = row.get("app");
                let n: i64 = row.get("n");
                (app, u32::try_from(n).unwrap_or(u32::MAX))
            })
            .collect())
    }

    async fn upsert_strava_pool_app(
        &self,
        client_id: &str,
        client_secret: &str,
        seat_cap: u32,
        label: Option<&str>,
    ) -> AppResult<()> {
        let aad = format!("strava_oauth_app_pool|{client_id}");
        let enc = self.encrypt_data_with_aad(client_secret, &aad)?;
        let now = Utc::now().timestamp();
        sqlx::query(
            r"
            INSERT INTO strava_oauth_app_pool (client_id, client_secret_encrypted, seat_cap, enabled, label, created_at, updated_at)
            VALUES (?1, ?2, ?3, 1, ?4, ?5, ?5)
            ON CONFLICT (client_id) DO UPDATE SET
                client_secret_encrypted = excluded.client_secret_encrypted,
                seat_cap = excluded.seat_cap,
                label = excluded.label,
                updated_at = excluded.updated_at
            ",
        )
        .bind(client_id)
        .bind(&enc)
        .bind(i64::from(seat_cap))
        .bind(label)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert Strava pool app: {e}")))?;
        Ok(())
    }

    async fn set_strava_pool_app_enabled(&self, client_id: &str, enabled: bool) -> AppResult<()> {
        sqlx::query(
            "UPDATE strava_oauth_app_pool SET enabled = ?1, updated_at = ?2 WHERE client_id = ?3",
        )
        .bind(i64::from(enabled))
        .bind(Utc::now().timestamp())
        .bind(client_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update Strava pool app: {e}")))?;
        Ok(())
    }

    async fn delete_strava_pool_app(&self, client_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM strava_oauth_app_pool WHERE client_id = ?1")
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to delete Strava pool app: {e}")))?;
        Ok(())
    }

    async fn delete_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        Self::delete_user_oauth_token(self, user_id, tenant_id, provider).await
    }
    async fn delete_tokens(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()> {
        Self::delete_user_oauth_tokens_impl(self, user_id, tenant_id).await
    }
    async fn refresh_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        Self::refresh_user_oauth_token(
            self,
            user_id,
            tenant_id,
            provider,
            access_token,
            refresh_token,
            expires_at,
        )
        .await
    }
    async fn store_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> AppResult<()> {
        Self::store_user_oauth_app_impl(
            self,
            user_id,
            provider,
            client_id,
            client_secret,
            redirect_uri,
        )
        .await
    }
    async fn get_user_oauth_app(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> AppResult<Option<UserOAuthApp>> {
        Self::get_user_oauth_app_impl(self, user_id, provider).await
    }
    async fn list_user_oauth_apps(&self, user_id: Uuid) -> AppResult<Vec<UserOAuthApp>> {
        Self::list_user_oauth_apps_impl(self, user_id).await
    }
    async fn remove_user_oauth_app(&self, user_id: Uuid, provider: &str) -> AppResult<()> {
        Self::remove_user_oauth_app_impl(self, user_id, provider).await
    }
    async fn get_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<DateTime<Utc>>> {
        Self::get_provider_last_sync(self, user_id, tenant_id, provider).await
    }
    async fn update_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        sync_time: DateTime<Utc>,
    ) -> AppResult<()> {
        Self::update_provider_last_sync(self, user_id, tenant_id, provider, sync_time).await
    }
}
