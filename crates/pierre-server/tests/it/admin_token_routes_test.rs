// ABOUTME: The admin-token handler set on the console mount, behind an admin's session
// ABOUTME: Pins admin gating, rotation, include_inactive, revoke's 404, lifetimes and the default grant

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin tokens used to have two handler stacks: the console's in
//! `pierre-routes-web-admin` and `pierre-cli`'s in `pierre-routes-admin`
//! (carnet#604). They disagreed — the console honoured `include_inactive` and
//! answered 404 for an unknown revoke while the token API ignored the one and
//! reported success for the other, and the token API let any `ManageAdminTokens`
//! holder revoke a super-admin token or grant permissions it did not hold.
//!
//! One handler set now serves the console mount (`/api/admin/tokens`, session
//! auth). These tests drive it there and assert on what was written, so each
//! settled divergence fails on the handler that had it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;
use secrecy::ExposeSecret;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use dravr_tronc::dates::Timestamp;
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_core::admin::models::{
    AdminPermission, AdminPermissions, CreateAdminTokenRequest, GeneratedAdminToken,
};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
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
pub(crate) fn console(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::cookie_admin_routes::<ServerContext>(admin_context(resources), resources)
}

/// The admin-token mount `pierre-cli` reaches.
pub(crate) fn token_api(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::routes(admin_context(resources))
}

/// Create an active user with `role` owning a tenant; return `"Bearer <jwt>"`.
pub(crate) async fn session(
    resources: &Arc<ServerContext>,
    email: &str,
    role: UserRole,
) -> Result<String> {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Token Test".to_owned()),
    );
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Timestamp::now());
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
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
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
pub(crate) async fn mint(
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

pub(crate) fn bearer(token: &GeneratedAdminToken) -> String {
    format!("Bearer {}", token.jwt_token.expose_secret())
}

pub(crate) async fn is_active(resources: &Arc<ServerContext>, token_id: &str) -> Result<bool> {
    Ok(resources
        .common
        .repos
        .admin
        .get_token_by_id(token_id)
        .await?
        .expect("the token row exists")
        .is_active)
}

pub(crate) fn listed_ids(body: &Value) -> Vec<String> {
    body["data"]["tokens"]
        .as_array()
        .expect("data.tokens is an array")
        .iter()
        .filter_map(|t| t["id"].as_str().map(ToOwned::to_owned))
        .collect()
}

// ===========================================================================
// Console session: an admin manages every token; a user does not
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

/// An admin's console session lists every token, admin ones included, and
/// an account that is not an admin sees none.
#[tokio::test]
#[serial]
async fn an_admin_lists_every_token_and_a_user_none() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-list@example.com", UserRole::Admin).await?;
    let athlete = session(&resources, "athlete-list@example.com", UserRole::User).await?;
    let service = mint(
        &resources,
        "svc-plain",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    let super_token = mint(
        &resources,
        "svc-super",
        vec![AdminPermission::ListKeys],
        true,
    )
    .await?;

    let response = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &athlete)
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 403, "a user lists no admin token");

    let body: Value = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &admin)
        .send(console(&resources))
        .await
        .json();
    let ids = listed_ids(&body);
    assert!(
        ids.contains(&service.token_id) && ids.contains(&super_token.token_id),
        "an admin sees every token"
    );
    let entry = &body["data"]["tokens"][0];
    assert!(
        entry["permissions"].is_array() && entry["usage_count"].is_number(),
        "the console dereferences a flat permissions array and usage_count: {entry}"
    );
    Ok(())
}

