// ABOUTME: Regression tests for the web-admin admin-token routes — rotate, include_inactive, revoke
// ABOUTME: Pins the rotate route's existence and super-admin gate, the inactive filter, and revoke's 404

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

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
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_web_admin::WebAdminRoutes;
use serial_test::serial;
use std::sync::Arc;

/// The console's "Rotate Key" button posted to `/api/admin/tokens/{id}/rotate`,
/// which only ever existed on the service-token router at `/admin/tokens/...`.
/// The mutation had no `onError`, so every rotation 404'd and reported nothing —
/// operators believed keys had been rotated when the old credential was still
/// live. These tests pin the route onto the cookie-auth surface, keep it behind
/// the same super-admin gate as its list/get/revoke siblings, and hold the two
/// adjacent defects: a hardcoded `list_tokens(false)` that made the Inactive
/// filter structurally empty, and a revoke that answered 200 for ids that were
/// never in the table.
const TEST_JWT_SECRET: &str = "test_jwt_secret_for_web_admin_token_routes";

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
        Some("Routes Test".to_owned()),
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

/// Seed a service token and return the full generated record, so the caller can
/// compare the pre-rotation JWT against the replacement.
async fn seed_admin_token(
    resources: &Arc<ServerContext>,
    service: &str,
) -> Result<GeneratedAdminToken> {
    let request = CreateAdminTokenRequest {
        service_name: service.to_owned(),
        service_description: Some("seeded by web_admin_token_routes_test".to_owned()),
        permissions: Some(vec![AdminPermission::ListKeys]),
        expires_in_days: Some(90),
        is_super_admin: false,
        tenant_id: None,
    };
    let generated = resources
        .common
        .repos
        .admin
        .create_token(&request, TEST_JWT_SECRET, &resources.auth.jwks_manager)
        .await?;
    Ok(generated)
}

// ===========================================================================
// Rotate — route existence and authz
// ===========================================================================

#[tokio::test]
#[serial]
async fn unauthenticated_rotate_is_rejected() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let seeded = seed_admin_token(&resources, "svc-rotate-unauth").await?;

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", seeded.token_id))
        .json(&serde_json::json!({}))
        .send(router(&resources))
        .await;

    // 404 here would mean the route is still missing — the original defect.
    assert_eq!(
        response.status(),
        401,
        "the rotate route must exist and reject an unauthenticated caller before any authz decision"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn plain_admin_cannot_rotate_a_token_and_it_stays_active() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let admin = user_with_role(
        &resources,
        "plain-admin-rotate@example.com",
        UserRole::Admin,
    )
    .await?;
    let seeded = seed_admin_token(&resources, "svc-rotate-target").await?;

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", seeded.token_id))
        .header("authorization", &admin)
        .json(&serde_json::json!({}))
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        403,
        "a plain admin must not rotate an admin token"
    );

    // Rotation deactivates before it mints. A guard that ran too late would
    // still 403 while having destroyed the operator's live credential, so the
    // status code alone is not the security property.
    let token = resources
        .common
        .repos
        .admin
        .get_token_by_id(&seeded.token_id)
        .await?
        .expect("seeded token must still exist");
    assert!(
        token.is_active,
        "the token must remain ACTIVE after a plain admin's rotate attempt"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn super_admin_rotate_issues_a_different_token_and_retires_the_old_one() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let super_admin =
        user_with_role(&resources, "super-rotate@example.com", UserRole::SuperAdmin).await?;
    let seeded = seed_admin_token(&resources, "svc-rotate-ok").await?;

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{}/rotate", seeded.token_id))
        .header("authorization", &super_admin)
        .json(&serde_json::json!({}))
        .send(router(&resources))
        .await;

    assert_eq!(
        response.status(),
        200,
        "a super-admin must be able to rotate"
    );

    let body: serde_json::Value = response.json();

    assert_eq!(body["success"], true);
    assert_eq!(
        body["old_token_id"], seeded.token_id,
        "the response must name the token that was retired"
    );
    assert_eq!(
        body["service_name"], "svc-rotate-ok",
        "the replacement must keep the rotated token's service identity"
    );

    // The whole point of a rotation: a genuinely new credential.
    let new_token_id = body["token_id"]
        .as_str()
        .expect("rotate response must carry the new token_id");
    assert_ne!(
        new_token_id, seeded.token_id,
        "rotation must mint a NEW token id, not echo the old one"
    );

    // `ApiKeyDetails` reads `data.jwt_token` off the mutation result to fill the
    // copy-once modal, so the plaintext JWT must be top-level and must not be
    // the credential that was just retired.
    let new_jwt = body["jwt_token"]
        .as_str()
        .expect("rotate response must carry the new jwt_token at the top level");
    assert!(!new_jwt.is_empty(), "the replacement JWT must not be empty");
    assert_ne!(
        new_jwt, seeded.jwt_token,
        "rotation must hand back a different JWT than the one it replaced"
    );

    // Persisted state, not just the response envelope.
    let old = resources
        .common
        .repos
        .admin
        .get_token_by_id(&seeded.token_id)
        .await?
        .expect("the rotated token row must survive as an inactive record");
    assert!(
        !old.is_active,
        "the pre-rotation token must be deactivated, or the old credential still works"
    );

    let new = resources
        .common
        .repos
        .admin
        .get_token_by_id(new_token_id)
        .await?
        .expect("the replacement token must be persisted");
    assert!(new.is_active, "the replacement token must be active");
    assert_eq!(new.service_name, "svc-rotate-ok");

    Ok(())
}

