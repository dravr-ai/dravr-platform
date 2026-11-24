// ABOUTME: carnet#1589 — a tenant is created with its owner's membership or not at all
// ABOUTME: Runs on whichever backend DATABASE_URL names: both engines hold a member to an existing account

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_tronc::dates::Timestamp;
use pierre_core::models::{Tenant, TenantId};
use pierre_test_support::db::create_test_db;
use pierre_test_support::server::create_test_user;
use uuid::Uuid;

/// A tenant named by `slug` that `owner` owns.
fn tenant(slug: &str, owner: Uuid) -> Tenant {
    Tenant {
        id: TenantId::generate(),
        name: "Solo tenant".to_owned(),
        slug: slug.to_owned(),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: owner,
        created_at: Timestamp::now(),
        updated_at: Timestamp::now(),
    }
}

/// A tenant is two rows: the tenant, and its owner's membership. They were
/// two statements on the pool, so a membership that could not be written
/// left a tenant no one owns holding its slug: no read of the account's
/// tenants finds it, and the next attempt to give the account its tenant
/// fails on the slug, every time. The membership fails here the way it can
/// in production, on the account it names (one deleted since it was read):
/// `tenant_users.user_id` references `users`. Nothing of the tenant is
/// left, and the slug is free for the attempt that follows.
#[tokio::test]
async fn a_tenant_whose_owner_cannot_be_recorded_is_not_created() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let slug = format!("user-{}", Uuid::new_v4());

    let no_such_account = Uuid::new_v4();
    repos
        .tenants
        .create(&tenant(&slug, no_such_account))
        .await
        .expect_err("a membership naming no account is refused");

    // Every read of a tenant joins its owner, so none can show a row left
    // without one. What shows it is the slug: it is unique, and free only
    // when the failed creation left no tenant row behind.
    let owner = create_test_user("owner-atomic@example.com", None);
    repos.users.create(&owner).await.unwrap();
    let created = tenant(&slug, owner.id);
    repos
        .tenants
        .create(&created)
        .await
        .expect("a creation that failed left no tenant holding the slug");
    let stored = repos.tenants.get_by_slug(&slug).await.unwrap();
    assert_eq!(stored.id, created.id);
    assert_eq!(stored.owner_user_id, owner.id);
    assert_eq!(
        repos
            .tenants
            .list_membership_tenant_ids(owner.id)
            .await
            .unwrap(),
        vec![created.id]
    );
}
