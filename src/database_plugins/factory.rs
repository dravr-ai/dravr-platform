// ABOUTME: Database factory and provider abstraction for multi-database support
// ABOUTME: Provides unified interface for SQLite and PostgreSQL with runtime database selection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//! Database factory for creating database providers
//!
//! This module provides automatic database type detection and creation
//! based on connection strings.

use super::DatabaseProvider;
use super::{
    A2ARepository, AdminRepository, ApiKeyRepository, ChatRepository, FitnessConfigRepository,
    ImpersonationRepository, InsightRepository, LlmCredentialRepository, LlmUsageRepository,
    NotificationRepository, OAuth2ServerRepository, OAuthClientStateRepository,
    OAuthTokenRepository, PasswordResetRepository, ProfileRepository, ProviderConnectionRepository,
    SecurityRepository, TenantRepository, ToolSelectionRepository, UsageCounterRepository,
    UsageRepository, UserMcpTokenRepository, UserRepository,
};
use crate::admin::models::{
    AdminToken, AdminTokenUsage, CreateAdminTokenRequest, GeneratedAdminToken,
};
use crate::api_keys::{ApiKey, ApiKeyUsage, ApiKeyUsageStats};
use crate::config::fitness::FitnessConfig;
use crate::config::social::SocialInsightsConfig;
use crate::database::{
    A2AUsage, A2AUsageStats, ConversationRecord, ConversationSummary, CreateUserMcpTokenRequest,
    MessageRecord, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};
use crate::errors::{AppError, AppResult};
use crate::models::OAuthNotification;
use crate::models::{
    AuthorizationCode, ConnectionType, OAuthApp, ProviderConnection, Tenant, TenantPlan,
    TenantToolOverride, ToolCatalogEntry, ToolCategory, User, UserOAuthApp, UserOAuthToken,
    UserStatus,
};
use crate::oauth2_client::OAuthClientState;
use crate::oauth2_server::models::{OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State};
use crate::pagination::{CursorPage, PaginationParams};
use crate::permissions::impersonation::ImpersonationSession;
use crate::security::audit::AuditEvent;
use crate::security::key_rotation::KeyVersion;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::models::a2a::{A2AClient, A2ASession, A2ATask, TaskStatus};
use pierre_core::models::usage::{InsertLlmUsage, LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::{
    AddMessageParams, JwtUsage, LlmUsageRecord, RequestLog, ToolUsage, UsageCounterRecord,
};
use pierre_core::models::{
    LlmCredentialRecord, LlmCredentialSummary, TenantId, TenantOAuthCredentials,
};
#[cfg(not(feature = "postgresql"))]
use tracing::error;
use tracing::{debug, info};
use uuid::Uuid;

#[cfg(feature = "postgresql")]
use super::postgres::PostgresDatabase;
#[cfg(feature = "postgresql")]
use crate::config::environment::PostgresPoolConfig;
// Phase 3: Use crate::database::Database directly (eliminates sqlite.rs wrapper)
use crate::database::Database as SqliteDatabase;

/// Supported database types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseType {
    /// `SQLite` embedded database
    SQLite,
    /// `PostgreSQL` database server
    PostgreSQL,
}

/// Database instance wrapper that delegates to the appropriate implementation
#[derive(Clone)]
pub enum Database {
    /// `SQLite` database instance
    SQLite(SqliteDatabase),
    /// `PostgreSQL` database instance (requires postgresql feature)
    #[cfg(feature = "postgresql")]
    PostgreSQL(PostgresDatabase),
}