// ===========================================================================
// include_inactive — the Inactive filter was structurally empty
// ===========================================================================

#[tokio::test]
#[serial]
async fn include_inactive_controls_whether_revoked_tokens_are_listed() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let super_admin = user_with_role(
        &resources,
        "super-list-inactive@example.com",
        UserRole::SuperAdmin,
    )
    .await?;
    let live = seed_admin_token(&resources, "svc-still-live").await?;
    let doomed = seed_admin_token(&resources, "svc-revoked").await?;

    let revoke = AxumTestRequest::post(&format!("/api/admin/tokens/{}/revoke", doomed.token_id))
        .header("authorization", &super_admin)
        .send(router(&resources))
        .await;
    assert_eq!(revoke.status(), 200, "revoking a real token must succeed");

    // Default: active only.
    let default_body: serde_json::Value = AxumTestRequest::get("/api/admin/tokens")
        .header("authorization", &super_admin)
        .send(router(&resources))
        .await
        .json();
    let default_ids: Vec<&str> = default_body["admin_tokens"]
        .as_array()
        .expect("admin_tokens must be an array")
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert!(
        default_ids.contains(&live.token_id.as_str()),
        "the live token must be listed by default"
    );
    assert!(
        !default_ids.contains(&doomed.token_id.as_str()),
        "a revoked token must NOT appear in the default listing"
    );
    assert_eq!(
        default_body["total_count"], 1,
        "only the one live token should be counted by default"
    );

    // include_inactive=true: both, with the revoked one flagged.
    let all_body: serde_json::Value =
        AxumTestRequest::get("/api/admin/tokens?include_inactive=true")
            .header("authorization", &super_admin)
            .send(router(&resources))
            .await
            .json();
    let all = all_body["admin_tokens"]
        .as_array()
        .expect("admin_tokens must be an array");
    assert_eq!(
        all_body["total_count"], 2,
        "include_inactive=true must return BOTH the live and the revoked token — \
         a hardcoded list_tokens(false) returns 1 here and empties the Inactive filter"
    );

    let revoked_entry = all
        .iter()
        .find(|t| t["id"].as_str() == Some(doomed.token_id.as_str()))
        .expect("the revoked token must be present when include_inactive=true");
    assert_eq!(
        revoked_entry["is_active"], false,
        "the revoked token must carry is_active=false so the console renders its Inactive badge"
    );
    assert_eq!(revoked_entry["service_name"], "svc-revoked");

    let live_entry = all
        .iter()
        .find(|t| t["id"].as_str() == Some(live.token_id.as_str()))
        .expect("the live token must still be present when include_inactive=true");
    assert_eq!(live_entry["is_active"], true);

    Ok(())
}

// ===========================================================================
// Revoke — an unknown id is not a successful revocation
// ===========================================================================

#[tokio::test]
#[serial]
async fn revoking_an_unknown_token_id_returns_404() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let super_admin = user_with_role(
        &resources,
        "super-revoke-missing@example.com",
        UserRole::SuperAdmin,
    )
    .await?;

    let response =
        AxumTestRequest::post("/api/admin/tokens/admin_00000000000000000000000000000000/revoke")
            .header("authorization", &super_admin)
            .send(router(&resources))
            .await;

    // `deactivate_token` is an unconditional UPDATE returning `()`, so without an
    // existence check the handler reported "revoked successfully" for ids that
    // were never in the table.
    assert_eq!(
        response.status(),
        404,
        "revoking a token id that does not exist must be a 404, not a fabricated success"
    );

    let body: serde_json::Value = response.json();
    assert_ne!(
        body["success"], true,
        "a miss must never be reported as a successful revocation"
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn revoking_a_real_token_deactivates_it() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let super_admin = user_with_role(
        &resources,
        "super-revoke-real@example.com",
        UserRole::SuperAdmin,
    )
    .await?;
    let seeded = seed_admin_token(&resources, "svc-revoke-real").await?;

    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{}/revoke", seeded.token_id))
        .header("authorization", &super_admin)
        .send(router(&resources))
        .await;

    // The 404 guard must not deny the right people: a real id still revokes.
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json();
    assert_eq!(body["success"], true);
    assert_eq!(body["token_id"], seeded.token_id);

    let token = resources
        .common
        .repos
        .admin
        .get_token_by_id(&seeded.token_id)
        .await?
        .expect("the revoked token row must still exist");
    assert!(!token.is_active, "the token must actually be deactivated");

    Ok(())
}
