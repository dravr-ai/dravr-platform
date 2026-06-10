// ABOUTME: Integration test for KEK re-wrap (KekProvider migration, e.g. local MEK -> GCP KMS)
// ABOUTME: Proves DEKs re-wrapped under a new KEK keep decrypting data, and the old KEK is invalidated
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for KEK re-wrap (ADR-017 Phase 6).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_auth::key_management::{
    DatabaseEncryptionKey, KeyManager, LocalKekProvider, MasterEncryptionKey,
};
#[cfg(feature = "postgresql")]
use pierre_config::environment::PostgresPoolConfig;
use pierre_database::backends::factory::Database;
use serial_test::serial;
use tempfile::TempDir;

const AAD: &str = "tenant-1|11111111-1111-1111-1111-111111111111|strava|user_oauth_tokens";

fn manager_with_mek(mek_byte: u8) -> (KeyManager, [u8; 32]) {
    let kek = Box::new(LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes(
        [mek_byte; 32],
    )));
    let temp_dek = DatabaseEncryptionKey::generate();
    let database_key = *temp_dek.as_bytes();
    (KeyManager::with_provider(kek, temp_dek), database_key)
}

async fn open_db(db_url: &str, database_key: Vec<u8>) -> Database {
    #[cfg(feature = "postgresql")]
    {
        Database::new(db_url, database_key, &PostgresPoolConfig::default())
            .await
            .unwrap()
    }
    #[cfg(not(feature = "postgresql"))]
    {
        Database::new(db_url, database_key).await.unwrap()
    }
}

#[tokio::test]
#[serial]
async fn rewrap_kek_preserves_data_and_invalidates_old_kek() {
    let temp_dir = TempDir::new().unwrap();
    let db_url = format!("sqlite:{}", temp_dir.path().join("rewrap.db").display());

    // Lifecycle 1: initialize under KEK A, encrypt data, then re-wrap DEKs to KEK B.
    let ciphertext = {
        let (mut key_manager, database_key) = manager_with_mek(0xAA);
        let mut database = open_db(&db_url, database_key.to_vec()).await;
        key_manager
            .complete_initialization(&mut database)
            .await
            .unwrap();

        let ciphertext = database
            .as_security_repository()
            .encrypt_data_with_aad("secret", AAD)
            .unwrap();

        let kek_b = Box::new(LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes(
            [0xBB; 32],
        )));
        key_manager.rewrap_deks(kek_b, &mut database).await.unwrap();
        ciphertext
    };

    // Lifecycle 2: a fresh manager on KEK B loads the (now B-wrapped) DEK and decrypts.
    {
        let (mut key_manager, database_key) = manager_with_mek(0xBB);
        let mut database = open_db(&db_url, database_key.to_vec()).await;
        key_manager
            .complete_initialization(&mut database)
            .await
            .unwrap();

        let plaintext = database
            .as_security_repository()
            .decrypt_data_with_aad(&ciphertext, AAD)
            .unwrap();
        assert_eq!(
            plaintext, "secret",
            "data must survive a KEK re-wrap — the DEK bytes are unchanged, only the wrapping"
        );
    }

    // Lifecycle 3: the old KEK A can no longer unwrap the re-wrapped DEK.
    {
        let (mut key_manager, database_key) = manager_with_mek(0xAA);
        let mut database = open_db(&db_url, database_key.to_vec()).await;
        let result = key_manager.complete_initialization(&mut database).await;
        assert!(
            result.is_err(),
            "after re-wrap, the superseded KEK must fail to unwrap the DEK"
        );
    }
}