impl Database {
    /// Get a descriptive string for the current database backend
    #[must_use]
    pub const fn backend_info(&self) -> &'static str {
        match self {
            Self::SQLite(_) => "SQLite (Local Development)",
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_) => "PostgreSQL (Cloud-Ready)",
        }
    }

    /// Get the database type enum
    #[must_use]
    pub const fn database_type(&self) -> DatabaseType {
        match self {
            Self::SQLite(_) => DatabaseType::SQLite,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_) => DatabaseType::PostgreSQL,
        }
    }

    /// Get detailed database information for logging/monitoring
    #[must_use]
    pub fn info_summary(&self) -> String {
        match self {
            Self::SQLite(_) => "Database Backend: SQLite\n\
                     Type: Embedded file-based database\n\
                     Use Case: Local development and testing\n\
                     Features: Zero-configuration, serverless, lightweight"
                .to_owned(),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_) => "Database Backend: PostgreSQL\n\
                     Type: Client-server relational database\n\
                     Use Case: Production and cloud deployments\n\
                     Features: Concurrent access, advanced queries, scalability"
                .to_owned(),
        }
    }

    /// Get the underlying `SQLite` connection pool if this is a `SQLite` database.
    ///
    /// Returns `None` for `PostgreSQL` databases.
    #[must_use]
    pub const fn sqlite_pool(&self) -> Option<&sqlx::Pool<sqlx::Sqlite>> {
        match self {
            Self::SQLite(db) => Some(db.pool()),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_) => None,
        }
    }

    /// Get a reference to the underlying `SQLite` database if this is a `SQLite` backend.
    ///
    /// Returns `None` for `PostgreSQL` databases. The returned reference implements
    /// domain-specific repository traits (`RecipeRepository`, `CoachesRepository`,
    /// `MobilityRepository`, `SocialRepository`).
    #[must_use]
    pub const fn sqlite_database(&self) -> Option<&SqliteDatabase> {
        match self {
            Self::SQLite(db) => Some(db),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_) => None,
        }
    }

    /// Get the underlying `PostgreSQL` connection pool if this is a `PostgreSQL` database.
    ///
    /// Returns `None` for `SQLite` databases.
    #[cfg(feature = "postgresql")]
    #[must_use]
    pub fn postgres_pool(&self) -> Option<&sqlx::Pool<sqlx::Postgres>> {
        match self {
            Self::SQLite(_) => None,
            Self::PostgreSQL(db) => Some(db.pool()),
        }
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
        match self {
            Self::SQLite(db) => db.update_encryption_key(new_key),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_encryption_key(new_key),
        }
    }

    /// Create a new database instance based on the connection string (internal implementation)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database URL format is unsupported or invalid
    /// - `PostgreSQL` feature is not enabled when `PostgreSQL` URL is provided
    /// - Database connection fails
    /// - Database initialization or migration fails
    /// - Encryption key is invalid
    async fn new_impl(
        database_url: &str,
        encryption_key: Vec<u8>,
        #[cfg(feature = "postgresql")] pool_config: &PostgresPoolConfig,
    ) -> AppResult<Self> {
        debug!(
            "Detecting database type from URL: {}",
            redact_database_url(database_url)
        );
        let db_type = detect_database_type(database_url)?;
        info!("Detected database type: {:?}", db_type);

        Self::create_database_instance(
            db_type,
            database_url,
            encryption_key,
            #[cfg(feature = "postgresql")]
            pool_config,
        )
        .await
    }

    async fn create_database_instance(
        db_type: DatabaseType,
        database_url: &str,
        encryption_key: Vec<u8>,
        #[cfg(feature = "postgresql")] pool_config: &PostgresPoolConfig,
    ) -> AppResult<Self> {
        match db_type {
            DatabaseType::SQLite => Self::initialize_sqlite(database_url, encryption_key).await,
            #[cfg(feature = "postgresql")]
            DatabaseType::PostgreSQL => {
                Self::initialize_postgresql(database_url, encryption_key, pool_config).await
            }
            #[cfg(not(feature = "postgresql"))]
            DatabaseType::PostgreSQL => Self::postgresql_not_enabled(),
        }
    }

    async fn initialize_sqlite(database_url: &str, encryption_key: Vec<u8>) -> AppResult<Self> {
        info!("Initializing SQLite database");
        let db = SqliteDatabase::new(database_url, encryption_key).await?;
        info!("SQLite database initialized successfully");
        Ok(Self::SQLite(db))
    }

    #[cfg(feature = "postgresql")]
    async fn initialize_postgresql(
        database_url: &str,
        encryption_key: Vec<u8>,
        pool_config: &PostgresPoolConfig,
    ) -> AppResult<Self> {
        info!("Initializing PostgreSQL database");
        let db = PostgresDatabase::new(database_url, encryption_key, pool_config).await?;
        info!("PostgreSQL database initialized successfully");
        Ok(Self::PostgreSQL(db))
    }

    #[cfg(not(feature = "postgresql"))]
    fn postgresql_not_enabled() -> AppResult<Self> {
        let err_msg = "PostgreSQL support not enabled. Enable the 'postgresql' feature flag.";
        error!("{}", err_msg);
        Err(AppError::config(err_msg))
    }

    /// Create a new database instance based on the connection string (public API)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database URL format is unsupported or invalid
    /// - `PostgreSQL` feature is not enabled when `PostgreSQL` URL is provided
    /// - Database connection fails
    /// - Database initialization or migration fails
    /// - Encryption key is invalid
    pub async fn new(
        database_url: &str,
        encryption_key: Vec<u8>,
        #[cfg(feature = "postgresql")] pool_config: &PostgresPoolConfig,
    ) -> AppResult<Self> {
        #[cfg(feature = "postgresql")]
        {
            Self::new_impl(database_url, encryption_key, pool_config).await
        }
        #[cfg(not(feature = "postgresql"))]
        {
            Self::new_impl(database_url, encryption_key).await
        }
    }

    /// Check if auto-approval is enabled for new user registrations
    ///
    /// Returns `Some(true/false)` if explicitly set in database,
    /// or `None` if no database setting exists (caller should use config default).
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn is_auto_approval_enabled(&self) -> AppResult<Option<bool>> {
        match self {
            Self::SQLite(db) => db.is_auto_approval_enabled().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.is_auto_approval_enabled().await,
        }
    }

    /// Set auto-approval enabled state
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn set_auto_approval_enabled(&self, enabled: bool) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.set_auto_approval_enabled(enabled).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.set_auto_approval_enabled(enabled).await,
        }
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
        match self {
            Self::SQLite(db) => db.get_social_insights_config().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_social_insights_config().await,
        }
    }

    /// Set social insights configuration in database
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails or JSON serialization fails
    pub async fn set_social_insights_config(&self, config: &SocialInsightsConfig) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.set_social_insights_config(config).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.set_social_insights_config(config).await,
        }
    }

    /// Delete social insights configuration from database (revert to defaults)
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn delete_social_insights_config(&self) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.delete_social_insights_config().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_social_insights_config().await,
        }
    }
}

/// Redact credentials from a database URL for safe logging.
///
/// Replaces `user:password@` with `user:***@` in connection strings.
/// `SQLite` URLs and URLs without credentials are returned unchanged.
fn redact_database_url(url: &str) -> String {
    // Only redact postgres-style URLs that may contain credentials
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        // Look for user:password@host pattern
        if let Some(at_pos) = after_scheme.find('@') {
            let userinfo = &after_scheme[..at_pos];
            if let Some(colon_pos) = userinfo.find(':') {
                let username = &userinfo[..colon_pos];
                let rest = &after_scheme[at_pos..];
                return format!("{}://{username}:***{rest}", &url[..scheme_end]);
            }
        }
    }
    url.to_owned()
}

/// Automatically detect database type from connection string.
///
/// # Errors
///
/// Returns an error if:
/// - Database URL format is not recognized (must start with `sqlite:` or `postgresql://`)
/// - `PostgreSQL` URL is provided but `PostgreSQL` feature is not enabled
/// - Connection string is malformed or empty
pub fn detect_database_type(database_url: &str) -> AppResult<DatabaseType> {
    if database_url.starts_with("sqlite:") {
        Ok(DatabaseType::SQLite)
    } else if database_url.starts_with("postgresql://") || database_url.starts_with("postgres://") {
        #[cfg(feature = "postgresql")]
        return Ok(DatabaseType::PostgreSQL);

        #[cfg(not(feature = "postgresql"))]
        return Err(AppError::config(
            "PostgreSQL connection string detected, but PostgreSQL support is not enabled. \
             Enable the 'postgresql' feature flag in Cargo.toml",
        ));
    } else {
        Err(AppError::config(format!(
            "Unsupported database URL format: {database_url}. \
             Supported formats: sqlite:path/to/db.sqlite, postgresql://user:pass@host/db"
        )))
    }
}

