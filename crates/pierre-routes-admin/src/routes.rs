// ABOUTME: AdminRoutes — the Axum router builders for the admin route group
// ABOUTME: Splits into programmatic admin-token routes and human cookie-auth routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin route assembly.
//!
//! Mirrors the layout that lived in `crate::routes::admin::mod` inside
//! `pierre-server` — the only behavior change is that diagnostic
//! (`/admin/diagnostics/*`) and tool-selection (`/admin/tools/*`) sub-routes
//! are now mounted by the composition root instead of being baked into
//! [`AdminRoutes::routes`]. Those endpoints need pierre-server-internal types
//! (`ToolRegistry`, `ToolSelectionService`) that should not flow through the
//! leaf crate.

use std::sync::Arc;

use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use pierre_runtime_context::MiddlewareCtx;

use crate::auth::middleware::admin_auth_middleware;
use crate::context::AdminApiContext;
use crate::handlers::contremaitre_admin;
use crate::handlers::{
    admin_rate_limit_override, api_keys, claim_verdicts, coach_followups, coach_grading,
    coach_notes, feature_flags, harness_config, memory_worker, myth_busting, settings, setup,
    tokens, users,
};

/// Admin routes implementation (Axum).
///
/// Provides administrative endpoints for user management, API keys, JWKS,
/// and server administration.
pub struct AdminRoutes;

