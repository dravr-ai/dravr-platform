// ABOUTME: Authorization tests for the admin feature-flag endpoints — token-based authZ only
// ABOUTME: Regression guard for the prod 403 where authZ wrongly re-derived from the DB role
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//! These tests pin the authorization contract of the admin feature-flag
//! handlers after they were aligned with every sibling admin endpoint:
//!
//! - Reads/writes gate on the [`ValidatedAdminToken`] permission set
//!   (`ViewConfiguration` / `ManageConfiguration`), never the DB role.
//! - Tenant-default routes additionally enforce
//!   [`ValidatedAdminToken::require_tenant_access`] so a tenant-scoped token
//!   cannot reach another tenant's defaults; super-admin tokens pass through.
//! - User-override routes are global (like `/api/admin/users/...`).
//!
//! The prod bug they guard against: a cookie admin (super-admin token) whose
//! underlying DB user was `Admin` (not `SuperAdmin`) got a 403 because the
//! handler re-loaded the DB role instead of trusting the token.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::Utc;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::admin::models::{AdminPermission, AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::ErrorCode;
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::{CoachingPersona, Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::handlers::feature_flags::{
    handle_admin_list_tenant_defaults, handle_admin_list_user_overrides,
    handle_admin_set_tenant_default, SetFeatureFlagRequest,
};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use uuid::Uuid;

/// Build a fully-wired [`AdminApiContext`] backed by a fresh test database,
/// returning it alongside the repository registry used for seeding.
async fn build_context() -> (
    Arc<AdminApiContext>,
    Arc<pierre_database::RepositoryRegistry>,
) {
    let database = common::create_test_database().await.unwrap();
    let auth_manager = common::create_test_auth_manager();
    let jwks_manager = common::get_shared_test_jwks();

    let database_arc = Arc::new((*database).clone());
    let repos_arc = Arc::new(database_arc.repositories());

    let context = AdminApiContext::new(AdminApiContextInit {
        database: database_arc,
        repos: repos_arc.clone(),
        jwt_secret: "test_admin_jwt_secret_for_feature_flag_authz".to_owned(),
        auth_manager,
        jwks_manager,
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(pierre_contremaitre::PromptRegistry::new()),
        tool_description_registry: Arc::new(pierre_contremaitre::ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(pierre_contremaitre::EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(pierre_contremaitre::MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        contremaitre_config: None,
    });

    (Arc::new(context), repos_arc)
}

/// Seed a tenant (and its owner user) and return the tenant id.
async fn seed_tenant(repos: &pierre_database::RepositoryRegistry) -> TenantId {
    let user_id = Uuid::new_v4();
    let tenant_id = TenantId::new();
    let now = Utc::now();
    let user = User {
        id: user_id,
        email: format!("owner+{user_id}@example.com"),
        display_name: None,
        password_hash: bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: Some(user_id),
        approved_at: Some(now),
        created_at: now,
        last_active: now,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "en".to_owned(),
        default_coach_id: None,
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
    };
    repos.users.create(&user).await.unwrap();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant {tenant_id}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: now,
        updated_at: now,
    };
    repos.tenants.create(&tenant).await.unwrap();
    repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    tenant_id
}

/// Cookie-style super-admin token, mirroring what `cookie_admin_middleware`
/// synthesizes for every web admin.
fn super_admin_token() -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("cookie:{}", Uuid::new_v4()),
        service_name: "admin@example.com".to_owned(),
        permissions: AdminPermissions::super_admin(),
        is_super_admin: true,
        tenant_id: None,
        user_info: None,
    }
}

/// Tenant-scoped admin token: holds the configuration permissions a real
/// config-managing admin carries, but is bound to a single tenant and is NOT
/// super-admin — so `require_permission` passes and the denial (if any) comes
/// from `require_tenant_access`, the boundary under test.
fn tenant_scoped_token(tenant_id: TenantId) -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("cookie:{}", Uuid::new_v4()),
        service_name: "tenant-admin@example.com".to_owned(),
        permissions: AdminPermissions::new(vec![
            AdminPermission::ViewConfiguration,
            AdminPermission::ManageConfiguration,
        ]),
        is_super_admin: false,
        tenant_id: Some(tenant_id.0.to_string()),
        user_info: None,
    }
}

