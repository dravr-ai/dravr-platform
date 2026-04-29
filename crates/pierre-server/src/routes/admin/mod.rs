// ABOUTME: Admin API route handlers for administrative operations and API key management
// ABOUTME: Provides REST endpoints for admin services with proper authentication and authorization
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin routes for administrative operations
//!
//! This module handles admin-specific operations like API key provisioning,
//! user management, and administrative functions. All handlers are thin
//! wrappers that delegate business logic to service layers.

mod api_keys;
mod billing;
mod claim_verdicts;
mod coach_followups;
mod coach_grading;
mod coach_notes;
mod diagnostics;
#[cfg(feature = "tools-verification")]
mod eval_harness;
pub mod harness_config;
mod memory_worker;
mod myth_busting;
mod settings;
mod setup;
mod store;
mod tokens;
mod types;
mod users;

pub use types::{
    AdminResponse, AdminSetupRequest, AdminSetupResponse, ApproveUserRequest, AutoApprovalResponse,
    CoachReviewQuery, DeleteUserRequest, ListApiKeysQuery, ListPendingCoachesQuery, ListUsersQuery,
    ProvisionApiKeyRequest, ProvisionApiKeyResponse, RateLimitInfo, RejectCoachRequest,
    RevokeKeyRequest, SuspendUserRequest, TenantCreatedInfo, UpdateAutoApprovalRequest,
    UserActivityQuery,
};

use std::sync::Arc;

use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use tracing::info;

use crate::{
    admin::{auth::AdminAuthService, middleware::admin_auth_middleware, JwksManager},
    email::ResendEmailService,
    mcp::ToolSelectionService,
    routes::tool_selection::{ToolSelectionContext, ToolSelectionRoutes},
    tools::registry::ToolRegistry,
};
use pierre_auth::auth::AuthManager;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;

/// Admin API context shared across all endpoints
#[derive(Clone)]
pub struct AdminApiContext {
    /// Database connection for persistence operations (lifecycle, system settings, pool access)
    pub database: Arc<Database>,
    /// Repository registry for data access via trait objects
    pub repos: Arc<RepositoryRegistry>,
    /// Admin authentication service
    pub auth_service: AdminAuthService,
    /// Authentication manager for token operations
    pub auth_manager: Arc<AuthManager>,
    /// JWT secret for admin token validation
    pub admin_jwt_secret: String,
    /// JWKS manager for key rotation and validation
    pub jwks_manager: Arc<JwksManager>,
    /// Default monthly request limit for admin-provisioned API keys
    pub admin_api_key_monthly_limit: u32,
    /// Tool selection service for managing per-tenant MCP tool availability
    pub tool_selection: Arc<ToolSelectionService>,
    /// Tool registry for diagnostics (schema size estimation)
    pub tool_registry: Option<Arc<ToolRegistry>>,
    /// Transactional email service for user lifecycle notifications (None when Resend is unconfigured)
    pub email_service: Option<Arc<ResendEmailService>>,
    /// Public frontend URL used to build sign-in links in outbound emails
    pub frontend_url: Option<String>,
}

impl AdminApiContext {
    /// Creates a new admin API context
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        database: Arc<Database>,
        repos: Arc<RepositoryRegistry>,
        jwt_secret: &str,
        auth_manager: Arc<AuthManager>,
        jwks_manager: Arc<JwksManager>,
        admin_api_key_monthly_limit: u32,
        admin_token_cache_ttl_secs: u64,
        tool_selection: Arc<ToolSelectionService>,
    ) -> Self {
        info!("AdminApiContext initialized with JWT signing key");
        let auth_service = AdminAuthService::new(
            Arc::clone(&repos.admin),
            jwks_manager.clone(),
            admin_token_cache_ttl_secs,
        );
        Self {
            database,
            repos,
            auth_service,
            auth_manager,
            admin_jwt_secret: jwt_secret.to_owned(),
            jwks_manager,
            admin_api_key_monthly_limit,
            tool_selection,
            tool_registry: None,
            email_service: None,
            frontend_url: None,
        }
    }
}

/// Admin routes implementation (Axum)
///
/// Provides administrative endpoints for user management, API keys, JWKS, and server administration.
pub struct AdminRoutes;