impl AdminRoutes {
    /// Programmatic admin routes mounted at `/admin/...` behind admin-token
    /// JWT auth.
    ///
    /// Used by `pierre-cli` and B2B partners for API key provisioning, admin
    /// token mint/rotate, and initial setup. The human-facing admin web UI
    /// (Claim Verdicts, Coach Grades, Myth Busting, Memory Worker, Coach
    /// Followups, Coach Notes Audit, Eval Harness, Harness Config) is mounted
    /// separately at `/api/admin/...` via [`Self::cookie_admin_routes`].
    pub fn routes(context: AdminApiContext) -> Router {
        let auth_service = context.auth_service.clone();
        let context = Arc::new(context);

        let api_key_routes = Self::api_key_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        let admin_token_routes = Self::admin_token_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service.clone(), admin_auth_middleware),
        );

        let user_routes = Self::user_routes(context.clone()).layer(middleware::from_fn_with_state(
            auth_service.clone(),
            admin_auth_middleware,
        ));

        let settings_routes = Self::settings_routes(context.clone()).layer(
            middleware::from_fn_with_state(auth_service, admin_auth_middleware),
        );

        // Setup routes are public (no auth required for initial setup)
        let setup_routes = Self::setup_routes(context);

        Router::new()
            .merge(api_key_routes)
            .merge(admin_token_routes)
            .merge(user_routes)
            .merge(settings_routes)
            .merge(setup_routes)
    }

    /// Human-admin routes mounted at `/api/admin/...` behind cookie/session
    /// auth + `is_admin` check.
    ///
    /// Counterpart to [`Self::routes`] — these power the admin web UI tabs
    /// (Claim Verdicts, Coach Grades, Myth Busting, Memory Worker, Coach
    /// Followups, Coach Notes Audit, Harness Config, and optionally Eval
    /// Harness when `tools-verification` is enabled). Single mount, single
    /// auth: there is no parallel `/admin/...` mount with admin-token auth
    /// for these routes.
    ///
    /// The cookie middleware is generic over [`MiddlewareCtx`]; the
    /// composition root in `pierre-server` passes `Arc<ServerContext>` as
    /// the layer state.
    pub fn cookie_admin_routes<C>(context: AdminApiContext, resources: &Arc<C>) -> Router
    where
        C: MiddlewareCtx,
    {
        let context = Arc::new(context);
        let cookie_layer = middleware::from_fn_with_state(
            Arc::clone(resources),
            pierre_middleware::cookie_admin_middleware::<C>,
        );

        let claim_verdict_routes = Self::claim_verdict_routes(context.clone());
        let memory_worker_routes = Self::memory_worker_routes(context.clone());
        let coach_followup_routes = Self::coach_followup_routes(context.clone());
        let coach_note_routes = Self::coach_note_routes(context.clone());
        let myth_busting_routes = Self::myth_busting_routes(context.clone());
        let coach_grading_routes = Self::coach_grading_routes(context.clone());
        let harness_config_routes = Self::harness_config_routes(context.clone());
        let feature_flag_admin_routes = Self::feature_flag_admin_routes(context.clone());
        let rate_limit_override_routes = Self::rate_limit_override_routes(Arc::clone(&context));

        let human_admin = Router::new()
            .merge(claim_verdict_routes)
            .merge(memory_worker_routes)
            .merge(coach_followup_routes)
            .merge(coach_note_routes)
            .merge(myth_busting_routes)
            .merge(coach_grading_routes)
            .merge(harness_config_routes)
            .merge(feature_flag_admin_routes)
            .merge(rate_limit_override_routes);

        let human_admin = human_admin.merge(contremaitre_admin::admin_routes(Arc::clone(&context)));

        #[cfg(feature = "tools-verification")]
        let human_admin = human_admin.merge(Self::eval_harness_routes(Arc::clone(&context)));

        drop(context);

        human_admin.layer(cookie_layer)
    }

    /// Eval harness fixture browser + CRUD + calibration routes (cookie auth, gated on tools-verification)
    #[cfg(feature = "tools-verification")]
    fn eval_harness_routes(context: Arc<AdminApiContext>) -> Router {
        use crate::handlers::eval_harness;
        Router::new()
            .route(
                "/api/admin/evals/fixtures",
                get(eval_harness::handle_list_fixtures),
            )
            .route(
                "/api/admin/evals/fixtures/{name}",
                get(eval_harness::handle_get_fixture)
                    .put(eval_harness::handle_put_fixture)
                    .delete(eval_harness::handle_delete_fixture),
            )
            .route(
                "/api/admin/evals/verdict-stats",
                get(eval_harness::handle_verdict_stats),
            )
            .with_state(context)
    }

    /// Coach grading routes (cookie auth)
    fn coach_grading_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/api/admin/coach-grading/summary",
                get(coach_grading::handle_get_summary),
            )
            .with_state(context)
    }

    /// Myth-busting summary + promote-topic feedback routes (cookie auth)
    fn myth_busting_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/api/admin/myth-busting/summary",
                get(myth_busting::handle_get_summary),
            )
            .route(
                "/api/admin/myth-busting/promote-topic",
                post(myth_busting::handle_promote_topic),
            )
            .with_state(context)
    }

    /// Coach note audit log + suppress routes (cookie auth)
    fn coach_note_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/api/admin/coach-notes/{note_id}/suppress",
                post(coach_notes::handle_suppress_note),
            )
            .route(
                "/api/admin/coach-notes/{note_id}/unsuppress",
                post(coach_notes::handle_unsuppress_note),
            )
            .route(
                "/api/admin/coach-notes/audit",
                get(coach_notes::handle_list_audit),
            )
            .with_state(context)
    }

    /// Coach followup triage routes (cookie auth)
    fn coach_followup_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/api/admin/coach-followups/pending",
                get(coach_followups::handle_list_pending_followups),
            )
            .route(
                "/api/admin/coach-followups/{followup_id}/cancel",
                post(coach_followups::handle_cancel_followup),
            )
            .with_state(context)
    }

    /// Memory extraction worker observability routes (cookie auth)
    fn memory_worker_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/api/admin/memory/worker-metrics",
                get(memory_worker::handle_get_memory_metrics),
            )
            .with_state(context)
    }

    /// Claim verdict triage routes (cookie auth)
    fn claim_verdict_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/api/admin/claim-verdicts",
                get(claim_verdicts::handle_list_claim_verdicts),
            )
            .route(
                "/api/admin/claim-verdicts/conversations/{conversation_id}",
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

    /// System settings routes — admin-token surface for auto-approval and
    /// social-insights config. The harness-config sub-route lives in
    /// [`Self::harness_config_routes`] under cookie auth at `/api/admin/...`.
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
            .with_state(context)
    }

    /// Per-user rate-limit override routes (cookie auth, mounted at `/api/admin/...`).
    ///
    /// `PUT /api/admin/users/{user_id}/rate-limit-override` and the matching
    /// `DELETE` revert the user to their tier default.
    fn rate_limit_override_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/api/admin/users/{user_id}/rate-limit-override",
                put(admin_rate_limit_override::handle_set)
                    .delete(admin_rate_limit_override::handle_clear),
            )
            .with_state(context)
    }

    /// Feature flag admin routes (cookie auth, mounted at `/api/admin/...`).
    ///
    /// Tenant defaults: `GET/PUT/DELETE /api/admin/tenants/{tenant_id}/features[/{key}]`.
    /// Per-user overrides: `GET/PUT/DELETE /api/admin/users/{user_id}/features[/{key}]`.
    fn feature_flag_admin_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/api/admin/tenants/{tenant_id}/features",
                get(feature_flags::handle_admin_list_tenant_defaults),
            )
            .route(
                "/api/admin/tenants/{tenant_id}/features/{key}",
                put(feature_flags::handle_admin_set_tenant_default)
                    .delete(feature_flags::handle_admin_clear_tenant_default),
            )
            .route(
                "/api/admin/users/{user_id}/features",
                get(feature_flags::handle_admin_list_user_overrides),
            )
            .route(
                "/api/admin/users/{user_id}/features/{key}",
                put(feature_flags::handle_admin_set_user_override)
                    .delete(feature_flags::handle_admin_clear_user_override),
            )
            .with_state(context)
    }

    /// Harness config routes (cookie auth, mounted at `/api/admin/...`).
    fn harness_config_routes(context: Arc<AdminApiContext>) -> Router {
        Router::new()
            .route(
                "/api/admin/settings/harness",
                get(harness_config::handle_get_harness_config),
            )
            .route(
                "/api/admin/settings/harness",
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
}
