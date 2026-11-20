// ABOUTME: The console's user management through the one admin handler set, over session auth
// ABOUTME: An admin does everything from the console; a user and a token without the super-admin flag stay refused
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The web console's user tabs, served by the admin-token handlers.
//!
//! The console used to have a second handler stack of its own in
//! `pierre-routes-web-admin`, kept in step with the admin-token one by a drift
//! gate (carnet#604). It is gone: `AdminRoutes::cookie_admin_routes` mounts the
//! same handlers `pierre-cli` reaches under `/admin`, and an admin's console
//! session carries every admin permission and the super-admin flag
//! (carnet#1567: one admin role, holding every power), so those handlers'
//! checks let it through. These tests drive the console mount the composition
//! root serves and assert on what each call wrote, and they pin the edges that
//! remain: an account that is not an admin is refused the console outright,
//! and a service token keeps the scope it was minted with, since the
//! super-admin flag on a token is not a user role.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;
use secrecy::ExposeSecret;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_config::mcp::AppBehaviorConfig;
use pierre_core::admin::models::{
    AdminPermission, AdminPermissions, CreateAdminTokenRequest, GeneratedAdminToken,
};
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use serde_json::{json, Value};
use serial_test::serial;
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

/// The admin context the composition root builds from a `ServerContext`.
fn admin_context(resources: &Arc<ServerContext>) -> AdminApiContext {
    admin_context_with(resources, resources.common.config.app_behavior.clone())
}

/// [`admin_context`] under a given app-behaviour config (auto-approval).
fn admin_context_with(
    resources: &Arc<ServerContext>,
    app_behavior: AppBehaviorConfig,
) -> AdminApiContext {
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
        app_behavior,
    })
}

/// The console mount: session auth in front of the admin handlers.
/// A router is consumed by each `send`, so build one per request.
fn router(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::cookie_admin_routes::<ServerContext>(admin_context(resources), resources)
}

/// The admin-token mount `pierre-cli` reaches.
fn token_router(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::routes(admin_context(resources))
}

/// A signed-in console session: the bearer header, the user id, its tenant.
struct Session {
    auth: String,
    user_id: Uuid,
    tenant_id: TenantId,
}

/// Create an active user with `role` owning a tenant, and sign it in.
async fn session_with_role(
    resources: &Arc<ServerContext>,
    email: &str,
    role: UserRole,
) -> Result<Session> {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Console Test".to_owned()),
    );
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
    Ok(Session {
        auth: format!("Bearer {token}"),
        user_id,
        tenant_id,
    })
}

/// Create an active user with `role`, give it a tenant, return `"Bearer <jwt>"`.
async fn user_with_role(
    resources: &Arc<ServerContext>,
    email: &str,
    role: UserRole,
) -> Result<String> {
    Ok(session_with_role(resources, email, role).await?.auth)
}

/// Seed a user with `status` and no tenant, as registration leaves one.
async fn seed_user(
    resources: &Arc<ServerContext>,
    email: &str,
    status: UserStatus,
) -> Result<Uuid> {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(email.to_owned(), password_hash, Some("Athlete".to_owned()));
    user.user_status = status;
    let user_id = user.id;
    resources.common.repos.users.create(&user).await?;
    Ok(user_id)
}

fn entries(body: &Value) -> Vec<Value> {
    body["data"]["emails"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[tokio::test]
#[serial]
async fn admin_allows_lists_and_removes_an_address() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin_email = "console-admin@example.com";
    let auth = user_with_role(&resources, admin_email, UserRole::Admin).await?;
    let target = "invited-athlete@example.com";

    let response = AxumTestRequest::post("/api/admin/pre-approved-emails")
        .header("Authorization", &auth)
        .json(&json!({ "email": target, "note": "beta cohort" }))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200, "the console must be able to allow");
    let body: Value = response.json();
    assert_eq!(
        body["data"]["outcome"].as_str(),
        Some("recorded"),
        "an unregistered address records a standing allow: {body}"
    );

    let response = AxumTestRequest::get("/api/admin/pre-approved-emails")
        .header("Authorization", &auth)
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    let listed = entries(&body);
    assert_eq!(listed.len(), 1, "the allow must be listed: {body}");
    assert_eq!(listed[0]["email"].as_str(), Some(target));
    assert_eq!(
        listed[0]["note"].as_str(),
        Some("beta cohort"),
        "the note must survive: {body}"
    );
    assert_eq!(
        listed[0]["allowed_by_email"].as_str(),
        Some(admin_email),
        "the allow must be attributed to the signed-in admin: {body}"
    );

    let response = AxumTestRequest::delete(&format!(
        "/api/admin/pre-approved-emails/{}",
        urlencoding::encode(target)
    ))
    .header("Authorization", &auth)
    .send(router(&resources))
    .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(
        body["data"]["removed"].as_bool(),
        Some(true),
        "removal must report the deletion: {body}"
    );

    let stored = resources.common.repos.pre_approved_emails.list().await?;
    assert!(stored.is_empty(), "the row must be gone from the table");
    Ok(())
}

