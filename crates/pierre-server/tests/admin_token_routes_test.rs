// ABOUTME: The one admin-token handler set over both mounts — console session and admin token
// ABOUTME: Pins super-admin gating, rotation, include_inactive, revoke's 404, and grant limits

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin tokens used to have two handler stacks: the console's in
//! `pierre-routes-web-admin` and `pierre-cli`'s in `pierre-routes-admin`
//! (carnet#604). They disagreed — the console honoured `include_inactive` and
//! answered 404 for an unknown revoke while the token API ignored the one and
//! reported success for the other, and the token API let any `ManageAdminTokens`
//! holder revoke a super-admin token or grant permissions it did not hold.
//!
//! One handler set now serves both mounts. These tests drive it through the
//! console mount (`/api/admin/tokens`, session auth) and the token mount
//! (`/admin/tokens`, admin-token auth) and assert on what was written, so each
//! settled divergence fails on the handler that had it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::models::{AdminPermission, CreateAdminTokenRequest, GeneratedAdminToken};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use serde_json::{json, Value};
use serial_test::serial;
use std::sync::Arc;

/// The admin context the composition root builds from a `ServerContext`.
fn admin_context(resources: &Arc<ServerContext>) -> AdminApiContext {
    AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: resources.auth.admin_jwt_secret.to_string(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
        harness_config_registry: resources.fitness.harness_config_registry.clone(),
        guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
        prompt_registry: resources.mcp.prompt_registry.clone(),
        tool_description_registry: resources.mcp.tool_description_registry.clone(),
        evidence_registry: resources.mcp.evidence_registry.clone(),
        messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
        cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
        persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
        training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
        contremaitre_config: None,
        app_behavior: resources.common.config.app_behavior.clone(),
    })
}

/// The console mount. A router is consumed by each `send`.
fn console(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::cookie_admin_routes::<ServerContext>(admin_context(resources), resources)
}

/// The admin-token mount `pierre-cli` reaches.
fn token_api(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::routes(admin_context(resources))
}

/// Create an active user with `role` owning a tenant; return `"Bearer <jwt>"`.
async fn session(resources: &Arc<ServerContext>, email: &str, role: UserRole) -> Result<String> {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Token Test".to_owned()),
    );
    user.is_admin = role.is_admin_or_higher();
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    let user_id = user.id;
    resources.common.repos.users.create(&user).await?;

    let tenant_id = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("Tenant for {email}"),
            slug: format!("tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await?;
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await?;

    Ok(format!(
        "Bearer {}",
        generate_test_token(resources, &user).await
    ))
}

