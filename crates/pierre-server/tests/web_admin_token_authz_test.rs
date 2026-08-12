// ABOUTME: Regression tests for super-admin gating on the web-admin /api/admin/tokens surface
// ABOUTME: A plain admin must not list, read or revoke admin tokens — revoking one is privilege escalation

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::models::{AdminPermission, CreateAdminTokenRequest};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_web_admin::WebAdminRoutes;
use serial_test::serial;
use std::sync::Arc;

/// `authenticate_admin` on this surface only proved *an* admin role, so every
/// handler below was reachable by a tenant-scoped `admin`. Listing leaks the
/// service inventory, and revoking a super-admin service token is a denial of
/// service against the platform's own operators — privilege escalation by
/// deletion. The programmatic twin (`pierre-routes-admin`) has always gated
/// this; the cookie-auth surface had drifted, which is exactly the "two systems
/// doing the same job" failure these tests exist to pin.
const TEST_JWT_SECRET: &str = "test_jwt_secret_for_web_admin_token_authz";

/// A router is consumed by each `send`, so build one per request.
fn router(resources: &Arc<ServerContext>) -> axum::Router {
    WebAdminRoutes::routes(resources.web_admin_context())
}

/// Create an active user with `role`, give it a tenant, return `"Bearer <jwt>"`.
async fn user_with_role(
    resources: &Arc<ServerContext>,
    email: &str,
    role: UserRole,
) -> Result<String> {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Authz Test".to_owned()),
    );
    user.is_admin = role.is_admin_or_higher();
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    resources.common.repos.users.create(&user).await?;

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant for {email}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    resources.common.repos.tenants.create(&tenant).await?;
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await?;

    let token = generate_test_token(resources, &user).await;
    Ok(format!("Bearer {token}"))
}

/// Seed a super-admin service token and return its id.
async fn seed_super_admin_token(resources: &Arc<ServerContext>, service: &str) -> Result<String> {
    let request = CreateAdminTokenRequest {
        service_name: service.to_owned(),
        service_description: Some("seeded by web_admin_token_authz_test".to_owned()),
        permissions: Some(vec![AdminPermission::ListKeys]),
        expires_in_days: None,
        is_super_admin: true,
        tenant_id: None,
    };
    let generated = resources
        .common
        .repos
        .admin
        .create_token(&request, TEST_JWT_SECRET, &resources.auth.jwks_manager)
        .await?;
    Ok(generated.token_id)
}

#[tokio::test]
#[serial]
async fn plain_admin_cannot_list_admin_tokens() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = user_with_role(&resources, "plain-admin-list@example.com", UserRole::Admin).await?;

    let response = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &admin)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        403,
        "a plain admin must not enumerate admin tokens"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn super_admin_can_list_admin_tokens() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let super_admin =
        user_with_role(&resources, "super-list@example.com", UserRole::SuperAdmin).await?;

    let response = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &super_admin)
        .send(router(&resources))
        .await;

    // The gate must deny the right people without denying the right people —
    // a 403-for-everyone "fix" would pass the test above and break the console.
    assert_eq!(
        response.status(),
        200,
        "a super-admin must still be able to list admin tokens"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn plain_admin_cannot_read_an_admin_token() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = user_with_role(&resources, "plain-admin-get@example.com", UserRole::Admin).await?;
    let token_id = seed_super_admin_token(&resources, "svc-read-target").await?;

    let response = AxumTestRequest::get(&format!("/api/admin/tokens/{token_id}"))
        .header("authorization", &admin)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        403,
        "a plain admin must not read an admin token's details"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn plain_admin_cannot_revoke_a_super_admin_token_and_it_stays_active() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = user_with_role(
        &resources,
        "plain-admin-revoke@example.com",
        UserRole::Admin,
    )
    .await?;
    let token_id = seed_super_admin_token(&resources, "svc-revoke-target").await?;

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{token_id}/revoke"))
        .header("authorization", &admin)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        403,
        "a plain admin must not revoke an admin token"
    );

    // The status code alone is not the security property. A handler could 403
    // *after* deactivating, or a future refactor could reorder the guard — what
    // actually matters is that the operator's token still works afterwards.
    let token = resources
        .common
        .repos
        .admin
        .get_token_by_id(&token_id)
        .await?
        .expect("seeded super-admin token must still exist");

    assert!(
        token.is_active,
        "the super-admin service token must remain ACTIVE after a plain admin's revoke attempt"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn unauthenticated_requests_are_rejected() -> Result<()> {
    let resources = create_test_server_resources().await?;

    let response = AxumTestRequest::get("/api/admin/tokens")
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        401,
        "an unauthenticated request must be rejected before any authz decision"
    );
    Ok(())
}