impl AdminRoutes {
    /// Create all admin routes (Axum)
    pub fn routes(context: AdminApiContext) -> Router {
        let auth_service = context.auth_service.clone();
        let tool_selection_context = ToolSelectionContext {
            tool_selection: context.tool_selection.clone(),
        };
        let context = Arc::new(context);

        // Protected routes require admin authentication
        let api_key_routes = Self::api_key_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        let user_routes = Self::user_routes(context.clone()).layer(middleware::from_fn_with_state(
            auth_service.clone(),
            admin_auth_middleware,
        ));

        let settings_routes = Self::settings_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        let admin_token_routes = Self::admin_token_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Tool selection routes for per-tenant MCP tool configuration
        let tool_selection_routes = ToolSelectionRoutes::routes(tool_selection_context).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Store review routes for admin coach review queue
        let store_review_routes = Self::store_review_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Diagnostics routes for system observability
        let diagnostics_routes = Self::diagnostics_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Tier 5.5 claim verdict triage routes
        let claim_verdict_routes = Self::claim_verdict_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Phase 2: admin billing read-side (tenant usage, invoice preview, CSV export).
        let billing_routes = Self::billing_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Memory extraction worker observability routes
        let memory_worker_routes = Self::memory_worker_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Coach followup triage routes
        let coach_followup_routes = Self::coach_followup_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Coach note compliance audit routes
        let coach_note_routes = Self::coach_note_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Myth-busting summary routes (Phase D)
        let myth_busting_routes = Self::myth_busting_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Coach grading routes (Phase D)
        let coach_grading_routes = Self::coach_grading_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        // Eval harness fixture browser routes (Phase B C16, gated on tools-verification)
        #[cfg(feature = "tools-verification")]
        let eval_harness_routes = Self::eval_harness_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service, admin_auth_middleware),
        );
        #[cfg(not(feature = "tools-verification"))]
        let _ = auth_service;

        // Setup routes are public (no auth required for initial setup)
        let setup_routes = Self::setup_routes(context);

        let merged = Router::new()
            .merge(api_key_routes)
            .merge(user_routes)
            .merge(settings_routes)
            .merge(admin_token_routes)
            .merge(tool_selection_routes)
            .merge(store_review_routes)
            .merge(diagnostics_routes)
            .merge(claim_verdict_routes)
            .merge(memory_worker_routes)
            .merge(coach_followup_routes)
            .merge(coach_note_routes)
            .merge(myth_busting_routes)
            .merge(coach_grading_routes)
            .merge(billing_routes)
            .merge(setup_routes);

        #[cfg(feature = "tools-verification")]
        let merged = merged.merge(eval_harness_routes);

        merged
    }

    /// Eval harness fixture browser + CRUD + calibration routes (Axum, gated on tools-verification)
    #[cfg(feature = "tools-verification")]
    fn eval_harness_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/admin/evals/fixtures",
                get(eval_harness::handle_list_fixtures),
            )
            .route(
                "/admin/evals/fixtures/{name}",
                get(eval_harness::handle_get_fixture)
                    .put(eval_harness::handle_put_fixture)
                    .delete(eval_harness::handle_delete_fixture),
            )
            .route(
                "/admin/evals/verdict-stats",
                get(eval_harness::handle_verdict_stats),
            )
            .with_state(context)
    }

    /// Phase 2 billing routes — tenant + per-user usage aggregates,
    /// monthly invoice preview, and CSV/JSON export of recent LLM
    /// usage rows.
    fn billing_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/api/admin/tenants/{id}/usage",
                get(billing::get_tenant_usage),
            )
            .route(
                "/api/admin/tenants/{id}/invoice",
                get(billing::get_tenant_invoice),
            )
            .route("/api/admin/users/{id}/usage", get(billing::get_user_usage))
            .route(
                "/api/admin/users/{id}/cost-timeseries",
                get(billing::get_user_cost_timeseries),
            )
            .route("/api/admin/billing/export", get(billing::export_billing))
            .with_state(context)
    }

    /// Coach grading routes (Axum)
    fn coach_grading_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/admin/coach-grading/summary",
                get(coach_grading::handle_get_summary),
            )
            .with_state(context)
    }

    /// Myth-busting summary routes (Axum)
    fn myth_busting_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/admin/myth-busting/summary",
                get(myth_busting::handle_get_summary),
            )
            .with_state(context)
    }

    /// Coach note audit log routes (Axum)
    fn coach_note_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/admin/coach-notes/audit",
                get(coach_notes::handle_list_audit),
            )
            .with_state(context)
    }

    /// Coach followup triage routes (Axum)
    fn coach_followup_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/admin/coach-followups/pending",
                get(coach_followups::handle_list_pending_followups),
            )
            .route(
                "/admin/coach-followups/{followup_id}/cancel",
                post(coach_followups::handle_cancel_followup),
            )
            .with_state(context)
    }

    /// Memory extraction worker observability routes (Axum)
    fn memory_worker_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/admin/memory/worker-metrics",
                get(memory_worker::handle_get_memory_metrics),
            )
            .with_state(context)
    }

    /// Tier 5.5 claim verdict triage routes (Axum)
    fn claim_verdict_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/admin/claim-verdicts",
                get(claim_verdicts::handle_list_claim_verdicts),
            )
            .route(
                "/admin/claim-verdicts/conversations/{conversation_id}",
                get(claim_verdicts::handle_list_verdicts_by_conversation),
            )
            .with_state(context)
    }

    /// API key management routes (Axum)
    fn api_key_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route("/admin/provision", post(api_keys::handle_provision_api_key))
            .route("/admin/revoke", post(api_keys::handle_revoke_api_key))
            .route("/admin/list", get(api_keys::handle_list_api_keys))
            .route("/admin/token-info", get(api_keys::handle_token_info))
            .with_state(context)
    }

    /// User management routes (Axum)
    fn user_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route("/admin/users", get(users::handle_list_users))
            .route("/admin/pending-users", get(users::handle_pending_users))
            .route(
                "/admin/approve-user/{user_id}",
                post(users::handle_approve_user),
            )
            .route(
                "/admin/suspend-user/{user_id}",
                post(users::handle_suspend_user),
            )
            .route(
                "/admin/users/{user_id}/reset-password",
                post(users::handle_reset_user_password),
            )
            .route(
                "/admin/users/{user_id}/rate-limit",
                get(users::handle_get_user_rate_limit),
            )
            .route(
                "/admin/users/{user_id}/activity",
                get(users::handle_get_user_activity),
            )
            .route("/admin/users/{user_id}", delete(users::handle_delete_user))
            .route(
                "/admin/users/{user_id}/tier",
                post(users::handle_set_user_tier),
            )
            .with_state(context)
    }

    /// System settings routes (Axum)
    fn settings_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/admin/settings/auto-approval",
                get(settings::handle_get_auto_approval),
            )
            .route(
                "/admin/settings/auto-approval",
                put(settings::handle_set_auto_approval),
            )
            .route(
                "/admin/settings/social-insights",
                get(settings::handle_get_social_insights_config),
            )
            .route(
                "/admin/settings/social-insights",
                put(settings::handle_set_social_insights_config),
            )
            .route(
                "/admin/settings/social-insights",
                delete(settings::handle_reset_social_insights_config),
            )
            .route(
                "/admin/settings/harness",
                get(harness_config::handle_get_harness_config),
            )
            .route(
                "/admin/settings/harness",
                put(harness_config::handle_put_harness_config),
            )
            .with_state(context)
    }

    /// Setup routes (Axum)
    fn setup_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route("/admin/setup", post(setup::handle_admin_setup))
            .route("/admin/setup/status", get(setup::handle_setup_status))
            .route("/admin/health", get(setup::handle_health))
            .with_state(context)
    }

    /// Admin token management routes (Axum)
    fn admin_token_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route("/admin/tokens", post(tokens::handle_create_admin_token))
            .route("/admin/tokens", get(tokens::handle_list_admin_tokens))
            .route(
                "/admin/tokens/{token_id}",
                get(tokens::handle_get_admin_token),
            )
            .route(
                "/admin/tokens/{token_id}/revoke",
                post(tokens::handle_revoke_admin_token),
            )
            .route(
                "/admin/tokens/{token_id}/rotate",
                post(tokens::handle_rotate_admin_token),
            )
            .with_state(context)
    }

    /// Diagnostics routes for system observability (Axum)
    fn diagnostics_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/admin/diagnostics/tool-schema-size",
                get(diagnostics::handle_tool_schema_size),
            )
            .route(
                "/admin/diagnostics/tronc-canary",
                post(diagnostics::handle_tronc_canary),
            )
            .with_state(context)
    }

    /// Store review queue routes for admin coach approval (Axum)
    fn store_review_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/admin/store/pending",
                get(store::handle_list_pending_coaches),
            )
            .route(
                "/admin/store/coaches/{coach_id}/approve",
                post(store::handle_approve_coach),
            )
            .route(
                "/admin/store/coaches/{coach_id}/reject",
                post(store::handle_reject_coach),
            )
            .with_state(context)
    }
}
