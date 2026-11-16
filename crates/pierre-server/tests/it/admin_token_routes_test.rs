// ABOUTME: The one admin-token handler set over both mounts — console session and admin token
// ABOUTME: Pins admin gating, rotation, include_inactive, revoke's 404, and grant limits

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

use crate::common;
use crate::helpers;
use secrecy::ExposeSecret;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
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
// Console session: an admin manages every token; a user and a scoped token do not
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

/// An admin's console session lists every token, admin ones included.
/// A `ManageAdminTokens` token without the super-admin flag still sees only
/// the tokens below it, and an account that is not an admin sees none.
#[tokio::test]
#[serial]
async fn an_admin_lists_every_token_and_a_scoped_token_only_non_super_ones() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-list@example.com", UserRole::Admin).await?;
    let athlete = session(&resources, "athlete-list@example.com", UserRole::User).await?;
    let delegate = mint(
        &resources,
        "delegate-list",
        vec![AdminPermission::ManageAdminTokens],
        false,
    )
    .await?;
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

    let body: Value = AxumTestRequest::get("/admin/tokens")
        .header("authorization", &bearer(&delegate))
        .send(token_api(&resources))
        .await
        .json();
    let ids = listed_ids(&body);
    assert!(
        ids.contains(&service.token_id),
        "a token without the flag sees service tokens"
    );
    assert!(
        !ids.contains(&operator.token_id),
        "a super-admin token is not shown to a token without the flag"
    );
    let entry = &body["data"]["tokens"][0];
    assert!(
        entry["permissions"].is_array() && entry["usage_count"].is_number(),
        "the console dereferences a flat permissions array and usage_count: {entry}"
    );

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
        ids.contains(&service.token_id) && ids.contains(&operator.token_id),
        "an admin sees every token"
    );
    Ok(())
}

/// An admin mints any token from the console: a service token, an admin
/// one, one carrying any permission. A `ManageAdminTokens` token without the
/// flag mints neither of the last two, and a user mints nothing.
#[tokio::test]
#[serial]
async fn an_admin_mints_any_token_and_a_scoped_token_never_a_super_one() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-mint@example.com", UserRole::Admin).await?;
    let athlete = session(&resources, "athlete-mint@example.com", UserRole::User).await?;
    let delegate = mint(
        &resources,
        "delegate-mint",
        vec![AdminPermission::ManageAdminTokens],
        false,
    )
    .await?;

    let response = AxumTestRequest::post("/api/admin/tokens")
        .header("authorization", &athlete)
        .json(&json!({ "service_name": "athlete-grab", "permissions": ["list_keys"] }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 403, "a user mints no admin token");

    let response = AxumTestRequest::post("/admin/tokens")
        .header("authorization", &bearer(&delegate))
        .json(&json!({ "service_name": "escalation", "is_super_admin": true }))
        .send(token_api(&resources))
        .await;
    assert_eq!(
        response.status(),
        403,
        "a super-admin token is not minted by a token without the flag"
    );
    let response = AxumTestRequest::post("/admin/tokens")
        .header("authorization", &bearer(&delegate))
        .json(&json!({ "service_name": "config-grab", "permissions": ["manage_configuration"] }))
        .send(token_api(&resources))
        .await;
    assert_eq!(
        response.status(),
        403,
        "a token without the flag cannot grant a permission it does not hold"
    );
    let all = resources.common.repos.admin.list_tokens(true).await?;
    assert_eq!(all.len(), 1, "the refused requests wrote nothing");

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
    assert_eq!(all.len(), 4, "the delegate and the admin's three tokens");
    Ok(())
}