#[tokio::test]
#[serial]
async fn allow_promotes_a_pending_account_from_the_console() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let auth = user_with_role(&resources, "promoting-admin@example.com", UserRole::Admin).await?;

    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut pending = User::new(
        "queued-athlete@example.com".to_owned(),
        password_hash,
        Some("Queued".to_owned()),
    );
    pending.user_status = UserStatus::Pending;
    let pending_id = pending.id;
    resources.common.repos.users.create(&pending).await?;

    let response = AxumTestRequest::post("/api/admin/pre-approved-emails")
        .header("Authorization", &auth)
        .json(&json!({ "email": "queued-athlete@example.com" }))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(
        body["data"]["outcome"].as_str(),
        Some("pending_approved"),
        "allowing a queued address must approve it now: {body}"
    );

    let promoted = resources
        .common
        .repos
        .users
        .get_global(pending_id)
        .await?
        .expect("the pending user must still exist");
    assert_eq!(
        promoted.user_status,
        UserStatus::Active,
        "the queued account must be active"
    );
    assert!(
        promoted.approved_by.is_some(),
        "the promotion must be attributed to the acting admin"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_plain_user_cannot_pre_approve_anyone() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let auth = user_with_role(&resources, "just-an-athlete@example.com", UserRole::User).await?;

    let response = AxumTestRequest::post("/api/admin/pre-approved-emails")
        .header("Authorization", &auth)
        .json(&json!({ "email": "gatecrasher@example.com" }))
        .send(router(&resources))
        .await;
    assert_eq!(
        response.status(),
        403,
        "a non-admin session must not reach the allow-list"
    );

    let stored = resources.common.repos.pre_approved_emails.list().await?;
    assert!(stored.is_empty(), "nothing may have been written");
    Ok(())
}

#[tokio::test]
#[serial]
async fn an_unauthenticated_caller_is_rejected() -> Result<()> {
    let resources = create_test_server_resources().await?;

    let response = AxumTestRequest::get("/api/admin/pre-approved-emails")
        .send(router(&resources))
        .await;
    assert_eq!(
        response.status(),
        401,
        "the route must exist and reject an unauthenticated caller"
    );
    Ok(())
}

/// Approve, suspend and reset through the console mount as an admin.
///
/// Before the console was routed through the admin-token handlers, this mount
/// did not serve these paths at all, and an admin's session lacked the
/// `ManageUsers` those handlers check; each call below would have failed.
#[tokio::test]
#[serial]
async fn an_admin_approves_suspends_and_resets_through_the_one_handler() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session_with_role(
        &resources,
        "plain-console-admin@example.com",
        UserRole::Admin,
    )
    .await?;
    let target = seed_user(&resources, "queued@example.com", UserStatus::Pending).await?;

    let response = AxumTestRequest::post(&format!("/api/admin/approve-user/{target}"))
        .header("Authorization", &admin.auth)
        .json(&json!({ "reason": "known athlete" }))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200, "an admin must be able to approve");
    let body: Value = response.json();
    assert_eq!(body["data"]["user"]["user_status"].as_str(), Some("active"));
    assert_eq!(body["data"]["reason"].as_str(), Some("known athlete"));

    let approved = resources
        .common
        .repos
        .users
        .get_global(target)
        .await?
        .expect("the approved user exists");
    assert_eq!(approved.user_status, UserStatus::Active);
    assert_eq!(
        approved.approved_by,
        Some(admin.user_id),
        "the approval is attributed to the signed-in admin"
    );
    assert!(
        resources
            .common
            .repos
            .users
            .get(target, admin.tenant_id)
            .await?
            .is_some(),
        "the approved user joins the tenant the admin works in"
    );
    let tokens = resources
        .common
        .repos
        .user_mcp_tokens
        .list_tokens(target)
        .await?;
    assert_eq!(tokens.len(), 1, "approval provisions the default MCP token");

    let response = AxumTestRequest::post(&format!("/api/admin/users/{target}/reset-password"))
        .header("Authorization", &admin.auth)
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200, "an admin must be able to reset");
    let body: Value = response.json();
    assert_eq!(body["data"]["email"].as_str(), Some("queued@example.com"));
    assert!(
        body["data"]["reset_token"]
            .as_str()
            .is_some_and(|token| token.contains('.')),
        "a selector.verifier reset token is returned once: {body}"
    );
    assert_eq!(
        body["data"]["reset_by"].as_str(),
        Some(admin.user_id.to_string().as_str()),
        "the reset is attributed to the signed-in admin"
    );

    let response = AxumTestRequest::post(&format!("/api/admin/suspend-user/{target}"))
        .header("Authorization", &admin.auth)
        .json(&json!({}))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200, "an admin must be able to suspend");
    let suspended = resources
        .common
        .repos
        .users
        .get_global(target)
        .await?
        .expect("the suspended user exists");
    assert_eq!(suspended.user_status, UserStatus::Suspended);
    Ok(())
}