// Implement DatabaseProvider for the enum by delegating to the appropriate implementation
#[async_trait]
#[async_trait]
impl UserRepository for Database {
    async fn create(&self, user: &User) -> AppResult<uuid::Uuid> {
        match self {
            Self::SQLite(db) => UserRepository::create(db, user).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => UserRepository::create(db, user).await,
        }
    }
    async fn get(&self, user_id: uuid::Uuid, tenant_id: TenantId) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.get(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get(user_id, tenant_id).await,
        }
    }
    async fn get_global(&self, user_id: uuid::Uuid) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.get_global(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_global(user_id).await,
        }
    }
    async fn get_by_email(&self, email: &str) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.get_by_email(email).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_email(email).await,
        }
    }
    async fn get_by_email_required(&self, email: &str) -> AppResult<User> {
        match self {
            Self::SQLite(db) => db.get_by_email_required(email).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_email_required(email).await,
        }
    }
    async fn get_by_firebase_uid(&self, firebase_uid: &str) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.get_by_firebase_uid(firebase_uid).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_firebase_uid(firebase_uid).await,
        }
    }
    async fn update_last_active(&self, user_id: uuid::Uuid) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_last_active(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_last_active(user_id).await,
        }
    }
    async fn count(&self) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => db.count().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.count().await,
        }
    }
    async fn get_by_status(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<User>> {
        match self {
            Self::SQLite(db) => db.get_by_status(status, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_status(status, tenant_id).await,
        }
    }
    async fn get_first_admin_user(&self) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.get_first_admin_user().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_first_admin_user().await,
        }
    }
    async fn get_by_status_cursor(
        &self,
        status: &str,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<User>> {
        match self {
            Self::SQLite(db) => db.get_by_status_cursor(status, params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_status_cursor(status, params).await,
        }
    }
    async fn update_status(
        &self,
        user_id: uuid::Uuid,
        new_status: UserStatus,
        approved_by: Option<uuid::Uuid>,
    ) -> AppResult<User> {
        match self {
            Self::SQLite(db) => db.update_status(user_id, new_status, approved_by).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_status(user_id, new_status, approved_by).await,
        }
    }
    async fn update_tenant_id(&self, user_id: uuid::Uuid, tenant_id: TenantId) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_tenant_id(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_tenant_id(user_id, tenant_id).await,
        }
    }
    async fn update_password(&self, user_id: uuid::Uuid, password_hash: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_password(user_id, password_hash).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_password(user_id, password_hash).await,
        }
    }
    async fn update_display_name(
        &self,
        user_id: uuid::Uuid,
        display_name: &str,
    ) -> AppResult<User> {
        match self {
            Self::SQLite(db) => db.update_display_name(user_id, display_name).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_display_name(user_id, display_name).await,
        }
    }
    async fn delete(&self, user_id: uuid::Uuid) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.delete(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete(user_id).await,
        }
    }
    async fn has_synthetic_activities(&self, user_id: Uuid) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.user_has_synthetic_activities_impl(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.has_synthetic_activities(user_id).await,
        }
    }
}

#[async_trait]
impl ProfileRepository for Database {
    async fn upsert_profile(
        &self,
        user_id: uuid::Uuid,
        profile_data: serde_json::Value,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.upsert_profile(user_id, profile_data).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.upsert_profile(user_id, profile_data).await,
        }
    }

    async fn get_profile(&self, user_id: uuid::Uuid) -> AppResult<Option<serde_json::Value>> {
        match self {
            Self::SQLite(db) => db.get_profile(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_profile(user_id).await,
        }
    }

    async fn create_goal(
        &self,
        user_id: uuid::Uuid,
        goal_data: serde_json::Value,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.create_goal(user_id, goal_data).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create_goal(user_id, goal_data).await,
        }
    }

    async fn get_goals(&self, user_id: uuid::Uuid) -> AppResult<Vec<serde_json::Value>> {
        match self {
            Self::SQLite(db) => db.get_goals(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_goals(user_id).await,
        }
    }

    async fn update_goal_progress(
        &self,
        goal_id: &str,
        user_id: Uuid,
        current_value: f64,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.update_goal_progress(goal_id, user_id, current_value)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.update_goal_progress(goal_id, user_id, current_value)
                    .await
            }
        }
    }

    async fn get_configuration(&self, user_id: &str) -> AppResult<Option<String>> {
        match self {
            Self::SQLite(db) => db.get_configuration(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_configuration(user_id).await,
        }
    }

    async fn save_configuration(&self, user_id: &str, config_json: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.save_configuration(user_id, config_json).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.save_configuration(user_id, config_json).await,
        }
    }
}

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