#[tokio::test]
async fn super_admin_lists_any_tenant_defaults() {
    let (context, repos) = build_context().await;
    let tenant = seed_tenant(&repos).await;
    repos
        .feature_flags
        .set_tenant_default(tenant.0, FeatureKey::ApiTokens, true, None)
        .await
        .unwrap();

    let resp = handle_admin_list_tenant_defaults(
        State(context),
        Extension(super_admin_token()),
        Path(tenant.0.to_string()),
    )
    .await
    .expect("super-admin should list any tenant's defaults");
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn tenant_admin_lists_own_tenant_defaults() {
    let (context, repos) = build_context().await;
    let tenant = seed_tenant(&repos).await;

    let resp = handle_admin_list_tenant_defaults(
        State(context),
        Extension(tenant_scoped_token(tenant)),
        Path(tenant.0.to_string()),
    )
    .await
    .expect("tenant admin should list its own tenant's defaults");
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn tenant_admin_denied_cross_tenant_defaults() {
    let (context, repos) = build_context().await;
    let tenant_a = seed_tenant(&repos).await;
    let tenant_b = seed_tenant(&repos).await;

    let err = handle_admin_list_tenant_defaults(
        State(context),
        Extension(tenant_scoped_token(tenant_a)),
        Path(tenant_b.0.to_string()),
    )
    .await
    .expect_err("tenant admin must not read another tenant's defaults");
    assert_eq!(err.code, ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn tenant_admin_denied_cross_tenant_default_write() {
    let (context, repos) = build_context().await;
    let tenant_a = seed_tenant(&repos).await;
    let tenant_b = seed_tenant(&repos).await;

    let err = handle_admin_set_tenant_default(
        State(context),
        Extension(tenant_scoped_token(tenant_a)),
        Path((
            tenant_b.0.to_string(),
            FeatureKey::ApiTokens.as_str().to_owned(),
        )),
        Json(SetFeatureFlagRequest { enabled: true }),
    )
    .await
    .expect_err("tenant admin must not write another tenant's defaults");
    assert_eq!(err.code, ErrorCode::PermissionDenied);
}

/// Regression: a cookie super-admin token reads user overrides regardless of
/// the DB role of the underlying admin. Previously the handler re-loaded the
/// DB role and returned 403 for an `Admin` (non-`SuperAdmin`) operator.
#[tokio::test]
async fn admin_lists_user_overrides_without_db_role_lookup() {
    let (context, _repos) = build_context().await;
    // A user id that is NOT seeded as an admin anywhere — proves the handler
    // never performs an admin-role DB lookup.
    let target_user = Uuid::new_v4();

    let resp = handle_admin_list_user_overrides(
        State(context),
        Extension(super_admin_token()),
        Path(target_user.to_string()),
    )
    .await
    .expect("admin should list user overrides via token authZ alone");
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_without_view_configuration_is_denied() {
    let (context, _repos) = build_context().await;
    let target_user = Uuid::new_v4();

    // Permission set deliberately omits ViewConfiguration; not super-admin.
    let token = ValidatedAdminToken {
        token_id: format!("cookie:{}", Uuid::new_v4()),
        service_name: "weak-admin@example.com".to_owned(),
        permissions: AdminPermissions::new(vec![AdminPermission::ProvisionKeys]),
        is_super_admin: false,
        tenant_id: None,
        user_info: None,
    };

    let err = handle_admin_list_user_overrides(
        State(context),
        Extension(token),
        Path(target_user.to_string()),
    )
    .await
    .expect_err("ViewConfiguration is required to read feature flags");
    assert_eq!(err.code, ErrorCode::PermissionDenied);
}