/// A service token without the super-admin flag, carrying `permissions` and
/// bound to `tenant`: the scope a machine credential keeps (the fold of
/// carnet#1567 was of the user role, never of this flag).
async fn scoped_token(
    resources: &Arc<ServerContext>,
    service: &str,
    permissions: AdminPermissions,
    tenant: Option<TenantId>,
) -> Result<String> {
    let request = CreateAdminTokenRequest {
        service_name: service.to_owned(),
        service_description: None,
        permissions: Some(permissions.to_vec()),
        expires_in_days: Some(1),
        is_super_admin: false,
        tenant_id: tenant.map(|t| t.to_string()),
        operator_user_id: None,
    };
    let generated = resources
        .common
        .repos
        .admin
        .create_token(
            &request,
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await?;
    Ok(format!("Bearer {}", generated.jwt_token.expose_secret()))
}

/// `default_admin` plus `ManageUsers`: what a scoped user-management token
/// carries.
fn manage_users_permissions() -> AdminPermissions {
    let mut permissions = AdminPermissions::default_admin();
    permissions.add_permission(AdminPermission::ManageUsers);
    permissions
}

/// An admin's reset reaches a user in any tenant. A token bound to one tenant
/// reaches only that tenant's users: a user elsewhere answers 404, exactly
/// like a missing one. An account that is not an admin reaches no one.
#[tokio::test]
#[serial]
async fn a_user_outside_the_tenant_is_reset_by_an_admin_and_not_by_a_scoped_token() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session_with_role(&resources, "scoped-admin@example.com", UserRole::Admin).await?;
    let elsewhere =
        session_with_role(&resources, "other-tenant@example.com", UserRole::User).await?;

    let scoped = scoped_token(
        &resources,
        "tenant-bound-reset",
        manage_users_permissions(),
        Some(admin.tenant_id),
    )
    .await?;
    let response = AxumTestRequest::post(&format!(
        "/admin/users/{}/reset-password",
        elsewhere.user_id
    ))
    .header("Authorization", &scoped)
    .send(token_router(&resources))
    .await;
    assert_eq!(
        response.status(),
        404,
        "a user in another tenant is not reachable by a token bound to one"
    );

    let outsider =
        session_with_role(&resources, "not-an-admin@example.com", UserRole::User).await?;
    let response = AxumTestRequest::post(&format!(
        "/api/admin/users/{}/reset-password",
        elsewhere.user_id
    ))
    .header("Authorization", &outsider.auth)
    .send(router(&resources))
    .await;
    assert_eq!(response.status(), 403, "a user resets no one's password");

    let response = AxumTestRequest::post(&format!(
        "/api/admin/users/{}/reset-password",
        elsewhere.user_id
    ))
    .header("Authorization", &admin.auth)
    .send(router(&resources))
    .await;
    assert_eq!(
        response.status(),
        200,
        "an admin reaches a user in any tenant"
    );
    Ok(())
}

