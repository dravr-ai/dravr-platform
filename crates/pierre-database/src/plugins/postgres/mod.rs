// ABOUTME: PostgreSQL database implementation for cloud and production deployments
// ABOUTME: Provides enterprise-grade database support with connection pooling and scalability
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//! `PostgreSQL` database implementation
//!
//! This module provides `PostgreSQL` support for cloud deployments,
//! implementing the same interface as the `SQLite` version.

/// A2A protocol repository implementations
pub mod a2a;
/// Admin, impersonation, and MCP token repository implementations
pub mod admin;
/// API key repository implementation
pub mod api_key;
/// Chat repository implementation
pub mod chat;
/// Coaches repository implementation
pub mod coaches;
/// Encryption support (AES-256-GCM)
pub mod encryption;
/// Messaging gateway repository implementations
pub mod messaging;
/// Mobility repository implementation (stretching exercises and yoga poses)
pub mod mobility;
/// OAuth token and authorization repository implementations
pub mod oauth;
/// Recipe repository implementation (CRUD with nutrition caching)
pub mod recipes;
/// Security and notification repository implementations
pub mod security;
/// Seeder repository for seed-only database operations
pub mod seeder;
/// Social insight repository implementation
pub mod social;
/// Store listings repository implementation (marketplace publishing workflow)
pub mod store_listings;
/// Tenant, tool selection, LLM credential, and fitness config repositories
pub mod tenant;
/// Usage tracking repository implementations
pub mod usage;
/// User and profile repository implementations
pub mod user;

use super::{shared, DatabaseProvider};
use crate::database::system_settings::{
    SystemSetting, SETTING_AUTO_APPROVAL_ENABLED, SETTING_SOCIAL_INSIGHTS_CONFIG,
};
use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as Base64Engine;
use chrono::{DateTime, Utc};
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::config::social::SocialInsightsConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::a2a::{A2ATask, TaskStatus};
use pierre_core::models::TenantId;
use pierre_core::models::TenantToolOverride;
use pierre_core::models::{TenantPlan, ToolCatalogEntry, ToolCategory, User, UserOAuthToken};
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{Pool, Postgres, Row};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

/// `PostgreSQL` database implementation
#[derive(Clone)]
pub struct PostgresDatabase {
    pool: Pool<Postgres>,
    encryption_key: Vec<u8>,
}

impl PostgresDatabase {
    /// Get a reference to the `PostgreSQL` connection pool
    #[must_use]
    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    /// Close the database connection pool
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Update the encryption key used for token encryption/decryption
    ///
    /// This is called after the actual DEK is loaded from the database during
    /// two-tier key management initialization. The database is initially created
    /// with a temporary key, then updated with the real key once it's loaded.
    ///
    /// # Safety
    /// Only call this once during startup, before any encrypted data operations.
    pub fn update_encryption_key(&mut self, new_key: Vec<u8>) {
        self.encryption_key = new_key;
    }

    /// Helper function to parse User from database row
    fn parse_user_from_row(row: &PgRow) -> AppResult<User> {
        shared::mappers::parse_user_from_row(row)
    }

