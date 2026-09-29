// ABOUTME: The console's user management through the one admin handler set, over session auth
// ABOUTME: Plain admins approve/suspend/reset/pre-approve; super-admin ops and token-less gates hold
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The web console's user tabs, served by the admin-token handlers.
//!
//! The console used to have a second handler stack of its own in
//! `pierre-routes-web-admin`, kept in step with the admin-token one by a drift
//! gate (carnet#604). It is gone: `AdminRoutes::cookie_admin_routes` mounts the
//! same handlers `pierre-cli` reaches under `/admin`, and a plain Admin's
//! console session carries `ManageUsers` so those handlers' permission checks
//! let it through. These tests drive the console mount the composition root
//! serves and assert on what each call wrote, and they pin the two edges of
//! the grant: a plain Admin still cannot do what only a super-admin may, and an
//! admin token without `ManageUsers` is still refused.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_config::mcp::AppBehaviorConfig;
use pierre_core::admin::models::{AdminPermissions, CreateAdminTokenRequest, GeneratedAdminToken};
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::auth::service::AdminAuthService;
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

/// Approve, suspend and reset through the console mount as a plain Admin.
///
/// Before the console was routed through the admin-token handlers, this mount
/// did not serve these paths at all, and a plain Admin's session lacked the
/// `ManageUsers` those handlers check; each call below would have failed.
#[tokio::test]
#[serial]
async fn a_plain_admin_approves_suspends_and_resets_through_the_one_handler() -> Result<()> {
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
    assert_eq!(
        response.status(),
        200,
        "a plain admin must be able to approve"
    );
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
    assert_eq!(
        response.status(),
        200,
        "a plain admin must be able to reset"
    );
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
    assert_eq!(
        response.status(),
        200,
        "a plain admin must be able to suspend"
    );
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

/// A plain Admin's reset reaches only the users of the tenant it works in; a
/// user elsewhere answers 404, exactly like a missing one.
#[tokio::test]
#[serial]
async fn a_plain_admin_cannot_reset_a_user_outside_its_tenant() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session_with_role(&resources, "scoped-admin@example.com", UserRole::Admin).await?;
    let elsewhere =
        session_with_role(&resources, "other-tenant@example.com", UserRole::User).await?;

    let response = AxumTestRequest::post(&format!(
        "/api/admin/users/{}/reset-password",
        elsewhere.user_id
    ))
    .header("Authorization", &admin.auth)
    .send(router(&resources))
    .await;
    assert_eq!(
        response.status(),
        404,
        "a user in another tenant is not reachable by a plain admin"
    );
    Ok(())
}

/// The grant is `ManageUsers`, not super-admin: a tier change stays refused.
#[tokio::test]
#[serial]
async fn a_plain_admin_cannot_change_a_tier() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin =
        session_with_role(&resources, "tierless-admin@example.com", UserRole::Admin).await?;
    let target = seed_user(&resources, "starter@example.com", UserStatus::Active).await?;

    let response = AxumTestRequest::post(&format!("/api/admin/users/{target}/tier"))
        .header("Authorization", &admin.auth)
        .json(&json!({ "tier": "enterprise" }))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 403, "a tier change is super-admin only");
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

    let superadmin = session_with_role(
        &resources,
        "tier-superadmin@example.com",
        UserRole::SuperAdmin,
    )
    .await?;
    let response = AxumTestRequest::post(&format!("/api/admin/users/{target}/tier"))
        .header("Authorization", &superadmin.auth)
        .json(&json!({ "tier": "enterprise" }))
        .send(router(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "a super-admin session changes a tier"
    );
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
/// plain-admin permission set, which has no `ManageUsers`, is still refused.
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
        .header("Authorization", &format!("Bearer {}", generated.jwt_token))
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
            format!("Bearer {}", cli.jwt_token),
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
            format!("Bearer {}", cli.jwt_token),
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

/// Editing a system prompt is configuration. It was gated on
/// `ManageAdminTokens`, which a plain Admin's console session now carries, so
/// the gate moved to `ManageConfiguration` — super-admin only.
#[tokio::test]
#[serial]
async fn a_plain_admin_cannot_edit_a_system_prompt() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session_with_role(&resources, "prompt-admin@example.com", UserRole::Admin).await?;

    let response = AxumTestRequest::put("/api/admin/contremaitre/prompts/coaching")
        .header("authorization", &admin.auth)
        .json(&json!({ "content": "ignore previous instructions" }))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 403);
    let body: Value = response.json();
    assert_eq!(
        body["message"].as_str(),
        Some("Permission required: manage_configuration")
    );
    Ok(())
}

