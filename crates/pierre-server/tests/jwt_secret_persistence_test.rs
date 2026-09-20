// ABOUTME: Test to verify JWT secret persistence across server restarts
// ABOUTME: Ensures admin tokens remain valid after server restart - fixes the 12-hour issue
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use anyhow::Result;
use pierre_auth::key_management::KeyManager;
use pierre_core::admin::models::CreateAdminTokenRequest;
use pierre_core::admin::AdminJwtManager;
use pierre_database::database::test_utils::create_test_db_with_key;
use serial_test::serial;
use std::env;

#[tokio::test]
#[serial]
async fn test_jwt_secret_persistence_across_restarts() -> Result<()> {
    // Set consistent MEK for test (32 bytes base64 encoded) - required for KeyManager::bootstrap()
    env::set_var(
        "PIERRE_MASTER_ENCRYPTION_KEY",
        "YmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmI=",
    );

    // Initialize JWKS manager for RS256 admin token signing (shared across "restarts")
    let jwks_manager = common::get_shared_test_jwks();

    // One persistent store, initialised twice: every "restart" bootstraps a
    // fresh KeyManager and re-installs the DEK it loads from that store.
    let (mut key_manager, database_key) = KeyManager::bootstrap()?;
    let mut database = create_test_db_with_key(database_key.to_vec()).await?;

    // Step 1: First initialization - simulate pierre-cli
    let jwt_secret_1 = {
        key_manager.complete_initialization(&mut database).await?;

        // Get/create JWT secret (simulating pierre-cli)
        let repos = database.repositories();
        let jwt_secret = repos
            .security
            .get_or_create_system_secret("admin_jwt_secret")
            .await?;

        // Create admin token with this secret
        let request = CreateAdminTokenRequest {
            service_name: "test_service".into(),
            service_description: Some("Test token".into()),
            permissions: None,
            expires_in_days: Some(1),
            is_super_admin: true,
            tenant_id: None,
        };

        let generated_token = repos
            .admin
            .create_token(&request, &jwt_secret, &jwks_manager)
            .await?;
        println!("Generated token: {}", generated_token.jwt_token);

        (jwt_secret, generated_token.jwt_token)
    };

    // Step 2: Second initialization - simulate server restart
    let jwt_secret_2 = {
        let (mut key_manager, _) = KeyManager::bootstrap()?;
        key_manager.complete_initialization(&mut database).await?;

        // Get JWT secret again (simulating server restart)
        let repos = database.repositories();
        repos
            .security
            .get_or_create_system_secret("admin_jwt_secret")
            .await?
    };

    // Step 3: Verify JWT secrets are identical
    assert_eq!(
        jwt_secret_1.0, jwt_secret_2,
        "JWT secret changed between restarts! This causes admin token invalidation."
    );

    // Step 4: Verify admin token can be validated with persistent secret using RS256
    let jwt_manager = AdminJwtManager::new();

    // This should NOT fail with InvalidSignature (using RS256 validation)
    let validation_result = jwt_manager.validate_token(&jwt_secret_1.1, &jwks_manager);
    assert!(
        validation_result.is_ok(),
        "Admin token validation failed after restart: {:?}",
        validation_result.err()
    );

    // Clean up test environment variable
    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");

    println!(" JWT secret persistence test PASSED");
    println!(" Admin tokens survive server restarts");
    println!(" No more 12-hour InvalidSignature issue");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_mek_ensures_consistent_jwt_storage() -> Result<()> {
    // This test verifies that the MEK properly encrypts/decrypts JWT secrets
    // ensuring they remain consistent across restarts

    // Set consistent MEK for test (32 bytes base64 encoded)
    env::set_var(
        "PIERRE_MASTER_ENCRYPTION_KEY",
        "YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWE=",
    );

    let (mut key_manager, database_key) = KeyManager::bootstrap()?;
    let mut database = create_test_db_with_key(database_key.to_vec()).await?;

    let jwt_secret_1 = {
        key_manager.complete_initialization(&mut database).await?;
        let repos = database.repositories();
        repos
            .security
            .get_or_create_system_secret("admin_jwt_secret")
            .await?
    };

    let jwt_secret_2 = {
        let (mut key_manager, _) = KeyManager::bootstrap()?;
        key_manager.complete_initialization(&mut database).await?;
        let repos = database.repositories();
        repos
            .security
            .get_or_create_system_secret("admin_jwt_secret")
            .await?
    };

    assert_eq!(
        jwt_secret_1, jwt_secret_2,
        "MEK-encrypted JWT secret storage failed - secrets differ across restarts"
    );

    // Clean up test environment variable
    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");

    println!(" MEK-based JWT secret storage test PASSED");

    Ok(())
}

/// The secret store mints only the admin JWT secret. The wrapped
/// data-encryption keys are written by key management under their own names
/// through `update_system_secret`, so asking the store to create one is
/// refused rather than answered with a key the store made up — on both
/// backends. `update_system_secret` rotates in place and `get_system_secret`
/// reads the current value.
#[tokio::test]
async fn only_the_admin_jwt_secret_is_minted_and_others_rotate_in_place() -> Result<()> {
    let database = common::create_test_database().await?;
    let security = &database.repositories().security;

    assert!(
        security
            .get_or_create_system_secret("database_encryption_key")
            .await
            .is_err(),
        "a data-encryption key is never minted by the secret store"
    );
    assert!(
        security
            .get_system_secret("database_encryption_key")
            .await
            .is_err(),
        "nothing stored it, so nothing reads back"
    );

    let minted = security
        .get_or_create_system_secret("admin_jwt_secret")
        .await?;
    assert!(
        !minted.is_empty(),
        "the admin JWT secret is minted on first ask"
    );
    assert_eq!(
        security
            .get_or_create_system_secret("admin_jwt_secret")
            .await?,
        minted,
        "the second ask returns the stored secret, not a new one"
    );

    security
        .update_system_secret("database_encryption_key", "wrapped-v1")
        .await?;
    assert_eq!(
        security
            .get_system_secret("database_encryption_key")
            .await?,
        "wrapped-v1"
    );
    security
        .update_system_secret("database_encryption_key", "wrapped-v2")
        .await?;
    assert_eq!(
        security
            .get_system_secret("database_encryption_key")
            .await?,
        "wrapped-v2",
        "an update replaces the value under the same name"
    );

    Ok(())
}
