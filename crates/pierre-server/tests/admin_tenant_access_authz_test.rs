// ABOUTME: Regression tests for cross-tenant authz on web-admin financial endpoints
// ABOUTME: A tenant-scoped admin must not read another tenant's LLM usage/invoice/billing (CWE-863)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

mod common;
mod helpers;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{ConversationTurnId, InsertLlmUsage, Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_web_admin::WebAdminRoutes;
use serde_json::Value;
use serial_test::serial;
use std::sync::Arc;
use uuid::Uuid;

/// Build a fresh `WebAdminRoutes` router backed by the given server resources.
/// A router is consumed by each `send`, so callers build one per request.
fn router(resources: &Arc<ServerContext>) -> axum::Router {
    WebAdminRoutes::routes(resources.web_admin_context())
}

/// Create a user with the given role, provision a dedicated tenant, associate
/// the two, and return `(user_id, tenant_id, "Bearer <jwt>")`.
async fn create_user_with_role(
    resources: &Arc<ServerContext>,
    email: &str,
    role: UserRole,
) -> (Uuid, TenantId, String) {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap();

    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Test User".to_owned()),
    );
    user.is_admin = role.is_admin_or_higher();
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::new();
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
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();

    let token = generate_test_token(resources, &user).await;
    (user_id, tenant_id, format!("Bearer {token}"))
}

/// Seed `count` LLM usage rows for the given tenant/user.
async fn seed_usage(resources: &Arc<ServerContext>, tenant_id: &str, user_id: &str, count: usize) {
    for i in 0..count {
        let offset = i as i64;
        let params = InsertLlmUsage {
            tenant_id,
            user_id,
            conversation_id: None,
            turn_id: ConversationTurnId::new(),
            provider: "openai",
            model: "gpt-4o-mini",
            prompt_tokens: 100 + (offset * 10),
            completion_tokens: 50 + (offset * 5),
            total_tokens: 150 + (offset * 15),
            cached_tokens: 0,
            call_type: "chat",
            tool_calls_count: 0,
            tools_called: "[]",
            execution_time_ms: Some(200),
            cost_usd: 0.0,
            call_sequence: None,
        };
        resources
            .common
            .repos
            .llm_usage
            .insert_llm_usage(&params)
            .await
            .unwrap();
    }
}

// ============================================================================
// handle_get_tenant_usage — /api/admin/tenants/{tenant_id}/usage
// ============================================================================

/// A tenant-scoped admin of tenant A must be denied usage for tenant B (403).
#[tokio::test]
#[serial]
async fn test_tenant_admin_denied_cross_tenant_usage() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_a_id, _a_tenant, a_token) =
        create_user_with_role(&resources, "authz_usage_admin_a@test.com", UserRole::Admin).await;
    let (b_id, b_tenant, _b_token) =
        create_user_with_role(&resources, "authz_usage_admin_b@test.com", UserRole::Admin).await;
    seed_usage(&resources, &b_tenant.to_string(), &b_id.to_string(), 3).await;

    let response = AxumTestRequest::get(&format!("/api/admin/tenants/{b_tenant}/usage"))
        .header("authorization", &a_token)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        403,
        "tenant-scoped admin A must be denied tenant B usage"
    );
    Ok(())
}

/// A super-admin (global operator) is allowed usage for any tenant (200).
#[tokio::test]
#[serial]
async fn test_super_admin_allowed_cross_tenant_usage() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_s_id, _s_tenant, s_token) = create_user_with_role(
        &resources,
        "authz_usage_super@test.com",
        UserRole::SuperAdmin,
    )
    .await;
    let (b_id, b_tenant, _b_token) =
        create_user_with_role(&resources, "authz_usage_admin_b2@test.com", UserRole::Admin).await;
    seed_usage(&resources, &b_tenant.to_string(), &b_id.to_string(), 2).await;

    let response = AxumTestRequest::get(&format!("/api/admin/tenants/{b_tenant}/usage"))
        .header("authorization", &s_token)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        200,
        "super-admin must be allowed cross-tenant usage"
    );
    let body: Value = response.json();
    let expected_tenant = b_tenant.to_string();
    assert_eq!(
        body["tenant_id"].as_str(),
        Some(expected_tenant.as_str()),
        "response should echo the requested tenant"
    );
    Ok(())
}

/// The fix must not break legitimate same-tenant access: an admin reading their
/// own tenant's usage still succeeds (200).
#[tokio::test]
#[serial]
async fn test_tenant_admin_allowed_own_tenant_usage() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (a_id, a_tenant, a_token) =
        create_user_with_role(&resources, "authz_usage_own@test.com", UserRole::Admin).await;
    seed_usage(&resources, &a_tenant.to_string(), &a_id.to_string(), 2).await;

    let response = AxumTestRequest::get(&format!("/api/admin/tenants/{a_tenant}/usage"))
        .header("authorization", &a_token)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        200,
        "admin must still read their own tenant's usage"
    );
    Ok(())
}

// ============================================================================
// handle_get_tenant_invoice — /api/admin/tenants/{tenant_id}/invoice
// ============================================================================

