// ABOUTME: Opens, in the test process, the scratch SQLite database a suite ran the real binary against
// ABOUTME: For seeding a row no command creates and for reading back what a command stored

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_database::backends::factory::Database;

/// The database at `url`, which the binary already created and migrated.
///
/// The key encrypts stored provider tokens. The suites here read and write
/// account and admin-token rows, which hold none, so any 32 bytes open the
/// database for them and the process environment is left alone.
pub(crate) async fn open(url: &str) -> Database {
    Database::new(
        url,
        vec![0; 32],
        #[cfg(feature = "postgresql")]
        &PostgresPoolConfig::default(),
    )
    .await
    .unwrap()
}
