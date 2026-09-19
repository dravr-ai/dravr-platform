// ABOUTME: Authorization tests for /api/admin/config — the global operator model, pinned
// ABOUTME: A plain Admin reads and writes every scope and the audit row names the client; non-admins are refused
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # `/api/admin/config/*` is the operator's surface
//!
//! The admin console is a global operator model and the operator account
//! is a plain `Admin` (a `SuperAdmin` requirement was ruled out on
//! 2026-06-03). So `require_admin` — the global role — is the whole gate:
//! an `Admin` reads and writes the system-wide document, any tenant's
//! overrides and any user's, and the integration suite drives exactly that
//! with `pierre-cli user create` accounts. What the handlers owe on top is
//! an honest audit row: the client address and agent the request carried,
//! read back through the audit endpoint whose SELECT used to omit the
//! `user_id` column it then read, and panicked on the first row.
//!
//! Every assertion pins a status AND the written value or the refusal
//! text, so a gate that refuses an operator (or admits an athlete) fails
//! the test rather than reading as "still 200".

mod common;
mod helpers;

use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::config::routes::{admin_config_router, AdminConfigState};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::auth::service::AdminAuthService;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// A real catalogued, runtime-configurable integer parameter (range 1..=20).
const PARAMETER: &str = "usage_quotas.max_coaches_per_user";

/// Build the `/api/admin/config` router exactly as `multitenant.rs` nests it.
/// A router is consumed by each `send`, so callers build one per request.
fn router(resources: &Arc<ServerContext>) -> axum::Router {
    let service = resources
        .agent
        .admin_config
        .as_ref()
        .expect("test resources initialise the admin config service");
    let admin_auth = AdminAuthService::new(
        Arc::clone(&resources.common.repos.admin),
        Arc::clone(&resources.auth.jwks_manager),
        0,
    );
    let state = Arc::new(AdminConfigState::new(
        Arc::clone(service),
        Arc::clone(resources),
        admin_auth,
    ));
    axum::Router::new().nest("/api/admin/config", admin_config_router(state))
}

/// Create a user with the given role owning a dedicated tenant, and return
/// `(user_id, tenant_id, "Bearer <jwt>")` — the JWT carries that tenant as
/// `active_tenant_id`, which is what binds a plain Admin's token to it.
async fn create_user_with_role(
    resources: &Arc<ServerContext>,
    email: &str,
    role: UserRole,
) -> (Uuid, TenantId, String) {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap();

    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Config Admin".to_owned()),
    );
    user.is_admin = role.is_admin_or_higher();
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

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

fn update_body(value: i64) -> Value {
    json!({
        "parameters": { PARAMETER: value },
        "reason": "authz test"
    })
}

/// The effective value of [`PARAMETER`] as the catalog reports it for `query`.
async fn effective_value(resources: &Arc<ServerContext>, auth: &str, query: &str) -> i64 {
    let response = AxumTestRequest::get(&format!("/api/admin/config/catalog{query}"))
        .header("authorization", auth)
        .send(router(resources))
        .await;
    assert_eq!(response.status(), 200, "catalog read must succeed");
    let body: Value = response.json();
    body["data"]["categories"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|category| category["parameters"].as_array().unwrap())
        .find(|parameter| parameter["key"] == PARAMETER)
        .and_then(|parameter| parameter["current_value"].as_i64())
        .expect("the catalog lists the parameter with an integer current value")
}

// ============================================================================
// The operator writes and reads every scope
// ============================================================================

#[tokio::test]
async fn an_admin_writes_a_system_wide_override_and_the_audit_row_names_the_client() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, _, auth) =
        create_user_with_role(&resources, "config-operator@authz.test", UserRole::Admin).await;

    let response = AxumTestRequest::put("/api/admin/config")
        .header("authorization", &auth)
        .header("x-forwarded-for", "203.0.113.7, 10.0.0.1")
        .header("user-agent", "authz-test/1.0")
        .json(&update_body(5))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200, "a plain Admin is the operator");
    let body: Value = response.json();
    assert_eq!(body["success"], true, "write must apply: {body}");
    assert_eq!(
        body["data"]["updated_count"], 1,
        "one parameter written: {body}"
    );

    assert_eq!(
        effective_value(&resources, &auth, "").await,
        5,
        "the system-wide override must be the value just written"
    );

    // The audit row carries the client address and agent the handler read
    // from the request, and the audit endpoint returns it — its SELECT once
    // omitted the user_id column it read, and panicked on the first row.
    let audit = AxumTestRequest::get("/api/admin/config/audit")
        .header("authorization", &auth)
        .send(router(&resources))
        .await;
    assert_eq!(audit.status(), 200);
    let audit: Value = audit.json();
    let entry = audit["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["config_key"] == PARAMETER)
        .expect("the write must leave an audit entry");
    assert_eq!(
        entry["ip_address"], "203.0.113.7",
        "first x-forwarded-for hop"
    );
    assert_eq!(entry["user_agent"], "authz-test/1.0");
    assert_eq!(entry["new_value"], 5);
}

#[tokio::test]
async fn an_admin_reads_and_writes_another_tenants_scope() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, _, admin_auth) =
        create_user_with_role(&resources, "config-admin-a@authz.test", UserRole::Admin).await;
    let (_, other_tenant, _) =
        create_user_with_role(&resources, "config-owner-b@authz.test", UserRole::Admin).await;

    let query = format!("?tenant_id={other_tenant}");
    for path in [
        format!("/api/admin/config/catalog{query}"),
        format!("/api/admin/config{query}"),
        format!("/api/admin/config/category/usage_quotas{query}"),
    ] {
        let response = AxumTestRequest::get(&path)
            .header("authorization", &admin_auth)
            .send(router(&resources))
            .await;
        assert_eq!(
            response.status(),
            200,
            "{path}: the operator reads any tenant"
        );
    }

    let response = AxumTestRequest::put(&format!("/api/admin/config{query}"))
        .header("authorization", &admin_auth)
        .json(&update_body(9))
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200, "the operator writes any tenant");
    let body: Value = response.json();
    assert_eq!(body["data"]["updated_count"], 1, "{body}");

    assert_eq!(
        effective_value(&resources, &admin_auth, &query).await,
        9,
        "the tenant override must resolve for that tenant"
    );
    assert_eq!(
        effective_value(&resources, &admin_auth, "").await,
        3,
        "a tenant override must leave the system-wide default alone"
    );
}

// ============================================================================
// Authentication
// ============================================================================

#[tokio::test]
async fn unauthenticated_and_non_admin_callers_are_refused() {
    let resources = create_test_server_resources().await.unwrap();

    let anonymous = AxumTestRequest::get("/api/admin/config/catalog")
        .send(router(&resources))
        .await;
    assert_eq!(anonymous.status(), 401);

    let (_, _, user_auth) =
        create_user_with_role(&resources, "config-athlete@authz.test", UserRole::User).await;
    for (method, path) in [
        ("get", "/api/admin/config/catalog"),
        ("put", "/api/admin/config"),
        ("post", "/api/admin/config/reset"),
    ] {
        let request = match method {
            "get" => AxumTestRequest::get(path),
            "put" => AxumTestRequest::put(path).json(&update_body(4)),
            _ => AxumTestRequest::post(path).json(&json!({ "category": "usage_quotas" })),
        };
        let athlete = request
            .header("authorization", &user_auth)
            .send(router(&resources))
            .await;
        assert_eq!(athlete.status(), 403, "{method} {path}");
        let body: Value = athlete.json();
        assert_eq!(body["message"], "Admin privileges required", "{body}");
    }
}