    /// Helper function to build A2A tasks query with dynamic filters
    fn build_a2a_tasks_query(
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<String> {
        use std::fmt::Write;
        let mut query = String::from(
            r"
            SELECT task_id, client_id, session_id, task_type, input_data,
                   status, result_data, method, created_at, updated_at
            FROM a2a_tasks
            ",
        );

        let mut conditions = Vec::new();
        let mut bind_count = 0;

        if client_id.is_some() {
            bind_count += 1;
            conditions.push(format!("client_id = ${bind_count}"));
        }

        if status_filter.is_some() {
            bind_count += 1;
            conditions.push(format!("status = ${bind_count}"));
        }

        if !conditions.is_empty() {
            query.push_str(" WHERE ");
            query.push_str(&conditions.join(" AND "));
        }

        query.push_str(" ORDER BY created_at DESC");

        if limit.is_some() {
            bind_count += 1;
            write!(query, " LIMIT ${bind_count}").map_err(|e| {
                AppError::database(format!("Failed to write LIMIT clause to query: {e}"))
            })?;
        }

        if offset.is_some() {
            bind_count += 1;
            write!(query, " OFFSET ${bind_count}").map_err(|e| {
                AppError::database(format!("Failed to write OFFSET clause to query: {e}"))
            })?;
        }

        Ok(query)
    }

    /// Helper function to parse A2A task from database row
    fn parse_a2a_task_from_row(row: &PgRow) -> AppResult<A2ATask> {
        shared::mappers::parse_a2a_task_from_row(row)
    }

    /// Map a `PostgreSQL` database row to `ToolCatalogEntry`
    fn map_pg_tool_catalog_row(row: &PgRow) -> AppResult<ToolCatalogEntry> {
        let id: String = row.get("id");
        let category_str: String = row.get("category");
        let min_plan_str: String = row.get("min_plan");
        let created_at: DateTime<Utc> = row.get("created_at");
        let updated_at: DateTime<Utc> = row.get("updated_at");

        Ok(ToolCatalogEntry {
            id,
            tool_name: row.get("tool_name"),
            display_name: row.get("display_name"),
            description: row.get("description"),
            category: ToolCategory::parse_str(&category_str)
                .ok_or_else(|| AppError::internal(format!("Invalid category: {category_str}")))?,
            is_enabled_by_default: row.get("is_enabled_by_default"),
            requires_provider: row.get("requires_provider"),
            min_plan: TenantPlan::parse_str(&min_plan_str)
                .ok_or_else(|| AppError::internal(format!("Invalid min_plan: {min_plan_str}")))?,
            created_at,
            updated_at,
        })
    }

    /// Map a `PostgreSQL` database row to `TenantToolOverride`
    fn map_pg_tenant_tool_override_row(row: &PgRow) -> TenantToolOverride {
        let id: Uuid = row.get("id");
        let tenant_id: TenantId = row.get("tenant_id");
        let enabled_by_user_id: Option<Uuid> = row.get("enabled_by_user_id");
        let created_at: DateTime<Utc> = row.get("created_at");
        let updated_at: DateTime<Utc> = row.get("updated_at");

        TenantToolOverride {
            id,
            tenant_id,
            tool_name: row.get("tool_name"),
            is_enabled: row.get("is_enabled"),
            enabled_by_user_id,
            reason: row.get("reason"),
            created_at,
            updated_at,
        }
    }
}

impl PostgresDatabase {
    /// Create new `PostgreSQL` database with provided pool configuration (internal implementation)
    /// This is called by the Database factory with centralized `ServerConfig`
    ///
    /// # Errors
    ///
    /// Returns an error if database connection or pool configuration fails
    async fn new_impl(
        database_url: &str,
        encryption_key: Vec<u8>,
        pool_config: &PostgresPoolConfig,
    ) -> AppResult<Self> {
        // Use pool configuration from ServerConfig (read once at startup)
        let max_connections = pool_config.max_connections;
        let min_connections = pool_config.min_connections;
        let acquire_timeout_secs = pool_config.acquire_timeout_secs;

        // Log connection pool configuration for debugging
        info!(
            "PostgreSQL pool config: max_connections={max_connections}, min_connections={min_connections}, timeout={acquire_timeout_secs}s, retries={}",
            pool_config.connection_retries
        );

        // Attempt connection with exponential backoff retry
        let pool = Self::connect_with_retry(
            database_url,
            max_connections,
            min_connections,
            acquire_timeout_secs,
            pool_config.connection_retries,
            pool_config.initial_retry_delay_ms,
            pool_config.max_retry_delay_ms,
        )
        .await?;

        let db = Self {
            pool,
            encryption_key,
        };

        // Run migrations
        db.migrate().await?;

        Ok(db)
    }

    /// Connect to `PostgreSQL` with exponential backoff retry on failure
    ///
    /// Handles transient connection failures (network issues, database restarts)
    /// by retrying with increasing delays between attempts.
    async fn connect_with_retry(
        database_url: &str,
        max_connections: u32,
        min_connections: u32,
        acquire_timeout_secs: u64,
        max_retries: u32,
        initial_delay_ms: u64,
        max_delay_ms: u64,
    ) -> AppResult<Pool<Postgres>> {
        let pool_options = PgPoolOptions::new()
            .max_connections(max_connections)
            .min_connections(min_connections)
            .acquire_timeout(Duration::from_secs(acquire_timeout_secs))
            .idle_timeout(Some(Duration::from_secs(300)))
            .max_lifetime(Some(Duration::from_secs(600)))
            // Test connections before returning to caller to detect stale connections
            .test_before_acquire(true);

        let mut last_error = None;
        let mut delay_ms = initial_delay_ms;

        for attempt in 0..=max_retries {
            match pool_options.clone().connect(database_url).await {
                Ok(pool) => {
                    if attempt > 0 {
                        info!(
                            "PostgreSQL connection established after {} retries",
                            attempt
                        );
                    }
                    return Ok(pool);
                }
                Err(e) => {
                    last_error = Some(e);

                    if attempt < max_retries {
                        warn!(
                            "PostgreSQL connection attempt {}/{} failed, retrying in {}ms: {}",
                            attempt + 1,
                            max_retries + 1,
                            delay_ms,
                            last_error.as_ref().map_or("unknown", |e| e
                                .as_database_error()
                                .map_or("connection error", |de| de.message()))
                        );
                        sleep(Duration::from_millis(delay_ms)).await;
                        // Exponential backoff with cap
                        delay_ms = (delay_ms * 2).min(max_delay_ms);
                    }
                }
            }
        }

        // All retries exhausted
        Err(AppError::database(format!(
            "Failed to connect to PostgreSQL after {} retries: {}",
            max_retries + 1,
            last_error.map_or_else(|| "unknown error".to_owned(), |e| e.to_string())
        )))
    }

