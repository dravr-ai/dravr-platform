// ABOUTME: Integration tests for version-aware data-at-rest encryption on whichever backend the test factory opens
// ABOUTME: Verifies the active DEK version tag, legacy un-tagged compatibility, and unknown-version rejection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for DEK versioning (ADR-017 Phase 3).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_database::backends::factory::Database;
use pierre_database::database::{generate_encryption_key, test_utils::create_test_db_with_key};

const AAD: &str = "tenant-1|11111111-1111-1111-1111-111111111111|strava|user_oauth_tokens";

async fn memory_db() -> Database {
    create_test_db_with_key(generate_encryption_key().to_vec())
        .await
        .unwrap()
}

#[tokio::test]
async fn aad_ciphertext_is_tagged_with_active_version_and_roundtrips() {
    let db = memory_db().await;

    let ciphertext = db
        .as_security_repository()
        .encrypt_data_with_aad("secret-token", AAD)
        .unwrap();
    assert!(
        ciphertext.starts_with("v1:"),
        "new ciphertext must carry the active DEK version tag, got: {ciphertext}"
    );

    let plaintext = db
        .as_security_repository()
        .decrypt_data_with_aad(&ciphertext, AAD)
        .unwrap();
    assert_eq!(plaintext, "secret-token");
}

#[tokio::test]
async fn legacy_untagged_ciphertext_still_decrypts() {
    let db = memory_db().await;

    let ciphertext = db
        .as_security_repository()
        .encrypt_data_with_aad("legacy-value", AAD)
        .unwrap();
    // Simulate pre-versioning ciphertext by stripping the version tag.
    let legacy = ciphertext.strip_prefix("v1:").expect("freshly tagged v1");

    let plaintext = db
        .as_security_repository()
        .decrypt_data_with_aad(legacy, AAD)
        .unwrap();
    assert_eq!(
        plaintext, "legacy-value",
        "un-tagged ciphertext must decrypt as the legacy version-1 key"
    );
}

#[tokio::test]
async fn unknown_dek_version_is_rejected() {
    let db = memory_db().await;

    let ciphertext = db
        .as_security_repository()
        .encrypt_data_with_aad("x", AAD)
        .unwrap();
    let payload = ciphertext.strip_prefix("v1:").unwrap();
    let forged = format!("v9:{payload}");

    assert!(
        db.as_security_repository()
            .decrypt_data_with_aad(&forged, AAD)
            .is_err(),
        "ciphertext tagged with an unavailable DEK version must be rejected"
    );
}

#[tokio::test]
async fn wrong_aad_still_fails_under_versioning() {
    let db = memory_db().await;

    let ciphertext = db
        .as_security_repository()
        .encrypt_data_with_aad("bound", AAD)
        .unwrap();
    let other_aad = "tenant-2|22222222-2222-2222-2222-222222222222|strava|user_oauth_tokens";
    assert!(
        db.as_security_repository()
            .decrypt_data_with_aad(&ciphertext, other_aad)
            .is_err(),
        "AAD binding must still be enforced on versioned ciphertext"
    );
}
