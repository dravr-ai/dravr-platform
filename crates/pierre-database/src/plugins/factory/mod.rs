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
use async_trait::async_trait;
use pierre_core::config::social::SocialInsightsConfig;
use pierre_core::errors::{AppError, AppResult};
use std::sync::Arc;
use tracing::{debug, info};

use crate::RepositoryRegistry;

#[cfg(feature = "postgresql")]
use super::postgres::PostgresDatabase;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
#[cfg(not(feature = "postgresql"))]
use tracing::error;
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
    /// Build a [`RepositoryRegistry`] from whichever backend this enum wraps.
    ///
    /// Call this once at startup. The returned registry holds `Arc<dyn Trait>`
    /// for every repository, eliminating per-call enum dispatch.
    #[must_use]
    pub fn into_repositories(self) -> RepositoryRegistry {
        match self {
            Self::SQLite(db) => RepositoryRegistry::from_sqlite(Arc::new(db)),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => RepositoryRegistry::from_postgres(Arc::new(db)),
        }
    }

    /// Build a [`RepositoryRegistry`] by cloning the inner backend.
    ///
    /// Works on any `Database` reference. The inner `SqliteDatabase` or
    /// `PostgresDatabase` is cloned (both hold connection pools which are
    /// `Arc`-based, so the clone is cheap).
    #[must_use]
    pub fn repositories(&self) -> RepositoryRegistry {
        match self {
            Self::SQLite(db) => RepositoryRegistry::from_sqlite(Arc::new(db.clone())),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => RepositoryRegistry::from_postgres(Arc::new(db.clone())),
        }
    }

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

    /// Get a reference to the underlying database as a `SecurityRepository`.
    ///
    /// Used during early initialization when the factory still owns the database
    /// mutably and a `RepositoryRegistry` is not yet available.
    #[must_use]
    pub fn as_security_repository(&self) -> &dyn super::SecurityRepository {
        match self {
            Self::SQLite(db) => db,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db,
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

    /// Create a database instance for seeder binaries.
    ///
    /// Uses a placeholder encryption key since seeders only insert reference data
    /// and never call encryption operations. Detects database type automatically
    /// from the connection URL and runs migrations.
    ///
    /// # Errors
    ///
    /// Returns an error if database connection or migration fails
    pub async fn init_for_seeding(database_url: &str) -> AppResult<Self> {
        // Seeders insert reference data only — no encryption needed
        let encryption_key = vec![0u8; 32];
        Self::new(
            database_url,
            encryption_key,
            #[cfg(feature = "postgresql")]
            &PostgresPoolConfig::default(),
        )
        .await
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