    /// Create new `PostgreSQL` database with provided pool configuration (public API)
    /// This is called by the Database factory with centralized `ServerConfig`
    ///
    /// # Errors
    ///
    /// Returns an error if database connection or pool configuration fails
    pub async fn new(
        database_url: &str,
        encryption_key: Vec<u8>,
        pool_config: &PostgresPoolConfig,
    ) -> AppResult<Self> {
        Self::new_impl(database_url, encryption_key, pool_config).await
    }
}

#[async_trait]
impl DatabaseProvider for PostgresDatabase {
    async fn new(database_url: &str, encryption_key: Vec<u8>) -> AppResult<Self> {
        // Use default pool configuration when called through trait
        // In practice, the Database factory calls the inherent impl's new() directly with config
        let pool_config = PostgresPoolConfig::default();
        Self::new_impl(database_url, encryption_key, &pool_config).await
    }

    async fn migrate(&self) -> AppResult<()> {
        info!("Running PostgreSQL database migrations...");

        // Fix stale migration entry: version 20260316000002 was applied from a
        // now-renamed file (nullable_user_id → 20260316000003). The version slot
        // is now occupied by verify_token.sql which has a different checksum.
        // Delete the stale entry so the correct migration can be applied cleanly.
        let deleted = sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 20260316000002")
            .execute(&self.pool)
            .await;
        match &deleted {
            Ok(r) => info!(
                "Migration cleanup: deleted {} stale 20260316000002 entries",
                r.rows_affected()
            ),
            Err(e) => warn!("Migration cleanup failed (non-fatal): {e}"),
        }

        sqlx::migrate!("./migrations_pg")
            .set_ignore_missing(true)
            .run(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("PostgreSQL migration failed: {e}")))?;

        info!("PostgreSQL database migrations completed successfully");
        Ok(())
    }
}

impl PostgresDatabase {
    /// Generate a new MCP token with secure random bytes
    fn generate_mcp_token() -> String {
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        format!("pmcp_{}", URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Hash a token for storage
    fn hash_mcp_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Update token usage statistics
    async fn update_user_mcp_token_usage(&self, token_id: &str) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE user_mcp_tokens
            SET last_used_at = $1, usage_count = usage_count + 1
            WHERE id = $2
            ",
        )
        .bind(chrono::Utc::now())
        .bind(token_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update user MCP token usage: {e}")))?;

        Ok(())
    }

    /// Convert database row to `UserOAuthToken` with decryption
    ///
    /// SECURITY: Decrypts OAuth tokens from database storage (AES-256-GCM with AAD)
    fn row_to_user_oauth_token(&self, row: &PgRow) -> AppResult<UserOAuthToken> {
        use sqlx::Row;

        let user_id: uuid::Uuid = row
            .try_get("user_id")
            .map_err(|e| AppError::database(format!("Failed to parse user_id column: {e}")))?;
        let tenant_id: String = row
            .try_get("tenant_id")
            .map_err(|e| AppError::database(format!("Failed to parse tenant_id column: {e}")))?;
        let provider: String = row
            .try_get("provider")
            .map_err(|e| AppError::database(format!("Failed to parse provider column: {e}")))?;

        // Decrypt access token
        let encrypted_access_token: String = row
            .try_get("access_token")
            .map_err(|e| AppError::database(format!("Failed to parse access_token column: {e}")))?;
        let access_token = shared::encryption::decrypt_oauth_token(
            self,
            &encrypted_access_token,
            &tenant_id,
            user_id,
            &provider,
        )?;

        // Decrypt refresh token (optional)
        let refresh_token = row
            .try_get::<Option<String>, _>("refresh_token")
            .map_err(|e| AppError::database(format!("Failed to parse refresh_token column: {e}")))?
            .map(|encrypted_rt| {
                shared::encryption::decrypt_oauth_token(
                    self,
                    &encrypted_rt,
                    &tenant_id,
                    user_id,
                    &provider,
                )
            })
            .transpose()?;

        Ok(UserOAuthToken {
            id: row
                .try_get("id")
                .map_err(|e| AppError::database(format!("Failed to parse id column: {e}")))?,
            user_id,
            tenant_id,
            provider,
            access_token,
            refresh_token,
            token_type: row.try_get("token_type").map_err(|e| {
                AppError::database(format!("Failed to parse token_type column: {e}"))
            })?,
            expires_at: row.try_get("expires_at").map_err(|e| {
                AppError::database(format!("Failed to parse expires_at column: {e}"))
            })?,
            scope: row.try_get("scope").ok(),
            created_at: row.try_get("created_at").map_err(|e| {
                AppError::database(format!("Failed to parse created_at column: {e}"))
            })?,
            updated_at: row.try_get("updated_at").map_err(|e| {
                AppError::database(format!("Failed to parse updated_at column: {e}"))
            })?,
        })
    }
}

// System settings operations for PostgreSQL
impl PostgresDatabase {
    /// Get a system setting by key
    async fn get_system_setting(&self, key: &str) -> AppResult<Option<SystemSetting>> {
        use sqlx::Row;

        let row = sqlx::query(
            r"
            SELECT key, value, description, updated_at
            FROM system_settings
            WHERE key = $1
            ",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get system setting: {e}")))?;

