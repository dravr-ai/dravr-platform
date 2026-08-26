// ABOUTME: Cookie-surface coverage for /api/admin/pre-approved-emails — the console's add-a-user path
// ABOUTME: Allow attributes the signed-in admin, listing reports it, and a plain user is refused
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The admin web app's half of the pre-approval allow-list.
//!
//! The console had no way to add a user from an email address: every path
//! required the person to self-register first and land in the pending queue.
//! These routes are the cookie twins of the bearer endpoints `pierre-cli user
//! allow` drives, sharing `pierre_services::pre_approval`, so an allow means
//! the same thing whichever surface records it — and here the operator is the
//! signed-in admin, so attribution is exact.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_web_admin::WebAdminRoutes;
use serde_json::{json, Value};
use serial_test::serial;
use std::sync::Arc;

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
        Some("Pre-approval Test".to_owned()),
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
