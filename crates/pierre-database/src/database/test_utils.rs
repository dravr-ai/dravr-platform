// ABOUTME: Test utilities for database operations and in-memory test database creation
// ABOUTME: Provides helper functions for creating isolated test database instances
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
use crate::backends::factory::Database;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::errors::AppResult;

/// Create a test database instance
///
/// # Errors
///
/// Returns an error if database initialization fails
pub async fn create_test_db() -> AppResult<Database> {
    // Use a simple in-memory database - each connection gets its own isolated instance
    let database_url = "sqlite::memory:";

    #[cfg(feature = "postgresql")]
    {
        Database::new(database_url, vec![0u8; 32], &PostgresPoolConfig::default()).await
    }

    #[cfg(not(feature = "postgresql"))]
    {
        Database::new(database_url, vec![0u8; 32]).await
    }
}
