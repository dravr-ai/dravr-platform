// ABOUTME: Proves a Database handle builds its RepositoryRegistry once and every clone shares it
// ABOUTME: Also proves one keyring install reaches every clone, those taken before it included
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `Database::repositories()` returns the registry built when the handle was
//! constructed, so the server resources, the runtime contexts and every call
//! site read the same repository instances. Every clone also shares one keyring
//! cell, so installing a new keyring reaches clones taken before the install;
//! these tests pin both halves against a real database from the test factory.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use secrecy::ExposeSecret;
use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;
use pierre_core::models::{Tenant, TenantId, User, UserOAuthToken};
use pierre_database::backends::factory::Database;
use pierre_database::backends::shared::keyring::Keyring;
use pierre_test_support::db::{create_test_db, create_test_db_with_key};
use uuid::Uuid;

/// Insert a user and a tenant it owns, returning both ids.
async fn seed_user_and_tenant(db: &Database) -> (Uuid, TenantId) {
    let repos = db.repositories();
    let user = User::new(
        format!("registry-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Registry Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap();
    let tenant = Tenant::new(
        "Registry Tenant".to_owned(),
        format!("registry-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        user.id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    (user.id, tenant.id)
}

#[tokio::test]
async fn every_call_and_every_clone_share_one_registry() {
    let db = create_test_db().await.expect("factory opens a database");

    let first = db.repositories();
    let second = db.repositories();
    assert!(
        Arc::ptr_eq(first, second),
        "two repositories() calls must return the same registry instance"
    );

    let cloned = db.clone();
    assert!(
        Arc::ptr_eq(db.repositories(), cloned.repositories()),
        "a clone of the Database must share the original's registry"
    );
    assert!(
        Arc::ptr_eq(&first.users, &cloned.repositories().users),
        "the shared registry hands out the same repository instances"
    );

    // The shared registry is live, not merely identical: a row written
    // through the original is read back through the clone.
    let (user_id, _) = seed_user_and_tenant(&db).await;
    let read_back = cloned
        .repositories()
        .users
        .get_global(user_id)
        .await
        .expect("lookup through the clone")
        .expect("the row written through the original is visible");
    assert_eq!(read_back.id, user_id);
    assert_eq!(read_back.display_name.as_deref(), Some("Registry Tester"));
}

#[tokio::test]
async fn installing_a_keyring_reaches_every_clone_taken_before_or_after() {
    let bootstrap_key = vec![1u8; 32];
    let rotated_key = vec![2u8; 32];
    let db = create_test_db_with_key(bootstrap_key.clone())
        .await
        .expect("factory opens a database under a caller key");
    let (user_id, tenant_id) = seed_user_and_tenant(&db).await;

    // A handle taken before the install shares the one keyring cell.
    let before = db.clone();
    let ring = Keyring::new(
        BTreeMap::from([(1, bootstrap_key.clone()), (2, rotated_key)]),
        2,
        BTreeMap::from([(1, bootstrap_key)]),
        1,
        1,
        true,
    )
    .expect("a valid keyring");
    db.keyring().install(ring, Utc::now());

    assert!(
        Arc::ptr_eq(before.repositories(), db.repositories()),
        "installing a keyring keeps the one registry"
    );
    assert_eq!(before.keyring().current().data_primary(), 2);

    let token = UserOAuthToken::new(
        user_id,
        tenant_id.to_string(),
        "strava".to_owned(),
        "access-token-under-v2".to_owned(),
        Some("refresh-token-under-v2".to_owned()),
        None,
        Some("read".to_owned()),
    );
    before
        .repositories()
        .oauth_tokens
        .upsert_token(&token)
        .await
        .expect("an earlier clone seals under the installed primary");

    let stored = db
        .repositories()
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .expect("the handle opens what the earlier clone sealed")
        .expect("the token row exists");
    assert_eq!(stored.access_token.expose_secret(), "access-token-under-v2");
    assert_eq!(
        stored
            .refresh_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("refresh-token-under-v2")
    );
}
