// ABOUTME: Per-tenant tool selection through the console's session mount
// ABOUTME: An admin reaches every tenant, a user none, and an override is attributed to its admin

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The console's tool-availability tab: `ToolSelectionRoutes::console_routes`
//! serves `/api/admin/tools` behind the console's session. These tests go
//! through the production composition and assert on the stored overrides.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use common::{create_test_server_resources, generate_test_token};
use dravr_tronc::dates::Timestamp;
use helpers::axum_test::AxumTestRequest;
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
            plan: "enterprise".to_owned(),
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

    Ok(Session {
        auth: format!("Bearer {}", generate_test_token(resources, &user).await),
        user_id,
        tenant_id,
    })
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

/// The console's tab: an admin manages its own tenant's tools, and the
/// override is attributed to it.
#[tokio::test]
#[serial]
async fn an_admin_manages_its_own_tenants_tools() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "tenant-admin@example.com", UserRole::Admin).await?;
    let tenant = admin.tenant_id;

    let response = AxumTestRequest::get(&format!("/api/admin/tools/tenant/{tenant}"))
        .header("authorization", &admin.auth)
        .send(app(&resources))
        .await;
    assert_eq!(
        response.status(),
        200,
        "an admin reads its own tenant's tools"
    );
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
    assert_eq!(
        response.status(),
        200,
        "an admin sets an override on its own tenant"
    );
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
        "an admin removes the override on its own tenant"
    );
    assert_eq!(stored_override(&resources, tenant).await?, None);
    Ok(())
}

/// An admin reaches every tenant's tools (carnet#1567: one admin role, with
/// cross-tenant reach). What is still refused: an account that is not an
/// admin, before any handler runs, and a service token minted without the
/// super-admin flag, which is bound to no tenant. Neither writes anything.
#[tokio::test]
#[serial]
async fn another_tenants_tools_are_an_admins_and_no_one_elses() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = session(&resources, "own-tenant@example.com", UserRole::Admin).await?;
    let other = session(&resources, "other-tenant@example.com", UserRole::User).await?;
    let outsider = session(&resources, "outsider@example.com", UserRole::User).await?;

    // A user's session is not an admin's: refused on the other tenant and on
    // its own, since the console is for admins.
    for tenant in [other.tenant_id, outsider.tenant_id] {
        let response = AxumTestRequest::post(&format!("/api/admin/tools/tenant/{tenant}/override"))
            .header("authorization", &outsider.auth)
            .json(&json!({ "tool_name": TOOL, "is_enabled": false }))
            .send(app(&resources))
            .await;
        assert_eq!(response.status(), 403, "a user is not an admin");
        assert_eq!(stored_override(&resources, tenant).await?, None);
    }

    // The admin does reach it, and the override is attributed to the admin.
    let response = AxumTestRequest::get(&format!("/api/admin/tools/tenant/{}", other.tenant_id))
        .header("authorization", &admin.auth)
        .send(app(&resources))
        .await;
    assert_eq!(response.status(), 200, "an admin reads any tenant's tools");
    let response = AxumTestRequest::post(&format!(
        "/api/admin/tools/tenant/{}/override",
        other.tenant_id
    ))
    .header("authorization", &admin.auth)
    .json(&json!({ "tool_name": TOOL, "is_enabled": false }))
    .send(app(&resources))
    .await;
    assert_eq!(
        response.status(),
        200,
        "an admin sets any tenant's override"
    );
    assert_eq!(
        stored_override(&resources, other.tenant_id).await?,
        Some((false, Some(admin.user_id))),
    );
    Ok(())
}