/// A tier change is an admin's: a user is refused the console, a token with
/// `ManageUsers` but without the super-admin flag is refused by the handler,
/// and neither writes anything.
#[tokio::test]
#[serial]
async fn a_tier_is_changed_by_an_admin_and_by_no_user_or_scoped_token() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let target = seed_user(&resources, "starter@example.com", UserStatus::Active).await?;

    let athlete =
        session_with_role(&resources, "tierless-athlete@example.com", UserRole::User).await?;
    let response = AxumTestRequest::post(&format!("/api/admin/users/{target}/tier"))
        .header("Authorization", &athlete.auth)
        .json(&json!({ "tier": "enterprise" }))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 403, "a user changes no tier");

    let scoped = scoped_token(
        &resources,
        "tierless-token",
        manage_users_permissions(),
        None,
    )
    .await?;
    let response = AxumTestRequest::post(&format!("/admin/users/{target}/tier"))
        .header("Authorization", &scoped)
        .json(&json!({ "tier": "enterprise" }))
        .send(token_router(&resources))
        .await;
    assert_eq!(
        response.status(),
        403,
        "ManageUsers is not enough: a tier change needs the super-admin flag"
    );
    let body: Value = response.json();
    assert_eq!(
        body["message"].as_str(),
        Some("Permission denied: super-admin token required")
    );
    let unchanged = resources
        .common
        .repos
        .users
        .get_global(target)
        .await?
        .expect("the target exists");
    assert_eq!(
        unchanged.tier,
        UserTier::Starter,
        "nothing may have been written"
    );

    let admin = session_with_role(&resources, "tier-admin@example.com", UserRole::Admin).await?;
    let response = AxumTestRequest::post(&format!("/api/admin/users/{target}/tier"))
        .header("Authorization", &admin.auth)
        .json(&json!({ "tier": "enterprise" }))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200, "an admin session changes a tier");
    let upgraded = resources
        .common
        .repos
        .users
        .get_global(target)
        .await?
        .expect("the target exists");
    assert_eq!(upgraded.tier, UserTier::Enterprise);
    Ok(())
}

/// The console grant does not leak into tokens: an admin token minted with the
/// default permission set, which has no `ManageUsers`, is still refused.
#[tokio::test]
#[serial]
async fn an_admin_token_without_manage_users_is_still_refused() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let target = seed_user(&resources, "still-queued@example.com", UserStatus::Pending).await?;

    let request = CreateAdminTokenRequest {
        service_name: "plain-cli-token".to_owned(),
        service_description: None,
        permissions: Some(AdminPermissions::default_admin().to_vec()),
        expires_in_days: Some(1),
        is_super_admin: false,
        tenant_id: None,
        operator_user_id: None,
    };
    let generated = resources
        .common
        .repos
        .admin
        .create_token(
            &request,
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await?;

    let response = AxumTestRequest::post(&format!("/admin/approve-user/{target}"))
        .header(
            "Authorization",
            &format!("Bearer {}", generated.jwt_token.expose_secret()),
        )
        .json(&json!({}))
        .send(token_router(&resources))
        .await;
    assert_eq!(response.status(), 403, "ManageUsers is required on a token");
    let body: Value = response.json();
    assert_eq!(
        body["message"].as_str(),
        Some("Permission denied: ManageUsers required")
    );
    let untouched = resources
        .common
        .repos
        .users
        .get_global(target)
        .await?
        .expect("the target exists");
    assert_eq!(
        untouched.user_status,
        UserStatus::Pending,
        "nothing was approved"
    );
    Ok(())
}

/// Mint a super-admin service token for the admin-token mount.
async fn super_token(resources: &Arc<ServerContext>, service: &str) -> Result<GeneratedAdminToken> {
    let request = CreateAdminTokenRequest {
        service_name: service.to_owned(),
        service_description: None,
        permissions: None,
        expires_in_days: Some(1),
        is_super_admin: true,
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

/// The listing pages one status at a time and, with `status=all`, every
/// account. The token API refused `all` as an invalid status, and the console
/// only ever had an unpaged dump of everyone.
#[tokio::test]
#[serial]
async fn the_user_listing_pages_every_status_on_both_mounts() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin =
        session_with_role(&resources, "directory-admin@example.com", UserRole::Admin).await?;
    let cli = super_token(&resources, "cli-directory").await?;
    let pending = seed_user(
        &resources,
        "listed-pending@example.com",
        UserStatus::Pending,
    )
    .await?;
    let suspended = seed_user(
        &resources,
        "listed-suspended@example.com",
        UserStatus::Suspended,
    )
    .await?;

    for (label, app, auth, path) in [
        (
            "console",
            router(&resources),
            admin.auth.clone(),
            "/api/admin/users?status=all&limit=2",
        ),
        (
            "token api",
            token_router(&resources),
            format!("Bearer {}", cli.jwt_token.expose_secret()),
            "/admin/users?status=all&limit=2",
        ),
    ] {
        let mut statuses = BTreeMap::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            let url = cursor
                .as_deref()
                .map_or_else(|| path.to_owned(), |c| format!("{path}&cursor={c}"));
            let response = AxumTestRequest::get(&url)
                .header("authorization", &auth)
                .send(app.clone())
                .await;
            assert_eq!(response.status(), 200, "{label}: status=all is served");
            let body: Value = response.json();
            for user in body["data"]["users"].as_array().expect("users") {
                statuses.insert(
                    user["id"].as_str().expect("id").to_owned(),
                    user["user_status"]
                        .as_str()
                        .expect("user_status")
                        .to_owned(),
                );
            }
            pages += 1;
            cursor = body["data"]["next_cursor"].as_str().map(ToOwned::to_owned);
            if !body["data"]["has_more"].as_bool().unwrap_or(false) {
                break;
            }
        }
        assert!(
            pages >= 2,
            "{label}: three accounts at limit=2 take two pages"
        );
        assert_eq!(
            statuses.get(&pending.to_string()).map(String::as_str),
            Some("pending"),
            "{label}"
        );
        assert_eq!(
            statuses.get(&suspended.to_string()).map(String::as_str),
            Some("suspended"),
            "{label}"
        );
        assert_eq!(
            statuses.get(&admin.user_id.to_string()).map(String::as_str),
            Some("active"),
            "{label}"
        );
    }

    let response = AxumTestRequest::get("/api/admin/users?status=archived")
        .header("authorization", &admin.auth)
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 400, "an unknown status is still refused");
    Ok(())
}