/// An admin mints any token from the console: a service token, an admin
/// one, one carrying any permission. A user mints nothing.
#[tokio::test]
#[serial]
async fn an_admin_mints_any_token_and_a_user_none() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-mint@example.com", UserRole::Admin).await?;
    let athlete = session(&resources, "athlete-mint@example.com", UserRole::User).await?;

    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &athlete)
        .json(&json!({ "service_name": "athlete-grab", "permissions": ["list_keys"] }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 403, "a user mints no admin token");
    let all = resources.common.repos.admin.list_tokens(true).await?;
    assert!(all.is_empty(), "the refused request wrote nothing");

    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&json!({ "service_name": "reporting", "permissions": ["list_keys"] }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 201, "an admin mints a service token");
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
        .json(&json!({ "service_name": "full-access", "is_super_admin": true }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 201, "an admin mints a super-admin token");
    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&json!({ "service_name": "config", "permissions": ["manage_configuration"] }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 201, "an admin grants any permission");
    let all = resources.common.repos.admin.list_tokens(true).await?;
    assert_eq!(all.len(), 3, "the admin's three tokens");
    Ok(())
}

/// A super-admin token is revoked or rotated by an admin, and by no user:
/// after the user's refusals it is still active.
#[tokio::test]
#[serial]
async fn a_super_token_stays_active_until_an_admin_revokes_it() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-revoke@example.com", UserRole::Admin).await?;
    let athlete = session(&resources, "athlete-revoke@example.com", UserRole::User).await?;
    let super_token = mint(
        &resources,
        "svc-admin",
        vec![AdminPermission::ListKeys],
        true,
    )
    .await?;

    for action in ["revoke", "rotate"] {
        let response = AxumTestRequest::post(&format!(
            "/api/admin/tokens/{}/{action}",
            super_token.token_id
        ))
        .header("authorization", &athlete)
        .json(&json!({}))
        .send(console(&resources))
        .await;
        assert_eq!(response.status(), 403, "a user must not {action} a token");
    }

    // A guard that ran after the deactivation would still 403 while having
    // destroyed the admin's credential; the row is what matters.
    assert!(is_active(&resources, &super_token.token_id).await?);

    let response = AxumTestRequest::post(&format!(
        "/api/admin/tokens/{}/revoke",
        super_token.token_id
    ))
    .header("authorization", &admin)
    .send(console(&resources))
    .await;
    assert_eq!(
        response.status(),
        200,
        "an admin revokes a super-admin token"
    );
    assert!(!is_active(&resources, &super_token.token_id).await?);
    Ok(())
}

#[tokio::test]
#[serial]
async fn an_admin_rotates_a_service_token_and_the_old_one_retires() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-rotate@example.com", UserRole::Admin).await?;
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
    // Not `assert_ne!`, which would print both tokens.
    assert!(
        new_jwt != seeded.jwt_token.expose_secret(),
        "a rotation hands out a new credential"
    );
    assert!(
        !is_active(&resources, &seeded.token_id).await?,
        "the old credential is retired"
    );
    assert!(is_active(&resources, new_id).await?);
    Ok(())
}

/// A revoked token has no replacement. Rotation used to mint one from any
/// row it found, so a credential someone revoked, or one withdrawn with its
/// admin's standing, came back live at its old scope; now it answers 400 and
/// the table is as it was.
#[tokio::test]
#[serial]
async fn a_revoked_token_is_not_rotated_back_to_life() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-revoked@example.com", UserRole::Admin).await?;
    let revoked = mint(&resources, "svc-revoked", vec![], true).await?;
    resources
        .common
        .repos
        .admin
        .deactivate_token(&revoked.token_id)
        .await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", revoked.token_id))
        .header("authorization", &admin)
        .json(&json!({}))
        .send(console(&resources))
        .await;
    let (status, body) = quotable(response);
    assert_eq!(status, 400, "a revoked token must not rotate: {body}");
    assert!(body.contains("cannot be rotated"), "{body}");
    assert!(!body.contains("jwt_token"), "no credential is handed out");

    let after = resources.common.repos.admin.list_tokens(true).await?;
    assert_eq!(after.len(), before, "no replacement was minted");
    assert!(
        after
            .iter()
            .all(|token| token.service_name != "svc-revoked" || !token.is_active),
        "nothing named like the revoked token is live"
    );
    Ok(())
}

// ===========================================================================
// The divergences settled on the one handler set
// ===========================================================================

/// The token API ignored `include_inactive`; the console honoured it.
#[tokio::test]
#[serial]
async fn include_inactive_lists_revoked_tokens() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "super-inactive@example.com", UserRole::Admin).await?;
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

    let body: Value = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &admin)
        .send(console(&resources))
        .await
        .json();
    let ids = listed_ids(&body);
    assert!(ids.contains(&live.token_id), "live token listed");
    assert!(
        !ids.contains(&doomed.token_id),
        "revoked token hidden by default"
    );

    let body: Value = AxumTestRequest::get("/api/admin/tokens?include_inactive=true")
        .header("authorization", &admin)
        .send(console(&resources))
        .await
        .json();
    let revoked = body["data"]["tokens"]
        .as_array()
        .expect("tokens")
        .iter()
        .find(|t| t["id"].as_str() == Some(doomed.token_id.as_str()))
        .expect("include_inactive lists the revoked token")
        .clone();
    assert_eq!(revoked["is_active"], false, "flagged inactive");
    Ok(())
}

