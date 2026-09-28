// ABOUTME: Per-tenant tool selection through one handler set, over the console and admin-token mounts
// ABOUTME: Tenant admins reach their own tenant only, and an override is attributed to its operator

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The console's tool-availability tab used to have its own handlers in
//! `pierre-routes-web-admin`, twins of `ToolSelectionRoutes` (carnet#604). The
//! twins disagreed: the console let a tenant's admins manage its tools, the
//! token API demanded a super-admin; and the token API attributed an override
//! by parsing the token *id* as a user id, which no admin token id is, so every
//! override it was asked to set failed.
//!
//! One handler set now serves `/api/admin/tools` (session) and `/admin/tools`
//! (admin token). These tests go through the production composition and assert
//! on the stored overrides.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::models::{CreateAdminTokenRequest, GeneratedAdminToken};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use serial_test::serial;
use uuid::Uuid;

/// A tool every catalogue carries.
const TOOL: &str = "get_activities";

/// A signed-in admin: the bearer header, the user id, the tenant it owns.
struct Session {
    auth: String,
    user_id: Uuid,
    tenant_id: TenantId,
}

async fn session(resources: &Arc<ServerContext>, email: &str, role: UserRole) -> Result<Session> {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Tools Test".to_owned()),
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
            plan: "enterprise".to_owned(),
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

    Ok(Session {
        auth: format!("Bearer {}", generate_test_token(resources, &user).await),
        user_id,
        tenant_id,
    })
}

/// Mint a super-admin token; `operator` makes it the shape a device login mints.
async fn super_token(
    resources: &Arc<ServerContext>,
    service: &str,
    operator: Option<Uuid>,
) -> Result<GeneratedAdminToken> {
    let request = CreateAdminTokenRequest {
        service_name: service.to_owned(),
        service_description: None,
        permissions: None,
        expires_in_days: Some(1),
        is_super_admin: true,
        tenant_id: None,
        operator_user_id: operator,
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

/// The production composition, one per request.
fn app(resources: &Arc<ServerContext>) -> Router {
    ProviderToolRouter::build_http_app(resources)
}

async fn stored_override(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
) -> Result<Option<(bool, Option<Uuid>)>> {
    let overrides = resources
        .common
        .repos
        .tool_selection
        .get_overrides(tenant_id)
        .await?;
    Ok(overrides
        .into_iter()
        .find(|o| o.tool_name == TOOL)
        .map(|o| (o.is_enabled, o.enabled_by_user_id)))
}

/// The console's tab: a plain Admin manages its own tenant's tools, and the
/// override is attributed to it.
#[tokio::test]
#[serial]
async fn a_plain_admin_manages_its_own_tenants_tools() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "tenant-admin@example.com", UserRole::Admin).await?;
    let tenant = admin.tenant_id;

    let response = AxumTestRequest::get(&format!("/api/admin/tools/tenant/{tenant}"))
        .header("authorization", &admin.auth)
        .send(app(&resources))
        .await;
    assert_eq!(response.status(), 200, "a tenant admin reads its tools");
    let body: Value = response.json();
    assert!(
        body["data"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(|t| t["tool_name"] == TOOL)),
        "the effective list carries the catalogue: {body}"
    );

    let response = AxumTestRequest::post(&format!("/api/admin/tools/tenant/{tenant}/override"))
        .header("authorization", &admin.auth)
        .json(&json!({ "tool_name": TOOL, "is_enabled": false, "reason": "pilot" }))
        .send(app(&resources))
        .await;
    assert_eq!(response.status(), 200, "a tenant admin sets an override");
    assert_eq!(
        stored_override(&resources, tenant).await?,
        Some((false, Some(admin.user_id))),
        "the override is stored and attributed to the signed-in admin"
    );

    let response = AxumTestRequest::get("/api/admin/tools/global-disabled")
        .header("authorization", &admin.auth)
        .send(app(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "every admin reads the global switch"
    );
    let body: Value = response.json();
    assert!(body["data"]["disabled_tools"].is_array());

    let response =
        AxumTestRequest::delete(&format!("/api/admin/tools/tenant/{tenant}/override/{TOOL}"))
            .header("authorization", &admin.auth)
            .send(app(&resources))
            .await;
    assert_eq!(
        response.status(),
        200,
        "a tenant admin removes its override"
    );
    assert_eq!(stored_override(&resources, tenant).await?, None);
    Ok(())
}

/// Membership is the gate: another tenant's tools are out of a plain Admin's
/// reach, and nothing is written.
#[tokio::test]
#[serial]
async fn a_plain_admin_cannot_touch_another_tenants_tools() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "own-tenant@example.com", UserRole::Admin).await?;
    let other = session(&resources, "other-tenant@example.com", UserRole::Admin).await?;

    let response = AxumTestRequest::post(&format!(
        "/api/admin/tools/tenant/{}/override",
        other.tenant_id
    ))
    .header("authorization", &admin.auth)
    .json(&json!({ "tool_name": TOOL, "is_enabled": false }))
    .send(app(&resources))
    .await;
    assert_eq!(response.status(), 403);
    let body: Value = response.json();
    assert_eq!(
        body["message"].as_str(),
        Some("Tool settings of this tenant are managed by its own admins"),
        "{body}"
    );
    assert_eq!(stored_override(&resources, other.tenant_id).await?, None);

    let response = AxumTestRequest::get(&format!("/api/admin/tools/tenant/{}", other.tenant_id))
        .header("authorization", &admin.auth)
        .send(app(&resources))
        .await;
    assert_eq!(response.status(), 403);
    Ok(())
}

/// The token API parsed the token id as the override's author, so a device
/// login could not set an override at all. The operator behind the token is
/// the author now, and a token that acts for no one is told so.
#[tokio::test]
#[serial]
async fn the_token_api_attributes_an_override_to_its_operator() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let owner = session(&resources, "tenant-owner@example.com", UserRole::User).await?;
    let operator = session(
        &resources,
        "device-operator@example.com",
        UserRole::SuperAdmin,
    )
    .await?;
    let device = super_token(&resources, "device-login-shaped", Some(operator.user_id)).await?;
    let service = super_token(&resources, "service-token", None).await?;
    let tenant = owner.tenant_id;

    let response = AxumTestRequest::post(&format!("/admin/tools/tenant/{tenant}/override"))
        .header("authorization", &format!("Bearer {}", device.jwt_token))
        .json(&json!({ "tool_name": TOOL, "is_enabled": false }))
        .send(app(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "a device-login token sets an override"
    );
    assert_eq!(
        stored_override(&resources, tenant).await?,
        Some((false, Some(operator.user_id))),
        "attributed to the approving operator"
    );

    let response = AxumTestRequest::post(&format!("/admin/tools/tenant/{tenant}/override"))
        .header("authorization", &format!("Bearer {}", service.jwt_token))
        .json(&json!({ "tool_name": TOOL, "is_enabled": true }))
        .send(app(&resources))
        .await;
    assert_eq!(
        response.status(),
        400,
        "a token acting for no one is refused"
    );
    assert_eq!(
        stored_override(&resources, tenant).await?,
        Some((false, Some(operator.user_id))),
        "the refused write changed nothing"
    );
    Ok(())
}