/// A super-admin token is revoked or rotated by an admin, and by no user and
/// no token without the flag: after their refusals it is still active.
#[tokio::test]
#[serial]
async fn a_super_token_stays_active_until_an_admin_revokes_it() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-revoke@example.com", UserRole::Admin).await?;
    let athlete = session(&resources, "athlete-revoke@example.com", UserRole::User).await?;
    let delegate = mint(
        &resources,
        "delegate-revoke",
        vec![AdminPermission::ManageAdminTokens],
        false,
    )
    .await?;
    let operator = mint(
        &resources,
        "svc-operator",
        vec![AdminPermission::ListKeys],
        true,
    )
    .await?;

    for action in ["revoke", "rotate"] {
        let response =
            AxumTestRequest::post(&format!("/api/admin/tokens/{}/{action}", operator.token_id))
                .header("authorization", &athlete)
                .json(&json!({}))
                .send(console(&resources))
                .await;
        assert_eq!(response.status(), 403, "a user must not {action} a token");
        let response =
            AxumTestRequest::post(&format!("/admin/tokens/{}/{action}", operator.token_id))
                .header("authorization", &bearer(&delegate))
                .json(&json!({}))
                .send(token_api(&resources))
                .await;
        assert_eq!(
            response.status(),
            403,
            "a token without the flag must not {action} a super-admin token"
        );
    }

    // A guard that ran after the deactivation would still 403 while having
    // destroyed the operator's credential; the row is what matters.
    assert!(is_active(&resources, &operator.token_id).await?);

    let response =
        AxumTestRequest::post(&format!("/api/admin/tokens/{}/revoke", operator.token_id))
            .header("authorization", &admin)
            .send(console(&resources))
            .await;
    assert_eq!(
        response.status(),
        200,
        "an admin revokes a super-admin token"
    );
    assert!(!is_active(&resources, &operator.token_id).await?);
    Ok(())
}

/// A rotation hands its caller a live token carrying the old one's
/// permissions, so a token without the super-admin flag rotates only a token
/// whose every permission it holds itself: the rule minting one takes, in
/// the same words. Refused, the token it named is still the live credential
/// and nothing was minted; a token within what the caller holds rotates.
#[tokio::test]
#[serial]
async fn a_scoped_token_rotates_only_a_token_whose_permissions_it_holds() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let delegate = mint(
        &resources,
        "delegate-rotate",
        vec![
            AdminPermission::ManageAdminTokens,
            AdminPermission::ListKeys,
        ],
        false,
    )
    .await?;
    let wider = mint(
        &resources,
        "svc-user-manager",
        vec![AdminPermission::ListKeys, AdminPermission::ManageUsers],
        false,
    )
    .await?;
    let within = mint(
        &resources,
        "svc-key-lister",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();

    let response = AxumTestRequest::post(&format!("/admin/tokens/{}/rotate", wider.token_id))
        .header("authorization", &bearer(&delegate))
        .json(&json!({}))
        .send(token_api(&resources))
        .await;
    let (status, body) = quotable(response);
    assert_eq!(
        status, 403,
        "a rotation must not hand out a permission the caller lacks: {body}"
    );
    assert!(
        body.contains("Cannot grant manage_users: the caller does not hold it"),
        "the refusal is the one minting gives: {body}"
    );
    assert!(!body.contains("jwt_token"), "no credential is handed out");
    assert!(
        is_active(&resources, &wider.token_id).await?,
        "a refused rotation leaves the credential it named live"
    );
    assert_eq!(
        resources.common.repos.admin.list_tokens(true).await?.len(),
        before,
        "nothing was minted"
    );

    // The positive control: the same caller, a token within what it holds.
    let response = AxumTestRequest::post(&format!("/admin/tokens/{}/rotate", within.token_id))
        .header("authorization", &bearer(&delegate))
        .json(&json!({}))
        .send(token_api(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "a token it could have minted rotates"
    );
    let body: Value = response.json();
    let new_id = body["data"]["token_id"].as_str().expect("new token id");
    assert!(!is_active(&resources, &within.token_id).await?);
    assert!(is_active(&resources, new_id).await?);
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
/// admin's standing, came back live at its old scope; now it answers 400 on
/// both mounts and the table is as it was.
#[tokio::test]
#[serial]
async fn a_revoked_token_is_not_rotated_back_to_life() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "admin-revoked@example.com", UserRole::Admin).await?;
    let cli = mint(&resources, "cli-operator", vec![], true).await?;
    let revoked = mint(&resources, "svc-revoked", vec![], true).await?;
    resources
        .common
        .repos
        .admin
        .deactivate_token(&revoked.token_id)
        .await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();

    let on_the_console =
        AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", revoked.token_id))
            .header("authorization", &admin)
            .json(&json!({}))
            .send(console(&resources))
            .await;
    let on_the_token_api =
        AxumTestRequest::post(&format!("/admin/tokens/{}/rotate", revoked.token_id))
            .header("authorization", &bearer(&cli))
            .json(&json!({}))
            .send(token_api(&resources))
            .await;
    for response in [on_the_console, on_the_token_api] {
        let (status, body) = quotable(response);
        assert_eq!(status, 400, "a revoked token must not rotate: {body}");
        assert!(body.contains("cannot be rotated"), "{body}");
        assert!(!body.contains("jwt_token"), "no credential is handed out");
    }

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
// The divergences settled for both mounts
// ===========================================================================

/// The token API ignored `include_inactive`; the console honoured it.
#[tokio::test]
#[serial]
async fn include_inactive_lists_revoked_tokens_on_both_mounts() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "super-inactive@example.com", UserRole::Admin).await?;
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
            admin.clone(),
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
    let admin = session(&resources, "super-missing@example.com", UserRole::Admin).await?;
    let cli = mint(&resources, "cli-missing", vec![], true).await?;
    let missing = "admin_00000000000000000000000000000000";

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{missing}/revoke"))
        .header("authorization", &admin)
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