/// Mint an admin token straight into the table.
async fn mint(
    resources: &Arc<ServerContext>,
    service: &str,
    permissions: Vec<AdminPermission>,
    is_super_admin: bool,
) -> Result<GeneratedAdminToken> {
    let request = CreateAdminTokenRequest {
        service_name: service.to_owned(),
        service_description: Some("seeded by admin_token_routes_test".to_owned()),
        permissions: Some(permissions),
        expires_in_days: Some(90),
        is_super_admin,
        tenant_id: None,
        operator_user_id: None,
    };
    Ok(resources
        .common
        .repos
        .admin
        .create_token(
            &request,
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await?)
}

fn bearer(token: &GeneratedAdminToken) -> String {
    format!("Bearer {}", token.jwt_token)
}

async fn is_active(resources: &Arc<ServerContext>, token_id: &str) -> Result<bool> {
    Ok(resources
        .common
        .repos
        .admin
        .get_token_by_id(token_id)
        .await?
        .expect("the token row exists")
        .is_active)
}

fn listed_ids(body: &Value) -> Vec<String> {
    body["data"]["tokens"]
        .as_array()
        .expect("data.tokens is an array")
        .iter()
        .filter_map(|t| t["id"].as_str().map(ToOwned::to_owned))
        .collect()
}

// ===========================================================================
// Console session: a plain Admin keeps non-super token management
// ===========================================================================

#[tokio::test]
#[serial]
async fn unauthenticated_console_requests_are_rejected() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let response = AxumTestRequest::get("/api/admin/tokens")
        .send(console(&resources))
        .await;
    assert_eq!(
        response.status(),
        401,
        "the route exists and refuses no session"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_plain_admin_lists_only_non_super_tokens() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "plain-list@example.com", UserRole::Admin).await?;
    let superadmin = session(&resources, "super-list@example.com", UserRole::SuperAdmin).await?;
    let service = mint(
        &resources,
        "svc-plain",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    let operator = mint(
        &resources,
        "svc-super",
        vec![AdminPermission::ListKeys],
        true,
    )
    .await?;

    let body: Value = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &admin)
        .send(console(&resources))
        .await
        .json();
    let ids = listed_ids(&body);
    assert!(
        ids.contains(&service.token_id),
        "a plain admin sees service tokens"
    );
    assert!(
        !ids.contains(&operator.token_id),
        "a super-admin token is not shown to a plain admin"
    );
    let entry = &body["data"]["tokens"][0];
    assert!(
        entry["permissions"].is_array() && entry["usage_count"].is_number(),
        "the console dereferences a flat permissions array and usage_count: {entry}"
    );

    let body: Value = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &superadmin)
        .send(console(&resources))
        .await
        .json();
    let ids = listed_ids(&body);
    assert!(ids.contains(&service.token_id) && ids.contains(&operator.token_id));
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_plain_admin_mints_a_service_token_but_not_a_super_one() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "plain-mint@example.com", UserRole::Admin).await?;

    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&json!({ "service_name": "reporting", "permissions": ["list_keys"] }))
        .send(console(&resources))
        .await;
    assert_eq!(
        response.status(),
        201,
        "a plain admin mints a service token"
    );
    let body: Value = response.json();
    let token_id = body["data"]["token_id"]
        .as_str()
        .expect("token id")
        .to_owned();
    assert!(body["data"]["jwt_token"]
        .as_str()
        .is_some_and(|t| !t.is_empty()));
    assert!(is_active(&resources, &token_id).await?);

    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&json!({ "service_name": "escalation", "is_super_admin": true }))
        .send(console(&resources))
        .await;
    assert_eq!(
        response.status(),
        403,
        "super-admin tokens are super-admin only"
    );

    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&json!({ "service_name": "config-grab", "permissions": ["manage_configuration"] }))
        .send(console(&resources))
        .await;
    assert_eq!(
        response.status(),
        403,
        "a plain admin cannot grant a permission it does not hold"
    );
    let all = resources.common.repos.admin.list_tokens(true).await?;
    assert_eq!(all.len(), 1, "only the permitted token was written");
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_plain_admin_cannot_revoke_or_rotate_a_super_token_and_it_stays_active() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "plain-revoke@example.com", UserRole::Admin).await?;
    let operator = mint(
        &resources,
        "svc-operator",
        vec![AdminPermission::ListKeys],
        true,
    )
    .await?;

    let response =
        AxumTestRequest::post(&format!("/api/admin/tokens/{}/revoke", operator.token_id))
            .header("authorization", &admin)
            .send(console(&resources))
            .await;
    assert_eq!(response.status(), 403);
    let response =
        AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", operator.token_id))
            .header("authorization", &admin)
            .json(&json!({}))
            .send(console(&resources))
            .await;
    assert_eq!(response.status(), 403);

    // A guard that ran after the deactivation would still 403 while having
    // destroyed the operator's credential; the row is what matters.
    assert!(is_active(&resources, &operator.token_id).await?);
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_plain_admin_rotates_a_service_token_and_the_old_one_retires() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "plain-rotate@example.com", UserRole::Admin).await?;
    let seeded = mint(
        &resources,
        "svc-rotate",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", seeded.token_id))
        .header("authorization", &admin)
        .json(&json!({}))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(body["data"]["old_token_id"], seeded.token_id);
    assert_eq!(body["data"]["service_name"], "svc-rotate");
    let new_id = body["data"]["token_id"].as_str().expect("new token id");
    assert_ne!(new_id, seeded.token_id, "rotation mints a new token");
    let new_jwt = body["data"]["jwt_token"].as_str().expect("new jwt");
    assert_ne!(new_jwt, seeded.jwt_token);
    assert!(
        !is_active(&resources, &seeded.token_id).await?,
        "the old credential is retired"
    );
    assert!(is_active(&resources, new_id).await?);
    Ok(())
}

// ===========================================================================
// The divergences settled for both mounts
// ===========================================================================

