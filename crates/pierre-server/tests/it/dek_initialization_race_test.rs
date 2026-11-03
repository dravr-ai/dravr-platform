// ABOUTME: Integration tests that key initialization never replaces a stored Database Encryption Key
// ABOUTME: A missing row stores one key, racing instances converge on it, an existing key survives every boot, one rotation wins
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression tests for carnet#696: two instances booting against a database
//! under connection-slot exhaustion each stored a fresh random DEK over the
//! real one, and every encrypted row became unreadable.
//!
//! Every clone of a [`Database`] shares one keyring, so a test that needs a
//! second *instance* opens a second handle over the same store with its own
//! keyring ([`open_instance`]), as a second process does.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use base64::engine::general_purpose::STANDARD as Base64Standard;
use base64::Engine as Base64Engine;
use chrono::{Duration, Utc};
use pierre_auth::key_management::{
    DatabaseEncryptionKey, KekProvider, KeyManager, LocalKekProvider, MasterEncryptionKey,
};
use pierre_auth::keyset::secrets::{
    insert_key_if_absent, secret_name, ACTIVE_VERSION_SECRET, V1_SECRET,
};
use pierre_auth::keyset::{read_keyset, KeyState, KeysetController, KEYSET_SECRET};
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::errors::ErrorCode;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::backends::shared::key_material::StoredSecretKind;
use pierre_database::backends::shared::keyring::{KeyFamily, Keyring, SealDomain};
use pierre_database::database::Database as SqliteDatabase;
use pierre_test_support::db::{create_concurrent_test_db, create_test_db_with_key};

const MEK_BYTES: [u8; 32] = [7u8; 32];
const AAD: &str = "tenant-1|user-1|strava|user_oauth_tokens";
const DOMAIN: SealDomain = SealDomain::Stored(StoredSecretKind::OAuthAccessToken);

fn kek() -> LocalKekProvider {
    LocalKekProvider::from_mek(MasterEncryptionKey::from_bytes(MEK_BYTES))
}

/// A manager as a booting instance builds it: the shared KEK and a fresh
/// random bootstrap key, returned beside it.
fn booting_manager() -> (KeyManager, [u8; 32]) {
    KeyManager::bootstrap_with(Box::new(kek()))
}

/// The key the store holds as version 1, unwrapped.
async fn stored_v1_key(database: &Database) -> Vec<u8> {
    let wrapped_base64 = database
        .as_security_repository()
        .get_system_secret(V1_SECRET)
        .await
        .unwrap();
    let wrapped = Base64Standard.decode(wrapped_base64).unwrap();
    kek().unwrap(&wrapped).await.unwrap()
}

/// A second handle over the same store, opened on `key` with a keyring of
/// its own: what another process booting against this database holds before
/// its key management initializes.
async fn open_instance(database: &Database, key: [u8; 32]) -> Database {
    match database.backend() {
        DatabaseBackend::SQLite(sqlite) => Database::from_sqlite(
            SqliteDatabase::from_pool(sqlite.pool().clone(), key.to_vec())
                .await
                .unwrap(),
        ),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            use sqlx::ConnectOptions;
            let url = pg.pool().connect_options().to_url_lossy();
            let config = PostgresPoolConfig {
                max_connections: 4,
                min_connections: 0,
                ..PostgresPoolConfig::default()
            };
            Database::new(url.as_str(), key.to_vec(), &config)
                .await
                .unwrap()
        }
    }
}

/// Assert that `database`'s keyring runs on exactly `key`: what it seals
/// opens under `key` alone, and what `key` alone seals opens through it.
fn assert_runs_on(database: &Database, key: &[u8], who: &str) {
    let oracle = Keyring::single(key.to_vec());
    let sealed = database
        .as_security_repository()
        .encrypt_data_with_aad(DOMAIN, who, AAD)
        .unwrap();
    assert_eq!(
        oracle.open(DOMAIN, &sealed, AAD).unwrap(),
        who,
        "{who} must seal under the stored key"
    );
    let from_oracle = oracle.seal(DOMAIN, who, AAD).unwrap();
    assert_eq!(
        database
            .as_security_repository()
            .decrypt_data_with_aad(DOMAIN, &from_oracle, AAD)
            .unwrap(),
        who,
        "{who} must open what the stored key sealed"
    );
}