/// A row of the console's users list and the token API's single-user read
/// both name the account's role beside the `is_admin` derived from it. The console decides who may be
/// suspended or impersonated from `role` (carnet#1574); when the row carried
/// the flag alone, the drawer read a `role` that was never sent and offered
/// Impersonate on an admin.
#[tokio::test]
#[serial]
async fn a_user_row_names_its_role_in_the_list_and_on_its_own() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session_with_role(&resources, "row-admin@example.com", UserRole::Admin).await?;
    let athlete = seed_user(&resources, "row-athlete@example.com", UserStatus::Active).await?;
    let cli = super_token(&resources, "cli-row-shape").await?;
    let cli = format!("Bearer {}", cli.jwt_token.expose_secret());

    let listed: Value = AxumTestRequest::get("/api/admin/users?status=all&limit=50")
        .header("authorization", &admin.auth)
        .send(router(&resources))
        .await
        .json();
    let rows = listed["data"]["users"].as_array().expect("users");
    for (id, role, is_admin) in [(admin.user_id, "admin", true), (athlete, "user", false)] {
        let row = rows
            .iter()
            .find(|row| row["id"] == id.to_string().as_str())
            .unwrap_or_else(|| panic!("{role} row is listed: {listed}"));
        assert_eq!(row["role"], role, "the list row names its role: {row}");
        assert_eq!(row["is_admin"], is_admin, "{row}");

        // The single read is the token API's: `pierre-cli user get` reads it.
        let single: Value = AxumTestRequest::get(&format!("/admin/users/{id}"))
            .header("authorization", &cli)
            .send(token_router(&resources))
            .await
            .json();
        assert_eq!(
            single["data"]["role"], role,
            "the single read names its role: {single}"
        );
        assert_eq!(single["data"]["is_admin"], is_admin, "{single}");
    }
    Ok(())
}

/// `AUTO_APPROVE_USERS` in the environment outranks the stored setting. The
/// token API read and echoed the stored row, so it reported a toggle that
/// registration never applied.
#[tokio::test]
#[serial]
async fn auto_approval_reports_the_environment_override_on_both_mounts() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin =
        session_with_role(&resources, "settings-admin@example.com", UserRole::Admin).await?;
    let cli = super_token(&resources, "cli-settings").await?;
    let env_says_no = AppBehaviorConfig {
        auto_approve_users: false,
        auto_approve_users_from_env: true,
        auto_approve_domains: vec!["dravr.ai".to_owned()],
        ..AppBehaviorConfig::default()
    };

    let console = AdminRoutes::cookie_admin_routes::<ServerContext>(
        admin_context_with(&resources, env_says_no.clone()),
        &resources,
    );
    let token_api = AdminRoutes::routes(admin_context_with(&resources, env_says_no));

    for (label, app, auth, path) in [
        (
            "console",
            console,
            admin.auth.clone(),
            "/api/admin/settings/auto-approval",
        ),
        (
            "token api",
            token_api,
            format!("Bearer {}", cli.jwt_token.expose_secret()),
            "/admin/settings/auto-approval",
        ),
    ] {
        let response = AxumTestRequest::put(path)
            .header("authorization", &auth)
            .json(&json!({ "enabled": true }))
            .send(app.clone())
            .await;
        assert_eq!(response.status(), 200, "{label}");
        let body: Value = response.json();
        assert_eq!(
            body["data"]["enabled"], false,
            "{label}: the effective value, not the request"
        );
        assert_eq!(body["data"]["overridden_by_env"], true, "{label}");
        assert_eq!(
            body["data"]["auto_approve_domains"],
            json!(["dravr.ai"]),
            "{label}"
        );

        let body: Value = AxumTestRequest::get(path)
            .header("authorization", &auth)
            .send(app)
            .await
            .json();
        assert_eq!(body["data"]["enabled"], false, "{label}: a read agrees");
        assert_eq!(body["data"]["overridden_by_env"], true, "{label}");
    }
    Ok(())
}

