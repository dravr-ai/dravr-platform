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
use crate::backends::shared::{self, encryption::HasEncryption};
use crate::column_decode::uuid_column;
use crate::database::password_reset_tokens::RESET_MAX_VERIFY_ATTEMPTS;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuthClientState;
use pierre_core::models::TenantId;
use pierre_core::models::{
    AuthorizationCode, ConnectionStatus, ConnectionType, OAuthClientGrant, ProviderConnection,
    StravaPoolApp, UserOAuthApp, UserOAuthToken,
};
use pierre_core::models::{
    DeviceAuthorization, OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State,
};
use sqlx::postgres::PgRow;
use sqlx::Row;
use tracing::warn;
use uuid::Uuid;

/// Map a `oauth_client_grants` row into an [`OAuthClientGrant`].
///
/// Extracts every column via `try_get` (never the panicking `r.get()`): all id
/// columns are TEXT and the timestamps are TIMESTAMPTZ, so a schema/type skew
/// surfaces as a structured `AppError` rather than a decode panic.
fn row_to_client_grant(row: &PgRow) -> AppResult<OAuthClientGrant> {
    Ok(OAuthClientGrant {
        id: row
            .try_get("id")
            .map_err(|e| AppError::database(format!("Failed to parse id column: {e}")))?,
        user_id: row
            .try_get("user_id")
            .map_err(|e| AppError::database(format!("Failed to parse user_id column: {e}")))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| AppError::database(format!("Failed to parse tenant_id column: {e}")))?,
        client_id: row
            .try_get("client_id")
            .map_err(|e| AppError::database(format!("Failed to parse client_id column: {e}")))?,
        scope: row
            .try_get("scope")
            .map_err(|e| AppError::database(format!("Failed to parse scope column: {e}")))?,
        granted_at: row
            .try_get("granted_at")
            .map_err(|e| AppError::database(format!("Failed to parse granted_at column: {e}")))?,
        revoked_at: row
            .try_get("revoked_at")
            .map_err(|e| AppError::database(format!("Failed to parse revoked_at column: {e}")))?,
    })
}

