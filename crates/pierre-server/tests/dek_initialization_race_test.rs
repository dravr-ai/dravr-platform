// ABOUTME: Integration tests that DEK initialization never replaces a stored Database Encryption Key
// ABOUTME: A missing row stores one key, racing fresh instances converge on it, and an existing key survives every boot
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression tests for carnet#696: two instances booting against a database
//! under connection-slot exhaustion each stored a fresh random DEK over the
//! real one, and every encrypted row became unreadable.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use base64::engine::general_purpose::STANDARD as Base64Standard;
use base64::Engine as Base64Engine;
use pierre_auth::key_management::{
    DatabaseEncryptionKey, KekProvider, KeyManager, LocalKekProvider, MasterEncryptionKey,
};
use pierre_core::errors::ErrorCode;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_test_support::db::{create_concurrent_test_db, create_test_db_with_key};

/// System-secret name the version-1 wrapped DEK is stored under.
const V1_SECRET: &str = "database_encryption_key";
/// System-secret name a version-2 wrapped DEK is stored under.
const V2_SECRET: &str = "database_encryption_key_v2";
/// System-secret name the active DEK version is stored under.
const ACTIVE_VERSION_SECRET: &str = "database_encryption_key_active_version";
const MEK_BYTES: [u8; 32] = [7u8; 32];

fn kek() -> LocalKekProvider {
    LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes(MEK_BYTES))
}

/// A manager as a booting instance builds it: the shared KEK and a fresh
/// random bootstrap DEK.
fn booting_manager() -> KeyManager {
    KeyManager::with_provider(Box::new(kek()), DatabaseEncryptionKey::generate())
}

/// The DEK the store holds as version 1, unwrapped.
async fn stored_v1_key(database: &Database) -> Vec<u8> {
    let wrapped_base64 = database
        .as_security_repository()
        .get_system_secret(V1_SECRET)
        .await
        .unwrap();
    let wrapped = Base64Standard.decode(wrapped_base64).unwrap();
    kek().unwrap(&wrapped).await.unwrap()
}

/// Run one raw statement against whichever backend the factory opened.
async fn execute_raw(database: &Database, sql: &str) {
    match database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql).execute(sqlite.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(sql).execute(pg.pool()).await.unwrap();
        }
    }
}

/// Run one statement binding `secret_type` as `$1` against whichever backend
/// the factory opened.
async fn execute_for_secret(database: &Database, sql: &str, secret_type: &str) {
    match database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql)
                .bind(secret_type)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(sql)
                .bind(secret_type)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