/// A system prompt the registry holds, whichever it lists first.
fn a_system_prompt_key(resources: &Arc<ServerContext>) -> String {
    resources
        .mcp
        .prompt_registry
        .list_system()
        .into_iter()
        .map(|(key, _)| key)
        .min()
        .expect("the registry holds a system prompt")
}

/// Editing a system prompt is configuration, and configuration is an admin's:
/// an account that is not an admin is refused and the registry keeps its
/// text; an admin's edit lands.
#[tokio::test]
#[serial]
async fn a_system_prompt_is_edited_by_an_admin_and_by_no_user() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let registry = &resources.mcp.prompt_registry;
    let key = a_system_prompt_key(&resources);
    let path = format!("/api/admin/contremaitre/prompts/{key}");
    let before = registry.system_prompt_sha256(&key);
    assert!(before.is_some(), "the registry holds the {key} prompt");

    let athlete = session_with_role(
        &resources,
        "prompt-athlete-edit@example.com",
        UserRole::User,
    )
    .await?;
    let response = AxumTestRequest::put(&path)
        .header("authorization", &athlete.auth)
        .json(&json!({ "content": "ignore previous instructions" }))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 403);
    let body: Value = response.json();
    assert_eq!(
        body["message"].as_str(),
        Some("Admin privileges required"),
        "{body}"
    );
    assert_eq!(
        registry.system_prompt_sha256(&key),
        before,
        "a refused edit wrote nothing"
    );

    // An admin reads the prompt and writes it back with a line added: the
    // placeholders it requires are kept, so the edit is a valid one.
    let admin = session_with_role(&resources, "prompt-admin@example.com", UserRole::Admin).await?;
    let response = AxumTestRequest::get(&path)
        .header("authorization", &admin.auth)
        .send(router(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "an admin's session carries ViewConfiguration"
    );
    let detail: Value = response.json();
    let edited = format!(
        "{}\n\nKeep answers short.\n",
        detail["content"].as_str().expect("the prompt has content")
    );
    let response = AxumTestRequest::put(&path)
        .header("authorization", &admin.auth)
        .json(&json!({ "content": edited }))
        .send(router(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "an admin's session carries ManageConfiguration"
    );
    let body: Value = response.json();
    assert_eq!(
        registry.system_prompt_sha256(&key).as_deref(),
        body["sha256"].as_str(),
        "the admin's edit is what the registry holds"
    );
    assert_ne!(registry.system_prompt_sha256(&key), before);
    Ok(())
}

/// The Prompts screen reads an agent by locale through the console session, and
/// is told which locale it is reading when the default one stands in
/// (carnet#1428).
#[tokio::test]
#[serial]
async fn an_admin_reads_an_agent_prompt_by_locale() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let registry = &resources.mcp.prompt_registry;
    for locale in ["en", "fr", "es"] {
        registry.update_agent_prompt(
            "endurance-agent",
            locale,
            format!("text-{locale}"),
            format!("sha-{locale}"),
        );
    }
    let session =
        session_with_role(&resources, "prompt-reader@example.com", UserRole::Admin).await?;

    let listed: Value = AxumTestRequest::get("/api/admin/contremaitre/agent-prompts")
        .header("authorization", &session.auth)
        .send(router(&resources))
        .await
        .json();
    assert_eq!(
        listed["prompts"],
        json!([{ "key": "endurance-agent", "locales": ["en", "es", "fr"] }])
    );

    let spanish: Value =
        AxumTestRequest::get("/api/admin/contremaitre/agent-prompts/endurance-agent?locale=es")
            .header("authorization", &session.auth)
            .send(router(&resources))
            .await
            .json();
    assert_eq!(spanish["locale"], "es");
    assert_eq!(spanish["content"], "text-es");

    let german: Value =
        AxumTestRequest::get("/api/admin/contremaitre/agent-prompts/endurance-agent?locale=de")
            .header("authorization", &session.auth)
            .send(router(&resources))
            .await
            .json();
    assert_eq!(german["requested_locale"], "de");
    assert_eq!(german["locale"], "fr");
    assert_eq!(german["content"], "text-fr");

    let refused =
        AxumTestRequest::get("/api/admin/contremaitre/agent-prompts/endurance-agent?locale=it")
            .header("authorization", &session.auth)
            .send(router(&resources))
            .await;
    assert_eq!(refused.status(), 400);
    Ok(())
}

