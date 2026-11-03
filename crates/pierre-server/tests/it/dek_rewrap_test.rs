// ABOUTME: Integration test for KEK re-wrap (KekProvider migration, e.g. local MEK -> GCP KMS)
// ABOUTME: Proves every key version re-wrapped under a new KEK keeps decrypting data, and the old KEK is invalidated
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for KEK re-wrap (ADR-017 Phase 6).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, Utc};
use pierre_auth::key_management::{KeyManager, LocalKekProvider, MasterEncryptionKey};
use pierre_auth::keyset::KeysetController;
use pierre_database::backends::factory::Database;
use pierre_database::backends::shared::key_material::StoredSecretKind;
use pierre_database::backends::shared::keyring::{KeyFamily, Keyring, SealDomain};
use pierre_test_support::db::create_test_db_with_key;
use serial_test::serial;

const AAD: &str = "tenant-1|11111111-1111-1111-1111-111111111111|strava|user_oauth_tokens";
const DOMAIN: SealDomain = SealDomain::Stored(StoredSecretKind::OAuthAccessToken);

fn manager_with_mek(mek_byte: u8) -> (KeyManager, [u8; 32]) {
    KeyManager::bootstrap_with(Box::new(LocalKekProvider::from_mek(
        MasterEncryptionKey::from_bytes([mek_byte; 32]),
    )))
}

fn decrypt(database: &Database, ciphertext: &str) -> String {
    database
        .as_security_repository()
        .decrypt_data_with_aad(DOMAIN, ciphertext, AAD)
        .unwrap()
}

#[tokio::test]
#[serial]
async fn rewrap_kek_preserves_data_and_invalidates_old_kek() {
    // One persistent store shared by every lifecycle below; each lifecycle
    // bootstraps its own manager and loads whatever keys it can unwrap from
    // that store, exactly as a restarted process would.
    // Lifecycle 1: initialize under KEK A, encrypt data under v1 and (after a
    // promoted rotation) v2, then re-wrap every key version to KEK B.
    let (mut key_manager, database_key) = manager_with_mek(0xAA);
    let database = create_test_db_with_key(database_key.to_vec())
        .await
        .unwrap();
    key_manager
        .complete_initialization(&database)
        .await
        .unwrap();

    let ciphertext_v1 = database
        .as_security_repository()
        .encrypt_data_with_aad(DOMAIN, "secret", AAD)
        .unwrap();
    assert!(ciphertext_v1.starts_with("v1:"));
    key_manager
        .rotate(&database, KeyFamily::Data)
        .await
        .unwrap();
    KeysetController::new(
        key_manager.keyset_loader().unwrap(),
        key_manager.kek(),
        Vec::new(),
        0,
    )
    .tick(
        database.as_security_repository(),
        Utc::now() + Duration::minutes(16),
    )
    .await
    .unwrap();
    let ciphertext_v2 = database
        .as_security_repository()
        .encrypt_data_with_aad(DOMAIN, "rotated-secret", AAD)
        .unwrap();
    assert!(ciphertext_v2.starts_with("v2:"));

    let kek_b = Box::new(LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes(
        [0xBB; 32],
    )));
    key_manager.rewrap_deks(kek_b, &database).await.unwrap();

    // Lifecycle 2: a fresh manager on KEK B loads the (now B-wrapped) keys and
    // decrypts. The handle starts from a bootstrap key alone, as a restarted
    // process's does, so only what B unwrapped can open the ciphertext.
    {
        let (mut key_manager, bootstrap_key) = manager_with_mek(0xBB);
        database
            .keyring()
            .install(Keyring::single(bootstrap_key.to_vec()), Utc::now());
        assert!(
            database
                .as_security_repository()
                .decrypt_data_with_aad(DOMAIN, &ciphertext_v1, AAD)
                .is_err(),
            "test setup: the restarted handle must not already hold the stored keys"
        );
        key_manager
            .complete_initialization(&database)
            .await
            .unwrap();

        assert_eq!(
            decrypt(&database, &ciphertext_v1),
            "secret",
            "data must survive a KEK re-wrap — the key bytes are unchanged, only the wrapping"
        );
        assert_eq!(
            decrypt(&database, &ciphertext_v2),
            "rotated-secret",
            "every key version is re-wrapped, not only the first"
        );
    }

    // Lifecycle 3: the old KEK A can no longer unwrap the re-wrapped keys.
    {
        let (mut key_manager, _) = manager_with_mek(0xAA);
        let result = key_manager.complete_initialization(&database).await;
        assert!(
            result.is_err(),
            "after re-wrap, the superseded KEK must fail to unwrap the keys"
        );
    }
}