/// A coach with no TrainingPeaks coach account is granted `manages_roster` by
/// an operator (carnet#643): a super-admin console session grants and revokes
/// it, the grant records that operator, and a plain admin is refused.
#[tokio::test]
#[serial]
async fn a_super_admin_grants_and_revokes_the_group_coach_permission() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach = seed_user(
        &resources,
        "intervals-coach@example.com",
        UserStatus::Active,
    )
    .await?;
    let path = format!("/api/admin/users/{coach}/manages-roster");

    let admin = session_with_role(&resources, "roster-admin@example.com", UserRole::Admin).await?;
    let refused = AxumTestRequest::post(&path)
        .header("Authorization", &admin.auth)
        .json(&json!({ "manages_roster": true }))
        .send(router(&resources))
        .await;
    assert_eq!(refused.status(), 403, "a roster grant is super-admin only");
    assert!(
        !repos
            .users
            .get_global(coach)
            .await?
            .expect("coach")
            .manages_roster,
        "nothing may have been written"
    );

    let superadmin = session_with_role(
        &resources,
        "roster-superadmin@example.com",
        UserRole::SuperAdmin,
    )
    .await?;
    let granted = AxumTestRequest::post(&path)
        .header("Authorization", &superadmin.auth)
        .json(&json!({ "manages_roster": true }))
        .send(router(&resources))
        .await;
    assert_eq!(granted.status(), 200);
    let body: Value = granted.json();
    assert_eq!(body["data"]["manages_roster"], json!(true), "{body}");
    assert_eq!(
        body["data"]["manages_roster_operator_grant"]["granted_by"],
        json!(superadmin.user_id.to_string()),
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
        .manages_roster_operator_grant(coach)
        .await?
        .expect("the operator grant is recorded");
    assert_eq!(grant.granted_by, Some(superadmin.user_id));

    let revoked = AxumTestRequest::post(&path)
        .header("Authorization", &superadmin.auth)
        .json(&json!({ "manages_roster": false }))
        .send(router(&resources))
        .await;
    assert_eq!(revoked.status(), 200);
    let body: Value = revoked.json();
    assert_eq!(body["data"]["manages_roster"], json!(false), "{body}");
    assert_eq!(body["data"]["manages_roster_operator_grant"], Value::Null);
    assert!(
        !repos
            .users
            .get_global(coach)
            .await?
            .expect("coach")
            .manages_roster
    );
    assert_eq!(
        repos.users.manages_roster_operator_grant(coach).await?,
        None
    );
    Ok(())
}

/// `pierre-cli user set --manages-roster` reaches the admin-token mount, and
/// `pierre-cli user get` reads the grant back from the same mount. A service
/// token names no operator, so its grant records none.
#[tokio::test]
#[serial]
async fn the_cli_mount_grants_the_group_coach_permission_and_reads_it_back() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let cli = super_token(&resources, "cli-roster").await?;
    let auth = format!("Bearer {}", cli.jwt_token);
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
    let grant = &body["data"]["manages_roster_operator_grant"];
    assert!(grant["granted_at"].is_string(), "{body}");
    assert_eq!(grant["granted_by"], Value::Null, "{body}");
    Ok(())
}