        row.map_or(Ok(None), |row| {
            let updated_at: DateTime<Utc> = row.get("updated_at");
            Ok(Some(SystemSetting {
                key: row.get("key"),
                value: row.get("value"),
                description: row.get("description"),
                updated_at,
            }))
        })
    }

    /// Set a system setting value (upsert)
    async fn set_system_setting(&self, key: &str, value: &str) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO system_settings (key, value, created_at, updated_at)
            VALUES ($1, $2, NOW(), NOW())
            ON CONFLICT (key) DO UPDATE SET
                value = $2,
                updated_at = NOW()
            ",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set system setting: {e}")))?;

        Ok(())
    }

    /// Check if auto-approval is enabled in database
    ///
    /// Returns `Some(true/false)` if explicitly set in database,
    /// or `None` if no database setting exists (caller should use config default).
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn is_auto_approval_enabled(&self) -> AppResult<Option<bool>> {
        match self
            .get_system_setting(SETTING_AUTO_APPROVAL_ENABLED)
            .await?
        {
            Some(setting) => Ok(Some(setting.value.eq_ignore_ascii_case("true"))),
            None => Ok(None),
        }
    }

    /// Set auto-approval enabled state
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn set_auto_approval_enabled(&self, enabled: bool) -> AppResult<()> {
        self.set_system_setting(
            SETTING_AUTO_APPROVAL_ENABLED,
            if enabled { "true" } else { "false" },
        )
        .await
    }

    /// Get social insights configuration from database
    ///
    /// Returns `Some(config)` if explicitly set in database,
    /// or `None` if no database setting exists (caller should use defaults).
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails or JSON deserialization fails
    pub async fn get_social_insights_config(&self) -> AppResult<Option<SocialInsightsConfig>> {
        match self
            .get_system_setting(SETTING_SOCIAL_INSIGHTS_CONFIG)
            .await?
        {
            Some(setting) => {
                let config: SocialInsightsConfig =
                    serde_json::from_str(&setting.value).map_err(|e| {
                        AppError::internal(format!("Failed to parse social insights config: {e}"))
                    })?;
                Ok(Some(config))
            }
            None => Ok(None),
        }
    }

    /// Set social insights configuration in database
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails or JSON serialization fails
    pub async fn set_social_insights_config(&self, config: &SocialInsightsConfig) -> AppResult<()> {
        let json = serde_json::to_string(config).map_err(|e| {
            AppError::internal(format!("Failed to serialize social insights config: {e}"))
        })?;
        self.set_system_setting(SETTING_SOCIAL_INSIGHTS_CONFIG, &json)
            .await
    }

    /// Delete social insights configuration from database (revert to defaults)
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn delete_social_insights_config(&self) -> AppResult<()> {
        sqlx::query("DELETE FROM system_settings WHERE key = $1")
            .bind(SETTING_SOCIAL_INSIGHTS_CONFIG)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to delete social insights config: {e}"))
            })?;
        Ok(())
    }
}