#[async_trait]
impl OAuthTokenRepository for PostgresDatabase {
    async fn get_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<DateTime<Utc>>> {
        // Outer Option: row may not exist. Inner Option: last_sync column is
        // nullable (NULL until the first successful sync) — decoding it as a
        // bare DateTime errors with "unexpected null" on never-synced rows.
        let last_sync: Option<Option<DateTime<Utc>>> = sqlx::query_scalar(
            "SELECT last_sync FROM user_oauth_tokens WHERE user_id = $1 AND tenant_id = $2 AND provider = $3",
        )
        .bind(user_id)
        .bind(tenant_id.to_string())
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get provider last sync: {e}")))?;

        Ok(last_sync.flatten())
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
        .bind(token.provider_user_id.as_deref())
        .bind(token.oauth_app_client_id.as_deref())
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
                   token_type, expires_at, scope, provider_user_id, created_at, updated_at, oauth_app_client_id
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
                       token_type, expires_at, scope, provider_user_id, created_at, updated_at, oauth_app_client_id
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
                       token_type, expires_at, scope, provider_user_id, created_at, updated_at, oauth_app_client_id
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
                   token_type, expires_at, scope, provider_user_id, created_at, updated_at, oauth_app_client_id
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

    async fn find_user_by_provider_user_id(
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

        // Extract via try_get (never the panicking r.get()): user_id is a native
        // UUID column, tenant_id is TEXT.
        match row {
            Some(row) => {
                let user_id: Uuid = row.try_get("user_id").map_err(|e| {
                    AppError::database(format!("Failed to parse user_id column: {e}"))
                })?;
                let tenant_id: String = row.try_get("tenant_id").map_err(|e| {
                    AppError::database(format!("Failed to parse tenant_id column: {e}"))
                })?;
                Ok(Some((user_id, tenant_id)))
            }
            None => Ok(None),
        }
    }

    async fn count_shared_app_seat_usage(&self, provider: &str) -> AppResult<u32> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(DISTINCT t.user_id)
            FROM user_oauth_tokens t
            WHERE t.provider = $1
              AND NOT EXISTS (
                  SELECT 1 FROM user_oauth_app_credentials a
                  WHERE a.user_id = t.user_id AND a.provider = $1
              )
            ",
        )
        .bind(provider)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count shared-app OAuth seats: {e}")))?;

        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    async fn list_strava_pool_apps(&self, only_enabled: bool) -> AppResult<Vec<StravaPoolApp>> {
        let sql = if only_enabled {
            "SELECT client_id, seat_cap, enabled, label, created_at, updated_at \
             FROM strava_oauth_app_pool WHERE enabled = TRUE ORDER BY created_at"
        } else {
            "SELECT client_id, seat_cap, enabled, label, created_at, updated_at \
             FROM strava_oauth_app_pool ORDER BY created_at"
        };
        let rows = sqlx::query(sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to list Strava pool apps: {e}")))?;
        rows.iter()
            .map(|row| {
                let seat_cap: i32 = row.try_get("seat_cap").map_err(|e| {
                    AppError::database(format!("Failed to parse seat_cap column: {e}"))
                })?;
                Ok(StravaPoolApp {
                    client_id: row.try_get("client_id").map_err(|e| {
                        AppError::database(format!("Failed to parse client_id column: {e}"))
                    })?,
                    seat_cap: u32::try_from(seat_cap).unwrap_or(0),
                    enabled: row.try_get("enabled").map_err(|e| {
                        AppError::database(format!("Failed to parse enabled column: {e}"))
                    })?,
                    label: row.try_get("label").ok(),
                    created_at: row.try_get("created_at").map_err(|e| {
                        AppError::database(format!("Failed to parse created_at column: {e}"))
                    })?,
                    updated_at: row.try_get("updated_at").map_err(|e| {
                        AppError::database(format!("Failed to parse updated_at column: {e}"))
                    })?,
                })
            })
            .collect()
    }

    async fn get_strava_pool_app_secret(&self, client_id: &str) -> AppResult<Option<String>> {
        let row = sqlx::query(
            "SELECT client_secret_encrypted FROM strava_oauth_app_pool WHERE client_id = $1",
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to load Strava pool app secret: {e}")))?;
        match row {
            Some(row) => {
                let enc: String = row.try_get("client_secret_encrypted").map_err(|e| {
                    AppError::database(format!("Failed to parse client_secret column: {e}"))
                })?;
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
        rows.iter()
            .map(|row| {
                let app: Option<String> = row.try_get("app").ok();
                let n: i64 = row.try_get("n").map_err(|e| {
                    AppError::database(format!("Failed to parse count column: {e}"))
                })?;
                Ok((app, u32::try_from(n).unwrap_or(u32::MAX)))
            })
            .collect()
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
        let now = chrono::Utc::now().timestamp();
        let seat_cap = i32::try_from(seat_cap).unwrap_or(i32::MAX);
        sqlx::query(
            r"
            INSERT INTO strava_oauth_app_pool (client_id, client_secret_encrypted, seat_cap, enabled, label, created_at, updated_at)
            VALUES ($1, $2, $3, TRUE, $4, $5, $5)
            ON CONFLICT (client_id) DO UPDATE SET
                client_secret_encrypted = EXCLUDED.client_secret_encrypted,
                seat_cap = EXCLUDED.seat_cap,
                label = EXCLUDED.label,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(client_id)
        .bind(&enc)
        .bind(seat_cap)
        .bind(label)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert Strava pool app: {e}")))?;
        Ok(())
    }

    async fn set_strava_pool_app_enabled(&self, client_id: &str, enabled: bool) -> AppResult<()> {
        sqlx::query(
            "UPDATE strava_oauth_app_pool SET enabled = $1, updated_at = $2 WHERE client_id = $3",
        )
        .bind(enabled)
        .bind(chrono::Utc::now().timestamp())
        .bind(client_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update Strava pool app: {e}")))?;
        Ok(())
    }

    async fn delete_strava_pool_app(&self, client_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM strava_oauth_app_pool WHERE client_id = $1")
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to delete Strava pool app: {e}")))?;
        Ok(())
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
        // Insert or update OAuth app credentials
        sqlx::query(
            r"
            INSERT INTO user_oauth_app_credentials (id, user_id, provider, client_id, client_secret, redirect_uri, created_at, updated_at)
            VALUES (gen_random_uuid()::TEXT, $1, $2, $3, $4, $5, NOW(), NOW())
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
            FROM user_oauth_app_credentials
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
            FROM user_oauth_app_credentials
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
            DELETE FROM user_oauth_app_credentials
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

    async fn store_client_grant(&self, grant: &OAuthClientGrant) -> AppResult<()> {
        // Active partial-unique index makes re-consent a no-op via ON CONFLICT DO NOTHING.
        sqlx::query(
            r"
            INSERT INTO oauth_client_grants
                (id, user_id, tenant_id, client_id, scope, granted_at, revoked_at)
            VALUES ($1, $2, $3, $4, $5, NOW(), NULL)
            ON CONFLICT (user_id, tenant_id, client_id, scope) WHERE revoked_at IS NULL DO NOTHING
            ",
        )
        .bind(&grant.id)
        .bind(&grant.user_id)
        .bind(&grant.tenant_id)
        .bind(&grant.client_id)
        .bind(&grant.scope)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store OAuth client grant: {e}")))?;

        Ok(())
    }

    async fn find_active_client_grant(
        &self,
        user_id: &str,
        tenant_id: &str,
        client_id: &str,
        scope: &str,
    ) -> AppResult<Option<OAuthClientGrant>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND client_id = $3 AND scope = $4
              AND revoked_at IS NULL
            LIMIT 1
            ",
        )
        .bind(user_id)
        .bind(tenant_id)
        .bind(client_id)
        .bind(scope)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query OAuth client grant: {e}")))?;

        row.map(|row| row_to_client_grant(&row)).transpose()
    }

    async fn list_client_grants(
        &self,
        user_id: &str,
        tenant_id: &str,
    ) -> AppResult<Vec<OAuthClientGrant>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND revoked_at IS NULL
            ORDER BY granted_at DESC
            ",
        )
        .bind(user_id)
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list OAuth client grants: {e}")))?;

        let mut grants = Vec::with_capacity(rows.len());
        for row in &rows {
            grants.push(row_to_client_grant(row)?);
        }
        Ok(grants)
    }

    async fn revoke_client_grant(
        &self,
        id: &str,
        user_id: &str,
        tenant_id: &str,
    ) -> AppResult<bool> {
        // user_id + tenant_id in the WHERE clause is the ownership check.
        let result = sqlx::query(
            r"
            UPDATE oauth_client_grants
            SET revoked_at = NOW()
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3 AND revoked_at IS NULL
            ",
        )
        .bind(id)
        .bind(user_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to revoke OAuth client grant: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn create_device_authorization(&self, da: &DeviceAuthorization) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO device_authorization
                (device_code_hash, user_code, status, approved_by, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ",
        )
        .bind(&da.device_code_hash)
        .bind(&da.user_code)
        .bind(&da.status)
        .bind(&da.approved_by)
        .bind(da.created_at)
        .bind(da.expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store device authorization: {e}")))?;
        Ok(())
    }

    async fn get_device_authorization_by_code_hash(
        &self,
        device_code_hash: &str,
    ) -> AppResult<Option<DeviceAuthorization>> {
        let row = sqlx::query(
            "SELECT device_code_hash, user_code, status, approved_by, created_at, expires_at \
             FROM device_authorization WHERE device_code_hash = $1",
        )
        .bind(device_code_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to load device authorization: {e}")))?;
        row.as_ref().map(row_to_device_authorization).transpose()
    }

    async fn get_device_authorization_by_user_code(
        &self,
        user_code: &str,
    ) -> AppResult<Option<DeviceAuthorization>> {
        let row = sqlx::query(
            "SELECT device_code_hash, user_code, status, approved_by, created_at, expires_at \
             FROM device_authorization WHERE user_code = $1",
        )
        .bind(user_code)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to load device authorization by user_code: {e}"
            ))
        })?;
        row.as_ref().map(row_to_device_authorization).transpose()
    }

    async fn approve_device_authorization(
        &self,
        user_code: &str,
        approved_by: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE device_authorization
            SET status = 'approved', approved_by = $2
            WHERE user_code = $1 AND status = 'pending'
            ",
        )
        .bind(user_code)
        .bind(approved_by)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to approve device authorization: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn deny_device_authorization(&self, user_code: &str) -> AppResult<bool> {
        let result = sqlx::query(
            "UPDATE device_authorization SET status = 'denied' \
             WHERE user_code = $1 AND status = 'pending'",
        )
        .bind(user_code)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to deny device authorization: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_device_authorization(&self, device_code_hash: &str) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM device_authorization WHERE device_code_hash = $1")
            .bind(device_code_hash)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to delete device authorization: {e}"))
            })?;
        Ok(result.rows_affected() > 0)
    }

    // ================================
    // OAuth Client State (CSRF + PKCE)
    // ================================
}

/// Map a `device_authorization` `PostgreSQL` row to a [`DeviceAuthorization`].
fn row_to_device_authorization(row: &PgRow) -> AppResult<DeviceAuthorization> {
    Ok(DeviceAuthorization {
        device_code_hash: row.try_get("device_code_hash").map_err(|e| {
            AppError::database(format!("Failed to parse device_code_hash column: {e}"))
        })?,
        user_code: row
            .try_get("user_code")
            .map_err(|e| AppError::database(format!("Failed to parse user_code column: {e}")))?,
        status: row
            .try_get("status")
            .map_err(|e| AppError::database(format!("Failed to parse status column: {e}")))?,
        approved_by: row.try_get("approved_by").ok(),
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::database(format!("Failed to parse created_at column: {e}")))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| AppError::database(format!("Failed to parse expires_at column: {e}")))?,
    })
}