#[async_trait]
impl ApiKeyRepository for Database {
    async fn create(&self, api_key: &ApiKey) -> AppResult<()> {
        match self {
            Self::SQLite(db) => ApiKeyRepository::create(db, api_key).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ApiKeyRepository::create(db, api_key).await,
        }
    }
    async fn get_by_prefix(&self, prefix: &str, hash: &str) -> AppResult<Option<ApiKey>> {
        match self {
            Self::SQLite(db) => db.get_by_prefix(prefix, hash).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_prefix(prefix, hash).await,
        }
    }
    async fn get_for_user(&self, user_id: uuid::Uuid) -> AppResult<Vec<ApiKey>> {
        match self {
            Self::SQLite(db) => ApiKeyRepository::get_for_user(db, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ApiKeyRepository::get_for_user(db, user_id).await,
        }
    }
    async fn update_last_used(&self, api_key_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_last_used(api_key_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_last_used(api_key_id).await,
        }
    }
    async fn deactivate(&self, api_key_id: &str, user_id: uuid::Uuid) -> AppResult<()> {
        match self {
            Self::SQLite(db) => ApiKeyRepository::deactivate(db, api_key_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ApiKeyRepository::deactivate(db, api_key_id, user_id).await,
        }
    }
    async fn get_by_id(
        &self,
        api_key_id: &str,
        user_id: Option<Uuid>,
    ) -> AppResult<Option<ApiKey>> {
        match self {
            Self::SQLite(db) => ApiKeyRepository::get_by_id(db, api_key_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ApiKeyRepository::get_by_id(db, api_key_id, user_id).await,
        }
    }
    async fn get_filtered(
        &self,
        user_email: Option<&str>,
        active_only: bool,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> AppResult<Vec<ApiKey>> {
        match self {
            Self::SQLite(db) => {
                ApiKeyRepository::get_filtered(db, user_email, active_only, limit, offset).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_filtered(user_email, active_only, limit, offset)
                    .await
            }
        }
    }
    async fn cleanup_expired(&self) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.cleanup_expired().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.cleanup_expired().await,
        }
    }
    async fn get_expired(&self) -> AppResult<Vec<ApiKey>> {
        match self {
            Self::SQLite(db) => db.get_expired().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_expired().await,
        }
    }
}

#[async_trait]
impl UsageRepository for Database {
    async fn record_api_key(&self, usage: &ApiKeyUsage) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.record_api_key(usage).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.record_api_key(usage).await,
        }
    }
    async fn get_api_key_current(&self, api_key_id: &str) -> AppResult<u32> {
        match self {
            Self::SQLite(db) => db.get_api_key_current(api_key_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_api_key_current(api_key_id).await,
        }
    }
    async fn get_api_key_stats(
        &self,
        api_key_id: &str,
        start_date: chrono::DateTime<chrono::Utc>,
        end_date: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<ApiKeyUsageStats> {
        match self {
            Self::SQLite(db) => db.get_api_key_stats(api_key_id, start_date, end_date).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_api_key_stats(api_key_id, start_date, end_date).await,
        }
    }
    async fn record_jwt_usage(&self, usage: &JwtUsage) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.record_jwt_usage(usage).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.record_jwt_usage(usage).await,
        }
    }
    async fn get_jwt_current_usage(&self, user_id: uuid::Uuid) -> AppResult<u32> {
        match self {
            Self::SQLite(db) => db.get_jwt_current_usage(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_jwt_current_usage(user_id).await,
        }
    }
    async fn get_request_logs(
        &self,
        user_id: Option<uuid::Uuid>,
        api_key_id: Option<&str>,
        start_time: Option<chrono::DateTime<chrono::Utc>>,
        end_time: Option<chrono::DateTime<chrono::Utc>>,
        status_filter: Option<&str>,
        tool_filter: Option<&str>,
    ) -> AppResult<Vec<RequestLog>> {
        match self {
            Self::SQLite(db) => {
                UsageRepository::get_request_logs(
                    db,
                    user_id,
                    api_key_id,
                    start_time,
                    end_time,
                    status_filter,
                    tool_filter,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_request_logs(
                    user_id,
                    api_key_id,
                    start_time,
                    end_time,
                    status_filter,
                    tool_filter,
                )
                .await
            }
        }
    }
    async fn get_system_stats(&self, tenant_id: Option<TenantId>) -> AppResult<(u64, u64)> {
        match self {
            Self::SQLite(db) => db.get_system_stats(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_system_stats(tenant_id).await,
        }
    }
    async fn get_top_tools_analysis(
        &self,
        user_id: uuid::Uuid,
        start_time: chrono::DateTime<chrono::Utc>,
        end_time: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<ToolUsage>> {
        match self {
            Self::SQLite(db) => {
                db.get_top_tools_analysis(user_id, start_time, end_time)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_top_tools_analysis(user_id, start_time, end_time)
                    .await
            }
        }
    }
}

#[async_trait]
impl UsageCounterRepository for Database {
    async fn increment_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
        amount: i64,
    ) -> AppResult<UsageCounterRecord> {
        match self {
            Self::SQLite(db) => {
                db.increment_usage_counter_impl(tenant_id, user_id, counter_key, period, amount)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.increment_counter(tenant_id, user_id, counter_key, period, amount)
                    .await
            }
        }
    }

    async fn get_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
    ) -> AppResult<UsageCounterRecord> {
        match self {
            Self::SQLite(db) => {
                db.get_usage_counter_impl(tenant_id, user_id, counter_key, period)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_counter(tenant_id, user_id, counter_key, period)
                    .await
            }
        }
    }

    async fn delete_old_counters(&self, period_before: &str) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.delete_old_usage_counters_impl(period_before).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_old_counters(period_before).await,
        }
    }
}

#[async_trait]
impl LlmUsageRepository for Database {
    async fn insert_llm_usage(&self, params: &InsertLlmUsage<'_>) -> AppResult<LlmUsageRecord> {
        match self {
            Self::SQLite(db) => db.insert_llm_usage_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.insert_llm_usage(params).await,
        }
    }

