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
use pierre_core::errors::{AppError, AppResult};
use pierre_core::redaction::redact_url;
use std::collections::HashMap;
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
use crate::database::system_settings::SystemSetting;
use crate::database::Database as SqliteDatabase;

/// Supported database types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseType {
    /// `SQLite` embedded database
    SQLite,
    /// `PostgreSQL` database server
    PostgreSQL,
}

/// The concrete backend a [`Database`] wraps.
///
/// Each variant holds the backend behind an `Arc` shared with the
/// [`RepositoryRegistry`] built from it, so the registry and the backend
/// handle are one instance rather than two copies.
#[derive(Clone)]
pub enum DatabaseBackend {
    /// `SQLite` database instance
    SQLite(Arc<SqliteDatabase>),
    /// `PostgreSQL` database instance (requires postgresql feature)
    #[cfg(feature = "postgresql")]
    PostgreSQL(Arc<PostgresDatabase>),
}

impl DatabaseBackend {
    /// Build the repository registry over this backend's shared instance.
    fn build_registry(&self) -> RepositoryRegistry {
        match self {
            Self::SQLite(db) => RepositoryRegistry::from_sqlite(Arc::clone(db)),
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => RepositoryRegistry::from_postgres(Arc::clone(db)),
        }
    }
}

/// Database handle: one backend plus the one [`RepositoryRegistry`] built over it.
///
/// The registry is built when the handle is constructed and rebuilt only when
/// the encryption keys change ([`Database::update_encryption_key`],
/// [`Database::install_dek_versions`]). Cloning a `Database` clones two `Arc`s,
/// so every clone shares the same backend and the same registry.
#[derive(Clone)]
pub struct Database {
    /// The concrete backend, shared with every repository in `repositories`
    backend: DatabaseBackend,
    /// Arc: shared by every clone of this handle and by the server resources
    /// and runtime contexts that read repositories from it
    repositories: Arc<RepositoryRegistry>,
}

impl Database {
    /// Wrap a `SQLite` backend, building its repository registry.
    #[must_use]
    pub fn from_sqlite(db: SqliteDatabase) -> Self {
        Self::from_backend(DatabaseBackend::SQLite(Arc::new(db)))
    }

    /// Wrap a `PostgreSQL` backend, building its repository registry.
    #[cfg(feature = "postgresql")]
    #[must_use]
    pub fn from_postgres(db: PostgresDatabase) -> Self {
        Self::from_backend(DatabaseBackend::PostgreSQL(Arc::new(db)))
    }

    fn from_backend(backend: DatabaseBackend) -> Self {
        let repositories = Arc::new(backend.build_registry());
        Self {
            backend,
            repositories,
        }
    }

    /// The repository registry for this database.
    ///
    /// Built once when the handle is constructed; every call, and every clone
    /// of this handle, returns the same `Arc`. Clone the `Arc` to hold the
    /// registry beyond the handle's borrow.
    #[must_use]
    pub const fn repositories(&self) -> &Arc<RepositoryRegistry> {
        &self.repositories
    }

    /// The concrete backend, for the call sites that need a raw pool.
    #[must_use]
    pub const fn backend(&self) -> &DatabaseBackend {
        &self.backend
    }

