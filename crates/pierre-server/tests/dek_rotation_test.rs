// ABOUTME: Integration test for DEK rotation — old data stays decryptable, new data uses the new version
// ABOUTME: Also proves the refresh-token blind index (HMAC) survives rotation (pinned to DEK v1)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for DEK rotation (ADR-017 Phase 4).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_auth::key_management::KeyManager;
use pierre_database::backends::factory::Database;
use pierre_database::backends::shared::encryption::HasEncryption;
use pierre_database::database::test_utils::create_test_db_with_key;
use serial_test::serial;
use std::env;

const AAD: &str = "tenant-1|11111111-1111-1111-1111-111111111111|strava|user_oauth_tokens";
const TEST_MEK: &str = "YmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmI=";

async fn open_db(database_key: Vec<u8>) -> Database {
    create_test_db_with_key(database_key).await.unwrap()
}

/// The refresh-token blind index on whichever backend the factory opened.
fn blind_index(database: &Database, token: &str) -> String {
    match database {
        Database::SQLite(db) => db.hash_token_for_storage(token).unwrap(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => db.hash_token_for_storage(token).unwrap(),
    }
}

#[tokio::test]
#[serial]
async fn dek_rotation_preserves_old_data_and_blind_index() {
    env::set_var("PIERRE_MASTER_ENCRYPTION_KEY", TEST_MEK);

    let (mut key_manager, database_key) = KeyManager::bootstrap().unwrap();
    let mut database = open_db(database_key.to_vec()).await;
    key_manager
        .complete_initialization(&mut database)
        .await
        .unwrap();

    // Encrypt under the initial (v1) DEK and capture the refresh-token blind index.
    let (ciphertext_v1, blind_index_before) = {
        let security = database.as_security_repository();
        let ct = security.encrypt_data_with_aad("token-1", AAD).unwrap();
        assert!(
            ct.starts_with("v1:"),
            "initial ciphertext must be v1, got: {ct}"
        );
        let hmac = blind_index(&database, "refresh-xyz");
        (ct, hmac)
    };

    // Rotate the DEK to version 2.
    let new_version = key_manager.rotate_dek(&mut database).await.unwrap();
    assert_eq!(
        new_version, 2,
        "rotation must advance the active DEK version"
    );

    {
        let security = database.as_security_repository();

        // 1. Old v1 ciphertext is still decryptable (no data loss on rotation).
        assert_eq!(
            security.decrypt_data_with_aad(&ciphertext_v1, AAD).unwrap(),
            "token-1",
            "data encrypted before rotation must remain decryptable"
        );

        // 2. New writes use the rotated (v2) DEK and round-trip.
        let ciphertext_v2 = security.encrypt_data_with_aad("token-2", AAD).unwrap();
        assert!(
            ciphertext_v2.starts_with("v2:"),
            "post-rotation ciphertext must be v2, got: {ciphertext_v2}"
        );
        assert_eq!(
            security.decrypt_data_with_aad(&ciphertext_v2, AAD).unwrap(),
            "token-2"
        );

        // 3. The refresh-token blind index (HMAC) is unchanged across rotation.
        let blind_index_after = blind_index(&database, "refresh-xyz");
        assert_eq!(
            blind_index_before, blind_index_after,
            "refresh-token blind index must survive DEK rotation (pinned to v1)"
        );
    }

    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");
}

#[tokio::test]
#[serial]
async fn dek_survives_reload_after_rotation() {
    env::set_var("PIERRE_MASTER_ENCRYPTION_KEY", TEST_MEK);

    // One persistent store shared by both lifecycles; the second bootstraps a
    // fresh manager and re-installs every DEK version it loads from that store.
    // First lifecycle: initialize, rotate, encrypt under v2.
    let (mut key_manager, database_key) = KeyManager::bootstrap().unwrap();
    let mut database = open_db(database_key.to_vec()).await;
    key_manager
        .complete_initialization(&mut database)
        .await
        .unwrap();
    key_manager.rotate_dek(&mut database).await.unwrap();
    let ciphertext_v2 = database
        .as_security_repository()
        .encrypt_data_with_aad("persisted", AAD)
        .unwrap();
    assert!(ciphertext_v2.starts_with("v2:"));

    // Second lifecycle: a fresh manager must reload all versions and decrypt v2 data.
    {
        let (mut key_manager, _) = KeyManager::bootstrap().unwrap();
        key_manager
            .complete_initialization(&mut database)
            .await
            .unwrap();
        let plaintext = database
            .as_security_repository()
            .decrypt_data_with_aad(&ciphertext_v2, AAD)
            .unwrap();
        assert_eq!(
            plaintext, "persisted",
            "after reload, the rotated active version must decrypt its own ciphertext"
        );
    }

    env::remove_var("PIERRE_MASTER_ENCRYPTION_KEY");
}
