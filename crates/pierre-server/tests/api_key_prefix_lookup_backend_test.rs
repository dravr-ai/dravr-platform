// ABOUTME: API-key repository contract against the REAL configured backend — the prefix lookup, the tier and
// ABOUTME: limits a key reads back with, the operator listing's filters, and same-day expiry cleanup
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

use chrono::{Duration, Utc};
use pierre_auth::api_keys::{ApiKeyManager, ApiKeyTier, CreateApiKeyRequest};
use pierre_core::models::{ApiKey, User, UserStatus, UserTier};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// A distinct active user per call, returned with the email the operator
/// listing filters on.
async fn fresh_user(repos: &RepositoryRegistry) -> (Uuid, String) {
    let uuid = Uuid::new_v4();
    let email = format!("api_keys_{uuid}@example.com");
    let mut user = User::new(email.clone(), "hashed".into(), Some("Key Owner".into()));
    user.id = uuid;
    user.user_status = UserStatus::Active;
    repos.users.create(&user).await.expect("user created");
    (uuid, email)
}

/// A key row of the given tier for the user, generated the way the API does
/// it and stored through the repository.
async fn stored_key(repos: &RepositoryRegistry, user_id: Uuid, tier: ApiKeyTier) -> ApiKey {
    let (api_key, _) = ApiKeyManager::new()
        .create_api_key(
            user_id,
            CreateApiKeyRequest {
                name: format!("{tier:?} key"),
                description: None,
                tier,
                rate_limit_requests: None,
                expires_in_days: None,
            },
        )
        .expect("key generated");
    repos.api_keys.create(&api_key).await.expect("key stored");
    api_key
}

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

/// A trial key reads back as a trial key. The Postgres lookup used to map the
/// stored `trial` onto `Starter` in the authentication path, so a trial key
/// authenticated with starter limits there while `SQLite` kept it on trial.
#[tokio::test]
async fn a_trial_key_reads_back_as_trial_on_the_configured_backend() {
    let db = backend_under_test().await;
    let repos = db.repositories();
    let (user_id, _) = fresh_user(&repos).await;
    let stored = stored_key(&repos, user_id, ApiKeyTier::Trial).await;

    let found = repos
        .api_keys
        .get_by_prefix(&stored.key_prefix, &stored.key_hash)
        .await
        .expect("lookup query executes")
        .expect("the key is found");
    assert_eq!(found.tier, ApiKeyTier::Trial);
    assert_eq!(
        found.rate_limit_requests, stored.rate_limit_requests,
        "the trial limit round-trips as stored"
    );
    assert_eq!(
        found.rate_limit_window_seconds,
        stored.rate_limit_window_seconds
    );

    let listed = repos.api_keys.get_for_user(user_id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].tier, ApiKeyTier::Trial);
}

/// An enterprise key's limit round-trips as the model set it on both
/// backends. `SQLite` used to store NULL for the tier and read it back as
/// `u32::MAX` whatever the model said; Postgres declares the column NOT NULL.
#[tokio::test]
async fn an_enterprise_key_reads_back_its_limit_on_the_configured_backend() {
    let db = backend_under_test().await;
    let repos = db.repositories();
    let (user_id, _) = fresh_user(&repos).await;
    let stored = stored_key(&repos, user_id, ApiKeyTier::Enterprise).await;

    let found = repos
        .api_keys
        .get_by_id(&stored.id, Some(user_id))
        .await
        .unwrap()
        .expect("the key is found");
    assert_eq!(found.tier, ApiKeyTier::Enterprise);
    assert_eq!(found.rate_limit_requests, stored.rate_limit_requests);
    assert_eq!(
        found.rate_limit_window_seconds,
        stored.rate_limit_window_seconds
    );
}

