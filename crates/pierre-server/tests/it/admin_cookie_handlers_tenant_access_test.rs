// ABOUTME: Tenant-access tests for the cookie-admin observability handlers ( grading, memory, myths)
// ABOUTME: A tenant-bound token is refused for another tenant at every site; its own tenant and a super-admin pass
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! These handlers under the cookie-admin mount take a client-supplied
//! `tenant_id` query. Grading, memory and myth busting gate on the scope rule
//! (`pierre_core::admin::scope`). A token bound to one tenant must not cross to
//! another.
//!
//! The tests drive the handlers directly (the `feature_flags_admin_authz_test`
//! pattern) with a token that holds the permissions but is bound to one
//! tenant, so the refusal can only come from the tenant gate under test, and
//! they pin its message, which both gates share.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use pierre_config::mcp::AppBehaviorConfig;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Extension;
use chrono::Utc;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::admin::models::{AdminPermission, AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{CoachingPersona, Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_routes_admin::handlers::{agent_grading, memory_worker, myth_busting};
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
    let repos_arc = Arc::clone(database_arc.repositories());

    let context = AdminApiContext::new(AdminApiContextInit {
        database: database_arc,
        repos: repos_arc.clone(),
        jwt_secret: "test_admin_jwt_secret_for_tenant_access".to_owned(),
        auth_manager,
        jwks_manager,
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(pierre_contremaitre::PromptRegistry::new()),
        tool_description_registry: Arc::new(pierre_contremaitre::ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(pierre_contremaitre::EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(pierre_contremaitre::MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        training_catalogue_registry: Arc::new(pierre_contremaitre::TrainingCatalogueRegistry::new()),
        contremaitre_config: None,
        app_behavior: AppBehaviorConfig::default(),
    });

    (Arc::new(context), repos_arc)
}

/// Seed a tenant (and its owner user) and return the tenant id.
async fn seed_tenant(repos: &pierre_database::RepositoryRegistry) -> TenantId {
    let user_id = Uuid::new_v4();
    let tenant_id = TenantId::generate();
    let now = Utc::now();
    let user = User {
        id: user_id,
        email: format!("owner+{user_id}@example.com"),
        display_name: None,
        password_hash: bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
        strava_token: None,
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
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
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
/// synthesizes for a `SuperAdmin`.
fn super_admin_token() -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("cookie:{}", Uuid::new_v4()),
        service_name: "admin@example.com".to_owned(),
        permissions: AdminPermissions::super_admin(),
        is_super_admin: true,
        tenant_id: None,
        user_info: None,
        operator_user_id: None,
    }
}

/// Tenant-bound admin token: holds every permission these handlers ask for,
/// but is bound to one tenant and is NOT super-admin — so `require_permission`
/// passes and a refusal can only come from the tenant gate.
fn tenant_bound_token(tenant_id: TenantId) -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("admin-token:{}", Uuid::new_v4()),
        service_name: "tenant-admin@example.com".to_owned(),
        permissions: AdminPermissions::new(vec![
            AdminPermission::ViewConfiguration,
            AdminPermission::ManageConfiguration,
        ]),
        is_super_admin: false,
        tenant_id: Some(tenant_id.as_uuid().to_string()),
        user_info: None,
        operator_user_id: None,
    }
}

/// The refusal `require_tenant_access` writes for a bound token naming
/// another tenant.
fn assert_cross_tenant_refusal(err: &AppError, bound: TenantId, requested: TenantId, site: &str) {
    assert_eq!(
        err.code,
        ErrorCode::PermissionDenied,
        "{site}: cross-tenant access is a permission failure"
    );
    assert_eq!(
        err.message,
        format!("Token is scoped to tenant {bound}, cannot access tenant {requested}"),
        "{site}: the refusal must come from the tenant gate, not the permission gate"
    );
}

/// The error a handler call produced — the handlers return
/// `AppResult<impl IntoResponse>`, whose `Ok` is not `Debug`, so `expect_err`
/// cannot be used on them directly.
fn refusal<T>(result: Result<T, AppError>, site: &str) -> AppError {
    match result {
        Err(err) => err,
        Ok(_) => panic!("{site}: a bound token must be refused for another tenant"),
    }
}

fn assert_ok(resp: impl IntoResponse, site: &str) {
    assert_eq!(
        resp.into_response().status(),
        StatusCode::OK,
        "{site}: a bound token reaches its own tenant"
    );
}

// ============================================================================
// agent_grading, memory_worker, myth_busting
// ============================================================================

#[tokio::test]
async fn grading_summary_reconciles_the_token_with_the_tenant_named() {
    let (context, repos) = build_context().await;
    let own = seed_tenant(&repos).await;
    let foreign = seed_tenant(&repos).await;

    let err = refusal(
        agent_grading::handle_get_summary(
            State(context.clone()),
            Extension(tenant_bound_token(own)),
            Query(agent_grading::AgentGradingQuery {
                tenant_id: Some(foreign.to_string()),
                limit: None,
            }),
        )
        .await,
        "grading summary",
    );
    assert_cross_tenant_refusal(&err, own, foreign, "grading summary");

    let ok = agent_grading::handle_get_summary(
        State(context.clone()),
        Extension(tenant_bound_token(own)),
        Query(agent_grading::AgentGradingQuery {
            tenant_id: Some(own.to_string()),
            limit: None,
        }),
    )
    .await
    .expect("own tenant summary reads");
    assert_ok(ok, "grading summary");

    let ok = agent_grading::handle_get_summary(
        State(context),
        Extension(super_admin_token()),
        Query(agent_grading::AgentGradingQuery {
            tenant_id: Some(foreign.to_string()),
            limit: None,
        }),
    )
    .await
    .expect("a super admin reads any tenant");
    assert_ok(ok, "grading summary (super-admin)");
}

#[tokio::test]
async fn memory_metrics_reconcile_the_token_with_the_tenant_named() {
    let (context, repos) = build_context().await;
    let own = seed_tenant(&repos).await;
    let foreign = seed_tenant(&repos).await;

    let err = refusal(
        memory_worker::handle_get_memory_metrics(
            State(context.clone()),
            Extension(tenant_bound_token(own)),
            Query(memory_worker::MemoryMetricsQuery {
                tenant_id: Some(foreign.to_string()),
            }),
        )
        .await,
        "memory metrics",
    );
    assert_cross_tenant_refusal(&err, own, foreign, "memory metrics");

    let ok = memory_worker::handle_get_memory_metrics(
        State(context),
        Extension(tenant_bound_token(own)),
        Query(memory_worker::MemoryMetricsQuery {
            tenant_id: Some(own.to_string()),
        }),
    )
    .await
    .expect("own tenant metrics read");
    assert_ok(ok, "memory metrics");
}

#[tokio::test]
async fn myth_busting_summary_reconciles_the_token_with_the_tenant_named() {
    let (context, repos) = build_context().await;
    let own = seed_tenant(&repos).await;
    let foreign = seed_tenant(&repos).await;

    let err = refusal(
        myth_busting::handle_get_summary(
            State(context.clone()),
            Extension(tenant_bound_token(own)),
            HeaderMap::new(),
            Query(myth_busting::MythBustingQuery {
                tenant_id: Some(foreign.to_string()),
                limit: None,
            }),
        )
        .await,
        "myth-busting summary",
    );
    assert_cross_tenant_refusal(&err, own, foreign, "myth-busting summary");

    let ok = myth_busting::handle_get_summary(
        State(context),
        Extension(tenant_bound_token(own)),
        HeaderMap::new(),
        Query(myth_busting::MythBustingQuery {
            tenant_id: Some(own.to_string()),
            limit: None,
        }),
    )
    .await
    .expect("own tenant summary reads");
    assert_ok(ok, "myth-busting summary");
}