    async fn get_llm_usage_aggregates(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageAggregateRow>> {
        match self {
            Self::SQLite(db) => db.get_llm_usage_aggregates_impl(tenant_id, since).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_llm_usage_aggregates(tenant_id, since).await,
        }
    }

    async fn get_llm_usage_daily_series(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>> {
        match self {
            Self::SQLite(db) => db.get_llm_usage_daily_series_impl(tenant_id, since).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_llm_usage_daily_series(tenant_id, since).await,
        }
    }
}

#[async_trait]
impl A2ARepository for Database {
    async fn create_client(
        &self,
        client: &A2AClient,
        client_secret: &str,
        api_key_id: &str,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.create_client(client, client_secret, api_key_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create_client(client, client_secret, api_key_id).await,
        }
    }
    async fn get_client(&self, client_id: &str) -> AppResult<Option<A2AClient>> {
        match self {
            Self::SQLite(db) => A2ARepository::get_client(db, client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => A2ARepository::get_client(db, client_id).await,
        }
    }
    async fn get_client_by_api_key_id(&self, api_key_id: &str) -> AppResult<Option<A2AClient>> {
        match self {
            Self::SQLite(db) => db.get_client_by_api_key_id(api_key_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_client_by_api_key_id(api_key_id).await,
        }
    }
    async fn get_client_by_name(&self, name: &str) -> AppResult<Option<A2AClient>> {
        match self {
            Self::SQLite(db) => db.get_client_by_name(name).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_client_by_name(name).await,
        }
    }
    async fn list_clients(&self, user_id: &uuid::Uuid) -> AppResult<Vec<A2AClient>> {
        match self {
            Self::SQLite(db) => db.list_clients(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_clients(user_id).await,
        }
    }
    async fn deactivate_client(&self, client_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.deactivate_client(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.deactivate_client(client_id).await,
        }
    }
    async fn get_client_credentials(&self, client_id: &str) -> AppResult<Option<(String, String)>> {
        match self {
            Self::SQLite(db) => db.get_client_credentials(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_client_credentials(client_id).await,
        }
    }
    async fn invalidate_client_sessions(&self, client_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.invalidate_client_sessions(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.invalidate_client_sessions(client_id).await,
        }
    }
    async fn deactivate_client_api_keys(&self, client_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.deactivate_client_api_keys(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.deactivate_client_api_keys(client_id).await,
        }
    }
    async fn create_session(
        &self,
        client_id: &str,
        user_id: Option<&uuid::Uuid>,
        granted_scopes: &[String],
        expires_in_hours: i64,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => {
                A2ARepository::create_session(
                    db,
                    client_id,
                    user_id,
                    granted_scopes,
                    expires_in_hours,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                A2ARepository::create_session(
                    db,
                    client_id,
                    user_id,
                    granted_scopes,
                    expires_in_hours,
                )
                .await
            }
        }
    }
    async fn get_session(&self, session_token: &str) -> AppResult<Option<A2ASession>> {
        match self {
            Self::SQLite(db) => A2ARepository::get_session(db, session_token).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => A2ARepository::get_session(db, session_token).await,
        }
    }
    async fn update_session_activity(&self, session_token: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_session_activity(session_token).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_session_activity(session_token).await,
        }
    }
    async fn get_active_sessions(&self, client_id: &str) -> AppResult<Vec<A2ASession>> {
        match self {
            Self::SQLite(db) => A2ARepository::get_active_sessions(db, client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => A2ARepository::get_active_sessions(db, client_id).await,
        }
    }
    async fn create_task(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        task_type: &str,
        input_data: &serde_json::Value,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => {
                db.create_task(client_id, session_id, task_type, input_data)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.create_task(client_id, session_id, task_type, input_data)
                    .await
            }
        }
    }
    async fn get_task(&self, task_id: &str) -> AppResult<Option<A2ATask>> {
        match self {
            Self::SQLite(db) => db.get_task(task_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_task(task_id).await,
        }
    }
    async fn list_tasks(
        &self,
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<A2ATask>> {
        match self {
            Self::SQLite(db) => db.list_tasks(client_id, status_filter, limit, offset).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_tasks(client_id, status_filter, limit, offset).await,
        }
    }
    async fn update_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
        result: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_task_status(task_id, status, result, error).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_task_status(task_id, status, result, error).await,
        }
    }
    async fn record_usage(&self, usage: &A2AUsage) -> AppResult<()> {
        match self {
            Self::SQLite(db) => A2ARepository::record_usage(db, usage).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => A2ARepository::record_usage(db, usage).await,
        }
    }
    async fn get_client_current_usage(&self, client_id: &str) -> AppResult<u32> {
        match self {
            Self::SQLite(db) => db.get_client_current_usage(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_client_current_usage(client_id).await,
        }
    }
    async fn get_usage_stats(
        &self,
        client_id: &str,
        start_date: chrono::DateTime<chrono::Utc>,
        end_date: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<A2AUsageStats> {
        match self {
            Self::SQLite(db) => {
                A2ARepository::get_usage_stats(db, client_id, start_date, end_date).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                A2ARepository::get_usage_stats(db, client_id, start_date, end_date).await
            }
        }
    }
    async fn get_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> AppResult<Vec<(chrono::DateTime<chrono::Utc>, u32, u32)>> {
        match self {
            Self::SQLite(db) => db.get_client_usage_history(client_id, days).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_client_usage_history(client_id, days).await,
        }
    }
}

#[async_trait]
impl AdminRepository for Database {
    async fn create_token(
        &self,
        request: &CreateAdminTokenRequest,
        admin_jwt_secret: &str,
        jwks_manager: &dyn JwtSigner,
    ) -> AppResult<GeneratedAdminToken> {
        match self {
            Self::SQLite(db) => {
                AdminRepository::create_token(db, request, admin_jwt_secret, jwks_manager).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                AdminRepository::create_token(db, request, admin_jwt_secret, jwks_manager).await
            }
        }
    }
    async fn get_token_by_id(&self, token_id: &str) -> AppResult<Option<AdminToken>> {
        match self {
            Self::SQLite(db) => db.get_token_by_id(token_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_token_by_id(token_id).await,
        }
    }
    async fn get_token_by_prefix(&self, token_prefix: &str) -> AppResult<Option<AdminToken>> {
        match self {
            Self::SQLite(db) => db.get_token_by_prefix(token_prefix).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_token_by_prefix(token_prefix).await,
        }
    }
    async fn list_tokens(&self, include_inactive: bool) -> AppResult<Vec<AdminToken>> {
        match self {
            Self::SQLite(db) => AdminRepository::list_tokens(db, include_inactive).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => AdminRepository::list_tokens(db, include_inactive).await,
        }
    }
    async fn deactivate_token(&self, token_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.deactivate_token(token_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.deactivate_token(token_id).await,
        }
    }
    async fn update_token_last_used(
        &self,
        token_id: &str,
        ip_address: Option<&str>,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_token_last_used(token_id, ip_address).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_token_last_used(token_id, ip_address).await,
        }
    }
    async fn record_token_usage(&self, usage: &AdminTokenUsage) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.record_token_usage(usage).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.record_token_usage(usage).await,
        }
    }
    async fn get_token_usage_history(
        &self,
        token_id: &str,
        start_date: chrono::DateTime<chrono::Utc>,
        end_date: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<AdminTokenUsage>> {
        match self {
            Self::SQLite(db) => {
                db.get_token_usage_history(token_id, start_date, end_date)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_token_usage_history(token_id, start_date, end_date)
                    .await
            }
        }
    }
    async fn record_provisioned_key(
        &self,
        admin_token_id: &str,
        api_key_id: &str,
        user_email: &str,
        tier: &str,
        rate_limit_requests: u32,
        rate_limit_period: &str,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.record_provisioned_key(
                    admin_token_id,
                    api_key_id,
                    user_email,
                    tier,
                    rate_limit_requests,
                    rate_limit_period,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.record_provisioned_key(
                    admin_token_id,
                    api_key_id,
                    user_email,
                    tier,
                    rate_limit_requests,
                    rate_limit_period,
                )
                .await
            }
        }
    }
    async fn get_provisioned_keys(
        &self,
        admin_token_id: Option<&str>,
        start_date: chrono::DateTime<chrono::Utc>,
        end_date: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<serde_json::Value>> {
        match self {
            Self::SQLite(db) => {
                db.get_provisioned_keys(admin_token_id, start_date, end_date)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_provisioned_keys(admin_token_id, start_date, end_date)
                    .await
            }
        }
    }
}

#[async_trait]
impl ImpersonationRepository for Database {
    async fn create_session(&self, session: &ImpersonationSession) -> AppResult<()> {
        match self {
            Self::SQLite(db) => ImpersonationRepository::create_session(db, session).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ImpersonationRepository::create_session(db, session).await,
        }
    }

    async fn get_session(&self, session_id: &str) -> AppResult<Option<ImpersonationSession>> {
        match self {
            Self::SQLite(db) => ImpersonationRepository::get_session(db, session_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ImpersonationRepository::get_session(db, session_id).await,
        }
    }

    async fn get_active_session(&self, user_id: Uuid) -> AppResult<Option<ImpersonationSession>> {
        match self {
            Self::SQLite(db) => ImpersonationRepository::get_active_session(db, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ImpersonationRepository::get_active_session(db, user_id).await,
        }
    }

    async fn end_session(&self, session_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => ImpersonationRepository::end_session(db, session_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ImpersonationRepository::end_session(db, session_id).await,
        }
    }

    async fn end_all_sessions(&self, impersonator_id: Uuid) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.end_all_sessions(impersonator_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.end_all_sessions(impersonator_id).await,
        }
    }

    async fn list_sessions(
        &self,
        impersonator_id: Option<Uuid>,
        target_user_id: Option<Uuid>,
        active_only: bool,
        limit: u32,
    ) -> AppResult<Vec<ImpersonationSession>> {
        match self {
            Self::SQLite(db) => {
                ImpersonationRepository::list_sessions(
                    db,
                    impersonator_id,
                    target_user_id,
                    active_only,
                    limit,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                ImpersonationRepository::list_sessions(
                    db,
                    impersonator_id,
                    target_user_id,
                    active_only,
                    limit,
                )
                .await
            }
        }
    }
}

#[async_trait]
impl UserMcpTokenRepository for Database {
    async fn create_token(
        &self,
        user_id: Uuid,
        request: &CreateUserMcpTokenRequest,
    ) -> AppResult<UserMcpTokenCreated> {
        match self {
            Self::SQLite(db) => UserMcpTokenRepository::create_token(db, user_id, request).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                UserMcpTokenRepository::create_token(db, user_id, request).await
            }
        }
    }

    async fn validate_token(&self, token_value: &str) -> AppResult<Uuid> {
        match self {
            Self::SQLite(db) => db.validate_token(token_value).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.validate_token(token_value).await,
        }
    }

    async fn list_tokens(&self, user_id: Uuid) -> AppResult<Vec<UserMcpTokenInfo>> {
        match self {
            Self::SQLite(db) => UserMcpTokenRepository::list_tokens(db, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => UserMcpTokenRepository::list_tokens(db, user_id).await,
        }
    }

    async fn revoke_token(&self, token_id: &str, user_id: Uuid) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.revoke_token(token_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.revoke_token(token_id, user_id).await,
        }
    }

    async fn get_token(&self, token_id: &str, user_id: Uuid) -> AppResult<Option<UserMcpToken>> {
        match self {
            Self::SQLite(db) => UserMcpTokenRepository::get_token(db, token_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => UserMcpTokenRepository::get_token(db, token_id, user_id).await,
        }
    }

    async fn cleanup_expired_tokens(&self) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.cleanup_expired_tokens().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.cleanup_expired_tokens().await,
        }
    }
}

#[async_trait]
impl TenantRepository for Database {
    async fn create(&self, tenant: &Tenant) -> AppResult<()> {
        match self {
            Self::SQLite(db) => TenantRepository::create(db, tenant).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => TenantRepository::create(db, tenant).await,
        }
    }
    async fn get_by_id(&self, tenant_id: TenantId) -> AppResult<Tenant> {
        match self {
            Self::SQLite(db) => TenantRepository::get_by_id(db, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => TenantRepository::get_by_id(db, tenant_id).await,
        }
    }
    async fn get_by_slug(&self, slug: &str) -> AppResult<Tenant> {
        match self {
            Self::SQLite(db) => db.get_by_slug(slug).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_slug(slug).await,
        }
    }
    async fn list_for_user(&self, user_id: uuid::Uuid) -> AppResult<Vec<Tenant>> {
        match self {
            Self::SQLite(db) => db.list_for_user(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_for_user(user_id).await,
        }
    }
    async fn store_oauth_credentials(&self, credentials: &TenantOAuthCredentials) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_oauth_credentials(credentials).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_oauth_credentials(credentials).await,
        }
    }
    async fn get_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<TenantOAuthCredentials>> {
        match self {
            Self::SQLite(db) => db.get_oauth_providers(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_oauth_providers(tenant_id).await,
        }
    }
    async fn get_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<TenantOAuthCredentials>> {
        match self {
            Self::SQLite(db) => db.get_oauth_credentials(tenant_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_oauth_credentials(tenant_id, provider).await,
        }
    }
    async fn create_oauth_app(&self, app: &OAuthApp) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.create_oauth_app(app).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create_oauth_app(app).await,
        }
    }
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> AppResult<OAuthApp> {
        match self {
            Self::SQLite(db) => db.get_oauth_app_by_client_id(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_oauth_app_by_client_id(client_id).await,
        }
    }
    async fn list_oauth_apps_for_user(&self, user_id: uuid::Uuid) -> AppResult<Vec<OAuthApp>> {
        match self {
            Self::SQLite(db) => db.list_oauth_apps_for_user(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_oauth_apps_for_user(user_id).await,
        }
    }
    async fn get_all(&self) -> AppResult<Vec<Tenant>> {
        match self {
            Self::SQLite(db) => TenantRepository::get_all(db).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => TenantRepository::get_all(db).await,
        }
    }
    async fn get_user_role(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<String>> {
        match self {
            Self::SQLite(db) => TenantRepository::get_user_role(db, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_user_role(user_id, tenant_id).await,
        }
    }
}

#[async_trait]
impl ToolSelectionRepository for Database {
    async fn get_tool_catalog(&self) -> AppResult<Vec<ToolCatalogEntry>> {
        match self {
            Self::SQLite(db) => db.get_tool_catalog_impl().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tool_catalog().await,
        }
    }

    async fn get_tool_catalog_entry(&self, tool_name: &str) -> AppResult<Option<ToolCatalogEntry>> {
        match self {
            Self::SQLite(db) => db.get_tool_catalog_entry_impl(tool_name).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tool_catalog_entry(tool_name).await,
        }
    }

    async fn get_tools_by_category(
        &self,
        category: ToolCategory,
    ) -> AppResult<Vec<ToolCatalogEntry>> {
        match self {
            Self::SQLite(db) => db.get_tools_by_category_impl(category).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tools_by_category(category).await,
        }
    }

    async fn get_tools_by_min_plan(&self, plan: TenantPlan) -> AppResult<Vec<ToolCatalogEntry>> {
        match self {
            Self::SQLite(db) => db.get_tools_by_min_plan_impl(plan).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tools_by_min_plan(plan).await,
        }
    }

    async fn get_overrides(&self, tenant_id: TenantId) -> AppResult<Vec<TenantToolOverride>> {
        match self {
            Self::SQLite(db) => db.get_tenant_tool_overrides_impl(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_overrides(tenant_id).await,
        }
    }

    async fn get_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> AppResult<Option<TenantToolOverride>> {
        match self {
            Self::SQLite(db) => db.get_tenant_tool_override_impl(tenant_id, tool_name).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_override(tenant_id, tool_name).await,
        }
    }

    async fn upsert_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
        is_enabled: bool,
        enabled_by_user_id: Option<Uuid>,
        reason: Option<String>,
    ) -> AppResult<TenantToolOverride> {
        match self {
            Self::SQLite(db) => {
                db.upsert_tenant_tool_override_impl(
                    tenant_id,
                    tool_name,
                    is_enabled,
                    enabled_by_user_id,
                    reason,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.upsert_override(tenant_id, tool_name, is_enabled, enabled_by_user_id, reason)
                    .await
            }
        }
    }

    async fn delete_override(&self, tenant_id: TenantId, tool_name: &str) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.delete_tenant_tool_override_impl(tenant_id, tool_name)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_override(tenant_id, tool_name).await,
        }
    }

    async fn count_enabled_tools(&self, tenant_id: TenantId) -> AppResult<usize> {
        match self {
            Self::SQLite(db) => db.count_enabled_tools_impl(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.count_enabled_tools(tenant_id).await,
        }
    }
}

#[async_trait]
impl LlmCredentialRepository for Database {
    async fn store_credentials(&self, record: &LlmCredentialRecord) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_credentials(record).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_credentials(record).await,
        }
    }

    async fn get_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<Option<LlmCredentialRecord>> {
        match self {
            Self::SQLite(db) => db.get_credentials(tenant_id, user_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_credentials(tenant_id, user_id, provider).await,
        }
    }

    async fn list_credentials(&self, tenant_id: TenantId) -> AppResult<Vec<LlmCredentialSummary>> {
        match self {
            Self::SQLite(db) => db.list_credentials(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_credentials(tenant_id).await,
        }
    }

    async fn delete_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.delete_credentials(tenant_id, user_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_credentials(tenant_id, user_id, provider).await,
        }
    }

    async fn get_admin_config_override(
        &self,
        config_key: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<String>> {
        match self {
            Self::SQLite(db) => db.get_admin_config_override(config_key, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_admin_config_override(config_key, tenant_id).await,
        }
    }
}

#[async_trait]
impl FitnessConfigRepository for Database {
    async fn save_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => {
                db.save_tenant_config(tenant_id, configuration_name, config)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.save_tenant_config(tenant_id, configuration_name, config)
                    .await
            }
        }
    }

    async fn save_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => {
                db.save_user_config(tenant_id, user_id, configuration_name, config)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.save_user_config(tenant_id, user_id, configuration_name, config)
                    .await
            }
        }
    }

    async fn get_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>> {
        match self {
            Self::SQLite(db) => db.get_tenant_config(tenant_id, configuration_name).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tenant_config(tenant_id, configuration_name).await,
        }
    }

    async fn get_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>> {
        match self {
            Self::SQLite(db) => {
                db.get_user_config(tenant_id, user_id, configuration_name)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_user_config(tenant_id, user_id, configuration_name)
                    .await
            }
        }
    }

    async fn list_tenant_configurations(&self, tenant_id: TenantId) -> AppResult<Vec<String>> {
        match self {
            Self::SQLite(db) => db.list_tenant_configurations(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_tenant_configurations(tenant_id).await,
        }
    }

    async fn list_user_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<String>> {
        match self {
            Self::SQLite(db) => db.list_user_configurations(tenant_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_user_configurations(tenant_id, user_id).await,
        }
    }

    async fn delete_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.delete_config(tenant_id, user_id, configuration_name)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.delete_config(tenant_id, user_id, configuration_name)
                    .await
            }
        }
    }
}

#[async_trait]
impl ChatRepository for Database {
    async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        system_prompt: Option<&str>,
    ) -> AppResult<ConversationRecord> {
        match self {
            Self::SQLite(db) => {
                db.chat_create_conversation_impl(user_id, tenant_id, title, model, system_prompt)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.create_conversation(user_id, tenant_id, title, model, system_prompt)
                    .await
            }
        }
    }
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<ConversationRecord>> {
        match self {
            Self::SQLite(db) => {
                db.chat_get_conversation_impl(conversation_id, user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_conversation(conversation_id, user_id, tenant_id)
                    .await
            }
        }
    }
    async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ConversationSummary>> {
        match self {
            Self::SQLite(db) => {
                db.chat_list_conversations_impl(user_id, tenant_id, limit, offset)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.list_conversations(user_id, tenant_id, limit, offset)
                    .await
            }
        }
    }
    async fn update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.chat_update_conversation_title_impl(conversation_id, user_id, tenant_id, title)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.update_conversation_title(conversation_id, user_id, tenant_id, title)
                    .await
            }
        }
    }
    async fn delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.chat_delete_conversation_impl(conversation_id, user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.delete_conversation(conversation_id, user_id, tenant_id)
                    .await
            }
        }
    }
    async fn add_message(&self, params: &AddMessageParams<'_>) -> AppResult<MessageRecord> {
        match self {
            Self::SQLite(db) => db.chat_add_message_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.add_message(params).await,
        }
    }
    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> AppResult<Vec<MessageRecord>> {
        match self {
            Self::SQLite(db) => db.chat_get_messages_impl(conversation_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_messages(conversation_id, user_id).await,
        }
    }
    async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        limit: i64,
    ) -> AppResult<Vec<MessageRecord>> {
        match self {
            Self::SQLite(db) => {
                db.chat_get_recent_messages_impl(conversation_id, user_id, limit)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_recent_messages(conversation_id, user_id, limit)
                    .await
            }
        }
    }
    async fn get_message_count(&self, conversation_id: &str, user_id: &str) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => {
                db.chat_get_message_count_impl(conversation_id, user_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_message_count(conversation_id, user_id).await,
        }
    }
    async fn count_conversations(&self, user_id: &str, tenant_id: TenantId) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => db.chat_count_conversations_impl(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.count_conversations(user_id, tenant_id).await,
        }
    }
    async fn delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => {
                db.chat_delete_all_user_conversations_impl(user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_all_user_conversations(user_id, tenant_id).await,
        }
    }
}

