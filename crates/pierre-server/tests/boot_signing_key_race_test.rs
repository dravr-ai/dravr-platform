// ABOUTME: Integration tests that boot-time signing keys (RSA keypair, admin JWT secret) are never replaced
// ABOUTME: A failed read stores nothing, an empty store gets one key, and racing instances converge on it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression tests for carnet#696 beyond the DEK: the RSA keypair that signs
//! user sessions and the admin JWT secret are key material a booting instance
//! creates when the store has none, so the same "read error ⇒ absent ⇒ write"
//! mistake would replace them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_auth::admin::jwks::JwksManager;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_test_support::db::{create_concurrent_test_db, create_test_db};

/// Key size for test keypairs; 2048 bits keeps generation fast.
const TEST_RSA_BITS: usize = 2048;

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

/// The active key id and public PEM a JWKS manager signs with.
fn active_identity(jwks: &JwksManager) -> (String, String) {
    let key = jwks.get_active_key().unwrap();
    (key.kid.clone(), key.export_public_key_pem().unwrap())
}

#[tokio::test]
async fn rsa_read_error_fails_and_stores_no_keypair() {
    let database = create_test_db().await.unwrap();
    execute_raw(
        &database,
        "ALTER TABLE rsa_keypairs RENAME TO rsa_keypairs_hidden",
    )
    .await;

    let result = ServerContext::load_or_create_jwks_manager(&database, TEST_RSA_BITS).await;
    assert!(
        result.is_err(),
        "a failed keypair read must fail, not sign with a key the store does not hold"
    );

    execute_raw(
        &database,
        "ALTER TABLE rsa_keypairs_hidden RENAME TO rsa_keypairs",
    )
    .await;
    assert!(
        database
            .as_security_repository()
            .load_rsa_keypairs()
            .await
            .unwrap()
            .is_empty(),
        "nothing is stored after a failed read"
    );
}

#[tokio::test]
async fn empty_store_gets_exactly_one_keypair_and_reboots_load_it() {
    let database = create_test_db().await.unwrap();

    let first = ServerContext::load_or_create_jwks_manager(&database, TEST_RSA_BITS)
        .await
        .unwrap();
    let stored = database
        .as_security_repository()
        .load_rsa_keypairs()
        .await
        .unwrap();
    assert_eq!(stored.len(), 1, "an empty store gets exactly one keypair");

    let second = ServerContext::load_or_create_jwks_manager(&database, TEST_RSA_BITS)
        .await
        .unwrap();
    assert_eq!(
        active_identity(&second),
        active_identity(&first),
        "a later boot signs with the stored keypair"
    );
    assert_eq!(
        database
            .as_security_repository()
            .load_rsa_keypairs()
            .await
            .unwrap()
            .len(),
        1,
        "a later boot stores nothing"
    );
}

#[tokio::test]
async fn existing_keypair_is_never_replaced() {
    let database = create_test_db().await.unwrap();
    let original = ServerContext::load_or_create_jwks_manager(&database, TEST_RSA_BITS)
        .await
        .unwrap();
    let (kid, original_public) = active_identity(&original);
    let original_private = original
        .get_active_key()
        .unwrap()
        .export_private_key_pem()
        .unwrap();

    // Another key under the same id, as a racing instance in the same second
    // would generate.
    let mut other = JwksManager::new();
    other
        .generate_rsa_key_pair_with_size(&kid, TEST_RSA_BITS)
        .unwrap();
    let other_key = other.get_active_key().unwrap();
    database
        .as_security_repository()
        .save_rsa_keypair(
            &kid,
            &other_key.export_private_key_pem().unwrap(),
            &other_key.export_public_key_pem().unwrap(),
            Utc::now(),
            true,
            2048,
        )
        .await
        .unwrap();

    let stored = database
        .as_security_repository()
        .load_rsa_keypairs()
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    let (stored_kid, stored_private, stored_public, _, _) = &stored[0];
    assert_eq!(stored_kid, &kid);
    assert_eq!(
        stored_private, &original_private,
        "the stored private key must not change"
    );
    assert_eq!(stored_public, &original_public);
}

#[tokio::test]
async fn concurrent_fresh_boots_sign_with_the_same_keypair() {
    for _ in 0..4 {
        let database = create_concurrent_test_db().await.unwrap();
        let (a, b) = tokio::join!(
            ServerContext::load_or_create_jwks_manager(&database, TEST_RSA_BITS),
            ServerContext::load_or_create_jwks_manager(&database, TEST_RSA_BITS),
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(
            active_identity(&a),
            active_identity(&b),
            "instances booting together must sign with the same keypair"
        );

        let stored_kids: Vec<String> = database
            .as_security_repository()
            .load_rsa_keypairs()
            .await
            .unwrap()
            .into_iter()
            .map(|(kid, ..)| kid)
            .collect();
        for jwks in [&a, &b] {
            for kid in &stored_kids {
                assert!(
                    jwks.get_key(kid).is_some(),
                    "every stored keypair verifies on every instance"
                );
            }
        }
    }
}

#[tokio::test]
async fn concurrent_admin_jwt_secret_creation_converges() {
    for _ in 0..8 {
        let database = create_concurrent_test_db().await.unwrap();
        let security_a = database.as_security_repository();
        let security_b = database.as_security_repository();
        let (a, b) = tokio::join!(
            security_a.get_or_create_system_secret("admin_jwt_secret"),
            security_b.get_or_create_system_secret("admin_jwt_secret"),
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a, b, "racing creators must return the one stored secret");
        assert_eq!(
            security_a
                .get_system_secret("admin_jwt_secret")
                .await
                .unwrap(),
            a
        );
    }
}

#[tokio::test]
async fn insert_if_absent_never_replaces_a_secret() {
    let database = create_test_db().await.unwrap();
    let security = database.as_security_repository();

    assert_eq!(
        security
            .insert_system_secret_if_absent("database_encryption_key", "first")
            .await
            .unwrap(),
        "first"
    );
    assert_eq!(
        security
            .insert_system_secret_if_absent("database_encryption_key", "second")
            .await
            .unwrap(),
        "first",
        "the stored value is returned and kept"
    );
    assert_eq!(
        security
            .get_system_secret("database_encryption_key")
            .await
            .unwrap(),
        "first"
    );
}
