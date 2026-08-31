// ABOUTME: Regression test for API-key prefix lookup against the REAL configured backend
// ABOUTME: Guards the Postgres get_by_prefix query that matched `id` instead of `key_prefix`
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Why this file exists separately from `database_api_keys_test.rs`.
//!
//! The bug this guards: `PostgreSQL` `get_by_prefix` read `WHERE id LIKE $1`
//! bound to `"{prefix}%"`, but `id` is a generated identifier that never
//! begins with `pk_live_`. The query matched zero rows, so EVERY API key
//! failed authentication on `PostgreSQL` — while provisioning, storage and
//! listing (different queries) all kept reporting the key active.
//!
//! [`create_test_db`] opens whichever backend `DATABASE_URL` names, so this
//! test exercises `PostgreSQL` wherever one is configured and `SQLite`
//! otherwise — the same contract on both.

use pierre_auth::api_keys::{ApiKeyManager, ApiKeyTier, CreateApiKeyRequest};
use pierre_core::models::{User, UserStatus, UserTier};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use uuid::Uuid;

/// The backend `DATABASE_URL` names, opened through the shared factory.
async fn backend_under_test() -> Database {
    create_test_db()
        .await
        .expect("Should connect to the configured database")
}

/// A stored API key must be retrievable by its own prefix and hash.
///
/// This is the single lookup every API-key authentication depends on:
/// `authenticate_api_key` hashes the presented key, extracts its prefix, and
/// calls `get_by_prefix`. A backend that returns `None` here rejects every API
/// key with `AuthInvalid`, no matter how valid the key is.
#[tokio::test]
async fn test_api_key_is_retrievable_by_prefix_on_the_configured_backend() {
    let db = backend_under_test().await;
    let repos = db.repositories();

    let uuid = Uuid::new_v4();
    let mut user = User::new(
        format!("prefix_lookup_{uuid}@example.com"),
        "hashed".into(),
        Some("Prefix Lookup".into()),
    );
    user.id = uuid;
    user.tier = UserTier::Professional;
    user.user_status = UserStatus::Active;
    repos.users.create(&user).await.expect("user created");

    let manager = ApiKeyManager::new();
    let (api_key, key_string) = manager
        .create_api_key(
            user.id,
            CreateApiKeyRequest {
                name: "prefix lookup regression".into(),
                description: Some("guards Postgres get_by_prefix".into()),
                tier: ApiKeyTier::Starter,
                rate_limit_requests: None,
                expires_in_days: None,
            },
        )
        .expect("key generated");
    repos.api_keys.create(&api_key).await.expect("key stored");

    // Derive prefix and hash exactly as the auth middleware does, rather than
    // reusing the stored struct — a lookup that only works against values the
    // caller already holds would not prove authentication works.
    let prefix = manager.extract_key_prefix(&key_string);
    let hash = manager.hash_key(&key_string);

    let found = repos
        .api_keys
        .get_by_prefix(&prefix, &hash)
        .await
        .expect("lookup query executes");

    let found = found.expect(
        "API key must be found by prefix+hash on this backend; None here means every API key \
         fails authentication (the Postgres `WHERE id LIKE` regression)",
    );
    assert_eq!(found.id, api_key.id, "lookup returned a different key");
    assert_eq!(found.user_id, user.id, "lookup returned another user's key");
}

/// A prefix that matches no stored key returns `None` rather than a stray row.
///
/// The broken query used `LIKE` with a `%` wildcard; this pins that lookup is
/// an exact match, so one key can never be returned for another's prefix.
#[tokio::test]
async fn test_unknown_prefix_returns_none() {
    let db = backend_under_test().await;

    let found = db
        .repositories()
        .api_keys
        .get_by_prefix("pk_live_definitelynotarealkeyprefix", "not-a-real-hash")
        .await
        .expect("lookup query executes");

    assert!(found.is_none(), "an unknown prefix must not match a key");
}