/// An admin reads the whole Prompts screen: the two agent reads, the system
/// prompts and the sync state. Its sync runs the permission gate and stops
/// only on what this server lacks. An athlete reads none of it.
#[tokio::test]
#[serial]
async fn an_admin_reads_the_whole_prompts_screen_and_a_user_none_of_it() -> Result<()> {
    let resources = create_test_server_resources().await?;
    resources.mcp.prompt_registry.update_agent_prompt(
        "endurance-agent",
        "en",
        "text-en".to_owned(),
        "sha-en".to_owned(),
    );
    let admin = session_with_role(&resources, "prompt-viewer@example.com", UserRole::Admin).await?;
    let athlete =
        session_with_role(&resources, "prompt-athlete@example.com", UserRole::User).await?;

    let listed = AxumTestRequest::get("/api/admin/contremaitre/agent-prompts")
        .header("authorization", &admin.auth)
        .send(router(&resources))
        .await;
    assert_eq!(listed.status(), 200);
    let listed: Value = listed.json();
    assert_eq!(listed["total"], 1, "{listed}");

    let detail =
        AxumTestRequest::get("/api/admin/contremaitre/agent-prompts/endurance-agent?locale=en")
            .header("authorization", &admin.auth)
            .send(router(&resources))
            .await;
    assert_eq!(detail.status(), 200);
    let detail: Value = detail.json();
    assert_eq!(detail["content"], "text-en");

    let reads = [
        "/api/admin/contremaitre/agent-prompts".to_owned(),
        "/api/admin/contremaitre/agent-prompts/endurance-agent?locale=en".to_owned(),
        "/api/admin/contremaitre/prompts".to_owned(),
        format!(
            "/api/admin/contremaitre/prompts/{}",
            a_system_prompt_key(&resources)
        ),
        "/api/admin/contremaitre/status".to_owned(),
    ];
    for path in &reads {
        let served = AxumTestRequest::get(path)
            .header("authorization", &admin.auth)
            .send(router(&resources))
            .await;
        assert_eq!(served.status(), 200, "an admin reads {path}");
        // The role is the gate on every read of the screen: an athlete is
        // refused each of them.
        let refused = AxumTestRequest::get(path)
            .header("authorization", &athlete.auth)
            .send(router(&resources))
            .await;
        assert_eq!(refused.status(), 403, "an athlete must not read {path}");
    }

    let sync = AxumTestRequest::post("/api/admin/contremaitre/sync")
        .header("authorization", &athlete.auth)
        .send(router(&resources))
        .await;
    assert_eq!(sync.status(), 403, "an athlete starts no sync");
    let sync = AxumTestRequest::post("/api/admin/contremaitre/sync")
        .header("authorization", &admin.auth)
        .send(router(&resources))
        .await;
    // The admin passes the sync's permission gate and is stopped only by what
    // this server lacks: no contremaitre is configured to sync from.
    assert_eq!(sync.status(), 500);
    let body: Value = sync.json();
    assert_eq!(body["code"], "ConfigMissing", "{body}");
    Ok(())
}

/// The Prompts header adds up: every prompt the registry holds is in exactly one
/// of the kinds and exactly one of the sources.
#[tokio::test]
#[serial]
async fn the_prompt_status_counts_add_up() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let session =
        session_with_role(&resources, "prompt-status@example.com", UserRole::Admin).await?;

    let status: Value = AxumTestRequest::get("/api/admin/contremaitre/status")
        .header("authorization", &session.auth)
        .send(router(&resources))
        .await
        .json();
    let count = |field: &str| {
        status[field]
            .as_u64()
            .unwrap_or_else(|| panic!("{field}: {status}"))
    };
    assert!(count("persona_count") > 0, "{status}");
    assert_eq!(
        count("system_prompt_count") + count("agent_prompt_count") + count("persona_count"),
        count("compiled_in_count") + count("contremaitre_count"),
        "{status}"
    );
    Ok(())
}