/// A tenant-scoped admin of tenant A must be denied invoice for tenant B (403).
#[tokio::test]
#[serial]
async fn test_tenant_admin_denied_cross_tenant_invoice() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_a_id, _a_tenant, a_token) = create_user_with_role(
        &resources,
        "authz_invoice_admin_a@test.com",
        UserRole::Admin,
    )
    .await;
    let (_b_id, b_tenant, _b_token) = create_user_with_role(
        &resources,
        "authz_invoice_admin_b@test.com",
        UserRole::Admin,
    )
    .await;

    let response = AxumTestRequest::get(&format!(
        "/api/admin/tenants/{b_tenant}/invoice?period=2026-07"
    ))
    .header("authorization", &a_token)
    .send(router(&resources))
    .await;

    assert_eq!(
        response.status(),
        403,
        "tenant-scoped admin must be denied a cross-tenant invoice"
    );
    Ok(())
}

// ============================================================================
// handle_export_billing — /api/admin/billing/export (platform-wide)
// ============================================================================

/// The platform-wide billing export must be super-admin-only: a regular admin
/// is denied (403).
#[tokio::test]
#[serial]
async fn test_billing_export_denied_for_tenant_admin() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_a_id, _a_tenant, a_token) =
        create_user_with_role(&resources, "authz_export_admin@test.com", UserRole::Admin).await;

    let response = AxumTestRequest::get("/api/admin/billing/export?period=2026-07&format=json")
        .header("authorization", &a_token)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        403,
        "platform-wide billing export must be super-admin only"
    );
    Ok(())
}

/// A super-admin is allowed the platform-wide billing export (200).
#[tokio::test]
#[serial]
async fn test_billing_export_allowed_for_super_admin() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_s_id, _s_tenant, s_token) = create_user_with_role(
        &resources,
        "authz_export_super@test.com",
        UserRole::SuperAdmin,
    )
    .await;

    let response = AxumTestRequest::get("/api/admin/billing/export?period=2026-07&format=json")
        .header("authorization", &s_token)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        200,
        "super-admin must be allowed the platform-wide billing export"
    );
    Ok(())
}

// ============================================================================
// handle_get_user_usage — /api/admin/users/{user_id}/usage (per-user)
// ============================================================================

/// A tenant-scoped admin must be denied the per-user usage of a user who
/// belongs to a different tenant (403).
#[tokio::test]
#[serial]
async fn test_tenant_admin_denied_cross_tenant_user_usage() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_a_id, _a_tenant, a_token) = create_user_with_role(
        &resources,
        "authz_user_usage_admin@test.com",
        UserRole::Admin,
    )
    .await;
    let (target_id, target_tenant, _t_token) = create_user_with_role(
        &resources,
        "authz_user_usage_target@test.com",
        UserRole::User,
    )
    .await;
    seed_usage(
        &resources,
        &target_tenant.to_string(),
        &target_id.to_string(),
        2,
    )
    .await;

    let response = AxumTestRequest::get(&format!("/api/admin/users/{target_id}/usage"))
        .header("authorization", &a_token)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        403,
        "admin must not read a foreign-tenant user's per-user usage"
    );
    Ok(())
}

/// A super-admin may read any user's per-user usage (200).
#[tokio::test]
#[serial]
async fn test_super_admin_allowed_cross_tenant_user_usage() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_s_id, _s_tenant, s_token) = create_user_with_role(
        &resources,
        "authz_user_usage_super@test.com",
        UserRole::SuperAdmin,
    )
    .await;
    let (target_id, target_tenant, _t_token) = create_user_with_role(
        &resources,
        "authz_user_usage_target2@test.com",
        UserRole::User,
    )
    .await;
    seed_usage(
        &resources,
        &target_tenant.to_string(),
        &target_id.to_string(),
        2,
    )
    .await;

    let response = AxumTestRequest::get(&format!("/api/admin/users/{target_id}/usage"))
        .header("authorization", &s_token)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        200,
        "super-admin must be allowed any user's per-user usage"
    );
    Ok(())
}

/// A tenant-scoped admin must be denied a foreign-tenant user's cost timeseries
/// (403) — the 5th endpoint in this CWE-863 class, now gated by the shared
/// `authorize_admin_for_user` helper alongside per-user usage.
#[tokio::test]
#[serial]
async fn test_tenant_admin_denied_cross_tenant_user_cost_timeseries() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_a_id, _a_tenant, a_token) = create_user_with_role(
        &resources,
        "authz_user_cost_admin@test.com",
        UserRole::Admin,
    )
    .await;
    let (target_id, target_tenant, _t_token) = create_user_with_role(
        &resources,
        "authz_user_cost_target@test.com",
        UserRole::User,
    )
    .await;
    seed_usage(
        &resources,
        &target_tenant.to_string(),
        &target_id.to_string(),
        2,
    )
    .await;

    let response = AxumTestRequest::get(&format!("/api/admin/users/{target_id}/cost-timeseries"))
        .header("authorization", &a_token)
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        403,
        "admin must not read a foreign-tenant user's cost timeseries"
    );
    Ok(())
}
