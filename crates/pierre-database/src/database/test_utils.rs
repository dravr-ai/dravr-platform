// ABOUTME: Test utilities for database operations and isolated test database creation
// ABOUTME: Honours DATABASE_URL so the PostgreSQL backend's own SQL is actually executed by tests
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
use crate::backends::factory::Database;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::errors::AppResult;

/// Create an isolated test database instance.
///
/// Uses `DATABASE_URL` when it names a `PostgreSQL` server, and in-memory
/// `SQLite` otherwise.
///
/// This previously pinned `sqlite::memory:` unconditionally — the `postgresql`
/// feature only changed the constructor signature, not the URL. `ci-postgres`
/// therefore ran the whole database suite against `SQLite`, and no test ever
/// executed the `PostgreSQL` backend's SQL. That is how a `get_by_prefix` query
/// matching `id` instead of `key_prefix` broke API-key authentication in every
/// deployed environment while its test stayed green.
///
/// # Errors
///
/// Returns an error if database initialization fails.
pub async fn create_test_db() -> AppResult<Database> {
    #[cfg(feature = "postgresql")]
    {
        if let Some(url) = postgres_test_url() {
            return create_isolated_postgres_db(&url).await;
        }
        Database::new(
            "sqlite::memory:",
            vec![0u8; 32],
            &PostgresPoolConfig::default(),
        )
        .await
    }

    #[cfg(not(feature = "postgresql"))]
    {
        Database::new("sqlite::memory:", vec![0u8; 32]).await
    }
}

/// Create an in-memory `SQLite` test database, ignoring `DATABASE_URL`.
///
/// For the handful of tests that need `Database::sqlite_pool()` — a raw pool
/// only the `SQLite` backend exposes — or that compare the two dialects'
/// schemas against each other. Those tests depend on the backend by
/// construction, so they say so explicitly rather than relying on
/// [`create_test_db`] happening to return `SQLite`.
///
/// # Errors
///
/// Returns an error if database initialization fails.
pub async fn create_sqlite_test_db() -> AppResult<Database> {
    #[cfg(feature = "postgresql")]
    {
        Database::new(
            "sqlite::memory:",
            vec![0u8; 32],
            &PostgresPoolConfig::default(),
        )
        .await
    }

    #[cfg(not(feature = "postgresql"))]
    {
        Database::new("sqlite::memory:", vec![0u8; 32]).await
    }
}

/// `DATABASE_URL` when it points at a `PostgreSQL` server, else `None`.
///
/// A `SQLite` `DATABASE_URL` (what local runs and the non-postgres CI job set)
/// falls through to the in-memory default, so this never redirects a test at a
/// developer's on-disk database.
#[cfg(feature = "postgresql")]
fn postgres_test_url() -> Option<String> {
    use std::env;

    let url = env::var("DATABASE_URL").ok()?;
    (url.starts_with("postgres://") || url.starts_with("postgresql://")).then_some(url)
}

/// Connect to `PostgreSQL` with a private schema per call.
///
/// `sqlite::memory:` gives every caller its own empty database; a shared
/// `PostgreSQL` server does not. Without isolation, tests running concurrently
/// (`ci-postgres` uses `--test-threads=4`) would see each other's rows and
/// collide on unique constraints. Each call therefore creates a uniquely-named
/// schema and pins `search_path` to it, so migrations and every subsequent
/// query land in a namespace only this caller uses.
///
/// The schema is intentionally left behind: the CI database is ephemeral, and
/// dropping it here would need a teardown hook the test harness does not offer.
#[cfg(feature = "postgresql")]
async fn create_isolated_postgres_db(base_url: &str) -> AppResult<Database> {
    use pierre_core::errors::AppError;
    use sqlx::{postgres::PgPoolOptions, Executor};
    use uuid::Uuid;

    let schema = format!("test_{}", Uuid::new_v4().simple());

    // Create the schema on a short-lived connection to the default search_path,
    // before the pooled connection below pins search_path to a schema that does
    // not exist yet.
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(base_url)
        .await
        .map_err(|e| AppError::database(format!("Test DB: cannot reach PostgreSQL: {e}")))?;
    admin
        .execute(format!("CREATE SCHEMA IF NOT EXISTS {schema}").as_str())
        .await
        .map_err(|e| AppError::database(format!("Test DB: cannot create schema {schema}: {e}")))?;
    admin.close().await;

    let separator = if base_url.contains('?') { '&' } else { '?' };
    let scoped_url = format!("{base_url}{separator}options=-c%20search_path%3D{schema}");

    Database::new(&scoped_url, vec![0u8; 32], &PostgresPoolConfig::default()).await
}