/// The key's own `created_at` is what is stored, not the moment the row was
/// written: Postgres used to leave the column to its default and a key
/// created earlier read back as created now.
#[tokio::test]
async fn created_at_round_trips_on_the_configured_backend() {
    let db = backend_under_test().await;
    let repos = db.repositories();
    let (user_id, _) = fresh_user(&repos).await;
    let mut api_key = stored_key(&repos, user_id, ApiKeyTier::Starter).await;
    // A second key, whose creation time is a day back.
    api_key.id = format!("key_{}", Uuid::new_v4().simple());
    api_key.key_prefix = format!("pk_live_{}", Uuid::new_v4().simple());
    api_key.key_hash = format!("hash_{}", Uuid::new_v4().simple());
    api_key.created_at = Utc::now() - Duration::days(1);
    repos.api_keys.create(&api_key).await.expect("key stored");

    let found = repos
        .api_keys
        .get_by_id(&api_key.id, None)
        .await
        .unwrap()
        .expect("the key is found");
    assert!(
        (found.created_at - api_key.created_at).num_seconds().abs() < 1,
        "created_at must be the key's own moment: stored {}, read {}",
        api_key.created_at,
        found.created_at
    );
}

/// The operator listing honours every filter on both backends. `SQLite`
/// used to ignore the email, treat `active_only = false` as "inactive only",
/// and cap the listing at ten rows when no limit was given.
#[tokio::test]
async fn the_operator_listing_filters_by_email_and_activity_on_the_configured_backend() {
    let db = backend_under_test().await;
    let repos = db.repositories();
    let (owner, owner_email) = fresh_user(&repos).await;
    let (other, _) = fresh_user(&repos).await;

    let mut keys = Vec::new();
    for _ in 0..3 {
        keys.push(stored_key(&repos, owner, ApiKeyTier::Starter).await);
    }
    stored_key(&repos, other, ApiKeyTier::Starter).await;
    repos
        .api_keys
        .deactivate(&keys[0].id, owner)
        .await
        .expect("deactivated");

    let active_owned = repos
        .api_keys
        .get_filtered(Some(&owner_email), true, None, None)
        .await
        .unwrap();
    assert_eq!(
        active_owned
            .iter()
            .map(|k| k.id.as_str())
            .collect::<Vec<_>>(),
        vec![keys[2].id.as_str(), keys[1].id.as_str()],
        "the owner's active keys, newest first, and nobody else's"
    );

    let all_owned = repos
        .api_keys
        .get_filtered(Some(&owner_email), false, None, None)
        .await
        .unwrap();
    assert_eq!(
        all_owned.len(),
        3,
        "active_only = false lists the deactivated key too"
    );
    assert!(all_owned.iter().any(|k| !k.is_active));

    let page = repos
        .api_keys
        .get_filtered(Some(&owner_email), false, Some(2), Some(1))
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(),
        vec![keys[1].id.as_str(), keys[0].id.as_str()],
        "limit and offset window the listing"
    );

    let everyone = repos
        .api_keys
        .get_filtered(None, true, None, None)
        .await
        .unwrap();
    assert!(
        everyone.iter().any(|k| k.user_id == other),
        "without an email the listing spans every user"
    );
}

/// A key that expired earlier today is cleaned up. The expiry used to be
/// compared against `CURRENT_TIMESTAMP`, which on `SQLite` is text of another
/// width than the stored RFC 3339 value, so a same-day expiry sorted after
/// "now" and the key stayed active until the next day.
#[tokio::test]
async fn a_key_expired_minutes_ago_is_cleaned_up_on_the_configured_backend() {
    let db = backend_under_test().await;
    let repos = db.repositories();
    let (user_id, _) = fresh_user(&repos).await;
    let mut api_key = stored_key(&repos, user_id, ApiKeyTier::Starter).await;
    api_key.id = format!("key_{}", Uuid::new_v4().simple());
    api_key.key_prefix = format!("pk_live_{}", Uuid::new_v4().simple());
    api_key.key_hash = format!("hash_{}", Uuid::new_v4().simple());
    api_key.expires_at = Some(Utc::now() - Duration::minutes(5));
    repos.api_keys.create(&api_key).await.expect("key stored");

    assert_eq!(
        repos.api_keys.cleanup_expired().await.unwrap(),
        1,
        "exactly the key that expired five minutes ago is deactivated"
    );
    let cleaned = repos
        .api_keys
        .get_by_id(&api_key.id, None)
        .await
        .unwrap()
        .expect("the key row remains");
    assert!(!cleaned.is_active);
    assert!(
        repos
            .api_keys
            .get_by_prefix(&api_key.key_prefix, &api_key.key_hash)
            .await
            .unwrap()
            .is_none(),
        "a deactivated key no longer authenticates"
    );
}
