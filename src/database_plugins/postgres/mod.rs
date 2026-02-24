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
/// Encryption support (AES-256-GCM)
pub mod encryption;
/// OAuth token and authorization repository implementations
pub mod oauth;
/// Security and notification repository implementations
pub mod security;
/// Social insight repository implementation
pub mod social;
/// Tenant, tool selection, LLM credential, and fitness config repositories
pub mod tenant;
/// Usage tracking repository implementations
pub mod usage;
/// User and profile repository implementations
pub mod user;

use super::{shared, DatabaseProvider};
use crate::config::environment::PostgresPoolConfig;
use crate::config::social::SocialInsightsConfig;
use crate::database::system_settings::{
    SystemSetting, SETTING_AUTO_APPROVAL_ENABLED, SETTING_SOCIAL_INSIGHTS_CONFIG,
};
use crate::errors::{AppError, AppResult};
use crate::models::TenantToolOverride;
use crate::models::{TenantPlan, ToolCatalogEntry, ToolCategory, User, UserOAuthToken};
use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as Base64Engine;
use chrono::{DateTime, Utc};
use pierre_core::models::a2a::{A2ATask, TaskStatus};
use pierre_core::models::TenantId;
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{Pool, Postgres, Row};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

/// Type alias for tool catalog seed data tuple
/// Fields: (id, `tool_name`, `display_name`, description, category, `is_enabled_by_default`, `requires_provider`, `min_plan`)
type ToolCatalogSeedEntry<'a> = (
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    bool,
    Option<&'a str>,
    &'a str,
);

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
        self.create_users_table().await?;
        self.create_user_profiles_table().await?;
        self.create_goals_table().await?;
        self.create_insights_table().await?;
        self.create_api_keys_tables().await?;
        self.create_a2a_tables().await?;
        self.create_admin_tables().await?;
        self.create_jwt_usage_table().await?;
        self.create_oauth_notifications_table().await?;
        self.create_rsa_keypairs_table().await?;
        self.create_tenant_tables().await?;
        self.create_tool_selection_tables().await?;
        self.create_chat_tables().await?;
        self.create_llm_usage_table().await?;
        self.create_usage_counters_table().await?;
        self.create_system_settings_table().await?;
        self.create_social_tables().await?;
        self.create_indexes().await?;
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

    async fn create_users_table(&self) -> AppResult<()> {
        // OAuth tokens are stored in user_oauth_tokens table, not here
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS users (
                id UUID PRIMARY KEY,
                email TEXT UNIQUE NOT NULL,
                display_name TEXT,
                password_hash TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'starter' CHECK (tier IN ('starter', 'professional', 'enterprise')),
                tenant_id TEXT,
                is_active BOOLEAN NOT NULL DEFAULT true,
                user_status TEXT NOT NULL DEFAULT 'pending' CHECK (user_status IN ('pending', 'active', 'suspended')),
                is_admin BOOLEAN NOT NULL DEFAULT false,
                role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('super_admin', 'admin', 'user')),
                approved_by UUID REFERENCES users(id),
                approved_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                last_active TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                firebase_uid TEXT,
                auth_provider TEXT NOT NULL DEFAULT 'email'
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create users table: {e}")))?;

        // Create unique index for Firebase UID lookups (enforces uniqueness for non-null values)
        sqlx::query(
            r"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_users_firebase_uid
            ON users(firebase_uid)
            WHERE firebase_uid IS NOT NULL
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create firebase_uid index: {e}")))?;

        // Create index for auth provider queries
        sqlx::query(
            r"
            CREATE INDEX IF NOT EXISTS idx_users_auth_provider ON users(auth_provider)
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create auth_provider index: {e}")))?;

        Ok(())
    }

    async fn create_user_profiles_table(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS user_profiles (
                user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
                profile_data JSONB NOT NULL,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create user_profiles table: {e}")))?;
        Ok(())
    }

    async fn create_goals_table(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS goals (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                goal_data JSONB NOT NULL,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create goals table: {e}")))?;
        Ok(())
    }

    async fn create_insights_table(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS insights (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                insight_type TEXT NOT NULL,
                content JSONB NOT NULL,
                metadata JSONB,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create insights table: {e}")))?;
        Ok(())
    }

    async fn create_api_keys_tables(&self) -> AppResult<()> {
        // Create api_keys table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS api_keys (
                id TEXT PRIMARY KEY,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                key_prefix TEXT NOT NULL,
                key_hash TEXT NOT NULL,
                description TEXT,
                tier TEXT NOT NULL CHECK (tier IN ('trial', 'starter', 'professional', 'enterprise')),
                is_active BOOLEAN NOT NULL DEFAULT true,
                rate_limit_requests INTEGER NOT NULL,
                rate_limit_window_seconds INTEGER NOT NULL,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                expires_at TIMESTAMPTZ,
                last_used_at TIMESTAMPTZ,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create api_keys table: {e}")))?;

        // Create api_key_usage table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS api_key_usage (
                id SERIAL PRIMARY KEY,
                api_key_id TEXT NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
                timestamp TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                endpoint TEXT NOT NULL,
                response_time_ms INTEGER,
                status_code SMALLINT NOT NULL,
                method TEXT,
                request_size_bytes INTEGER,
                response_size_bytes INTEGER,
                ip_address INET,
                user_agent TEXT
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create api_key_usage table: {e}")))?;
        Ok(())
    }

    async fn create_a2a_tables(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS a2a_clients (
                client_id TEXT PRIMARY KEY,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                description TEXT,
                client_secret_hash TEXT NOT NULL,
                api_key_hash TEXT NOT NULL,
                capabilities TEXT[] NOT NULL DEFAULT '{}',
                redirect_uris TEXT[] NOT NULL DEFAULT '{}',
                contact_email TEXT,
                is_active BOOLEAN NOT NULL DEFAULT true,
                rate_limit_per_minute INTEGER NOT NULL DEFAULT 100,
                rate_limit_per_day INTEGER DEFAULT 10000,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create a2a_clients table: {e}")))?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS a2a_sessions (
                session_token TEXT PRIMARY KEY,
                client_id TEXT NOT NULL REFERENCES a2a_clients(client_id) ON DELETE CASCADE,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                granted_scopes TEXT[] NOT NULL DEFAULT '{}',
                is_active BOOLEAN NOT NULL DEFAULT true,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                expires_at TIMESTAMPTZ NOT NULL,
                last_active_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create a2a_sessions table: {e}")))?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS a2a_tasks (
                task_id TEXT PRIMARY KEY,
                session_token TEXT NOT NULL REFERENCES a2a_sessions(session_token) ON DELETE CASCADE,
                task_type TEXT NOT NULL,
                parameters JSONB NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                result JSONB,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create a2a_tasks table: {e}")))?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS a2a_usage (
                id SERIAL PRIMARY KEY,
                client_id TEXT NOT NULL REFERENCES a2a_clients(client_id) ON DELETE CASCADE,
                session_token TEXT REFERENCES a2a_sessions(session_token) ON DELETE SET NULL,
                timestamp TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                endpoint TEXT NOT NULL,
                response_time_ms INTEGER,
                status_code SMALLINT NOT NULL,
                method TEXT,
                request_size_bytes INTEGER,
                response_size_bytes INTEGER,
                ip_address INET,
                user_agent TEXT,
                protocol_version TEXT NOT NULL DEFAULT 'v1',
                client_capabilities TEXT[] DEFAULT '{}',
                granted_scopes TEXT[] DEFAULT '{}'
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create a2a_usage table: {e}")))?;
        Ok(())
    }

    async fn create_admin_tables(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS admin_tokens (
                id TEXT PRIMARY KEY,
                service_name TEXT NOT NULL,
                service_description TEXT,
                token_hash TEXT NOT NULL,
                token_prefix TEXT NOT NULL,
                jwt_secret_hash TEXT NOT NULL,
                permissions TEXT NOT NULL DEFAULT '[]',
                is_super_admin BOOLEAN NOT NULL DEFAULT false,
                is_active BOOLEAN NOT NULL DEFAULT true,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                expires_at TIMESTAMPTZ,
                last_used_at TIMESTAMPTZ,
                last_used_ip INET,
                usage_count BIGINT NOT NULL DEFAULT 0
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create admin_tokens table: {e}")))?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS admin_token_usage (
                id SERIAL PRIMARY KEY,
                admin_token_id TEXT NOT NULL REFERENCES admin_tokens(id) ON DELETE CASCADE,
                timestamp TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                action TEXT NOT NULL,
                target_resource TEXT,
                ip_address INET,
                user_agent TEXT,
                request_size_bytes INTEGER,
                success BOOLEAN NOT NULL,
                method TEXT,
                response_time_ms INTEGER
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create admin_token_usage table: {e}"))
        })?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS admin_provisioned_keys (
                id SERIAL PRIMARY KEY,
                admin_token_id TEXT NOT NULL REFERENCES admin_tokens(id) ON DELETE CASCADE,
                api_key_id TEXT NOT NULL,
                user_email TEXT NOT NULL,
                requested_tier TEXT NOT NULL,
                provisioned_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                provisioned_by_service TEXT NOT NULL,
                rate_limit_requests INTEGER NOT NULL,
                rate_limit_period TEXT NOT NULL,
                key_status TEXT NOT NULL DEFAULT 'active',
                revoked_at TIMESTAMPTZ,
                revoked_reason TEXT
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create admin_provisioned_keys table: {e}"
            ))
        })?;
        Ok(())
    }

    async fn create_jwt_usage_table(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS jwt_usage (
                id SERIAL PRIMARY KEY,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                timestamp TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                endpoint TEXT NOT NULL,
                response_time_ms INTEGER,
                status_code INTEGER NOT NULL,
                method TEXT,
                request_size_bytes INTEGER,
                response_size_bytes INTEGER,
                ip_address INET,
                user_agent TEXT
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create jwt_usage table: {e}")))?;
        Ok(())
    }

    /// Create OAuth notifications table for MCP resource delivery
    async fn create_oauth_notifications_table(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS oauth_notifications (
                id TEXT PRIMARY KEY,
                user_id UUID NOT NULL,
                provider TEXT NOT NULL,
                success BOOLEAN NOT NULL DEFAULT true,
                message TEXT NOT NULL,
                expires_at TEXT,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                read_at TIMESTAMPTZ,
                FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create oauth_notifications table: {e}"))
        })?;

        // Create indices for efficient queries
        sqlx::query(
            r"
            CREATE INDEX IF NOT EXISTS idx_oauth_notifications_user_id
            ON oauth_notifications (user_id)
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_oauth_notifications_user_id: {e}"
            ))
        })?;

        sqlx::query(
            r"
            CREATE INDEX IF NOT EXISTS idx_oauth_notifications_user_unread
            ON oauth_notifications (user_id, read_at)
            WHERE read_at IS NULL
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_oauth_notifications_user_unread: {e}"
            ))
        })?;

        Ok(())
    }

    /// Create RSA keypairs table for JWT signing key persistence
    async fn create_rsa_keypairs_table(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS rsa_keypairs (
                kid TEXT PRIMARY KEY,
                private_key_pem TEXT NOT NULL,
                public_key_pem TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                is_active BOOLEAN NOT NULL DEFAULT false,
                key_size_bits INTEGER NOT NULL
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create rsa_keypairs table: {e}")))?;

        // Create index for active key lookup
        sqlx::query(
            r"
            CREATE INDEX IF NOT EXISTS idx_rsa_keypairs_active
            ON rsa_keypairs (is_active)
            WHERE is_active = true
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_rsa_keypairs_active: {e}"
            ))
        })?;

        Ok(())
    }

    /// Creates complete multi-tenant database schema with all required tables
    ///
    /// JUSTIFICATION for `#[allow(clippy::too_many_lines)]`:
    /// - Creates 6 interdependent tables in a single atomic migration
    /// - Each table has comprehensive schema: constraints, foreign keys, indexes, defaults
    /// - Splitting into separate functions obscures the complete schema structure
    /// - Database migrations benefit from having the full DDL in one location for review
    /// - Tables: `tenants`, `tenant_oauth_credentials`, `tenant_users`, `tenant_provider_usage`,
    ///   `oauth_apps`, `authorization_codes`, `user_oauth_tokens`
    #[allow(clippy::too_many_lines)]
    async fn create_tenant_tables(&self) -> AppResult<()> {
        // Create tenants table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS tenants (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                name VARCHAR(255) NOT NULL,
                slug VARCHAR(100) UNIQUE NOT NULL,
                domain VARCHAR(255) UNIQUE,
                subscription_tier VARCHAR(50) DEFAULT 'starter' CHECK (subscription_tier IN ('starter', 'professional', 'enterprise')),
                is_active BOOLEAN DEFAULT true,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            "
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create tenants table: {e}")))?;

        // Create tenant_oauth_credentials table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS tenant_oauth_credentials (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                provider VARCHAR(50) NOT NULL,
                client_id VARCHAR(255) NOT NULL,
                client_secret_encrypted TEXT NOT NULL,
                redirect_uri VARCHAR(500) NOT NULL,
                scopes TEXT[] DEFAULT '{}',
                rate_limit_per_day INTEGER DEFAULT 15000,
                is_active BOOLEAN DEFAULT true,
                configured_by UUID REFERENCES users(id),
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(tenant_id, provider)
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create tenant_oauth_credentials table: {e}"
            ))
        })?;

        // Create tenant_users table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS tenant_users (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                role VARCHAR(50) DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'billing', 'member')),
                invited_at TIMESTAMPTZ NOT NULL,
                joined_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(tenant_id, user_id)
            )
            "
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create tenant_users table: {e}")))?;

        // Create tenant_provider_usage table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS tenant_provider_usage (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                provider VARCHAR(50) NOT NULL,
                usage_date DATE NOT NULL,
                request_count INTEGER DEFAULT 0,
                error_count INTEGER DEFAULT 0,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(tenant_id, provider, usage_date)
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create tenant_provider_usage table: {e}"))
        })?;

        // Create OAuth Apps table for app registration
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS oauth_apps (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                client_id VARCHAR(255) UNIQUE NOT NULL,
                client_secret VARCHAR(255) NOT NULL,
                name VARCHAR(255) NOT NULL,
                description TEXT,
                redirect_uris TEXT[] NOT NULL DEFAULT '{}',
                scopes TEXT[] NOT NULL DEFAULT '{}',
                app_type VARCHAR(50) DEFAULT 'web' CHECK (app_type IN ('desktop', 'web', 'mobile', 'server')),
                owner_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                is_active BOOLEAN DEFAULT true,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create oauth_apps table: {e}")))?;

        // Create Authorization Code table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS authorization_codes (
                code VARCHAR(255) PRIMARY KEY,
                client_id VARCHAR(255) NOT NULL REFERENCES oauth_apps(client_id) ON DELETE CASCADE,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                redirect_uri VARCHAR(500) NOT NULL,
                scope VARCHAR(500) NOT NULL,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                expires_at TIMESTAMPTZ NOT NULL
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create authorization_codes table: {e}"))
        })?;

        // Create user_oauth_tokens table for per-user, per-tenant OAuth tokens
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS user_oauth_tokens (
                id VARCHAR(255) PRIMARY KEY,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                tenant_id VARCHAR(255) NOT NULL,
                provider VARCHAR(50) NOT NULL,
                access_token TEXT NOT NULL,
                refresh_token TEXT,
                token_type VARCHAR(50) DEFAULT 'bearer',
                expires_at TIMESTAMPTZ,
                scope TEXT,
                last_sync TIMESTAMPTZ,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(user_id, tenant_id, provider)
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create user_oauth_tokens table: {e}"))
        })?;

        Ok(())
    }

    /// Create tool selection tables for per-tenant MCP tool configuration
    async fn create_tool_selection_tables(&self) -> AppResult<()> {
        // Create tool_catalog table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS tool_catalog (
                id VARCHAR(255) PRIMARY KEY,
                tool_name VARCHAR(255) NOT NULL UNIQUE,
                display_name VARCHAR(255) NOT NULL,
                description TEXT NOT NULL,
                category VARCHAR(50) NOT NULL CHECK (category IN (
                    'fitness', 'analysis', 'goals', 'nutrition',
                    'recipes', 'sleep', 'configuration', 'connections'
                )),
                is_enabled_by_default BOOLEAN NOT NULL DEFAULT true,
                requires_provider VARCHAR(50),
                min_plan VARCHAR(50) NOT NULL DEFAULT 'starter' CHECK (min_plan IN ('starter', 'professional', 'enterprise')),
                created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create tool_catalog table: {e}")))?;

        // Create tenant_tool_overrides table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS tenant_tool_overrides (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                tool_name VARCHAR(255) NOT NULL REFERENCES tool_catalog(tool_name) ON DELETE CASCADE,
                is_enabled BOOLEAN NOT NULL,
                enabled_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
                reason TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(tenant_id, tool_name)
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create tenant_tool_overrides table: {e}"))
        })?;

        // Create indexes for tool selection tables
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tool_catalog_category ON tool_catalog(category)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_tool_catalog_category: {e}"
            ))
        })?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tool_catalog_min_plan ON tool_catalog(min_plan)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_tool_catalog_min_plan: {e}"
            ))
        })?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tenant_tool_overrides_tenant ON tenant_tool_overrides(tenant_id)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_tenant_tool_overrides_tenant: {e}"
            ))
        })?;

        // Seed tool_catalog with default tools
        self.seed_tool_catalog().await?;

        Ok(())
    }

    /// Create chat tables for AI conversation storage
    async fn create_chat_tables(&self) -> AppResult<()> {
        // Create chat_conversations table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS chat_conversations (
                id VARCHAR(255) PRIMARY KEY,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                tenant_id VARCHAR(255) NOT NULL,
                title TEXT NOT NULL,
                model VARCHAR(255) NOT NULL DEFAULT 'gemini-2.0-flash-exp',
                system_prompt TEXT,
                total_tokens BIGINT NOT NULL DEFAULT 0,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create chat_conversations table: {e}"))
        })?;

        // Create chat_messages table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS chat_messages (
                id VARCHAR(255) PRIMARY KEY,
                conversation_id VARCHAR(255) NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
                role VARCHAR(50) NOT NULL CHECK (role IN ('system', 'user', 'assistant')),
                content TEXT NOT NULL,
                token_count BIGINT,
                prompt_tokens BIGINT,
                model VARCHAR(255),
                finish_reason VARCHAR(50),
                activity_list TEXT,
                created_at TIMESTAMPTZ NOT NULL
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create chat_messages table: {e}")))?;

        // Create indexes for chat tables
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_chat_conversations_user ON chat_conversations(user_id)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_chat_conversations_user: {e}"
            ))
        })?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_chat_conversations_tenant ON chat_conversations(tenant_id)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_chat_conversations_tenant: {e}"
            ))
        })?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_chat_conversations_updated ON chat_conversations(updated_at DESC)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_chat_conversations_updated: {e}"
            ))
        })?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_chat_messages_conversation ON chat_messages(conversation_id)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_chat_messages_conversation: {e}"
            ))
        })?;

        Ok(())
    }

    /// Create LLM usage tracking table for cost analysis and quota enforcement
    async fn create_llm_usage_table(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS llm_usage (
                id VARCHAR(255) PRIMARY KEY,
                tenant_id VARCHAR(255) NOT NULL,
                user_id VARCHAR(255) NOT NULL,
                conversation_id VARCHAR(255),
                provider VARCHAR(255) NOT NULL,
                model VARCHAR(255) NOT NULL,
                prompt_tokens BIGINT NOT NULL,
                completion_tokens BIGINT NOT NULL,
                total_tokens BIGINT NOT NULL,
                call_type VARCHAR(50) NOT NULL,
                tool_calls_count BIGINT DEFAULT 0,
                execution_time_ms BIGINT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create llm_usage table: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_llm_usage_tenant_user_date ON llm_usage(tenant_id, user_id, created_at)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_llm_usage_tenant_user_date: {e}"
            ))
        })?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_llm_usage_tenant_date ON llm_usage(tenant_id, created_at)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_llm_usage_tenant_date: {e}"
            ))
        })?;

        Ok(())
    }

    /// Create usage counters table for rate limiting and quota enforcement
    async fn create_usage_counters_table(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS usage_counters (
                tenant_id VARCHAR(255) NOT NULL,
                user_id VARCHAR(255) NOT NULL,
                counter_key VARCHAR(255) NOT NULL,
                period VARCHAR(50) NOT NULL,
                value BIGINT NOT NULL DEFAULT 0,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (tenant_id, user_id, counter_key, period)
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create usage_counters table: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_usage_counters_tenant_user ON usage_counters(tenant_id, user_id)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_usage_counters_tenant_user: {e}"
            ))
        })?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_usage_counters_period ON usage_counters(period)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_usage_counters_period: {e}"
            ))
        })?;

        Ok(())
    }

    /// Seed the `tool_catalog` table with default tools
    async fn seed_tool_catalog(&self) -> AppResult<()> {
        // Check if tools already exist
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tool_catalog")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to count tool catalog: {e}")))?;

        if count > 0 {
            return Ok(());
        }

        // Seed tool catalog with all ToolId variants (same as SQLite migration)
        let tools: Vec<ToolCatalogSeedEntry<'_>> = vec![
            (
                "tc-001",
                "get_activities",
                "Get Activities",
                "Get user fitness activities with optional filtering and limits",
                "fitness",
                true,
                None,
                "starter",
            ),
            (
                "tc-002",
                "get_athlete",
                "Get Athlete Profile",
                "Get user athlete profile and basic information",
                "fitness",
                true,
                None,
                "starter",
            ),
            (
                "tc-003",
                "get_stats",
                "Get Statistics",
                "Get user performance statistics and metrics",
                "fitness",
                true,
                None,
                "starter",
            ),
            (
                "tc-004",
                "analyze_activity",
                "Analyze Activity",
                "Analyze a specific activity with detailed performance insights",
                "analysis",
                true,
                None,
                "starter",
            ),
            (
                "tc-005",
                "get_activity_intelligence",
                "Activity Intelligence",
                "Get AI-powered intelligence analysis for an activity",
                "analysis",
                true,
                None,
                "starter",
            ),
            (
                "tc-006",
                "get_connection_status",
                "Connection Status",
                "Check OAuth connection status for fitness providers",
                "connections",
                true,
                None,
                "starter",
            ),
            (
                "tc-007",
                "connect_provider",
                "Connect Provider",
                "Connect to a fitness data provider via OAuth",
                "connections",
                true,
                None,
                "starter",
            ),
            (
                "tc-008",
                "disconnect_provider",
                "Disconnect Provider",
                "Disconnect user from a fitness data provider",
                "connections",
                true,
                None,
                "starter",
            ),
            (
                "tc-009",
                "set_goal",
                "Set Goal",
                "Set a new fitness goal for the user",
                "goals",
                true,
                None,
                "starter",
            ),
            (
                "tc-010",
                "suggest_goals",
                "Suggest Goals",
                "Get AI-suggested fitness goals based on activity history",
                "goals",
                true,
                None,
                "starter",
            ),
            (
                "tc-011",
                "analyze_goal_feasibility",
                "Goal Feasibility",
                "Analyze whether a goal is achievable given current fitness level",
                "goals",
                true,
                None,
                "professional",
            ),
            (
                "tc-012",
                "track_progress",
                "Track Progress",
                "Track progress towards fitness goals",
                "goals",
                true,
                None,
                "starter",
            ),
            (
                "tc-013",
                "calculate_metrics",
                "Calculate Metrics",
                "Calculate custom fitness metrics and performance indicators",
                "analysis",
                true,
                None,
                "starter",
            ),
            (
                "tc-014",
                "analyze_performance_trends",
                "Performance Trends",
                "Analyze performance trends over time",
                "analysis",
                true,
                None,
                "professional",
            ),
            (
                "tc-015",
                "compare_activities",
                "Compare Activities",
                "Compare two activities for performance analysis",
                "analysis",
                true,
                None,
                "starter",
            ),
            (
                "tc-016",
                "detect_patterns",
                "Detect Patterns",
                "Detect patterns and insights in activity data",
                "analysis",
                true,
                None,
                "professional",
            ),
            (
                "tc-017",
                "generate_recommendations",
                "Generate Recommendations",
                "Generate personalized training recommendations",
                "analysis",
                true,
                None,
                "professional",
            ),
            (
                "tc-018",
                "calculate_fitness_score",
                "Fitness Score",
                "Calculate overall fitness score based on recent activities",
                "analysis",
                true,
                None,
                "starter",
            ),
            (
                "tc-019",
                "predict_performance",
                "Predict Performance",
                "Predict future performance based on training patterns",
                "analysis",
                true,
                None,
                "enterprise",
            ),
            (
                "tc-020",
                "analyze_training_load",
                "Training Load",
                "Analyze training load and recovery metrics",
                "analysis",
                true,
                None,
                "professional",
            ),
            (
                "tc-021",
                "get_configuration_catalog",
                "Configuration Catalog",
                "Get the complete configuration catalog with all available parameters",
                "configuration",
                true,
                None,
                "starter",
            ),
            (
                "tc-022",
                "get_configuration_profiles",
                "Configuration Profiles",
                "Get available configuration profiles (Research, Elite, Recreational, etc.)",
                "configuration",
                true,
                None,
                "starter",
            ),
            (
                "tc-023",
                "get_user_configuration",
                "Get User Config",
                "Get current user configuration settings and overrides",
                "configuration",
                true,
                None,
                "starter",
            ),
            (
                "tc-024",
                "update_user_configuration",
                "Update User Config",
                "Update user configuration parameters and session overrides",
                "configuration",
                true,
                None,
                "starter",
            ),
            (
                "tc-025",
                "calculate_personalized_zones",
                "Personalized Zones",
                "Calculate personalized training zones based on user VO2 max",
                "configuration",
                true,
                None,
                "starter",
            ),
            (
                "tc-026",
                "validate_configuration",
                "Validate Config",
                "Validate configuration parameters against safety rules",
                "configuration",
                true,
                None,
                "starter",
            ),
            (
                "tc-027",
                "analyze_sleep_quality",
                "Sleep Quality",
                "Analyze sleep quality from Fitbit/Garmin data using NSF/AASM guidelines",
                "sleep",
                true,
                None,
                "professional",
            ),
            (
                "tc-028",
                "calculate_recovery_score",
                "Recovery Score",
                "Calculate holistic recovery score combining TSB, sleep quality, and HRV",
                "sleep",
                true,
                None,
                "professional",
            ),
            (
                "tc-029",
                "suggest_rest_day",
                "Rest Day Suggestion",
                "AI-powered rest day recommendation based on recovery indicators",
                "sleep",
                true,
                None,
                "professional",
            ),
            (
                "tc-030",
                "track_sleep_trends",
                "Sleep Trends",
                "Track sleep patterns and correlate with performance over time",
                "sleep",
                true,
                None,
                "professional",
            ),
            (
                "tc-031",
                "optimize_sleep_schedule",
                "Sleep Schedule",
                "Optimize sleep duration based on training load and recovery needs",
                "sleep",
                true,
                None,
                "enterprise",
            ),
            (
                "tc-032",
                "get_fitness_config",
                "Get Fitness Config",
                "Get user fitness configuration settings including heart rate zones",
                "configuration",
                true,
                None,
                "starter",
            ),
            (
                "tc-033",
                "set_fitness_config",
                "Set Fitness Config",
                "Save user fitness configuration settings for zones and thresholds",
                "configuration",
                true,
                None,
                "starter",
            ),
            (
                "tc-034",
                "list_fitness_configs",
                "List Fitness Configs",
                "List all available fitness configuration names for the user",
                "configuration",
                true,
                None,
                "starter",
            ),
            (
                "tc-035",
                "delete_fitness_config",
                "Delete Fitness Config",
                "Delete a specific fitness configuration by name",
                "configuration",
                true,
                None,
                "starter",
            ),
            (
                "tc-036",
                "calculate_daily_nutrition",
                "Daily Nutrition",
                "Calculate daily calorie and macronutrient needs using Mifflin-St Jeor BMR formula",
                "nutrition",
                true,
                None,
                "starter",
            ),
            (
                "tc-037",
                "get_nutrient_timing",
                "Nutrient Timing",
                "Get optimal pre/post-workout nutrition recommendations following ISSN guidelines",
                "nutrition",
                true,
                None,
                "professional",
            ),
            (
                "tc-038",
                "search_food",
                "Search Food",
                "Search USDA FoodData Central database for foods by name/description",
                "nutrition",
                true,
                None,
                "starter",
            ),
            (
                "tc-039",
                "get_food_details",
                "Food Details",
                "Get detailed nutritional information for a specific food from USDA database",
                "nutrition",
                true,
                None,
                "starter",
            ),
            (
                "tc-040",
                "analyze_meal_nutrition",
                "Meal Nutrition",
                "Analyze total calories and macronutrients for a meal of multiple foods",
                "nutrition",
                true,
                None,
                "starter",
            ),
            (
                "tc-041",
                "get_recipe_constraints",
                "Recipe Constraints",
                "Get macro targets for LLM recipe generation by training phase",
                "recipes",
                true,
                None,
                "starter",
            ),
            (
                "tc-042",
                "validate_recipe",
                "Validate Recipe",
                "Validate recipe nutrition against USDA and calculate macros",
                "recipes",
                true,
                None,
                "starter",
            ),
            (
                "tc-043",
                "save_recipe",
                "Save Recipe",
                "Save validated recipe with cached nutrition data",
                "recipes",
                true,
                None,
                "starter",
            ),
            (
                "tc-044",
                "list_recipes",
                "List Recipes",
                "List saved recipes with optional meal timing filter",
                "recipes",
                true,
                None,
                "starter",
            ),
            (
                "tc-045",
                "get_recipe",
                "Get Recipe",
                "Get a specific recipe by ID",
                "recipes",
                true,
                None,
                "starter",
            ),
            (
                "tc-046",
                "delete_recipe",
                "Delete Recipe",
                "Delete a recipe from collection",
                "recipes",
                true,
                None,
                "starter",
            ),
            (
                "tc-047",
                "search_recipes",
                "Search Recipes",
                "Search recipes by name, tags, or description",
                "recipes",
                true,
                None,
                "starter",
            ),
        ];

        for (
            id,
            tool_name,
            display_name,
            description,
            category,
            is_enabled,
            requires_provider,
            min_plan,
        ) in tools
        {
            sqlx::query(
                r"
                INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT (tool_name) DO NOTHING
                ",
            )
            .bind(id)
            .bind(tool_name)
            .bind(display_name)
            .bind(description)
            .bind(category)
            .bind(is_enabled)
            .bind(requires_provider)
            .bind(min_plan)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to seed tool {tool_name}: {e}")))?;
        }

        Ok(())
    }

    async fn create_user_indexes(&self) -> AppResult<()> {
        // User and profile indexes
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_users_email ON users(email)")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to create index idx_users_email: {e}"))
            })?;

        Ok(())
    }

    async fn create_api_key_indexes(&self) -> AppResult<()> {
        // API key indexes
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_api_keys_user_id ON api_keys(user_id)")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to create index idx_api_keys_user_id: {e}"))
            })?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_api_key_usage_api_key_id ON api_key_usage(api_key_id)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_api_key_usage_api_key_id: {e}"
            ))
        })?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_api_key_usage_timestamp ON api_key_usage(timestamp)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_api_key_usage_timestamp: {e}"
            ))
        })?;

        Ok(())
    }

    async fn create_a2a_indexes(&self) -> AppResult<()> {
        // A2A indexes
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_a2a_clients_user_id ON a2a_clients(user_id)")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!(
                    "Failed to create index idx_a2a_clients_user_id: {e}"
                ))
            })?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_a2a_usage_client_id ON a2a_usage(client_id)")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!(
                    "Failed to create index idx_a2a_usage_client_id: {e}"
                ))
            })?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_a2a_usage_timestamp ON a2a_usage(timestamp)")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!(
                    "Failed to create index idx_a2a_usage_timestamp: {e}"
                ))
            })?;

        Ok(())
    }

    async fn create_admin_token_indexes(&self) -> AppResult<()> {
        // Admin token indexes
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_admin_tokens_service ON admin_tokens(service_name)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_admin_tokens_service: {e}"
            ))
        })?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_admin_tokens_prefix ON admin_tokens(token_prefix)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_admin_tokens_prefix: {e}"
            ))
        })?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_admin_usage_token_id ON admin_token_usage(admin_token_id)")
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to create index idx_admin_usage_token_id: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_admin_usage_timestamp ON admin_token_usage(timestamp)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_admin_usage_timestamp: {e}"
            ))
        })?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_admin_provisioned_token ON admin_provisioned_keys(admin_token_id)")
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to create index idx_admin_provisioned_token: {e}")))?;

        Ok(())
    }

    async fn create_jwt_usage_indexes(&self) -> AppResult<()> {
        // JWT usage indexes
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_jwt_usage_user_id ON jwt_usage(user_id)")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to create index idx_jwt_usage_user_id: {e}"))
            })?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_jwt_usage_timestamp ON jwt_usage(timestamp)")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!(
                    "Failed to create index idx_jwt_usage_timestamp: {e}"
                ))
            })?;

        Ok(())
    }

    async fn create_tenant_indexes(&self) -> AppResult<()> {
        // Tenant indexes
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_tenant_oauth_credentials_tenant_provider ON tenant_oauth_credentials(tenant_id, provider)")
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to create index idx_tenant_oauth_credentials_tenant_provider: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tenant_users_tenant ON tenant_users(tenant_id)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_tenant_users_tenant: {e}"
            ))
        })?;

        // UserOAuthToken indexes
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_user_oauth_tokens_user ON user_oauth_tokens(user_id)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to create index idx_user_oauth_tokens_user: {e}"
            ))
        })?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_user_oauth_tokens_tenant_provider ON user_oauth_tokens(tenant_id, provider)")
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to create index idx_user_oauth_tokens_tenant_provider: {e}")))?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_tenant_users_user ON tenant_users(user_id)")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to create index idx_tenant_users_user: {e}"))
            })?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_tenant_usage_date ON tenant_provider_usage(tenant_id, provider, usage_date)")
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to create index idx_tenant_usage_date: {e}")))?;

        Ok(())
    }

    async fn create_system_settings_table(&self) -> AppResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS system_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                description TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create system_settings table: {e}")))?;

        // Insert default auto_approval_enabled setting if not present
        sqlx::query(
            r"
            INSERT INTO system_settings (key, value, description, created_at, updated_at)
            VALUES ($1, $2, $3, NOW(), NOW())
            ON CONFLICT (key) DO NOTHING
            ",
        )
        .bind("auto_approval_enabled")
        .bind("false")
        .bind("When enabled, new user registrations are automatically approved without admin intervention")
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to seed system_settings defaults: {e}"))
        })?;

        Ok(())
    }

    /// Create all social feature tables (friend connections, settings, insights, reactions, adaptations)
    async fn create_social_tables(&self) -> AppResult<()> {
        // Friend connections: bidirectional friend relationships
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS friend_connections (
                id UUID PRIMARY KEY,
                initiator_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                receiver_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'accepted', 'declined', 'blocked')),
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL,
                accepted_at TIMESTAMPTZ,
                UNIQUE(initiator_id, receiver_id),
                CHECK (initiator_id != receiver_id)
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create friend_connections table: {e}"))
        })?;

        // Indexes for friend connections
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_friend_connections_initiator
             ON friend_connections(initiator_id, status)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create index: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_friend_connections_receiver
             ON friend_connections(receiver_id, status)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create index: {e}")))?;

        // User social settings: privacy and sharing preferences
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS user_social_settings (
                user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
                discoverable BOOLEAN NOT NULL DEFAULT true,
                default_visibility TEXT NOT NULL DEFAULT 'friends_only'
                    CHECK (default_visibility IN ('friends_only', 'public')),
                share_activity_types TEXT NOT NULL DEFAULT '["run", "ride", "swim"]',
                notify_friend_requests BOOLEAN NOT NULL DEFAULT true,
                notify_insight_reactions BOOLEAN NOT NULL DEFAULT true,
                notify_adapted_insights BOOLEAN NOT NULL DEFAULT true,
                insight_sharing_policy TEXT NOT NULL DEFAULT 'friends_only',
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create user_social_settings table: {e}"))
        })?;

        // Shared insights: coach-generated insights users share with friends
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS shared_insights (
                id UUID PRIMARY KEY,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                visibility TEXT NOT NULL DEFAULT 'friends_only'
                    CHECK (visibility IN ('friends_only', 'public')),
                insight_type TEXT NOT NULL
                    CHECK (insight_type IN ('achievement', 'milestone', 'training_tip', 'recovery', 'motivation', 'coaching_insight')),
                sport_type TEXT,
                content TEXT NOT NULL,
                title TEXT,
                training_phase TEXT,
                reaction_count INTEGER NOT NULL DEFAULT 0,
                adapt_count INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ,
                source_activity_id TEXT,
                coach_generated BOOLEAN NOT NULL DEFAULT false
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create shared_insights table: {e}"))
        })?;

        // Indexes for shared insights
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_shared_insights_user
             ON shared_insights(user_id, created_at DESC)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create index: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_shared_insights_activity
             ON shared_insights(source_activity_id)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create index: {e}")))?;

        // Insight reactions
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS insight_reactions (
                id UUID PRIMARY KEY,
                insight_id UUID NOT NULL REFERENCES shared_insights(id) ON DELETE CASCADE,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                reaction_type TEXT NOT NULL
                    CHECK (reaction_type IN ('like', 'celebrate', 'inspire', 'support')),
                created_at TIMESTAMPTZ NOT NULL,
                UNIQUE(insight_id, user_id)
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create insight_reactions table: {e}"))
        })?;

        // Indexes for insight reactions
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_insight_reactions_insight
             ON insight_reactions(insight_id, reaction_type)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create index: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_insight_reactions_user
             ON insight_reactions(user_id, created_at DESC)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create index: {e}")))?;

        // Adapted insights: personalized versions of shared insights
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS adapted_insights (
                id UUID PRIMARY KEY,
                source_insight_id UUID NOT NULL REFERENCES shared_insights(id) ON DELETE CASCADE,
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                adapted_content TEXT NOT NULL,
                adaptation_context TEXT,
                was_helpful BOOLEAN,
                created_at TIMESTAMPTZ NOT NULL,
                UNIQUE(source_insight_id, user_id)
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create adapted_insights table: {e}")))?;

        // Indexes for adapted insights
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_adapted_insights_user
             ON adapted_insights(user_id, created_at DESC)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create index: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_adapted_insights_source
             ON adapted_insights(source_insight_id)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create index: {e}")))?;

        // PostgreSQL trigger functions for maintaining denormalized reaction/adapt counts
        sqlx::query(
            r"
            CREATE OR REPLACE FUNCTION update_shared_insight_reaction_count()
            RETURNS TRIGGER AS $$
            BEGIN
                IF TG_OP = 'INSERT' THEN
                    UPDATE shared_insights
                    SET reaction_count = reaction_count + 1,
                        updated_at = NOW()
                    WHERE id = NEW.insight_id;
                ELSIF TG_OP = 'DELETE' THEN
                    UPDATE shared_insights
                    SET reaction_count = reaction_count - 1,
                        updated_at = NOW()
                    WHERE id = OLD.insight_id;
                END IF;
                RETURN NULL;
            END;
            $$ LANGUAGE plpgsql
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create trigger function: {e}")))?;

        sqlx::query(
            r"
            CREATE OR REPLACE FUNCTION update_shared_insight_adapt_count()
            RETURNS TRIGGER AS $$
            BEGIN
                IF TG_OP = 'INSERT' THEN
                    UPDATE shared_insights
                    SET adapt_count = adapt_count + 1,
                        updated_at = NOW()
                    WHERE id = NEW.source_insight_id;
                ELSIF TG_OP = 'DELETE' THEN
                    UPDATE shared_insights
                    SET adapt_count = adapt_count - 1,
                        updated_at = NOW()
                    WHERE id = OLD.source_insight_id;
                END IF;
                RETURN NULL;
            END;
            $$ LANGUAGE plpgsql
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create trigger function: {e}")))?;

        // Create triggers (DROP IF EXISTS + CREATE to be idempotent)
        sqlx::query("DROP TRIGGER IF EXISTS trg_insight_reactions_change ON insight_reactions")
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to drop trigger: {e}")))?;

        sqlx::query(
            r"
            CREATE TRIGGER trg_insight_reactions_change
            AFTER INSERT OR DELETE ON insight_reactions
            FOR EACH ROW
            EXECUTE FUNCTION update_shared_insight_reaction_count()
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create trigger: {e}")))?;

        sqlx::query("DROP TRIGGER IF EXISTS trg_adapted_insights_change ON adapted_insights")
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to drop trigger: {e}")))?;

        sqlx::query(
            r"
            CREATE TRIGGER trg_adapted_insights_change
            AFTER INSERT OR DELETE ON adapted_insights
            FOR EACH ROW
            EXECUTE FUNCTION update_shared_insight_adapt_count()
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create trigger: {e}")))?;

        Ok(())
    }

    async fn create_indexes(&self) -> AppResult<()> {
        self.create_user_indexes().await?;
        self.create_api_key_indexes().await?;
        self.create_a2a_indexes().await?;
        self.create_admin_token_indexes().await?;
        self.create_jwt_usage_indexes().await?;
        self.create_tenant_indexes().await?;

        Ok(())
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
