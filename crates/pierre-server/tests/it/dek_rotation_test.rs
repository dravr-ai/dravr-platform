// ABOUTME: Integration test for data-key rotation — staged until promoted, old data stays decryptable, new data uses v2
// ABOUTME: Also proves the refresh-token blind index survives a data rotation (the index primary does not move)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for data-key rotation (ADR-017 Phase 4, carnet#1347).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, Utc};
use pierre_auth::key_management::KeyManager;
use pierre_auth::keyset::KeysetController;
use pierre_database::backends::factory::Database;
use pierre_database::backends::shared::key_material::StoredSecretKind;
use pierre_database::backends::shared::keyring::{KeyFamily, Keyring, SealDomain};
use pierre_test_support::db::create_test_db_with_key;
use serial_test::serial;

use crate::common::bootstrap_keys;

const AAD: &str = "tenant-1|11111111-1111-1111-1111-111111111111|strava|user_oauth_tokens";
const DOMAIN: SealDomain = SealDomain::Stored(StoredSecretKind::OAuthAccessToken);
const TEST_MEK: [u8; 32] = [b'b'; 32];

/// A fresh database booted by a manager; also returns the bootstrap key the
/// database stored as v1.
async fn boot() -> (KeyManager, Database, [u8; 32]) {
    let (mut key_manager, database_key) = bootstrap_keys(TEST_MEK);
    let database = create_test_db_with_key(database_key.to_vec())
        .await
        .unwrap();
    key_manager
        .complete_initialization(&database)
        .await
        .unwrap();
    (key_manager, database, database_key)
}

/// Stage a data rotation and let the controller promote it once the fleet
/// window has passed.
async fn rotate_and_promote(key_manager: &KeyManager, database: &Database) -> u32 {
    let staged = key_manager.rotate(database, KeyFamily::Data).await.unwrap();
    let controller = KeysetController::new(
        key_manager.keyset_loader().unwrap(),
        key_manager.kek(),
        Vec::new(),
        0,
    );
    controller
        .tick(
            database.as_security_repository(),
            Utc::now() + Duration::minutes(16),
        )
        .await
        .unwrap();
    staged
}

#[tokio::test]
#[serial]
async fn dek_rotation_preserves_old_data_and_blind_index() {
    let (key_manager, database, _) = boot().await;
    let security = database.as_security_repository();

    // Encrypt under the initial (v1) key and capture the refresh-token blind index.
    let ciphertext_v1 = security
        .encrypt_data_with_aad(DOMAIN, "token-1", AAD)
        .unwrap();
    assert!(
        ciphertext_v1.starts_with("v1:"),
        "initial ciphertext must be v1, got: {ciphertext_v1}"
    );
    let blind_index_before = database.keyring().hash_for_storage("refresh-xyz").unwrap();

    // Staging alone changes nothing a write does: v2 is not primary yet.
    let staged = key_manager
        .rotate(&database, KeyFamily::Data)
        .await
        .unwrap();
    assert_eq!(staged, 2, "rotation must stage the next data version");
    let while_staged = security
        .encrypt_data_with_aad(DOMAIN, "token-s", AAD)
        .unwrap();
    assert!(
        while_staged.starts_with("v1:"),
        "a staged key must not seal before it is promoted, got: {while_staged}"
    );

    // The controller promotes it once the fleet window has passed.
    let controller = KeysetController::new(
        key_manager.keyset_loader().unwrap(),
        key_manager.kek(),
        Vec::new(),
        0,
    );
    controller
        .tick(security, Utc::now() + Duration::minutes(16))
        .await
        .unwrap();
    assert_eq!(database.keyring().current().data_primary(), 2);

    // 1. Old v1 ciphertext is still decryptable (no data loss on rotation).
    assert_eq!(
        security
            .decrypt_data_with_aad(DOMAIN, &ciphertext_v1, AAD)
            .unwrap(),
        "token-1",
        "data encrypted before rotation must remain decryptable"
    );

    // 2. New writes use the rotated (v2) key and round-trip.
    let ciphertext_v2 = security
        .encrypt_data_with_aad(DOMAIN, "token-2", AAD)
        .unwrap();
    assert!(
        ciphertext_v2.starts_with("v2:"),
        "post-rotation ciphertext must be v2, got: {ciphertext_v2}"
    );
    assert_eq!(
        security
            .decrypt_data_with_aad(DOMAIN, &ciphertext_v2, AAD)
            .unwrap(),
        "token-2"
    );

    // 3. The refresh-token blind index is unchanged: a data rotation does not
    //    move the index primary.
    assert_eq!(database.keyring().current().index_primary(), 1);
    let blind_index_after = database.keyring().hash_for_storage("refresh-xyz").unwrap();
    assert_eq!(
        blind_index_before, blind_index_after,
        "refresh-token blind index must survive a data rotation (index stays on v1)"
    );
}

#[tokio::test]
#[serial]
async fn dek_survives_reload_after_rotation() {
    // First lifecycle: initialize, rotate, encrypt under v2.
    let (key_manager, database, bootstrap_key) = boot().await;
    assert_eq!(rotate_and_promote(&key_manager, &database).await, 2);
    let ciphertext_v2 = database
        .as_security_repository()
        .encrypt_data_with_aad(DOMAIN, "persisted", AAD)
        .unwrap();
    assert!(ciphertext_v2.starts_with("v2:"));

    // A restarted process opens its handle on a bootstrap key alone: v2 is
    // not in it, so only the reload can make the ciphertext open.
    database
        .keyring()
        .install(Keyring::single(bootstrap_key.to_vec()), Utc::now());
    assert!(
        database
            .as_security_repository()
            .decrypt_data_with_aad(DOMAIN, &ciphertext_v2, AAD)
            .is_err(),
        "test setup: a handle holding only v1 must not open v2 ciphertext"
    );

    // Second lifecycle: a fresh manager must reload every version and decrypt v2 data.
    let (mut key_manager, _) = bootstrap_keys(TEST_MEK);
    key_manager
        .complete_initialization(&database)
        .await
        .unwrap();
    assert_eq!(database.keyring().current().data_primary(), 2);
    assert_eq!(
        database
            .as_security_repository()
            .decrypt_data_with_aad(DOMAIN, &ciphertext_v2, AAD)
            .unwrap(),
        "persisted",
        "after reload, the rotated primary must decrypt its own ciphertext"
    );
}