/// Make reads of the stored `secret_type` row fail while the row stays in
/// place.
///
/// On `SQLite` the value is re-typed as a BLOB, byte for byte, so the read
/// fails but an upsert would still succeed — the incident's shape, where the
/// read timed out and the write went through. `PostgreSQL` types the column
/// strictly, so there the column is hidden behind a rename instead, which
/// fails every secret read.
async fn break_reads(database: &Database, secret_type: &str) {
    match database.backend() {
        DatabaseBackend::SQLite(_) => {
            execute_for_secret(
                database,
                "UPDATE system_secrets SET secret_value = CAST(secret_value AS BLOB) \
                 WHERE secret_type = $1",
                secret_type,
            )
            .await;
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(_) => {
            execute_raw(
                database,
                "ALTER TABLE system_secrets RENAME COLUMN secret_value TO secret_value_hidden",
            )
            .await;
        }
    }
}

/// Undo [`break_reads`].
async fn restore_reads(database: &Database, secret_type: &str) {
    match database.backend() {
        DatabaseBackend::SQLite(_) => {
            execute_for_secret(
                database,
                "UPDATE system_secrets SET secret_value = CAST(secret_value AS TEXT) \
                 WHERE secret_type = $1",
                secret_type,
            )
            .await;
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(_) => {
            execute_raw(
                database,
                "ALTER TABLE system_secrets RENAME COLUMN secret_value_hidden TO secret_value",
            )
            .await;
        }
    }
}

#[tokio::test]
async fn read_error_never_writes_a_key_and_fails_initialization() {
    let mut first = booting_manager();
    let mut database = create_test_db_with_key(first.database_key().to_vec())
        .await
        .unwrap();
    first.complete_initialization(&mut database).await.unwrap();
    let original_row = database
        .as_security_repository()
        .get_system_secret(V1_SECRET)
        .await
        .unwrap();

    break_reads(&database, V1_SECRET).await;
    let read_error = database
        .as_security_repository()
        .get_system_secret(V1_SECRET)
        .await
        .unwrap_err();
    assert_ne!(
        read_error.code,
        ErrorCode::ResourceNotFound,
        "a failed read of a present row is not a not-found"
    );

    let mut booting = booting_manager();
    let mut booting_database = database.clone();
    let result = booting.complete_initialization(&mut booting_database).await;
    assert!(
        result.is_err(),
        "a failed DEK read must fail initialization, not store a fresh key"
    );

    restore_reads(&database, V1_SECRET).await;
    assert_eq!(
        database
            .as_security_repository()
            .get_system_secret(V1_SECRET)
            .await
            .unwrap(),
        original_row,
        "the stored DEK row must be exactly what it was before the failed boot"
    );
}

#[tokio::test]
async fn missing_secret_is_reported_as_not_found() {
    let database = create_test_db_with_key(vec![0u8; 32]).await.unwrap();

    let error = database
        .as_security_repository()
        .get_system_secret("no_such_secret")
        .await
        .unwrap_err();

    assert_eq!(
        error.code,
        ErrorCode::ResourceNotFound,
        "a missing row must be a typed not-found, distinguishable from a failed read: {error}"
    );
}

#[tokio::test]
async fn fresh_database_stores_exactly_the_booting_key() {
    let mut manager = booting_manager();
    let bootstrap_key = *manager.database_key();
    let mut database = create_test_db_with_key(bootstrap_key.to_vec())
        .await
        .unwrap();

    manager
        .complete_initialization(&mut database)
        .await
        .unwrap();

    assert_eq!(
        stored_v1_key(&database).await,
        bootstrap_key.to_vec(),
        "a fresh database stores the booting instance's DEK as version 1"
    );
    assert_eq!(*manager.database_key(), bootstrap_key);
    let v2 = database
        .as_security_repository()
        .get_system_secret("database_encryption_key_v2")
        .await
        .unwrap_err();
    assert_eq!(
        v2.code,
        ErrorCode::ResourceNotFound,
        "only one key is stored"
    );
}

#[tokio::test]
async fn existing_key_is_never_replaced_by_a_booting_instance() {
    let mut first = booting_manager();
    let mut database = create_test_db_with_key(first.database_key().to_vec())
        .await
        .unwrap();
    first.complete_initialization(&mut database).await.unwrap();
    let original_key = *first.database_key();
    let original_row = database
        .as_security_repository()
        .get_system_secret(V1_SECRET)
        .await
        .unwrap();

    // A second instance boots with its own random bootstrap DEK.
    let mut second = booting_manager();
    assert_ne!(*second.database_key(), original_key);
    let mut second_database = database.clone();
    second
        .complete_initialization(&mut second_database)
        .await
        .unwrap();

    let row_after = database
        .as_security_repository()
        .get_system_secret(V1_SECRET)
        .await
        .unwrap();
    assert_eq!(
        row_after, original_row,
        "the stored DEK row must not change"
    );
    assert_eq!(
        *second.database_key(),
        original_key,
        "the second instance adopts the stored DEK, not its bootstrap key"
    );

    // Data the first instance encrypted decrypts on the second.
    let aad = "tenant-1|user-1|strava|user_oauth_tokens";
    let ciphertext = database
        .as_security_repository()
        .encrypt_data_with_aad("token", aad)
        .unwrap();
    assert_eq!(
        second_database
            .as_security_repository()
            .decrypt_data_with_aad(&ciphertext, aad)
            .unwrap(),
        "token"
    );
}

#[tokio::test]
async fn concurrent_fresh_initializations_converge_on_one_key() {
    for _ in 0..8 {
        // Reset to a fresh store for every round: each round is an empty
        // database that two instances boot against at once.
        let database = create_concurrent_test_db().await.unwrap();
        let mut database_a = database.clone();
        let mut database_b = database.clone();
        let mut manager_a = booting_manager();
        let mut manager_b = booting_manager();
        assert_ne!(*manager_a.database_key(), *manager_b.database_key());

        let (result_a, result_b) = tokio::join!(
            manager_a.complete_initialization(&mut database_a),
            manager_b.complete_initialization(&mut database_b),
        );
        result_a.unwrap();
        result_b.unwrap();

        let stored = stored_v1_key(&database).await;
        assert_eq!(
            manager_a.database_key().to_vec(),
            stored,
            "instance A must run on the one stored DEK"
        );
        assert_eq!(
            manager_b.database_key().to_vec(),
            stored,
            "instance B must run on the one stored DEK"
        );

        let aad = "tenant-1|user-1|strava|user_oauth_tokens";
        let ciphertext = database_a
            .as_security_repository()
            .encrypt_data_with_aad("token", aad)
            .unwrap();
        assert_eq!(
            database_b
                .as_security_repository()
                .decrypt_data_with_aad(&ciphertext, aad)
                .unwrap(),
            "token",
            "ciphertext written by one instance must decrypt on the other"
        );
    }
}

#[tokio::test]
async fn rotation_that_loses_the_race_fails_and_keeps_the_winner() {
    let mut manager = booting_manager();
    let original_key = *manager.database_key();
    let mut database = create_test_db_with_key(original_key.to_vec())
        .await
        .unwrap();
    manager
        .complete_initialization(&mut database)
        .await
        .unwrap();

    // A concurrent rotation stored version 2 first.
    let winner = Base64Standard.encode(kek().wrap(&[9u8; 32]).await.unwrap());
    database
        .as_security_repository()
        .insert_system_secret_if_absent(V2_SECRET, &winner)
        .await
        .unwrap();

    let result = manager.rotate_dek(&mut database).await;
    assert!(
        result.is_err(),
        "a rotation whose version was stored first by another must fail"
    );
    assert_eq!(
        database
            .as_security_repository()
            .get_system_secret(V2_SECRET)
            .await
            .unwrap(),
        winner,
        "the version the other rotation stored must not change"
    );
    assert_ne!(
        database
            .as_security_repository()
            .get_system_secret(ACTIVE_VERSION_SECRET)
            .await
            .ok()
            .as_deref(),
        Some("2"),
        "the losing rotation must not activate a version it did not store"
    );
    assert_eq!(
        *manager.database_key(),
        original_key,
        "the losing rotation keeps running on its current DEK"
    );
}

#[tokio::test]
async fn failed_active_version_read_fails_initialization() {
    let mut first = booting_manager();
    let mut database = create_test_db_with_key(first.database_key().to_vec())
        .await
        .unwrap();
    first.complete_initialization(&mut database).await.unwrap();
    assert_eq!(first.rotate_dek(&mut database).await.unwrap(), 2);

    break_reads(&database, ACTIVE_VERSION_SECRET).await;
    let mut booting = booting_manager();
    let mut booting_database = database.clone();
    let result = booting.complete_initialization(&mut booting_database).await;
    restore_reads(&database, ACTIVE_VERSION_SECRET).await;

    assert!(
        result.is_err(),
        "a failed active-version read must fail initialization, not boot on version 1"
    );
    assert_eq!(
        database
            .as_security_repository()
            .get_system_secret(ACTIVE_VERSION_SECRET)
            .await
            .unwrap(),
        "2",
        "the stored active version is untouched"
    );
}