/// The token API reported a revocation for an id that was never in the table.
#[tokio::test]
#[serial]
async fn revoking_an_unknown_token_is_404() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "super-missing@example.com", UserRole::Admin).await?;
    let missing = "admin_00000000000000000000000000000000";

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{missing}/revoke"))
        .header("authorization", &admin)
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 404);
    let body: Value = response.json();
    assert_ne!(
        body["success"], true,
        "a miss is never a successful revocation"
    );
    Ok(())
}

/// A request that names no permission is granted the default set and no
/// super-admin flag.
#[tokio::test]
#[serial]
async fn a_token_minted_with_no_list_carries_the_default_set() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-default@example.com", UserRole::Admin).await?;

    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&json!({ "service_name": "default-minted" }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json();
    let token_id = body["data"]["token_id"].as_str().expect("token id");
    let minted = resources
        .common
        .repos
        .admin
        .get_token_by_id(token_id)
        .await?
        .expect("the minted row");
    assert_eq!(
        minted.permissions,
        AdminPermissions::default_admin(),
        "the default set"
    );
    assert!(!minted.is_super_admin);
    Ok(())
}

/// The token API rotated on a fixed year whatever the request asked for.
#[tokio::test]
#[serial]
async fn rotation_honours_the_requested_lifetime() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-lifetime@example.com", UserRole::Admin).await?;
    let seeded = mint(
        &resources,
        "svc-short",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", seeded.token_id))
        .header("authorization", &admin)
        .json(&json!({ "expires_in_days": 7 }))
        .send(console(&resources))
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
    let days = (expires_at - Timestamp::now()).num_days();
    assert!((6..=7).contains(&days), "a 7-day rotation, got {days} days");
    Ok(())
}

/// A lifetime no date can hold is refused with a 400 and changes nothing:
/// no token is minted, and a rotation leaves the token it was asked to
/// replace live.
#[tokio::test]
#[serial]
async fn a_lifetime_past_any_date_is_refused_and_changes_nothing() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-forever@example.com", UserRole::Admin).await?;
    let seeded = mint(
        &resources,
        "svc-lifetime",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();
    let too_many = json!({ "service_name": "svc-forever", "expires_in_days": u64::MAX });

    let minted = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&too_many)
        .send(console(&resources))
        .await;
    let (status, answer) = quotable(minted);
    assert_eq!(status, 400, "mint: {answer}");

    let rotated = AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", seeded.token_id))
        .header("authorization", &admin)
        .json(&too_many)
        .send(console(&resources))
        .await;
    let (status, answer) = quotable(rotated);
    assert_eq!(status, 400, "rotate: {answer}");

    assert_eq!(
        resources.common.repos.admin.list_tokens(true).await?.len(),
        before,
        "nothing was minted"
    );
    assert!(
        is_active(&resources, &seeded.token_id).await?,
        "a refused rotation leaves the token live"
    );

    // The positive control: the same request with a lifetime a date holds.
    let minted = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&json!({ "service_name": "svc-month", "expires_in_days": 30 }))
        .send(console(&resources))
        .await;
    let (status, answer) = quotable(minted);
    assert_eq!(status, 201, "mint: {answer}");
    Ok(())
}

/// A token named like a device login, as if another admin approved it, is
/// refused and not minted.
#[tokio::test]
#[serial]
async fn a_token_named_like_a_device_login_is_refused() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-prefix@example.com", UserRole::Admin).await?;

    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&json!({
            "service_name": "device-cli:someone-else@example.com",
            "is_super_admin": false,
        }))
        .send(console(&resources))
        .await;
    let (status, body) = quotable(response);
    assert_eq!(status, 400, "{body}");
    let listed = resources.common.repos.admin.list_tokens(true).await?;
    assert!(
        listed
            .iter()
            .all(|token| !token.service_name.starts_with("device-cli:")),
        "the refused name was not minted"
    );
    Ok(())
}

