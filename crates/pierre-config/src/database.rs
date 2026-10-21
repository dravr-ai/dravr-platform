// ABOUTME: Database configuration types for SQLite and PostgreSQL connections
// ABOUTME: Handles connection pools, backups, and SQLx settings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::constants::{defaults, limits};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::redaction::redact_url;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize, Serializer};
use std::env;
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::path::PathBuf;

/// Type-safe database configuration
///
/// A `PostgreSQL` connection string carries the database password, so Debug,
/// Display and serialization all print it through [`redact_url`]; the raw
/// string is read only through [`DatabaseUrl::connection_string`].
#[derive(Clone, Serialize, Deserialize)]
pub enum DatabaseUrl {
    /// `SQLite` database with file path
    SQLite {
        /// Path to `SQLite` database file
        path: PathBuf,
    },
    /// `PostgreSQL` connection
    PostgreSQL {
        /// `PostgreSQL` connection string
        #[serde(serialize_with = "serialize_redacted_url")]
        connection_string: SecretString,
    },
    /// In-memory `SQLite` (for testing)
    Memory,
}

impl DatabaseUrl {
    /// Parse from string with validation
    ///
    /// # Errors
    ///
    /// Returns an error if the database URL format is invalid or unsupported
    pub fn parse_url(s: &str) -> AppResult<Self> {
        if s.starts_with("sqlite:") {
            let path_str = s.strip_prefix("sqlite:").unwrap_or(s);
            if path_str == ":memory:" {
                Ok(Self::Memory)
            } else {
                Ok(Self::SQLite {
                    path: PathBuf::from(path_str),
                })
            }
        } else if s.starts_with("postgresql://") || s.starts_with("postgres://") {
            Ok(Self::PostgreSQL {
                connection_string: SecretString::from(s),
            })
        } else {
            // Fallback: treat as SQLite file path
            Ok(Self::SQLite {
                path: PathBuf::from(s),
            })
        }
    }

    /// The connection string a pool connects with, credentials included;
    /// read it with `expose_secret()` at the point of connecting
    #[must_use]
    pub fn connection_string(&self) -> SecretString {
        match self {
            Self::SQLite { path } => SecretString::from(format!("sqlite:{}", path.display())),
            Self::PostgreSQL { connection_string } => connection_string.clone(),
            Self::Memory => SecretString::from("sqlite::memory:"),
        }
    }

    /// Check if this is an in-memory database
    #[must_use]
    pub const fn is_memory(&self) -> bool {
        matches!(self, Self::Memory)
    }

    /// Check if this is a `SQLite` database
    #[must_use]
    pub const fn is_sqlite(&self) -> bool {
        matches!(self, Self::SQLite { .. } | Self::Memory)
    }

    /// Check if this is a `PostgreSQL` database
    #[must_use]
    pub const fn is_postgresql(&self) -> bool {
        matches!(self, Self::PostgreSQL { .. })
    }
}

impl Default for DatabaseUrl {
    fn default() -> Self {
        Self::SQLite {
            path: PathBuf::from("./data/users.db"),
        }
    }
}

impl Display for DatabaseUrl {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(&redact_url(self.connection_string().expose_secret()))
    }
}

impl Debug for DatabaseUrl {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::SQLite { path } => f.debug_struct("SQLite").field("path", path).finish(),
            Self::PostgreSQL { connection_string } => f
                .debug_struct("PostgreSQL")
                .field(
                    "connection_string",
                    &redact_url(connection_string.expose_secret()),
                )
                .finish(),
            Self::Memory => f.write_str("Memory"),
        }
    }
}

/// Serialize a connection string through [`redact_url`], so a serialized
/// configuration names the database host and never its password.
fn serialize_redacted_url<S: Serializer>(
    connection_string: &SecretString,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&redact_url(connection_string.expose_secret()))
}

/// Database connection and management configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatabaseConfig {
    /// Database URL (`SQLite` path or `PostgreSQL` connection string)
    pub url: DatabaseUrl,
    /// Enable database migrations on startup
    pub auto_migrate: bool,
    /// Database backup configuration
    pub backup: BackupConfig,
    /// `PostgreSQL` connection pool configuration
    pub postgres_pool: PostgresPoolConfig,
}

impl DatabaseConfig {
    /// Load database configuration from environment
    ///
    /// # Errors
    ///
    /// Returns an error if database environment variables are invalid
    pub fn from_env() -> AppResult<Self> {
        Ok(Self {
            url: DatabaseUrl::parse_url(
                &env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_owned()),
            )
            .unwrap_or_default(),
            auto_migrate: env_var_or("AUTO_MIGRATE", "true")
                .parse()
                .map_err(|e| AppError::invalid_input(format!("Invalid AUTO_MIGRATE value: {e}")))?,
            backup: BackupConfig::from_env()?,
            postgres_pool: PostgresPoolConfig::from_env(),
        })
    }
}

/// Configuration for automatic database backups
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackupConfig {
    /// Enable automatic backups
    pub enabled: bool,
    /// Backup interval in seconds
    pub interval_seconds: u64,
    /// Number of backups to retain
    pub retention_count: u32,
    /// Backup directory path
    pub directory: PathBuf,
}

