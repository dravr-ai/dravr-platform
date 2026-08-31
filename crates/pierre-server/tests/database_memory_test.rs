// ABOUTME: Tests to ensure test databases don't create physical files in the working directory
// ABOUTME: Validates isolation between two databases opened through the test factory
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests to ensure test databases don't create physical files in the working directory

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::models::User;
use pierre_database::database::generate_encryption_key;
use pierre_database::database::test_utils::create_test_db_with_key;
use std::env;
use std::fs;

#[tokio::test]
async fn test_memory_database_no_physical_files() -> Result<()> {
    let encryption_key = generate_encryption_key().to_vec();

    // Open the test database - this must NOT create any physical files here
    let database = create_test_db_with_key(encryption_key).await?;

    // Verify no physical files are created with memory database patterns
    let current_dir = env::current_dir()?;
    let entries = fs::read_dir(&current_dir)?;

    for entry in entries {
        let entry = entry?;
        let filename = entry.file_name();
        let filename_str = filename.to_string_lossy();

        // Check for problematic files that shouldn't exist
        assert!(
            !filename_str.starts_with(":memory:test_"),
            "Found physical file that should be in-memory: {filename_str}"
        );

        assert!(
            !filename_str.starts_with("sqlite::memory:"),
            "Found physical file with memory database URL: {filename_str}"
        );
    }

    // Test basic database functionality to ensure it works
    let user = User::new(
        "test@memory.test".to_owned(),
        "password_hash".to_owned(),
        Some("Memory Test User".to_owned()),
    );

    let repos = database.repositories();
    let user_id = repos.users.create(&user).await?;
    let retrieved_user = repos.users.get_global(user_id).await?.unwrap();

    assert_eq!(retrieved_user.email, "test@memory.test");
    assert_eq!(
        retrieved_user.display_name,
        Some("Memory Test User".to_owned())
    );

    Ok(())
}

#[tokio::test]
async fn test_multiple_memory_databases_isolated() -> Result<()> {
    let encryption_key1 = generate_encryption_key().to_vec();
    let encryption_key2 = generate_encryption_key().to_vec();

    // Create two separate databases
    let database1 = create_test_db_with_key(encryption_key1).await?;

    let database2 = create_test_db_with_key(encryption_key2).await?;

    // Create users in each database
    let user1 = User::new(
        "user1@test.com".to_owned(),
        "hash1".to_owned(),
        Some("User 1".to_owned()),
    );

    let user2 = User::new(
        "user2@test.com".to_owned(),
        "hash2".to_owned(),
        Some("User 2".to_owned()),
    );

    let repos1 = database1.repositories();
    let repos2 = database2.repositories();
    let user1_id = repos1.users.create(&user1).await?;
    let user2_id = repos2.users.create(&user2).await?;

    // Verify isolation - each database only contains its own user
    assert!(repos1.users.get_global(user1_id).await?.is_some());
    assert!(repos2.users.get_global(user2_id).await?.is_some());

    // User1 should not exist in database2 and vice versa
    assert!(database2
        .repositories()
        .users
        .get_global(user1_id)
        .await?
        .is_none());
    assert!(database1
        .repositories()
        .users
        .get_global(user2_id)
        .await?
        .is_none());

    Ok(())
}
