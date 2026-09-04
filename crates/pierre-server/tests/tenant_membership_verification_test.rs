// ABOUTME: Proves the MCP/A2A tenant-context helper refuses a user with no tenant_users row
// ABOUTME: A missing membership must be an authorization error, never a context carrying a default role
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use http::{HeaderMap, HeaderValue};
use pierre_auth::tenant::TenantRole;
use pierre_core::errors::ErrorCode;
use pierre_core::models::CoachingPersona;
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use pierre_mcp_transport::tenant_isolation::extract_tenant_context_internal;
use std::sync::Arc;
use uuid::Uuid;

/// Create a user who owns a freshly created tenant. `tenants.create` writes the
/// owner's `tenant_users` row, so the returned pair is a real membership.
async fn user_owning_a_tenant(repos: &RepositoryRegistry) -> (Uuid, TenantId) {
    let user_id = Uuid::new_v4();
    let tenant_id = TenantId::generate();
    let now = Utc::now();
    let user = User {
        id: user_id,
        email: format!("membership+{user_id}@example.com"),
        display_name: None,
        password_hash: bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: Some(user_id),
        approved_at: Some(now),
        created_at: now,
        last_active: now,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "en".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };
    repos.users.create(&user).await.unwrap();

    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant {tenant_id}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: now,
        updated_at: now,
    };
    repos.tenants.create(&tenant).await.unwrap();

    (user_id, tenant_id)
}

#[tokio::test]
async fn member_gets_the_role_recorded_in_tenant_users() {
    let db = create_test_db().await.unwrap();
    let repos = Arc::new(db.repositories());
    let (user_id, tenant_id) = user_owning_a_tenant(&repos).await;

    let ctx = extract_tenant_context_internal(&repos, Some(user_id), Some(tenant_id), None)
        .await
        .expect("a real membership resolves")
        .expect("a named tenant yields a context");

    assert_eq!(ctx.tenant_id, tenant_id);
    assert_eq!(ctx.user_id, user_id);
    assert_eq!(
        ctx.role(),
        Some(TenantRole::Owner),
        "the role must be the one stored in tenant_users, not a default"
    );
    assert!(ctx.is_admin(), "an owner is an admin of their own tenant");
}

#[tokio::test]
async fn explicit_tenant_id_without_membership_is_refused() {
    let db = create_test_db().await.unwrap();
    let repos = Arc::new(db.repositories());
    let (outsider_id, _own_tenant) = user_owning_a_tenant(&repos).await;
    let (_owner_id, foreign_tenant) = user_owning_a_tenant(&repos).await;

    let err =
        extract_tenant_context_internal(&repos, Some(outsider_id), Some(foreign_tenant), None)
            .await
            .expect_err("a user with no tenant_users row for the tenant must be refused");

    assert_eq!(
        err.code,
        ErrorCode::AuthInvalid,
        "non-membership is an authorization failure"
    );
    assert!(
        err.message.contains(&outsider_id.to_string())
            && err.message.contains(&foreign_tenant.to_string()),
        "the refusal must name the user and the tenant, got: {}",
        err.message
    );
}

#[tokio::test]
async fn x_tenant_id_header_without_membership_is_refused() {
    let db = create_test_db().await.unwrap();
    let repos = Arc::new(db.repositories());
    let (outsider_id, _own_tenant) = user_owning_a_tenant(&repos).await;
    let (_owner_id, foreign_tenant) = user_owning_a_tenant(&repos).await;

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-tenant-id",
        HeaderValue::from_str(&foreign_tenant.to_string()).unwrap(),
    );

    let err = extract_tenant_context_internal(&repos, Some(outsider_id), None, Some(&headers))
        .await
        .expect_err("a header naming a foreign tenant must be refused too");

    assert_eq!(err.code, ErrorCode::AuthInvalid);
    assert!(
        err.message.contains(&foreign_tenant.to_string()),
        "the header branch must apply the same membership rule, got: {}",
        err.message
    );
}

#[tokio::test]
async fn a_userless_call_yields_a_context_with_no_role() {
    let db = create_test_db().await.unwrap();
    let repos = Arc::new(db.repositories());
    let (_owner_id, tenant_id) = user_owning_a_tenant(&repos).await;

    let ctx = extract_tenant_context_internal(&repos, None, Some(tenant_id), None)
        .await
        .expect("naming a tenant without a user is a scoped operation")
        .expect("a named tenant yields a context");

    assert_eq!(ctx.tenant_id, tenant_id);
    assert_eq!(
        ctx.role(),
        None,
        "no membership was looked up, so no role may be claimed"
    );
    assert!(!ctx.is_admin());
}
