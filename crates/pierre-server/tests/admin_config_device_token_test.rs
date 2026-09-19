// ABOUTME: Tests /api/admin/config with the admin token a device login mints — what pierre-cli holds after auth login
// ABOUTME: A super-admin device token is the approving operator; a service token names no one and is refused
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use axum::http::StatusCode;
use axum::Router;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_core::admin::models::{CreateAdminTokenRequest, DEVICE_CLI_SERVICE_PREFIX};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_mcp_server::config::routes::{admin_config_router, AdminConfigState};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::auth::service::AdminAuthService;
use serde_json::{json, Value};
use std::future::Future;
use std::sync::Arc;
use uuid::Uuid;

const KEY: &str = "usage_quotas.max_active_conversations";

/// The config routes exactly as the server mounts them, with the admin-token
/// validator the admin routes share.
fn router(resources: &Arc<ServerContext>) -> Router {
    let service = resources
        .agent
        .admin_config
        .clone()
        .expect("test resources carry the admin config service");
    let admin_auth = AdminAuthService::new(
        Arc::clone(&resources.common.repos.admin),
        Arc::clone(&resources.auth.jwks_manager),
        0,
    );
    let state = Arc::new(AdminConfigState::new(
        service,
        Arc::clone(resources),
        admin_auth,
    ));
    Router::new().nest("/api/admin/config", admin_config_router(state))
}

/// An active user with `role` and a tenant of their own.
async fn user_with_role(resources: &Arc<ServerContext>, role: UserRole) -> User {
    let email = format!("cfg-{}@dravr.test", Uuid::new_v4());
    let mut user = User::new(
        email.clone(),
        bcrypt::hash("password123", 4).unwrap(),
        Some("Config Test".to_owned()),
    );
    user.is_admin = role.is_admin_or_higher();
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant for {email}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user.id,
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
        .update_tenant_id(user.id, tenant_id)
        .await
        .unwrap();
    user
}

/// Mint an admin token the way the device grant does (`super_admin` = true)
/// or the way `token generate` does, and return its `Bearer` header.
async fn admin_token(resources: &Arc<ServerContext>, service: &str, super_admin: bool) -> String {
    let request = if super_admin {
        CreateAdminTokenRequest::super_admin(service.to_owned())
    } else {
        CreateAdminTokenRequest::new(service.to_owned())
    };
    let minted = resources
        .common
        .repos
        .admin
        .create_token(
            &request,
            &resources.auth.admin_jwt_secret,
            &resources.auth.jwks_manager,
        )
        .await
        .unwrap();
    format!("Bearer {}", minted.jwt_token)
}

/// Who the override rows for `KEY` at `user_id` scope are audited as — one
/// entry per row, so an empty list is "nothing was written".
async fn override_authors(resources: &Arc<ServerContext>, user_id: Uuid) -> Vec<String> {
    let user = user_id.to_string();
    match &*resources.agent.database {
        Database::SQLite(sqlite) => sqlx::query_scalar(
            "SELECT created_by FROM admin_config_overrides WHERE config_key = $1 AND user_id = $2",
        )
        .bind(KEY)
        .bind(&user)
        .fetch_all(sqlite.pool())
        .await
        .unwrap(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => sqlx::query_scalar(
            "SELECT CAST(created_by AS TEXT) FROM admin_config_overrides \
             WHERE config_key = $1 AND user_id = $2::uuid",
        )
        .bind(KEY)
        .bind(&user)
        .fetch_all(pg.pool())
        .await
        .unwrap(),
    }
}

fn put_for(auth: &str, user_id: Uuid, router: Router) -> impl Future<Output = AxumTestResponse> {
    AxumTestRequest::put(&format!("/api/admin/config?user_id={user_id}"))
        .header("authorization", auth)
        .json(&json!({
            "parameters": { KEY: 0 },
            "reason": "admin_config_device_token_test",
        }))
        .send(router)
}

#[tokio::test]
async fn a_device_login_token_reads_and_writes_as_the_approving_super_admin() {
    let resources = create_test_server_resources().await.unwrap();
    let operator = user_with_role(&resources, UserRole::SuperAdmin).await;
    let athlete = user_with_role(&resources, UserRole::User).await;
    let device_token = admin_token(
        &resources,
        &format!("{DEVICE_CLI_SERVICE_PREFIX}{}", operator.email),
        true,
    )
    .await;

    let catalog = AxumTestRequest::get("/api/admin/config/catalog")
        .header("authorization", &device_token)
        .send(router(&resources))
        .await;
    assert_eq!(catalog.status_code(), StatusCode::OK, "{}", catalog.text());

    let written = put_for(&device_token, athlete.id, router(&resources)).await;
    assert_eq!(written.status_code(), StatusCode::OK, "{}", written.text());
    let body: Value = written.json();
    let data = body.get("data").unwrap_or(&body);
    assert_eq!(data["updated_count"], 1, "{body}");
    assert_eq!(data["validation_errors"], json!([]), "{body}");

    assert_eq!(
        override_authors(&resources, athlete.id).await,
        vec![operator.id.to_string()],
        "the write is audited as the super-admin who approved the device login"
    );
}

#[tokio::test]
async fn a_service_token_names_no_operator_and_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = user_with_role(&resources, UserRole::User).await;
    let service_token = admin_token(&resources, "ops-bot", true).await;

    // Auth failures ship sanitized, so the reason is not on the wire; the
    // effect is: a super-admin service token writes nothing.
    let refused = put_for(&service_token, athlete.id, router(&resources)).await;
    assert_eq!(refused.status_code(), StatusCode::UNAUTHORIZED);
    assert!(
        override_authors(&resources, athlete.id).await.is_empty(),
        "a token that names no operator must not write an audited row"
    );
}

#[tokio::test]
async fn a_device_prefixed_token_that_is_not_super_admin_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let operator = user_with_role(&resources, UserRole::SuperAdmin).await;
    let athlete = user_with_role(&resources, UserRole::User).await;
    let weak_token = admin_token(
        &resources,
        &format!("{DEVICE_CLI_SERVICE_PREFIX}{}", operator.email),
        false,
    )
    .await;

    let refused = put_for(&weak_token, athlete.id, router(&resources)).await;
    assert_eq!(refused.status_code(), StatusCode::UNAUTHORIZED);
    assert!(
        override_authors(&resources, athlete.id).await.is_empty(),
        "a device-shaped name does not stand in for the super-admin flag"
    );
}

#[tokio::test]
async fn the_admin_consoles_user_jwt_still_works_and_a_plain_user_is_still_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let operator = user_with_role(&resources, UserRole::SuperAdmin).await;
    let athlete = user_with_role(&resources, UserRole::User).await;

    let operator_jwt = format!(
        "Bearer {}",
        generate_test_token(&resources, &operator).await
    );
    let ok = put_for(&operator_jwt, athlete.id, router(&resources)).await;
    assert_eq!(ok.status_code(), StatusCode::OK, "{}", ok.text());

    let athlete_jwt = format!("Bearer {}", generate_test_token(&resources, &athlete).await);
    let refused = put_for(&athlete_jwt, athlete.id, router(&resources)).await;
    assert_ne!(
        refused.status_code(),
        StatusCode::OK,
        "a non-admin session must not write config"
    );

    let garbage = put_for("Bearer not-a-token", athlete.id, router(&resources)).await;
    assert_eq!(garbage.status_code(), StatusCode::UNAUTHORIZED);
}