/// The stored keyset document's text, or `None` when there is none.
async fn keyset_text(database: &Database) -> Option<String> {
    read_keyset(database.as_security_repository())
        .await
        .unwrap()
        .map(|(text, _)| text)
}

/// Stage a data rotation and let the controller promote it.
async fn rotate_and_promote(manager: &KeyManager, database: &Database) {
    manager.rotate(database, KeyFamily::Data).await.unwrap();
    KeysetController::new(
        manager.keyset_loader().unwrap(),
        manager.kek(),
        Vec::new(),
        0,
    )
    .tick(
        database.as_security_repository(),
        Utc::now() + Duration::minutes(16),
    )
    .await
    .unwrap();
}

/// Run one raw statement against whichever backend the factory opened.
///
/// Only the `PostgreSQL` arms of [`break_reads`] and [`restore_reads`] call
/// it, so it exists only where that backend is compiled in; a default-feature
/// build (the macOS cross-platform lane) would otherwise deny it as dead code.
#[cfg(feature = "postgresql")]
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
/// fails every secret read (the keyset document's included).
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

/// A fresh database booted by one instance.
async fn booted_database() -> (KeyManager, Database, [u8; 32]) {
    let (mut manager, bootstrap_key) = booting_manager();
    let database = create_test_db_with_key(bootstrap_key.to_vec())
        .await
        .unwrap();
    manager.complete_initialization(&database).await.unwrap();
    (manager, database, bootstrap_key)
}