#[async_trait]
impl SecurityRepository for Database {
    async fn store_key_version(&self, version: &KeyVersion) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_key_version(version).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_key_version(version).await,
        }
    }
    async fn get_key_versions(&self, tenant_id: Option<TenantId>) -> AppResult<Vec<KeyVersion>> {
        match self {
            Self::SQLite(db) => db.get_key_versions(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_key_versions(tenant_id).await,
        }
    }
    async fn get_current_key_version(
        &self,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<KeyVersion>> {
        match self {
            Self::SQLite(db) => db.get_current_key_version(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_current_key_version(tenant_id).await,
        }
    }
    async fn update_key_version_status(
        &self,
        tenant_id: Option<TenantId>,
        version: u32,
        is_active: bool,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.update_key_version_status(tenant_id, version, is_active)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.update_key_version_status(tenant_id, version, is_active)
                    .await
            }
        }
    }
    async fn delete_old_key_versions(
        &self,
        tenant_id: Option<TenantId>,
        keep_count: u32,
    ) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.delete_old_key_versions(tenant_id, keep_count).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_old_key_versions(tenant_id, keep_count).await,
        }
    }
    async fn store_audit_event(&self, event: &AuditEvent) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_audit_event(event).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_audit_event(event).await,
        }
    }
    async fn get_audit_events(
        &self,
        tenant_id: Option<TenantId>,
        event_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<AuditEvent>> {
        match self {
            Self::SQLite(db) => db.get_audit_events(tenant_id, event_type, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_audit_events(tenant_id, event_type, limit).await,
        }
    }
    async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.get_or_create_system_secret(secret_type).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_or_create_system_secret(secret_type).await,
        }
    }
    async fn get_system_secret(&self, secret_type: &str) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.get_system_secret(secret_type).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_system_secret(secret_type).await,
        }
    }
    async fn update_system_secret(&self, secret_type: &str, new_value: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_system_secret(secret_type, new_value).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_system_secret(secret_type, new_value).await,
        }
    }
    async fn save_rsa_keypair(
        &self,
        kid: &str,
        private_key_pem: &str,
        public_key_pem: &str,
        created_at: chrono::DateTime<chrono::Utc>,
        is_active: bool,
        key_size_bits: i32,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.save_rsa_keypair(
                    kid,
                    private_key_pem,
                    public_key_pem,
                    created_at,
                    is_active,
                    usize::try_from(key_size_bits).unwrap_or(2048),
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.save_rsa_keypair(
                    kid,
                    private_key_pem,
                    public_key_pem,
                    created_at,
                    is_active,
                    key_size_bits,
                )
                .await
            }
        }
    }
    async fn load_rsa_keypairs(
        &self,
    ) -> AppResult<Vec<(String, String, String, chrono::DateTime<chrono::Utc>, bool)>> {
        match self {
            Self::SQLite(db) => db.load_rsa_keypairs().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.load_rsa_keypairs().await,
        }
    }
    async fn update_rsa_keypair_active_status(&self, kid: &str, is_active: bool) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_rsa_keypair_active_status(kid, is_active).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_rsa_keypair_active_status(kid, is_active).await,
        }
    }
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.encrypt_data_with_aad(data, aad),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.encrypt_data_with_aad(data, aad),
        }
    }
    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.decrypt_data_with_aad(encrypted, aad),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.decrypt_data_with_aad(encrypted, aad),
        }
    }
}

