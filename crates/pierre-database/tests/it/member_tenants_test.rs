// ABOUTME: The one definition of an athlete's tenants: every membership row, and the tenant their user row names
// ABOUTME: Holds TenantRepository::list_membership_tenant_ids to it, the set the export and the operator tools read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

async fn fresh_user(repos: &RepositoryRegistry, label: &str) -> User {
    let user = User::new(
        format!("{label}-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some(label.to_owned()),
    );
    repos.users.create(&user).await.unwrap();
    user
}

async fn tenant_of(repos: &RepositoryRegistry, owner: Uuid) -> TenantId {
    let tenant = Tenant::new(
        "Member Tenant".to_owned(),
        format!("member-tenant-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        owner,
    );
    repos.tenants.create(&tenant).await.unwrap();
    tenant.id
}

/// Point `user_id`'s user row at `tenant_id` without a membership row: what
/// an account created before memberships carries.
async fn home_tenant_without_membership(db: &Database, user_id: Uuid, tenant_id: TenantId) {
    const SQL: &str = "UPDATE users SET tenant_id = $1 WHERE id = $2";
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(SQL)
                .bind(tenant_id.to_string())
                .bind(user_id.to_string())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(SQL)
                .bind(tenant_id.to_string())
                .bind(user_id)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn an_athletes_tenants_are_every_membership_and_the_home_tenant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = fresh_user(repos, "owner").await;
    let athlete = fresh_user(repos, "athlete").await;
    let stranger = fresh_user(repos, "stranger").await;
    let home = tenant_of(repos, owner.id).await;
    let member = tenant_of(repos, athlete.id).await;
    let unrelated = tenant_of(repos, stranger.id).await;
    home_tenant_without_membership(&db, athlete.id, home).await;

    let mut expected = vec![home, member];
    expected.sort_by_key(ToString::to_string);
    assert_eq!(
        repos
            .tenants
            .list_membership_tenant_ids(athlete.id)
            .await
            .unwrap(),
        expected,
        "bug: the athlete's tenants are their membership and their user row's tenant, in id order"
    );
    assert!(
        !repos
            .tenants
            .list_membership_tenant_ids(athlete.id)
            .await
            .unwrap()
            .contains(&unrelated),
        "bug: a tenant the athlete neither joined nor is homed in is not theirs"
    );
    assert_eq!(
        repos
            .tenants
            .list_membership_tenant_ids(owner.id)
            .await
            .unwrap(),
        vec![home],
        "bug: a tenant named by both a membership and the user row is listed once"
    );
}