#[tokio::test]
async fn read_error_never_writes_a_key_and_fails_initialization() {
    let (_first, database, _) = booted_database().await;
    let original_row = database
        .as_security_repository()
        .get_system_secret(V1_SECRET)
        .await
        .unwrap();
    let original_keyset = keyset_text(&database).await;

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

    let (mut booting, _) = booting_manager();
    let result = booting.complete_initialization(&database).await;
    assert!(
        result.is_err(),
        "a failed key read must fail initialization, not store a fresh key"
    );

    restore_reads(&database, V1_SECRET).await;
    assert_eq!(
        database
            .as_security_repository()
            .get_system_secret(V1_SECRET)
            .await
            .unwrap(),
        original_row,
        "the stored key row must be exactly what it was before the failed boot"
    );
    assert_eq!(
        keyset_text(&database).await,
        original_keyset,
        "the failed boot must not rewrite the keyset document"
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
    let (_manager, database, bootstrap_key) = booted_database().await;

    assert_eq!(
        stored_v1_key(&database).await,
        bootstrap_key.to_vec(),
        "a fresh database stores the booting instance's key as version 1"
    );
    assert_runs_on(&database, &bootstrap_key, "the booting instance");

    let (_, doc) = read_keyset(database.as_security_repository())
        .await
        .unwrap()
        .expect("a fresh database gets its keyset document at boot");
    assert_eq!(doc.primary(KeyFamily::Data).unwrap(), 1);
    assert_eq!(doc.primary(KeyFamily::Index).unwrap(), 1);
    assert_eq!(
        doc.versions(KeyFamily::Data).len(),
        1,
        "only one key is recorded"
    );

    let v2 = database
        .as_security_repository()
        .get_system_secret(&secret_name(KeyFamily::Data, 2))
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
    let (_first, database, original_key) = booted_database().await;
    let original_row = database
        .as_security_repository()
        .get_system_secret(V1_SECRET)
        .await
        .unwrap();
    // Sealed before the second instance boots.
    let ciphertext = database
        .as_security_repository()
        .encrypt_data_with_aad(DOMAIN, "token", AAD)
        .unwrap();

    // A second instance boots with its own random bootstrap key.
    let (mut second, second_key) = booting_manager();
    assert_ne!(second_key, original_key);
    let second_database = open_instance(&database, second_key).await;
    assert!(
        second_database
            .as_security_repository()
            .decrypt_data_with_aad(DOMAIN, &ciphertext, AAD)
            .is_err(),
        "test setup: before initializing, the second instance holds only its bootstrap key"
    );
    second
        .complete_initialization(&second_database)
        .await
        .unwrap();

    let row_after = database
        .as_security_repository()
        .get_system_secret(V1_SECRET)
        .await
        .unwrap();
    assert_eq!(
        row_after, original_row,
        "the stored key row must not change"
    );
    assert_runs_on(&second_database, &original_key, "the second instance");

    // Data the first instance encrypted decrypts on the second.
    assert_eq!(
        second_database
            .as_security_repository()
            .decrypt_data_with_aad(DOMAIN, &ciphertext, AAD)
            .unwrap(),
        "token"
    );
}

#[tokio::test]
async fn concurrent_fresh_initializations_converge_on_one_key() {
    for _ in 0..8 {
        // Reset to a fresh store for every round: each round is an empty
        // database that two instances boot against at once, each with a
        // handle and a keyring of its own.
        let database = create_concurrent_test_db().await.unwrap();
        let (mut manager_a, key_a) = booting_manager();
        let (mut manager_b, key_b) = booting_manager();
        assert_ne!(key_a, key_b);
        let database_a = open_instance(&database, key_a).await;
        let database_b = open_instance(&database, key_b).await;

        let (result_a, result_b) = tokio::join!(
            manager_a.complete_initialization(&database_a),
            manager_b.complete_initialization(&database_b),
        );
        result_a.unwrap();
        result_b.unwrap();

        let stored = stored_v1_key(&database).await;
        assert!(
            stored == key_a || stored == key_b,
            "the stored key must be one of the booting instances' keys"
        );
        assert_runs_on(&database_a, &stored, "instance A");
        assert_runs_on(&database_b, &stored, "instance B");

        let ciphertext = database_a
            .as_security_repository()
            .encrypt_data_with_aad(DOMAIN, "token", AAD)
            .unwrap();
        assert_eq!(
            database_b
                .as_security_repository()
                .decrypt_data_with_aad(DOMAIN, &ciphertext, AAD)
                .unwrap(),
            "token",
            "ciphertext written by one instance must decrypt on the other"
        );
    }
}

#[tokio::test]
async fn rotation_that_loses_the_race_fails_and_keeps_the_winner() {
    let (manager, database, original_key) = booted_database().await;
    let original_keyset = keyset_text(&database).await;

    // A concurrent rotation stored version 2 first.
    let winner_bytes = kek().wrap(&[9u8; 32]).await.unwrap();
    assert!(insert_key_if_absent(
        database.as_security_repository(),
        KeyFamily::Data,
        2,
        &winner_bytes
    )
    .await
    .unwrap());
    let winner = Base64Standard.encode(&winner_bytes);

    let result = manager.rotate(&database, KeyFamily::Data).await;
    assert!(
        result.is_err(),
        "a rotation whose version was stored first by another must fail"
    );
    assert_eq!(
        database
            .as_security_repository()
            .get_system_secret(&secret_name(KeyFamily::Data, 2))
            .await
            .unwrap(),
        winner,
        "the version the other rotation stored must not change"
    );
    let (_, doc) = read_keyset(database.as_security_repository())
        .await
        .unwrap()
        .unwrap();
    assert!(
        !doc.versions(KeyFamily::Data).contains_key(&2),
        "the losing rotation must not record a version it did not store"
    );
    assert_eq!(doc.primary(KeyFamily::Data).unwrap(), 1);
    assert_eq!(
        keyset_text(&database).await,
        original_keyset,
        "the losing rotation leaves the keyset document as it was"
    );
    assert_eq!(database.keyring().current().data_primary(), 1);
    assert_runs_on(&database, &original_key, "the losing rotation's instance");
}

#[tokio::test]
async fn concurrent_rotations_of_one_family_stage_exactly_one() {
    let (manager_a, database_a, _) = booted_database().await;
    let (mut manager_b, key_b) = booting_manager();
    let database_b = open_instance(&database_a, key_b).await;
    manager_b
        .complete_initialization(&database_b)
        .await
        .unwrap();

    let (result_a, result_b) = tokio::join!(
        manager_a.rotate(&database_a, KeyFamily::Data),
        manager_b.rotate(&database_b, KeyFamily::Data),
    );
    let staged: Vec<u32> = result_a.into_iter().chain(result_b).collect();
    assert_eq!(
        staged,
        vec![2],
        "exactly one of two concurrent rotations of a family stages, as v2"
    );

    let (_, doc) = read_keyset(database_a.as_security_repository())
        .await
        .unwrap()
        .unwrap();
    let versions = doc.versions(KeyFamily::Data);
    assert_eq!(
        versions.len(),
        2,
        "one version staged beside v1: {versions:?}"
    );
    assert_eq!(versions[&2].state, KeyState::Staged);
    assert_eq!(doc.primary(KeyFamily::Data).unwrap(), 1);

    // The staged row is the one the document records: it unwraps, and a
    // promotion seals under it on both instances.
    let staged_row = database_a
        .as_security_repository()
        .get_system_secret(&secret_name(KeyFamily::Data, 2))
        .await
        .unwrap();
    let staged_key = kek()
        .unwrap(&Base64Standard.decode(staged_row).unwrap())
        .await
        .unwrap();
    assert!(
        manager_a
            .rotate(&database_a, KeyFamily::Data)
            .await
            .is_err(),
        "one rotation per family at a time: a staged version refuses another"
    );
    KeysetController::new(
        manager_a.keyset_loader().unwrap(),
        manager_a.kek(),
        Vec::new(),
        0,
    )
    .tick(
        database_a.as_security_repository(),
        Utc::now() + Duration::minutes(16),
    )
    .await
    .unwrap();
    manager_b
        .keyset_loader()
        .unwrap()
        .refresh(database_b.as_security_repository())
        .await
        .unwrap();
    let oracle = Keyring::new(
        [(2, staged_key.clone())].into(),
        2,
        [(2, staged_key)].into(),
        2,
        0,
        false,
    )
    .unwrap();
    for (who, database) in [("instance A", &database_a), ("instance B", &database_b)] {
        let sealed = database
            .as_security_repository()
            .encrypt_data_with_aad(DOMAIN, who, AAD)
            .unwrap();
        assert!(sealed.starts_with("v2:"), "{who} seals under v2: {sealed}");
        assert_eq!(oracle.open(DOMAIN, &sealed, AAD).unwrap(), who);
    }
}

#[tokio::test]
async fn failed_active_version_read_fails_initialization() {
    // The legacy layout: no keyset document, data v1 and v2 stored, the
    // active-version secret naming v2.
    let (_first, database, _) = booted_database().await;
    let security = database.as_security_repository();
    security.delete_system_secret(KEYSET_SECRET).await.unwrap();
    let wrapped = kek()
        .wrap(DatabaseEncryptionKey::generate().as_bytes())
        .await
        .unwrap();
    assert!(insert_key_if_absent(security, KeyFamily::Data, 2, &wrapped)
        .await
        .unwrap());
    security
        .update_system_secret(ACTIVE_VERSION_SECRET, "2")
        .await
        .unwrap();

    break_reads(&database, ACTIVE_VERSION_SECRET).await;
    let (mut booting, _) = booting_manager();
    let result = booting.complete_initialization(&database).await;
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
    assert!(
        keyset_text(&database).await.is_none(),
        "a failed boot must not adopt the legacy layout"
    );
}

#[tokio::test]
async fn failed_keyset_read_fails_initialization() {
    // The adopted layout: the keyset document names v2 primary.
    let (first, database, _) = booted_database().await;
    rotate_and_promote(&first, &database).await;
    let keyset_before = keyset_text(&database).await.unwrap();
    let v1_row = database
        .as_security_repository()
        .get_system_secret(V1_SECRET)
        .await
        .unwrap();

    break_reads(&database, KEYSET_SECRET).await;
    let (mut booting, _) = booting_manager();
    let result = booting.complete_initialization(&database).await;
    restore_reads(&database, KEYSET_SECRET).await;

    assert!(
        result.is_err(),
        "a failed keyset read must fail initialization, not boot on the legacy layout"
    );
    assert_eq!(
        keyset_text(&database).await.as_deref(),
        Some(keyset_before.as_str()),
        "the keyset document is untouched"
    );
    let (_, doc) = read_keyset(database.as_security_repository())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(doc.primary(KeyFamily::Data).unwrap(), 2);
    assert_eq!(
        database
            .as_security_repository()
            .get_system_secret(V1_SECRET)
            .await
            .unwrap(),
        v1_row,
        "the v1 key row is untouched"
    );
    // The booting manager ran over a clone, which shares the first
    // instance's keyring: it keeps running on the document's primary.
    let sealed = database
        .as_security_repository()
        .encrypt_data_with_aad(DOMAIN, "after", AAD)
        .unwrap();
    assert!(
        sealed.starts_with("v2:"),
        "the refused boot installed nothing into the shared keyring: {sealed}"
    );
}