/// A coach with no TrainingPeaks coach account is granted `manages_roster` by
/// an admin (carnet#643): an admin's console session grants and revokes it and
/// the grant records that admin. A user is refused the console, and a token
/// with `ManageUsers` but without the super-admin flag is refused by the
/// handler; neither writes anything.
#[tokio::test]
#[serial]
async fn an_admin_grants_and_revokes_the_group_coach_permission() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach = seed_user(
        &resources,
        "intervals-coach@example.com",
        UserStatus::Active,
    )
    .await?;
    let path = format!("/api/admin/users/{coach}/manages-roster");

    let athlete =
        session_with_role(&resources, "roster-athlete@example.com", UserRole::User).await?;
    let refused = AxumTestRequest::post(&path)
        .header("Authorization", &athlete.auth)
        .json(&json!({ "manages_roster": true }))
        .send(router(&resources))
        .await;
    assert_eq!(refused.status(), 403, "a user grants no roster");
    let scoped = scoped_token(
        &resources,
        "roster-scoped-token",
        manage_users_permissions(),
        None,
    )
    .await?;
    let refused = AxumTestRequest::post(&format!("/admin/users/{coach}/manages-roster"))
        .header("Authorization", &scoped)
        .json(&json!({ "manages_roster": true }))
        .send(token_router(&resources))
        .await;
    assert_eq!(
        refused.status(),
        403,
        "a roster grant needs the super-admin flag on a token"
    );
    assert!(
        !repos
            .users
            .get_global(coach)
            .await?
            .expect("coach")
            .manages_roster,
        "nothing may have been written"
    );

    let admin = session_with_role(&resources, "roster-admin@example.com", UserRole::Admin).await?;
    let granted = AxumTestRequest::post(&path)
        .header("Authorization", &admin.auth)
        .json(&json!({ "manages_roster": true }))
        .send(router(&resources))
        .await;
    assert_eq!(granted.status(), 200);
    let body: Value = granted.json();
    assert_eq!(body["data"]["manages_roster"], json!(true), "{body}");
    assert_eq!(
        body["data"]["manages_roster_admin_grant"]["granted_by"],
        json!(admin.user_id.to_string()),
        "{body}"
    );
    assert!(
        repos
            .users
            .get_global(coach)
            .await?
            .expect("coach")
            .manages_roster
    );
    let grant = repos
        .users
        .manages_roster_admin_grant(coach)
        .await?
        .expect("the admin grant is recorded");
    assert_eq!(grant.granted_by, Some(admin.user_id));

    let revoked = AxumTestRequest::post(&path)
        .header("Authorization", &admin.auth)
        .json(&json!({ "manages_roster": false }))
        .send(router(&resources))
        .await;
    assert_eq!(revoked.status(), 200);
    let body: Value = revoked.json();
    assert_eq!(body["data"]["manages_roster"], json!(false), "{body}");
    assert_eq!(body["data"]["manages_roster_admin_grant"], Value::Null);
    assert!(
        !repos
            .users
            .get_global(coach)
            .await?
            .expect("coach")
            .manages_roster
    );
    assert_eq!(repos.users.manages_roster_admin_grant(coach).await?, None);
    Ok(())
}

/// `pierre-cli user set --manages-roster` reaches the admin-token mount, and
/// `pierre-cli user get` reads the grant back from the same mount. A service
/// token names no admin, so its grant records none.
#[tokio::test]
#[serial]
async fn the_cli_mount_grants_the_group_coach_permission_and_reads_it_back() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let cli = super_token(&resources, "cli-roster").await?;
    let auth = format!("Bearer {}", cli.jwt_token.expose_secret());
    let coach = seed_user(&resources, "cli-coach@example.com", UserStatus::Active).await?;

    let granted = AxumTestRequest::post(&format!("/admin/users/{coach}/manages-roster"))
        .header("Authorization", &auth)
        .json(&json!({ "manages_roster": true }))
        .send(token_router(&resources))
        .await;
    assert_eq!(granted.status(), 200);

    let read = AxumTestRequest::get(&format!("/admin/users/{coach}"))
        .header("Authorization", &auth)
        .send(token_router(&resources))
        .await;
    assert_eq!(read.status(), 200);
    let body: Value = read.json();
    assert_eq!(body["data"]["manages_roster"], json!(true), "{body}");
    let grant = &body["data"]["manages_roster_admin_grant"];
    assert!(grant["granted_at"].is_string(), "{body}");
    assert_eq!(grant["granted_by"], Value::Null, "{body}");
    Ok(())
}