    /// Get a descriptive string for the current database backend
    #[must_use]
    pub const fn backend_info(&self) -> &'static str {
        match self.backend {
            DatabaseBackend::SQLite(_) => "SQLite (Local Development)",
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(_) => "PostgreSQL (Cloud-Ready)",
        }
    }

    /// Get the underlying `SQLite` connection pool if this is a `SQLite` database.
    ///
    /// Returns `None` for `PostgreSQL` databases.
    #[must_use]
    pub fn sqlite_pool(&self) -> Option<&sqlx::Pool<sqlx::Sqlite>> {
        self.sqlite_database().map(SqliteDatabase::pool)
    }

    /// Get a reference to the underlying `SQLite` database if this is a `SQLite` backend.
    ///
    /// Returns `None` for `PostgreSQL` databases. The returned reference implements
    /// domain-specific repository traits (`RecipeRepository`, `AgentsRepository`,
    /// `MobilityRepository`).
    #[must_use]
    pub fn sqlite_database(&self) -> Option<&SqliteDatabase> {
        match &self.backend {
            DatabaseBackend::SQLite(db) => Some(db),
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(_) => None,
        }
    }

    /// Get the underlying `PostgreSQL` connection pool if this is a `PostgreSQL` database.
    ///
    /// Returns `None` for `SQLite` databases.
    #[cfg(feature = "postgresql")]
    #[must_use]
    pub fn postgres_pool(&self) -> Option<&sqlx::Pool<sqlx::Postgres>> {
        match &self.backend {
            DatabaseBackend::SQLite(_) => None,
            DatabaseBackend::PostgreSQL(db) => Some(db.pool()),
        }
    }

    /// Get a reference to the underlying database as a `SecurityRepository`.
    ///
    /// Used during key-management initialization, while the caller holds the
    /// handle mutably to install the loaded DEK versions.
    #[must_use]
    pub fn as_security_repository(&self) -> &dyn super::SecurityRepository {
        match &self.backend {
            DatabaseBackend::SQLite(db) => db.as_ref(),
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => db.as_ref(),
        }
    }

    /// Update the encryption key used for token encryption/decryption
    ///
    /// This is called after the actual DEK is loaded from the database during
    /// two-tier key management initialization. The database is initially created
    /// with a temporary key, then updated with the real key once it's loaded.
    ///
    /// The backend carries its keys by value, so the backend is copied with the
    /// new key and the repository registry is rebuilt over the copy. A registry
    /// `Arc` or a `Database` clone taken before this call keeps the old key.
    ///
    /// # Safety
    /// Only call this once during startup, before any encrypted data operations.
    pub fn update_encryption_key(&mut self, new_key: Vec<u8>) {
        match &mut self.backend {
            DatabaseBackend::SQLite(db) => Arc::make_mut(db).update_encryption_key(new_key),
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => Arc::make_mut(db).update_encryption_key(new_key),
        }
        self.repositories = Arc::new(self.backend.build_registry());
    }

    /// Install a full set of DEK versions (after load-all-versions or a rotation).
    ///
    /// `active_key` (version `active_version`) encrypts new data; `prior_versions`
    /// are retained for decrypt-only. The blind-index (HMAC) key is pinned to
    /// version 1, falling back to the active key only when v1 is itself active.
    ///
    /// The backend carries its keys by value, so the backend is copied with the
    /// new versions and the repository registry is rebuilt over the copy. A
    /// registry `Arc` or a `Database` clone taken before this call keeps the
    /// previous keys.
    ///
    /// # Safety
    /// Call during startup or rotation while holding `&mut self`, before serving
    /// concurrent encrypted-data operations.
    pub fn install_dek_versions(
        &mut self,
        active_version: u32,
        active_key: Vec<u8>,
        prior_versions: HashMap<u32, Vec<u8>>,
    ) {
        // Only one arm runs per call, so the owned `active_key`/`prior_versions`
        // move into the active backend without a double-move.
        match &mut self.backend {
            DatabaseBackend::SQLite(db) => {
                Arc::make_mut(db).install_dek_versions(active_version, active_key, prior_versions);
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                Arc::make_mut(db).install_dek_versions(active_version, active_key, prior_versions);
            }
        }
        self.repositories = Arc::new(self.backend.build_registry());
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
            redact_url(database_url)
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
        Ok(Self::from_sqlite(db))
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
        Ok(Self::from_postgres(db))
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
        match &self.backend {
            DatabaseBackend::SQLite(db) => db.is_auto_approval_enabled().await,
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => db.is_auto_approval_enabled().await,
        }
    }

    /// Set auto-approval enabled state
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn set_auto_approval_enabled(&self, enabled: bool) -> AppResult<()> {
        match &self.backend {
            DatabaseBackend::SQLite(db) => db.set_auto_approval_enabled(enabled).await,
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => db.set_auto_approval_enabled(enabled).await,
        }
    }

    /// Read a generic key/value entry from the `system_settings` table.
    ///
    /// Used by feature surfaces that persist a single JSON document under
    /// a stable key (e.g., the Tier 6 harness configuration).
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_system_setting(&self, key: &str) -> AppResult<Option<SystemSetting>> {
        match &self.backend {
            DatabaseBackend::SQLite(db) => db.get_system_setting(key).await,
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => db.get_system_setting(key).await,
        }
    }

    /// Write a generic key/value entry to the `system_settings` table.
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails.
    pub async fn set_system_setting(&self, key: &str, value: &str) -> AppResult<()> {
        match &self.backend {
            DatabaseBackend::SQLite(db) => db.set_system_setting(key, value).await,
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => db.set_system_setting(key, value).await,
        }
    }
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
        match &self.backend {
            DatabaseBackend::SQLite(db) => db.migrate().await,
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => db.migrate().await,
        }
    }
}