/// The token API ignored `include_inactive`; the console honoured it.
#[tokio::test]
#[serial]
async fn include_inactive_lists_revoked_tokens_on_both_mounts() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let superadmin = session(
        &resources,
        "super-inactive@example.com",
        UserRole::SuperAdmin,
    )
    .await?;
    let cli = mint(&resources, "cli-operator", vec![], true).await?;
    let live = mint(
        &resources,
        "svc-live",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    let doomed = mint(
        &resources,
        "svc-doomed",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    resources
        .common
        .repos
        .admin
        .deactivate_token(&doomed.token_id)
        .await?;

    for (label, router, auth, base) in [
        (
            "console",
            console(&resources),
            superadmin.clone(),
            "/api/admin/tokens",
        ),
        (
            "token api",
            token_api(&resources),
            bearer(&cli),
            "/admin/tokens",
        ),
    ] {
        let body: Value = AxumTestRequest::get(base)
            .header("authorization", &auth)
            .send(router.clone())
            .await
            .json();
        let ids = listed_ids(&body);
        assert!(ids.contains(&live.token_id), "{label}: live token listed");
        assert!(
            !ids.contains(&doomed.token_id),
            "{label}: revoked token hidden by default"
        );

        let body: Value = AxumTestRequest::get(&format!("{base}?include_inactive=true"))
            .header("authorization", &auth)
            .send(router)
            .await
            .json();
        let revoked = body["data"]["tokens"]
            .as_array()
            .expect("tokens")
            .iter()
            .find(|t| t["id"].as_str() == Some(doomed.token_id.as_str()))
            .unwrap_or_else(|| panic!("{label}: include_inactive lists the revoked token"))
            .clone();
        assert_eq!(revoked["is_active"], false, "{label}: flagged inactive");
    }
    Ok(())
}

/// The token API reported a revocation for an id that was never in the table.
#[tokio::test]
#[serial]
async fn revoking_an_unknown_token_is_404_on_both_mounts() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let superadmin = session(
        &resources,
        "super-missing@example.com",
        UserRole::SuperAdmin,
    )
    .await?;
    let cli = mint(&resources, "cli-missing", vec![], true).await?;
    let missing = "admin_00000000000000000000000000000000";

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{missing}/revoke"))
        .header("authorization", &superadmin)
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 404, "console");
    let response = AxumTestRequest::post(&format!("/admin/tokens/{missing}/revoke"))
        .header("authorization", &bearer(&cli))
        .send(token_api(&resources))
        .await;
    assert_eq!(response.status(), 404, "token api");
    let body: Value = response.json();
    assert_ne!(
        body["success"], true,
        "a miss is never a successful revocation"
    );
    Ok(())
}

/// A `ManageAdminTokens` token that is not super-admin could revoke a
/// super-admin token on the token API.
#[tokio::test]
#[serial]
async fn a_non_super_token_cannot_revoke_a_super_token() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let delegate = mint(
        &resources,
        "delegate",
        vec![AdminPermission::ManageAdminTokens],
        false,
    )
    .await?;
    let operator = mint(&resources, "operator", vec![], true).await?;

    let response = AxumTestRequest::post(&format!("/admin/tokens/{}/revoke", operator.token_id))
        .header("authorization", &bearer(&delegate))
        .send(token_api(&resources))
        .await;
    assert_eq!(response.status(), 403);
    assert!(
        is_active(&resources, &operator.token_id).await?,
        "the operator token still works"
    );
    Ok(())
}

/// A `ManageAdminTokens` token that is not super-admin could mint a token
/// carrying permissions it did not hold.
#[tokio::test]
#[serial]
async fn a_non_super_token_cannot_grant_what_it_does_not_hold() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let delegate = mint(
        &resources,
        "delegate-grant",
        vec![AdminPermission::ManageAdminTokens],
        false,
    )
    .await?;

    let response = AxumTestRequest::post("/admin/tokens")
        .header("authorization", &bearer(&delegate))
        .json(&json!({ "service_name": "grab", "permissions": ["manage_users"] }))
        .send(token_api(&resources))
        .await;
    assert_eq!(response.status(), 403);
    let body: Value = response.json();
    assert_eq!(
        body["message"].as_str(),
        Some("Cannot grant manage_users: the caller does not hold it")
    );
    assert_eq!(
        resources.common.repos.admin.list_tokens(true).await?.len(),
        1,
        "nothing was minted"
    );
    Ok(())
}

/// The token API rotated on a fixed year whatever the request asked for.
#[tokio::test]
#[serial]
async fn rotation_honours_the_requested_lifetime() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let cli = mint(&resources, "cli-rotate", vec![], true).await?;
    let seeded = mint(
        &resources,
        "svc-short",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;

    let response = AxumTestRequest::post(&format!("/admin/tokens/{}/rotate", seeded.token_id))
        .header("authorization", &bearer(&cli))
        .json(&json!({ "expires_in_days": 7 }))
        .send(token_api(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    let new_id = body["data"]["token_id"].as_str().expect("new token id");
    let expires_at = resources
        .common
        .repos
        .admin
        .get_token_by_id(new_id)
        .await?
        .expect("replacement persisted")
        .expires_at
        .expect("the replacement expires");
    let days = (expires_at - chrono::Utc::now()).num_days();
    assert!((6..=7).contains(&days), "a 7-day rotation, got {days} days");
    Ok(())
}