#[async_trait]
impl NotificationRepository for Database {
    async fn store(
        &self,
        user_id: Uuid,
        provider: &str,
        success: bool,
        message: &str,
        expires_at: Option<&str>,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => {
                NotificationRepository::store(db, user_id, provider, success, message, expires_at)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                NotificationRepository::store(db, user_id, provider, success, message, expires_at)
                    .await
            }
        }
    }

    async fn get_unread(&self, user_id: Uuid) -> AppResult<Vec<OAuthNotification>> {
        match self {
            Self::SQLite(db) => NotificationRepository::get_unread(db, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => NotificationRepository::get_unread(db, user_id).await,
        }
    }

    async fn mark_read(&self, notification_id: &str, user_id: Uuid) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                NotificationRepository::mark_read(db, notification_id, user_id).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                NotificationRepository::mark_read(db, notification_id, user_id).await
            }
        }
    }

    async fn mark_all_read(&self, user_id: Uuid) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => NotificationRepository::mark_all_read(db, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => NotificationRepository::mark_all_read(db, user_id).await,
        }
    }

    async fn get_all(
        &self,
        user_id: Uuid,
        limit: Option<i64>,
    ) -> AppResult<Vec<OAuthNotification>> {
        match self {
            Self::SQLite(db) => NotificationRepository::get_all(db, user_id, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => NotificationRepository::get_all(db, user_id, limit).await,
        }
    }
}