/// A super-admin token minted with a lifetime of zero days never expires,
/// and the answer says what was minted.
#[tokio::test]
#[serial]
async fn a_super_admin_token_minted_for_zero_days_never_expires() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(
        &resources,
        "admin-forever-super@example.com",
        UserRole::Admin,
    )
    .await?;

    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&json!({
            "service_name": "super_admin_service",
            "service_description": "Super admin service for testing",
            "is_super_admin": true,
            "expires_in_days": 0
        }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json();
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["service_name"], "super_admin_service");
    assert_eq!(body["data"]["is_super_admin"], true);
    assert!(
        body["data"]["expires_at"].is_null(),
        "{}",
        body["data"]["expires_at"]
    );
    Ok(())
}

/// A permission the server does not know is refused with a 400 naming it as
/// invalid, and nothing is minted.
#[tokio::test]
#[serial]
async fn an_unknown_permission_is_refused() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-unknown@example.com", UserRole::Admin).await?;

    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &admin)
        .json(&json!({
            "service_name": "invalid_service",
            "permissions": ["invalid_permission"]
        }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json();
    assert_eq!(body["success"], false);
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("Invalid permission")),
        "{body}"
    );
    assert!(resources
        .common
        .repos
        .admin
        .list_tokens(true)
        .await?
        .is_empty());
    Ok(())
}

/// A super-admin token rotates, and its replacement keeps the privilege
/// level.
#[tokio::test]
#[serial]
async fn a_rotated_super_admin_token_stays_super_admin() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(
        &resources,
        "admin-rotate-super@example.com",
        UserRole::Admin,
    )
    .await?;
    let target = mint(&resources, "rotatable_super_admin", vec![], true).await?;

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", target.token_id))
        .header("authorization", &admin)
        .json(&json!({ "expires_in_days": 60 }))
        .send(console(&resources))
        .await;
    let (status, body) = quotable(response);
    assert_eq!(status, 200, "{body}");
    let rotated: Value = serde_json::from_str(&body)?;
    assert_eq!(rotated["success"], true);
    assert_eq!(rotated["data"]["old_token_id"], target.token_id);
    assert_eq!(rotated["data"]["is_super_admin"], true);
    let new_id = rotated["data"]["token_id"].as_str().expect("new token id");
    let stored = resources
        .common
        .repos
        .admin
        .get_token_by_id(new_id)
        .await?
        .expect("the replacement row");
    assert!(
        stored.is_super_admin,
        "the stored replacement is super-admin"
    );
    Ok(())
}

/// Mint an admin token bound to `tenant` straight into the table, as
/// `pierre-cli token generate --tenant-id` does.
pub(crate) async fn mint_bound(
    resources: &Arc<ServerContext>,
    service: &str,
    permissions: Vec<AdminPermission>,
    tenant: TenantId,
) -> Result<GeneratedAdminToken> {
    let request = CreateAdminTokenRequest {
        service_name: service.to_owned(),
        service_description: Some("seeded by admin_token_routes_test".to_owned()),
        permissions: Some(permissions),
        expires_in_days: Some(90),
        is_super_admin: false,
        tenant_id: Some(tenant.to_string()),
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

/// What stands where an answer carried a credential.
const TOKEN_NOT_QUOTED: &str = "[a minted token, not quoted]";

/// Where an admin answer carries a credential it hands out: a minted or
/// rotated admin token, a password reset token, a sign-up code.
const HANDED_OUT: [&str; 3] = ["/data/jwt_token", "/data/reset_token", "/data/code"];

/// Replace each credential an answer hands out ([`HANDED_OUT`]). Its key
/// stays, so an answer that handed a credential out still shows that it
/// did.
fn unquote_minted_token(answer: &mut Value) {
    for place in HANDED_OUT {
        if let Some(credential) = answer.pointer_mut(place) {
            *credential = json!(TOKEN_NOT_QUOTED);
        }
    }
}

/// The status and the answer of `response`, fit to quote in a failed
/// assertion: an answer that mints carries the credential, on the very
/// failure where a mint, a rotation or a reset that should have been
/// refused went through, and it is replaced before the answer is returned
/// ([`unquote_minted_token`]). An answer that is not JSON carries none and
/// is returned as it is.
pub(crate) fn quotable(response: AxumTestResponse) -> (u16, String) {
    let status = response.status();
    let text = response.text();
    let Ok(mut answer) = serde_json::from_str::<Value>(&text) else {
        return (status, text);
    };
    unquote_minted_token(&mut answer);
    (status, answer.to_string())
}
