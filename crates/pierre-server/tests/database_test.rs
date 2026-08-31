// ABOUTME: Unit tests for database functionality
// ABOUTME: Validates database behavior, edge cases, and error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils;

/// Create a test database instance
///
/// # Errors
///
/// Returns an error if database initialization fails
pub async fn create_test_db() -> Result<Database> {
    test_utils::create_test_db().await.map_err(Into::into)
}