#[async_trait]
impl InsightRepository for Database {
    async fn store(
        &self,
        user_id: uuid::Uuid,
        insight_data: serde_json::Value,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => InsightRepository::store(db, user_id, insight_data).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => InsightRepository::store(db, user_id, insight_data).await,
        }
    }
    async fn get_for_user(
        &self,
        user_id: uuid::Uuid,
        insight_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<serde_json::Value>> {
        match self {
            Self::SQLite(db) => {
                InsightRepository::get_for_user(db, user_id, insight_type, limit).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                InsightRepository::get_for_user(db, user_id, insight_type, limit).await
            }
        }
    }
}

#[async_trait]
impl DatabaseProvider for Database {
    async fn new(database_url: &str, encryption_key: Vec<u8>) -> AppResult<Self> {
        #[cfg(feature = "postgresql")]
        {
            let pool_config = PostgresPoolConfig::default();
            Self::new_impl(database_url, encryption_key, &pool_config).await
        }
        #[cfg(not(feature = "postgresql"))]
        {
            Self::new_impl(database_url, encryption_key).await
        }
    }
    async fn migrate(&self) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.migrate().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.migrate().await,
        }
    }
}

// Implement HasEncryption for the factory Database enum
// Delegates to the appropriate backend for HMAC-SHA256 hashing and AES-256-GCM encryption
impl super::shared::encryption::HasEncryption for Database {
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String> {
        match self {
            // SQLite Database has an inherent method that takes priority, no ambiguity
            Self::SQLite(db) => db.encrypt_data_with_aad(data, aad),
            // PostgresDatabase implements both DatabaseProvider and HasEncryption traits
            // with the same method name, so we must disambiguate via UFCS
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                use super::shared::encryption::HasEncryption as HE;
                HE::encrypt_data_with_aad(db, data, aad)
            }
        }
    }

    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.decrypt_data_with_aad(encrypted, aad),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                use super::shared::encryption::HasEncryption as HE;
                HE::decrypt_data_with_aad(db, encrypted, aad)
            }
        }
    }

    fn hash_token_for_storage(&self, token: &str) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.hash_token_for_storage(token),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                use super::shared::encryption::HasEncryption as HE;
                HE::hash_token_for_storage(db, token)
            }
        }
    }
}