#[async_trait]
impl OAuthClientStateRepository for PostgresDatabase {
    async fn store_oauth_client_state(&self, state: &OAuthClientState) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO oauth_client_states (state, provider, user_id, tenant_id, redirect_uri, scope, pkce_code_verifier, created_at, expires_at, used, oauth_app_client_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&state.state)
        .bind(&state.provider)
        .bind(state.user_id.map(|u| u.to_string()))
        .bind(&state.tenant_id)
        .bind(&state.redirect_uri)
        .bind(&state.scope)
        .bind(&state.pkce_code_verifier)
        .bind(state.created_at)
        .bind(state.expires_at)
        .bind(state.used)
        .bind(&state.oauth_app_client_id)
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
             RETURNING state, provider, user_id, tenant_id, redirect_uri, scope, pkce_code_verifier, created_at, expires_at, used, oauth_app_client_id",
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
                user_id: row
                    .try_get::<Option<String>, _>("user_id")
                    .map_err(|e| {
                        AppError::database(format!("Failed to parse user_id column: {e}"))
                    })?
                    .and_then(|s| Uuid::parse_str(&s).ok()),
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
                oauth_app_client_id: row.try_get("oauth_app_client_id").ok(),
                created_at: row.try_get::<DateTime<Utc>, _>("created_at").map_err(|e| {
                    AppError::database(format!("Failed to parse created_at column: {e}"))
                })?,
                expires_at: row.try_get::<DateTime<Utc>, _>("expires_at").map_err(|e| {
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
                metadata = EXCLUDED.metadata,
                status = 'active',
                status_changed_at = EXCLUDED.connected_at,
                last_error = NULL,
                notified_at = NULL
            ",
        )
        .bind(&id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
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
        .bind(tenant_id.to_string())
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
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, status, metadata
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
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, status, metadata
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
            let last_used_at: Option<DateTime<Utc>> = row.try_get("last_used_at").ok().flatten();

            let user_id_from_db: String = row.get("user_id");
            let parsed_user_id = uuid_column("provider_connections.user_id", &user_id_from_db)?;

            connections.push(ProviderConnection {
                id: row.get("id"),
                user_id: parsed_user_id,
                tenant_id: row.get("tenant_id"),
                provider: row.get("provider"),
                connection_type: ConnectionType::from_str_value(&conn_type_str)
                    .unwrap_or(ConnectionType::Manual),
                connected_at,
                last_used_at,
                status: ConnectionStatus::from_str_value(&row.get::<String, _>("status")),
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

    async fn touch_last_used(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE provider_connections
               SET last_used_at = $1
             WHERE user_id = $2 AND tenant_id = $3 AND provider = $4
            ",
        )
        .bind(Utc::now())
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(provider)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn resolve_most_recent(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<ProviderConnection>> {
        let row_opt = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, status, metadata
                  FROM provider_connections
                 WHERE user_id = $1 AND tenant_id = $2
                 ORDER BY CASE WHEN status = 'active' THEN 0 ELSE 1 END, last_used_at DESC NULLS LAST, connected_at DESC
                 LIMIT 1
                ",
            )
            .bind(user_id.to_string())
            .bind(tid.to_string())
            .fetch_optional(&self.pool)
            .await?
        } else {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, status, metadata
                  FROM provider_connections
                 WHERE user_id = $1
                 ORDER BY CASE WHEN status = 'active' THEN 0 ELSE 1 END, last_used_at DESC NULLS LAST, connected_at DESC
                 LIMIT 1
                ",
            )
            .bind(user_id.to_string())
            .fetch_optional(&self.pool)
            .await?
        };

        let Some(row) = row_opt else {
            return Ok(None);
        };

        let conn_type_str: String = row.get("connection_type");
        let connected_at: DateTime<Utc> = row.get("connected_at");
        let last_used_at: Option<DateTime<Utc>> = row.try_get("last_used_at").ok().flatten();
        let user_id_from_db: String = row.get("user_id");
        let parsed_user_id = uuid_column("provider_connections.user_id", &user_id_from_db)?;

        Ok(Some(ProviderConnection {
            id: row.get("id"),
            user_id: parsed_user_id,
            tenant_id: row.get("tenant_id"),
            provider: row.get("provider"),
            connection_type: ConnectionType::from_str_value(&conn_type_str)
                .unwrap_or(ConnectionType::Manual),
            connected_at,
            last_used_at,
            status: ConnectionStatus::from_str_value(&row.get::<String, _>("status")),
            metadata: row.get("metadata"),
        }))
    }

    async fn mark_needs_reauth(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        error_code: Option<&str>,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE provider_connections
               SET status = 'needs_reauth',
                   status_changed_at = $1,
                   last_error = $2
             WHERE user_id = $3 AND tenant_id = $4 AND provider = $5
               AND status != 'needs_reauth'
            ",
        )
        .bind(Utc::now())
        .bind(error_code)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(provider)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn mark_active(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE provider_connections
               SET status = 'active',
                   status_changed_at = $1,
                   last_error = NULL,
                   notified_at = NULL
             WHERE user_id = $2 AND tenant_id = $3 AND provider = $4
               AND status != 'active'
            ",
        )
        .bind(Utc::now())
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(provider)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn claim_reauth_notification(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE provider_connections
               SET notified_at = $1
             WHERE user_id = $2 AND tenant_id = $3 AND provider = $4
               AND status = 'needs_reauth'
               AND notified_at IS NULL
            ",
        )
        .bind(Utc::now())
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(provider)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}

#[async_trait]
impl PasswordResetRepository for PostgresDatabase {
    async fn store_token(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        created_by: &str,
    ) -> AppResult<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(1);

        sqlx::query(
            r"
            INSERT INTO password_reset_tokens (id, user_id, selector, token_hash, expires_at, created_by, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ",
        )
        .bind(id.to_string())
        .bind(user_id)
        .bind(selector)
        .bind(verifier_hash)
        .bind(expires_at)
        .bind(created_by)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store password reset token: {e}")))?;

        Ok(id)
    }

    async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid> {
        // Uniform error for every failure mode so the endpoint never reveals which hit.
        let invalid =
            || AppError::not_found("Password reset token is invalid, expired, or already used");
        let now = Utc::now();

        // Look up the single live token for this selector — no global token_hash scan.
        let row = sqlx::query(
            r"
            SELECT user_id, token_hash, attempt_count
            FROM password_reset_tokens
            WHERE selector = $1
              AND used_at IS NULL
              AND expires_at > $2
            ",
        )
        .bind(selector)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to load reset token: {e}")))?;

        let Some(row) = row else {
            return Err(invalid());
        };
        let user_id: Uuid = row.get("user_id");
        let stored_hash: String = row.get("token_hash");
        // PG INTEGER maps to i32.
        let attempts: i32 = row.get("attempt_count");

        // Brute-force lockout: past the attempt cap, invalidate the token outright.
        if i64::from(attempts) >= RESET_MAX_VERIFY_ATTEMPTS {
            sqlx::query("UPDATE password_reset_tokens SET used_at = $1 WHERE selector = $2")
                .bind(now)
                .bind(selector)
                .execute(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to lock reset token: {e}")))?;
            return Err(invalid());
        }

        // Compare SHA-256 hashes (not the secret): a timing side-channel on the hash
        // cannot forge the verifier without a preimage, so a plain compare is safe. A
        // wrong guess costs one attempt and does NOT consume the token.
        if stored_hash != verifier_hash {
            sqlx::query(
                "UPDATE password_reset_tokens SET attempt_count = attempt_count + 1 WHERE selector = $1",
            )
            .bind(selector)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to record reset attempt: {e}")))?;
            return Err(invalid());
        }

        // Success: atomically claim the token. The `used_at IS NULL` guard makes this
        // exactly-once — a concurrent request that read the same live row loses the race
        // here (0 rows) and is rejected, preventing token replay.
        let claimed = sqlx::query(
            "UPDATE password_reset_tokens SET used_at = $1 WHERE selector = $2 AND used_at IS NULL",
        )
        .bind(now)
        .bind(selector)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to consume reset token: {e}")))?;

        if claimed.rows_affected() == 0 {
            return Err(invalid());
        }

        Ok(user_id)
    }

    async fn store_token_with_ttl(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        created_by: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + chrono::Duration::minutes(ttl_minutes);

        sqlx::query(
            r"
            INSERT INTO password_reset_tokens (id, user_id, selector, token_hash, expires_at, created_by, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ",
        )
        .bind(id.to_string())
        .bind(user_id)
        .bind(selector)
        .bind(verifier_hash)
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
        .bind(user_id)
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
        .bind(user_id)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count recent reset tokens: {e}")))?;

        Ok(row.get::<i64, _>("cnt"))
    }
}
