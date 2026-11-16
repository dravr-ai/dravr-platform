// ABOUTME: carnet#1589 — the paged status listing keeps the members of one tenant when it is handed one
// ABOUTME: Runs on whichever backend DATABASE_URL names: the tenant filter binds a tenant id on both engines

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{TimeDelta, Utc};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::pagination::PaginationParams;
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use pierre_test_support::server::create_test_user;
use uuid::Uuid;

/// A plain account in `status`, created `age_minutes` ago so a listing has
/// one order.
async fn account(
    repos: &RepositoryRegistry,
    email: &str,
    status: UserStatus,
    age_minutes: i64,
) -> Uuid {
    let mut user: User = create_test_user(email, None);
    user.user_status = status;
    user.created_at = Utc::now() - TimeDelta::minutes(age_minutes);
    repos.users.create(&user).await.unwrap();
    user.id
}

/// A tenant `owner` owns, which makes `owner` its first member.
async fn tenant_of(repos: &RepositoryRegistry, owner: Uuid, label: &str) -> TenantId {
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("{label} tenant"),
            slug: format!("{label}-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    tenant_id
}

/// Every id the listing of `status` names in `tenant`, page by page of two.
async fn listed(repos: &RepositoryRegistry, status: &str, tenant: Option<TenantId>) -> Vec<Uuid> {
    let mut ids = Vec::new();
    let mut cursor = None;
    loop {
        let page = repos
            .users
            .get_by_status_cursor(status, tenant, &PaginationParams::forward(cursor, 2))
            .await
            .unwrap();
        assert!(page.items.len() <= 2, "a page holds what was asked for");
        ids.extend(page.items.iter().map(|user| user.id));
        match page.next_cursor {
            Some(next) if page.has_more => cursor = Some(next),
            _ => return ids,
        }
    }
}

/// A listing handed a tenant names that tenant's members and no one else,
/// on the first page and on the ones after it, whatever the status asked
/// for; handed none, it names every account. A sign-up is a member of no
/// tenant, so no tenant's listing names it, as the read of one user in a
/// tenant does not find it.
///
/// The pages after the first are read by another statement with its own
/// tenant filter. `elder`, outside the club, is older than two of its
/// members, so the club's second page has an account to leave out: with
/// every outsider newer than every member, no later page ever met one
/// (carnet#1589, test review).
#[tokio::test]
async fn a_status_listing_handed_a_tenant_names_its_members_only() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = account(repos, "owner@example.com", UserStatus::Active, 50).await;
    let club = tenant_of(repos, owner, "club").await;
    let second = account(repos, "second@example.com", UserStatus::Active, 40).await;
    let third = account(repos, "third@example.com", UserStatus::Suspended, 30).await;
    for member in [second, third] {
        repos.users.update_tenant_id(member, club).await.unwrap();
    }
    let elder = account(repos, "elder@example.com", UserStatus::Active, 45).await;
    let other = tenant_of(repos, elder, "other").await;
    let elsewhere = account(repos, "elsewhere@example.com", UserStatus::Active, 20).await;
    repos
        .users
        .update_tenant_id(elsewhere, other)
        .await
        .unwrap();
    let waiting = account(repos, "waiting@example.com", UserStatus::Pending, 10).await;

    // Newest first, across the page boundary at two: the second page starts
    // after `second`, where `elder` is the next account by age.
    assert_eq!(
        listed(repos, "all", Some(club)).await,
        vec![third, second, owner]
    );
    assert_eq!(
        listed(repos, "active", Some(club)).await,
        vec![second, owner]
    );
    assert_eq!(
        listed(repos, "all", Some(other)).await,
        vec![elsewhere, elder]
    );
    assert_eq!(
        listed(repos, "pending", Some(club)).await,
        Vec::<Uuid>::new(),
        "a sign-up is a member of no tenant"
    );

    assert_eq!(
        listed(repos, "all", None).await,
        vec![waiting, elsewhere, third, second, elder, owner],
        "handed no tenant, the listing names every account"
    );
    assert_eq!(listed(repos, "pending", None).await, vec![waiting]);
    for id in [owner, second, third] {
        assert!(
            repos.users.get(id, club).await.unwrap().is_some(),
            "the read of one user in the tenant finds each account the listing names"
        );
    }
    assert!(repos.users.get(waiting, club).await.unwrap().is_none());
}