/// A request that names no permission is granted the default set, and that
/// set is held to the rule a named one is: a caller holding only
/// `ManageAdminTokens` asked for none of the defaults and holds none, so it
/// is refused in the sentence minting a named permission is refused with,
/// and nothing is minted. Every default is held to it, not the first: a
/// caller holding all of them but the last in name order is refused for
/// that one. A caller that holds the defaults mints them, and a caller with
/// the super-admin flag mints them whatever it holds.
#[tokio::test]
#[serial]
async fn a_token_minted_with_no_list_carries_only_what_its_caller_holds() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let delegate = mint(
        &resources,
        "delegate-default",
        vec![AdminPermission::ManageAdminTokens],
        false,
    )
    .await?;
    let mut held = AdminPermissions::default_admin().to_vec();
    held.push(AdminPermission::ManageAdminTokens);
    let key_manager = mint(&resources, "key-manager", held, false).await?;
    let all_but_the_last = mint(
        &resources,
        "all-but-the-last",
        vec![
            AdminPermission::ManageAdminTokens,
            AdminPermission::ListKeys,
            AdminPermission::ProvisionKeys,
            AdminPermission::RevokeKeys,
        ],
        false,
    )
    .await?;
    let flagged = mint(&resources, "flagged", vec![], true).await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();

    // The refusal names the first default, in name order, the caller does
    // not hold, in the sentence a named one gets: the first of the four for
    // a caller holding none, the last for a caller holding the other three.
    for (caller, name, refused) in [
        (
            &delegate,
            "default-grab",
            "Cannot grant list_keys: the caller does not hold it",
        ),
        (
            &all_but_the_last,
            "default-most",
            "Cannot grant update_key_limits: the caller does not hold it",
        ),
    ] {
        let response = AxumTestRequest::post("/admin/tokens")
            .header("authorization", &bearer(caller))
            .json(&json!({ "service_name": name }))
            .send(token_api(&resources))
            .await;
        assert_eq!(response.status(), 403, "{name}");
        let mut body: Value = response.json();
        unquote_minted_token(&mut body);
        assert_eq!(body["message"].as_str(), Some(refused), "{name}");
        assert!(
            body["data"].is_null(),
            "{name}: no credential is handed out: {body}"
        );
        assert_eq!(
            resources.common.repos.admin.list_tokens(true).await?.len(),
            before,
            "{name}: nothing was minted"
        );
    }

    // The positive controls: the same request from a caller that holds the
    // defaults, and from one with the flag.
    for (caller, name) in [
        (&key_manager, "default-held"),
        (&flagged, "default-flagged"),
    ] {
        let response = AxumTestRequest::post("/admin/tokens")
            .header("authorization", &bearer(caller))
            .json(&json!({ "service_name": name }))
            .send(token_api(&resources))
            .await;
        assert_eq!(response.status(), 201, "{name}");
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
            "{name}: the default set"
        );
        assert!(!minted.is_super_admin, "{name}");
    }
    assert_eq!(
        resources.common.repos.admin.list_tokens(true).await?.len(),
        before + 2
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

/// A lifetime no date can hold is refused with a 400 on both surfaces and
/// changes nothing: no token is minted, and a rotation leaves the token it
/// was asked to replace live.
#[tokio::test]
#[serial]
async fn a_lifetime_past_any_date_is_refused_and_changes_nothing() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let cli = mint(&resources, "cli-lifetime", vec![], true).await?;
    let seeded = mint(
        &resources,
        "svc-lifetime",
        vec![AdminPermission::ListKeys],
        false,
    )
    .await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();
    let too_many = json!({ "service_name": "svc-forever", "expires_in_days": u64::MAX });

    let minted = AxumTestRequest::post("/admin/tokens")
        .header("authorization", &bearer(&cli))
        .json(&too_many)
        .send(token_api(&resources))
        .await;
    let (status, answer) = quotable(minted);
    assert_eq!(status, 400, "mint: {answer}");

    let rotated = AxumTestRequest::post(&format!("/admin/tokens/{}/rotate", seeded.token_id))
        .header("authorization", &bearer(&cli))
        .json(&too_many)
        .send(token_api(&resources))
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

    // The positive control: the same two requests with a lifetime a date holds.
    let minted = AxumTestRequest::post("/admin/tokens")
        .header("authorization", &bearer(&cli))
        .json(&json!({ "service_name": "svc-month", "expires_in_days": 30 }))
        .send(token_api(&resources))
        .await;
    let (status, answer) = quotable(minted);
    assert_eq!(status, 201, "mint: {answer}");
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

/// The tenant the stored token `token_id` is bound to.
async fn tenant_of(resources: &Arc<ServerContext>, token_id: &str) -> Result<Option<String>> {
    Ok(resources
        .common
        .repos
        .admin
        .get_token_by_id(token_id)
        .await?
        .expect("the token row exists")
        .tenant_id)
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

/// `POST {path}` on the token mount as `caller`, with `body`: the status
/// and the answer, fit to quote ([`quotable`]).
pub(crate) async fn post_as(
    resources: &Arc<ServerContext>,
    caller: &GeneratedAdminToken,
    path: &str,
    body: Value,
) -> (u16, String) {
    let response = AxumTestRequest::post(path)
        .header("authorization", &bearer(caller))
        .json(&body)
        .send(token_api(resources))
        .await;
    quotable(response)
}

/// A `ManageAdminTokens` holder bound to one tenant stays inside it on
/// every token write (carnet#1589). It neither revokes nor rotates a token
/// bound to no tenant or to another one: revoking is a denial of service
/// outside its tenant, and a rotation would hand it a replacement that
/// reaches the platform, or a tenant it does not. Each is answered 404, as
/// a token that does not exist is (`admin_token_reach_test` compares the
/// two answers), leaves the token live and mints nothing. These answered
/// 403 with a sentence naming the token's binding until the read of a
/// token outside reach became a 404 and the writes followed it.
/// Inside its tenant it revokes and rotates,
/// the replacement keeping the tenant, and what it mints is bound to its
/// tenant too. A holder bound to no tenant is the control: it revokes the
/// token bound to none and mints one bound to none.
#[tokio::test]
#[serial]
async fn a_tenant_bound_token_revokes_rotates_and_mints_only_inside_its_tenant() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (own_tenant, other_tenant) = (TenantId::generate(), TenantId::generate());
    let managing = vec![
        AdminPermission::ManageAdminTokens,
        AdminPermission::ListKeys,
    ];
    let lister = vec![AdminPermission::ListKeys];
    let bound = mint_bound(&resources, "club-desk", managing.clone(), own_tenant).await?;
    let platform = mint(&resources, "platform-lister", lister.clone(), false).await?;
    let elsewhere = mint_bound(&resources, "other-club", lister.clone(), other_tenant).await?;
    let rotated_here = mint_bound(&resources, "club-rotated", lister.clone(), own_tenant).await?;
    let revoked_here = mint_bound(&resources, "club-revoked", lister.clone(), own_tenant).await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();

    for outside in [&platform, &elsewhere] {
        let path = format!("/admin/tokens/{}/revoke", outside.token_id);
        let (status, body) = post_as(&resources, &bound, &path, json!({})).await;
        assert_eq!(status, 404, "{}: {body}", outside.service_name);

        let path = format!("/admin/tokens/{}/rotate", outside.token_id);
        let (status, body) = post_as(&resources, &bound, &path, json!({})).await;
        assert_eq!(status, 404, "{}: {body}", outside.service_name);
        assert!(!body.contains("jwt_token"), "no credential is handed out");

        assert!(
            is_active(&resources, &outside.token_id).await?,
            "a refused write leaves {} live",
            outside.service_name
        );
    }
    assert_eq!(
        resources.common.repos.admin.list_tokens(true).await?.len(),
        before,
        "nothing was minted"
    );

    // Inside its tenant: a rotation, whose replacement keeps the tenant.
    let path = format!("/admin/tokens/{}/rotate", rotated_here.token_id);
    let (status, body) = post_as(&resources, &bound, &path, json!({})).await;
    assert_eq!(status, 200, "a token of its own tenant rotates: {body}");
    let rotated: Value = serde_json::from_str(&body)?;
    let replacement = rotated["data"]["token_id"].as_str().expect("new token id");
    assert!(!is_active(&resources, &rotated_here.token_id).await?);
    assert!(is_active(&resources, replacement).await?);
    assert_eq!(
        tenant_of(&resources, replacement).await?,
        Some(own_tenant.to_string())
    );

    // A revocation.
    let path = format!("/admin/tokens/{}/revoke", revoked_here.token_id);
    let (status, body) = post_as(&resources, &bound, &path, json!({})).await;
    assert_eq!(status, 200, "a token of its own tenant is revoked: {body}");
    assert!(!is_active(&resources, &revoked_here.token_id).await?);

    // And a mint: the request names no tenant, and the token is bound to
    // the caller's.
    let (status, body) = post_as(
        &resources,
        &bound,
        "/admin/tokens",
        json!({ "service_name": "club-minted", "permissions": ["list_keys"] }),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let minted: Value = serde_json::from_str(&body)?;
    let minted_id = minted["data"]["token_id"].as_str().expect("token id");
    assert_eq!(
        tenant_of(&resources, minted_id).await?,
        Some(own_tenant.to_string()),
        "a bound caller mints nothing that reaches the platform"
    );

    // The control: the same permissions, bound to no tenant.
    let unbound = mint(&resources, "platform-desk", managing, false).await?;
    let (status, body) = post_as(
        &resources,
        &unbound,
        "/admin/tokens",
        json!({ "service_name": "platform-minted", "permissions": ["list_keys"] }),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let minted: Value = serde_json::from_str(&body)?;
    let minted_id = minted["data"]["token_id"].as_str().expect("token id");
    assert_eq!(tenant_of(&resources, minted_id).await?, None);
    let path = format!("/admin/tokens/{}/revoke", platform.token_id);
    let (status, body) = post_as(&resources, &unbound, &path, json!({})).await;
    assert_eq!(status, 200, "{body}");
    assert!(!is_active(&resources, &platform.token_id).await?);
    Ok(())
}

/// A `ManageAdminTokens` holder bound to no tenant, without the super-admin
/// flag, reads no tenant's data, so it neither revokes nor rotates a token
/// bound to one (carnet#1589): the replacement would read what it cannot.
/// Each is answered 404, as a token that does not exist is (403 with a
/// sentence naming the binding, until the writes followed the read), and
/// leaves the token live and mints nothing. A token bound to
/// no tenant it rotates and revokes, and the platform's callers, a
/// super-admin token and an admin's console session, write the tenant's.
#[tokio::test]
#[serial]
async fn an_unbound_token_revokes_and_rotates_only_tokens_bound_to_no_tenant() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let tenant = TenantId::generate();
    let managing = vec![
        AdminPermission::ManageAdminTokens,
        AdminPermission::ListKeys,
    ];
    let lister = vec![AdminPermission::ListKeys];
    let unbound = mint(&resources, "platform-desk", managing, false).await?;
    let club = mint_bound(&resources, "club-lister", lister.clone(), tenant).await?;
    let club_too = mint_bound(&resources, "club-second", lister.clone(), tenant).await?;
    let free = mint(&resources, "free-lister", lister.clone(), false).await?;
    let before = resources.common.repos.admin.list_tokens(true).await?.len();

    for act in ["revoke", "rotate"] {
        let path = format!("/admin/tokens/{}/{act}", club.token_id);
        let (status, body) = post_as(&resources, &unbound, &path, json!({})).await;
        assert_eq!(status, 404, "{act}: {body}");
        assert!(!body.contains("jwt_token"), "no credential is handed out");
    }
    assert!(
        is_active(&resources, &club.token_id).await?,
        "a refused write leaves the tenant's token live"
    );
    assert_eq!(
        resources.common.repos.admin.list_tokens(true).await?.len(),
        before,
        "nothing was minted"
    );

    // Within its reach: a token bound to no tenant rotates, the replacement
    // bound to none as well, and is revoked.
    let path = format!("/admin/tokens/{}/rotate", free.token_id);
    let (status, body) = post_as(&resources, &unbound, &path, json!({})).await;
    assert_eq!(status, 200, "a token bound to no tenant rotates: {body}");
    let rotated: Value = serde_json::from_str(&body)?;
    let replacement = rotated["data"]["token_id"].as_str().expect("new token id");
    assert_eq!(tenant_of(&resources, replacement).await?, None);
    let path = format!("/admin/tokens/{replacement}/revoke");
    let (status, body) = post_as(&resources, &unbound, &path, json!({})).await;
    assert_eq!(status, 200, "a token bound to no tenant is revoked: {body}");
    assert!(!is_active(&resources, replacement).await?);

    // The platform's callers are not restricted: a super-admin token rotates
    // the tenant's token, and an admin's console session revokes one.
    let root = mint(&resources, "root-desk", lister, true).await?;
    let path = format!("/admin/tokens/{}/rotate", club.token_id);
    let (status, body) = post_as(&resources, &root, &path, json!({})).await;
    assert_eq!(status, 200, "a super-admin token rotates it: {body}");
    let rotated: Value = serde_json::from_str(&body)?;
    let replacement = rotated["data"]["token_id"].as_str().expect("new token id");
    assert_eq!(
        tenant_of(&resources, replacement).await?,
        Some(tenant.to_string())
    );
    let admin = session(&resources, "admin-unbound@example.com", UserRole::Admin).await?;
    let path = format!("/api/admin/tokens/{}/revoke", club_too.token_id);
    let response = AxumTestRequest::post(&path)
        .header("authorization", &admin)
        .json(&json!({}))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 200, "an admin's session revokes it");
    assert!(!is_active(&resources, &club_too.token_id).await?);
    Ok(())
}