impl BackupConfig {
    /// Load backup configuration from environment
    ///
    /// # Errors
    ///
    /// Returns an error if backup environment variables are invalid
    pub fn from_env() -> AppResult<Self> {
        Ok(Self {
            enabled: env_var_or("BACKUP_ENABLED", "true").parse().map_err(|e| {
                AppError::invalid_input(format!("Invalid BACKUP_ENABLED value: {e}"))
            })?,
            interval_seconds: env_var_or(
                "BACKUP_INTERVAL",
                &limits::DEFAULT_BACKUP_INTERVAL_SECS.to_string(),
            )
            .parse()
            .map_err(|e| AppError::invalid_input(format!("Invalid BACKUP_INTERVAL value: {e}")))?,
            retention_count: env_var_or(
                "BACKUP_RETENTION",
                &limits::DEFAULT_BACKUP_RETENTION_COUNT.to_string(),
            )
            .parse()
            .map_err(|e| AppError::invalid_input(format!("Invalid BACKUP_RETENTION value: {e}")))?,
            directory: PathBuf::from(env_var_or("BACKUP_DIRECTORY", defaults::DEFAULT_BACKUP_DIR)),
        })
    }
}

/// `SQLx` connection pool configuration for controlling database connections
///
/// These settings apply to both `SQLite` and `PostgreSQL` connection pools.
/// Values of `None` use `SQLx` defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlxConfig {
    /// Maximum time a connection can sit idle before being closed (seconds)
    /// Set via `SQLX_IDLE_TIMEOUT_SECS` environment variable
    /// None = use `SQLx` default (10 minutes for `PostgreSQL`, none for `SQLite`)
    pub idle_timeout_secs: Option<u64>,
    /// Maximum lifetime of a connection before it's closed (seconds)
    /// Set via `SQLX_MAX_LIFETIME_SECS` environment variable
    /// None = use `SQLx` default (30 minutes)
    pub max_lifetime_secs: Option<u64>,
    /// Whether to test connections before acquiring from pool
    /// Set via `SQLX_TEST_BEFORE_ACQUIRE` environment variable
    pub test_before_acquire: bool,
    /// Statement cache capacity per connection
    /// Set via `SQLX_STATEMENT_CACHE_CAPACITY` environment variable
    /// None = use `SQLx` default (100)
    pub statement_cache_capacity: Option<usize>,
}

impl Default for SqlxConfig {
    fn default() -> Self {
        Self {
            idle_timeout_secs: None,        // Use SQLx default
            max_lifetime_secs: None,        // Use SQLx default
            test_before_acquire: true,      // Enable by default for reliability
            statement_cache_capacity: None, // Use SQLx default (100)
        }
    }
}

impl SqlxConfig {
    /// Load from environment variables
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            idle_timeout_secs: env::var("SQLX_IDLE_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok()),
            max_lifetime_secs: env::var("SQLX_MAX_LIFETIME_SECS")
                .ok()
                .and_then(|s| s.parse().ok()),
            test_before_acquire: env::var("SQLX_TEST_BEFORE_ACQUIRE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(true),
            statement_cache_capacity: env::var("SQLX_STATEMENT_CACHE_CAPACITY")
                .ok()
                .and_then(|s| s.parse().ok()),
        }
    }
}

/// Get environment variable or default value
fn env_var_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{DatabaseConfig, DatabaseUrl};
    use secrecy::ExposeSecret;

    const PG_URL: &str = "postgresql://pierre:hunter2-pg@db.internal:5432/pierre";
    const REDACTED_PG_URL: &str = "postgresql://***:***@db.internal:5432/pierre";

    #[test]
    fn a_postgres_url_prints_through_redact_url() {
        let url = DatabaseUrl::parse_url(PG_URL).unwrap();
        assert_eq!(
            format!("{url:?}"),
            format!("PostgreSQL {{ connection_string: \"{REDACTED_PG_URL}\" }}")
        );
        assert_eq!(url.to_string(), REDACTED_PG_URL);

        let config = DatabaseConfig {
            url,
            ..DatabaseConfig::default()
        };
        let debug = format!("{config:#?}");
        assert!(!debug.contains("hunter2-pg"), "{debug}");
        assert!(debug.contains(REDACTED_PG_URL), "{debug}");
    }

    #[test]
    fn a_postgres_url_serializes_redacted_and_reads_back_whole() {
        let url = DatabaseUrl::parse_url(PG_URL).unwrap();
        assert_eq!(
            serde_json::to_string(&url).unwrap(),
            format!("{{\"PostgreSQL\":{{\"connection_string\":\"{REDACTED_PG_URL}\"}}}}")
        );

        let read: DatabaseUrl = serde_json::from_str(&format!(
            "{{\"PostgreSQL\":{{\"connection_string\":\"{PG_URL}\"}}}}"
        ))
        .unwrap();
        assert_eq!(read.connection_string().expose_secret(), PG_URL);
    }

    #[test]
    fn the_connection_string_keeps_the_credentials() {
        let url = DatabaseUrl::parse_url(PG_URL).unwrap();
        assert_eq!(url.connection_string().expose_secret(), PG_URL);
        assert_eq!(
            DatabaseUrl::parse_url("sqlite:./data/users.db")
                .unwrap()
                .connection_string()
                .expose_secret(),
            "sqlite:./data/users.db"
        );
        assert_eq!(
            DatabaseUrl::Memory.connection_string().expose_secret(),
            "sqlite::memory:"
        );
    }

    #[test]
    fn a_sqlite_url_prints_its_path() {
        let url = DatabaseUrl::parse_url("sqlite:./data/users.db").unwrap();
        assert_eq!(format!("{url:?}"), "SQLite { path: \"./data/users.db\" }");
        assert_eq!(url.to_string(), "sqlite:./data/users.db");
        assert_eq!(format!("{:?}", DatabaseUrl::Memory), "Memory");
    }
}
