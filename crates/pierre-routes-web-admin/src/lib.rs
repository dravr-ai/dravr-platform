// ABOUTME: Web admin route group — /api/admin/* surface accessible via browser cookie auth
// ABOUTME: Decoupled from pierre-server via WebAdminContext; mounted by the composition root
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Web Admin Routes
//!
//! Hosts the `/api/admin/*` REST surface that the browser admin UI calls.
//! Unlike `/admin/*` routes (admin service-token auth via `pierre-routes-admin`),
//! these routes accept standard user JWT/cookie authentication and gate on
//! `users.is_admin = true` via [`pierre_middleware::admin_guard::require_admin`].
//!
//! Endpoints covered:
//! - Admin roles (`users/{id}/promote`, `users/{id}/demote`, `admins`)
//! - Per-user profile (`users/{id}/admin-profile`)
//! - Per-user tool overrides (`tools/user/{id}/*`)
//! - Tenant plan (`tenants/{id}/plan`)
//! - Analytics (`analytics/recent-activity`)
//! - Billing / usage (`users/{id}/usage`, `cost-timeseries`,
//!   `tenants/{id}/usage|invoice`, `billing/export`)
//!
//! Every operation the admin-token API also serves — the user listing and
//! lifecycle, the pre-approved emails, auto-approval, admin tokens, and the
//! per-tenant tool overrides — is not here: the console reaches the one handler
//! for each in `pierre-routes-admin`, mounted under `/api/admin` behind session
//! auth.
//!
//! Everything is wired through [`WebAdminContext`] — a concrete state struct
//! collecting the Arc handles every handler pulls from the composition root's
//! `ServerContext`. This matches the precedent set by `AuthRoutesContext`,
//! `AdminApiContext`, and `ChatPipelineContext`.

#![warn(missing_docs)]

use std::fmt::Write as _;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::{AuthManager, AuthResult};
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_config::security::llm_base_url_allowlist as config_llm_base_url_allowlist;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::usage::{LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_middleware::tenant_path::TenantPath;
use pierre_middleware::{extract_auth_from_headers, require_admin, McpAuthMiddleware};
use pierre_runtime_context::DataContext;
use pierre_services::admin_ops;
use pierre_services::pricing::cost_for_aggregate;
use pierre_tool_runtime::tool_selection::ToolSelectionService;

/// Shared state for every web-admin route handler in this crate.
///
/// Collects the Arc handles the handlers need from the composition root's
/// `ServerContext`. The struct is `Clone` (every field is an `Arc` or a
/// trivially cloneable `DataContext`) so Axum can pass it as state through
/// the router.
///
/// Implements [`pierre_runtime_context::MiddlewareCtx`] so cookie-auth
/// helpers ([`extract_auth_from_headers`], [`require_admin`]) work against
/// this context directly.
#[derive(Clone)]
pub struct WebAdminContext {
    /// JWT auth manager (token mint / parse).
    pub auth_manager: Arc<AuthManager>,
    /// JWKS manager — signing-key source for `auth_manager`.
    pub jwks_manager: Arc<JwksManager>,
    /// Stateless CSRF token manager.
    pub csrf_manager: Arc<CsrfTokenManager>,
    /// Inbound `Authorization` header / cookie auth pipeline.
    pub auth_middleware: Arc<McpAuthMiddleware>,
    /// Repository registry — full registry kept by design. `WebAdminContext`
    /// implements `MiddlewareCtx::repos() -> &Arc<RepositoryRegistry>` (below)
    /// so admin handlers can narrow via trait bounds; the context-holder
    /// pattern requires the master Arc to live here per #9 N+ classification.
    pub repos: Arc<RepositoryRegistry>,
    /// Data context bundle handed to every `admin_ops::*` service call.
    pub data: DataContext,
    /// Tool-selection service — backs the per-user `/api/admin/tools/user/*` surface.
    pub tool_selection: Arc<ToolSelectionService>,
}

#[async_trait::async_trait]
impl pierre_runtime_context::MiddlewareCtx for WebAdminContext {
    fn auth_manager(&self) -> &Arc<AuthManager> {
        &self.auth_manager
    }

    fn jwks_manager(&self) -> &Arc<JwksManager> {
        &self.jwks_manager
    }

    fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.repos
    }

    fn csrf_manager(&self) -> &Arc<CsrfTokenManager> {
        &self.csrf_manager
    }

    async fn authenticate_request(&self, auth_header: Option<&str>) -> AppResult<AuthResult> {
        self.auth_middleware.authenticate_request(auth_header).await
    }

    fn llm_base_url_allowlist(&self) -> &[String] {
        config_llm_base_url_allowlist()
    }
}

/// Request to set a per-user tool override
#[derive(Deserialize)]
struct SetToolOverrideRequest {
    tool_name: String,
    is_enabled: bool,
    reason: Option<String>,
}

/// Request to set a tenant's plan
#[derive(Deserialize)]
struct SetTenantPlanRequest {
    /// `starter` | `professional` | `enterprise`
    plan: String,
}

/// Response for admin privilege change (promote/demote)
#[derive(Serialize)]
struct AdminPrivilegeChangeResponse {
    success: bool,
    message: String,
    user: AdminPrivilegeChangeUser,
}

/// User data in admin privilege change response
#[derive(Serialize)]
struct AdminPrivilegeChangeUser {
    id: String,
    email: String,
    is_admin: bool,
    role: String,
}

/// Admin user entry for the list-admins response
#[derive(Serialize)]
struct AdminListEntry {
    id: String,
    email: String,
    display_name: Option<String>,
    role: String,
    user_status: String,
    created_at: String,
}

/// Response for listing all admins
#[derive(Serialize)]
struct AdminListResponse {
    count: usize,
    admins: Vec<AdminListEntry>,
}

/// Query parameters for the usage-range endpoints (per-user and per-tenant).
#[derive(Debug, Deserialize)]
pub struct UsageRangeQuery {
    /// Inclusive window start (`RFC3339`). Defaults to first of the month.
    pub from: Option<String>,
}

/// Query parameters for the tenant invoice preview.
#[derive(Debug, Deserialize)]
pub struct InvoicePeriodQuery {
    /// `YYYY-MM` period to bill.
    pub period: String,
}

/// Query parameters for the bulk CSV/JSON export endpoint.
#[derive(Debug, Deserialize)]
pub struct BillingExportQuery {
    /// `YYYY-MM` period.
    pub period: String,
    /// `csv` (default) or `json`.
    pub format: Option<String>,
    /// Maximum rows to return (clamped to `10_000`).
    pub limit: Option<i64>,
}

/// Per-(provider, model, `call_type`) rollup with the USD cost folded in.
///
/// The base `LlmUsageAggregateRow` only carries token counts; this variant
/// is what the admin User Details panel renders so each row shows tokens
/// + cost without the client having to re-implement the pricing table.
#[derive(Debug, Serialize)]
pub struct LlmUsageAggregateRowWithCost {
    /// Underlying token + call rollup.
    #[serde(flatten)]
    pub row: LlmUsageAggregateRow,
    /// Estimated USD cost for this (provider, model) line item.
    pub cost_usd: f64,
}

/// Per-user usage response — same shape as the tenant variant so the
/// admin UI can render either with one component.
#[derive(Debug, Serialize)]
pub struct UserUsageResponse {
    /// User UUID.
    pub user_id: String,
    /// Window start (`RFC3339`).
    pub from: String,
    /// Per-(provider, model, `call_type`) rollup, including USD cost per row.
    pub by_model: Vec<LlmUsageAggregateRowWithCost>,
    /// Sum of `cost_usd` across `by_model` for the window.
    pub total_cost_usd: f64,
    /// Daily time series over the window.
    pub daily: Vec<LlmUsageDailyRow>,
}

/// Per-user cost time series, daily granularity.
#[derive(Debug, Serialize)]
pub struct UserCostTimeseriesResponse {
    /// User UUID.
    pub user_id: String,
    /// Window start (`RFC3339`).
    pub from: String,
    /// Daily time series points.
    pub daily: Vec<LlmUsageDailyRow>,
}

/// Aggregated usage snapshot for a tenant over a window.
#[derive(Debug, Serialize)]
pub struct TenantUsageResponse {
    /// Tenant UUID.
    pub tenant_id: String,
    /// Window start (`RFC3339`).
    pub from: String,
    /// Per-(provider, model, `call_type`) rollup.
    pub by_model: Vec<LlmUsageAggregateRow>,
    /// Daily time series over the window.
    pub daily: Vec<LlmUsageDailyRow>,
}

/// Invoice preview — echoes the period + the tenant aggregate for the window.
#[derive(Debug, Serialize)]
pub struct TenantInvoiceResponse {
    /// Tenant UUID.
    pub tenant_id: String,
    /// `YYYY-MM` period.
    pub period: String,
    /// Aggregate rollup (sum tokens + call count per provider/model).
    pub by_model: Vec<LlmUsageAggregateRow>,
    /// Daily points for the period.
    pub daily: Vec<LlmUsageDailyRow>,
}

/// Web admin routes - accessible via browser for admin users
pub struct WebAdminRoutes;

impl WebAdminRoutes {
    /// Create all web admin routes
    pub fn routes(context: WebAdminContext) -> Router {
        Router::new()
            // User management (pending users, approve, suspend, password reset,
            // rate limit, activity, tier, pre-approved emails), the rate-limit
            // override and the feature-flag routes are served by the admin-token
            // handlers, mounted for the console in
            // `pierre_routes_admin::AdminRoutes::cookie_admin_routes`.
            .route(
                "/api/admin/users/{user_id}/admin-profile",
                get(Self::handle_get_user_admin_profile),
            )
            // Per-tenant tool overrides and the globally disabled list are
            // served by `pierre_routes_admin::ToolSelectionRoutes::console_routes`.
            // Per-user tool allow/deny (overlay on top of the tenant computation)
            .route(
                "/api/admin/tools/user/{user_id}",
                get(Self::handle_get_user_tools),
            )
            .route(
                "/api/admin/tools/user/{user_id}/override",
                post(Self::handle_set_user_tool_override),
            )
            .route(
                "/api/admin/tools/user/{user_id}/override/{tool_name}",
                delete(Self::handle_remove_user_tool_override),
            )
            // Per-tenant plan (super-admin)
            .route(
                "/api/admin/tenants/{tenant_id}/plan",
                get(Self::handle_get_tenant_plan).put(Self::handle_set_tenant_plan),
            )
            .route(
                "/api/admin/analytics/recent-activity",
                get(Self::handle_recent_activity),
            )
            .route(
                "/api/admin/users/{user_id}/promote",
                post(Self::handle_promote_user),
            )
            .route(
                "/api/admin/users/{user_id}/demote",
                post(Self::handle_demote_user),
            )
            .route("/api/admin/admins", get(Self::handle_list_admins))
            // Billing / usage routes
            .route(
                "/api/admin/users/{user_id}/usage",
                get(Self::handle_get_user_usage),
            )
            .route(
                "/api/admin/tenants/{tenant_id}/usage",
                get(Self::handle_get_tenant_usage),
            )
            .route(
                "/api/admin/tenants/{tenant_id}/invoice",
                get(Self::handle_get_tenant_invoice),
            )
            .route(
                "/api/admin/billing/export",
                get(Self::handle_export_billing),
            )
            .with_state(context)
    }

    /// Authenticate user from authorization header or cookie, requiring admin privileges.
    ///
    /// The context is wrapped in an `Arc` on the fly so the generic
    /// [`extract_auth_from_headers`] helper (which keys off the
    /// [`pierre_runtime_context::MiddlewareCtx`] trait) can pick up the
    /// blanket impl on [`WebAdminContext`]. The clone is cheap — every
    /// field already lives behind an `Arc`.
    async fn authenticate_admin(
        headers: &HeaderMap,
        resources: &WebAdminContext,
    ) -> Result<AuthResult, AppError> {
        let ctx = Arc::new(resources.clone());
        let auth = extract_auth_from_headers(headers, &ctx).await?;

        // Verify admin privileges using centralized guard
        require_admin(auth.user_id, &resources.repos.users).await?;

        Ok(auth)
    }

    /// Authorize the acting admin to read a target user's per-user financial data
    /// (CWE-863). Super-admins (global operators) may read any user's; a
    /// tenant-scoped admin may read only for a user who shares one of their
    /// tenants. Returns `NotFound` for a missing target (404 before 403). This is
    /// the single source of truth for per-user financial authz — the sibling
    /// endpoints drifting out of sync is exactly what caused this disclosure.
    async fn authorize_admin_for_user(
        resources: &WebAdminContext,
        admin_user_id: Uuid,
        target_user_id: &str,
    ) -> Result<(), AppError> {
        let user_uuid = Uuid::parse_str(target_user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
        let admin = resources
            .data
            .repos()
            .users
            .get_global(admin_user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Admin user not found"))?;
        // Existence check first so a missing target returns 404, not 403.
        resources
            .data
            .repos()
            .users
            .get_global(user_uuid)
            .await?
            .ok_or_else(|| AppError::not_found("User not found"))?;
        if !admin.role.is_super_admin() {
            let admin_tenants = resources
                .data
                .repos()
                .tenants
                .list_for_user(admin_user_id)
                .await?;
            let target_tenants = resources
                .data
                .repos()
                .tenants
                .list_for_user(user_uuid)
                .await?;
            let shares_tenant = target_tenants
                .iter()
                .any(|t| admin_tenants.iter().any(|a| a.id == t.id));
            if !shares_tenant {
                return Err(AppError::new(
                    ErrorCode::PermissionDenied,
                    "Admin is not permitted to view this user's financial data",
                ));
            }
        }
        Ok(())
    }

    /// Handle GET /api/admin/users/{user_id}/admin-profile — returns the user's
    /// coaching persona, installed agents, and joined groups for the admin
    /// User Details drawer.
    async fn handle_get_user_admin_profile(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let profile =
            admin_ops::compute_user_admin_profile(&resources.data, auth.user_id, user_uuid).await?;

        Ok((StatusCode::OK, Json(profile)).into_response())
    }

    // =========================================================================
    // Tool Selection Routes (web admin versions with cookie auth)
    // =========================================================================

    /// PUT `/api/admin/tenants/{tenant_id}/plan` - set a tenant's plan (unlocks
    /// plan-gated tools). Super-admin only (billing-adjacent). Busts the
    /// in-process tool-selection cache so the change is effective immediately.
    async fn handle_set_tenant_plan(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        TenantPath(tenant_id): TenantPath,
        Json(request): Json<SetTenantPlanRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::require_super_admin(auth.user_id, &resources.data).await?;

        let updated =
            admin_ops::set_tenant_plan(&resources.repos, tenant_id, &request.plan).await?;
        resources.tool_selection.invalidate_tenant(tenant_id).await;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Tenant {tenant_id} plan set to {}", updated.plan),
                "data": { "tenant_id": tenant_id.to_string(), "plan": updated.plan }
            })),
        )
            .into_response())
    }

    /// GET `/api/admin/tenants/{tenant_id}/plan` - read a tenant's current plan.
    ///
    /// Tenant-scoped admin (or super-admin) — reading the plan is display-level
    /// (the Tenant Plan card preselects it); changing it stays super-admin only.
    async fn handle_get_tenant_plan(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        TenantPath(tenant_id): TenantPath,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::verify_admin_tenant_access(&resources.data, auth.user_id, tenant_id).await?;

        let tenant = resources.repos.tenants.get_by_id(tenant_id).await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Tenant {tenant_id} plan is {}", tenant.plan),
                "data": { "tenant_id": tenant_id.to_string(), "plan": tenant.plan }
            })),
        )
            .into_response())
    }

    /// GET `/api/admin/tools/user/{user_id}` - effective tools for a user, with
    /// the per-user overlay applied on top of the tenant computation.
    async fn handle_get_user_tools(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        Self::authorize_admin_for_user(&resources, auth.user_id, &user_id).await?;

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
        let tenants = resources.repos.tenants.list_for_user(user_uuid).await?;
        let tenant_id = tenants
            .first()
            .map(|t| t.id)
            .ok_or_else(|| AppError::not_found(format!("User {user_uuid} belongs to no tenant")))?;

        let tools = resources
            .tool_selection
            .get_effective_tools_for_user(tenant_id, user_uuid)
            .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Retrieved {} effective tools for user {user_uuid}", tools.len()),
                "data": tools
            })),
        )
            .into_response())
    }

    /// POST `/api/admin/tools/user/{user_id}/override` - set a per-user tool
    /// override (force-enable/disable a tool for this user).
    async fn handle_set_user_tool_override(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
        Json(request): Json<SetToolOverrideRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        Self::authorize_admin_for_user(&resources, auth.user_id, &user_id).await?;

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
        let override_entry = admin_ops::set_user_tool_override(
            &resources.repos,
            user_uuid,
            &request.tool_name,
            request.is_enabled,
            Some(auth.user_id),
            request.reason.clone(),
        )
        .await?;

        let action = if request.is_enabled {
            "enabled"
        } else {
            "disabled"
        };
        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Tool '{}' {} for user {user_uuid}", request.tool_name, action),
                "data": override_entry
            })),
        )
            .into_response())
    }

    /// DELETE `/api/admin/tools/user/{user_id}/override/{tool_name}` - remove a
    /// per-user tool override (revert the tool to plan/tenant/default).
    async fn handle_remove_user_tool_override(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path((user_id, tool_name)): Path<(String, String)>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        Self::authorize_admin_for_user(&resources, auth.user_id, &user_id).await?;

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
        let deleted =
            admin_ops::remove_user_tool_override(&resources.repos, user_uuid, &tool_name).await?;

        if deleted {
            Ok((
                StatusCode::OK,
                Json(serde_json::json!({
                    "success": true,
                    "message": format!("Override removed for tool '{tool_name}' on user {user_uuid}")
                })),
            )
                .into_response())
        } else {
            Err(AppError::not_found(format!(
                "No override found for tool '{tool_name}' on user {user_uuid}"
            )))
        }
    }

    /// GET `/api/admin/analytics/recent-activity` - Real-time activity feed for admin dashboard
    ///
    /// Returns recent LLM calls, recent conversations, and summary stats for the
    /// activity tab polling endpoint.
    async fn handle_recent_activity(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let activity = admin_ops::fetch_recent_activity(&resources.data).await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "recent_llm_calls": activity.recent_llm_calls,
                "recent_conversations": activity.recent_conversations,
                "summary": {
                    "active_conversations": activity.summary.active_conversations,
                    "llm_calls_today": activity.summary.llm_calls_today,
                    "total_tokens_today": activity.summary.total_tokens_today,
                    "estimated_cost_today": activity.summary.estimated_cost_today,
                }
            })),
        )
            .into_response())
    }

    /// Promote a user to admin (super-admin only)
    async fn handle_promote_user(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            admin_user_id = %auth.user_id,
            target_user_id = %user_id,
            "Web admin promoting user to admin"
        );

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let result =
            admin_ops::promote_user_to_admin(&resources.data, auth.user_id, user_uuid).await?;

        Ok((
            StatusCode::OK,
            Json(AdminPrivilegeChangeResponse {
                success: true,
                message: "User promoted to admin successfully".to_owned(),
                user: AdminPrivilegeChangeUser {
                    id: result.user_id,
                    email: result.email,
                    is_admin: result.is_admin,
                    role: result.role,
                },
            }),
        )
            .into_response())
    }

    /// Demote an admin user back to a regular user (super-admin only)
    async fn handle_demote_user(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            admin_user_id = %auth.user_id,
            target_user_id = %user_id,
            "Web admin demoting user from admin"
        );

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let result =
            admin_ops::demote_user_from_admin(&resources.data, auth.user_id, user_uuid).await?;

        Ok((
            StatusCode::OK,
            Json(AdminPrivilegeChangeResponse {
                success: true,
                message: "User demoted from admin successfully".to_owned(),
                user: AdminPrivilegeChangeUser {
                    id: result.user_id,
                    email: result.email,
                    is_admin: result.is_admin,
                    role: result.role,
                },
            }),
        )
            .into_response())
    }

    /// List all admin users across all tenants (super-admin only)
    async fn handle_list_admins(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            admin_user_id = %auth.user_id,
            "Web admin listing all admin users"
        );

        let admins = admin_ops::list_all_admins(&resources.data, auth.user_id).await?;

        let entries: Vec<AdminListEntry> = admins
            .into_iter()
            .map(|a| AdminListEntry {
                id: a.id,
                email: a.email,
                display_name: a.display_name,
                role: a.role,
                user_status: a.user_status,
                created_at: a.created_at,
            })
            .collect();

        let count = entries.len();

        Ok((
            StatusCode::OK,
            Json(AdminListResponse {
                count,
                admins: entries,
            }),
        )
            .into_response())
    }

    /// `GET /api/admin/users/{user_id}/usage?from=<rfc3339>` — per-user
    /// aggregates + daily series. Powers the admin UI Usage tab.
    async fn handle_get_user_usage(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
        Query(q): Query<UsageRangeQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        Self::authorize_admin_for_user(&resources, auth.user_id, &user_id).await?;

        let from = resolve_start(q.from.as_deref())?.to_rfc3339();
        let raw_rows = resources
            .repos
            .llm_usage
            .get_llm_usage_aggregates_by_user(&user_id, &from)
            .await?;
        let daily = resources
            .repos
            .llm_usage
            .get_llm_usage_daily_series_by_user(&user_id, &from)
            .await?;

        let by_model: Vec<LlmUsageAggregateRowWithCost> = raw_rows
            .into_iter()
            .map(|row| {
                let cost_usd = cost_for_aggregate(&row);
                LlmUsageAggregateRowWithCost { row, cost_usd }
            })
            .collect();
        let total_cost_usd: f64 = by_model.iter().map(|r| r.cost_usd).sum();

        Ok((
            StatusCode::OK,
            Json(UserUsageResponse {
                user_id,
                from,
                by_model,
                total_cost_usd,
                daily,
            }),
        )
            .into_response())
    }

    /// `GET /api/admin/tenants/{tenant_id}/usage?from=<rfc3339>`
    async fn handle_get_tenant_usage(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(tenant_id): Path<String>,
        Query(q): Query<UsageRangeQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        // SECURITY (CWE-863): this reads financial data scoped to a
        // client-supplied tenant. A tenant-scoped admin must not read another
        // tenant's usage; `verify_admin_tenant_access` restricts regular admins
        // to their own tenant while passing super-admin (global operators).
        let tenant = TenantId::parse_str(&tenant_id)
            .map_err(|_| AppError::invalid_input("Invalid tenant_id format"))?;
        admin_ops::verify_admin_tenant_access(&resources.data, auth.user_id, tenant).await?;
        let from = resolve_start(q.from.as_deref())?.to_rfc3339();
        let by_model = resources
            .repos
            .llm_usage
            .get_llm_usage_aggregates(&tenant_id, &from)
            .await?;
        let daily = resources
            .repos
            .llm_usage
            .get_llm_usage_daily_series(&tenant_id, &from)
            .await?;
        Ok((
            StatusCode::OK,
            Json(TenantUsageResponse {
                tenant_id,
                from,
                by_model,
                daily,
            }),
        )
            .into_response())
    }

    /// `GET /api/admin/tenants/{tenant_id}/invoice?period=YYYY-MM`
    async fn handle_get_tenant_invoice(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Path(tenant_id): Path<String>,
        Query(q): Query<InvoicePeriodQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        // SECURITY (CWE-863): invoice/billing data scoped to a client-supplied
        // tenant. Restrict tenant-scoped admins to their own tenant; super-admins
        // (global operators) pass.
        let tenant = TenantId::parse_str(&tenant_id)
            .map_err(|_| AppError::invalid_input("Invalid tenant_id format"))?;
        admin_ops::verify_admin_tenant_access(&resources.data, auth.user_id, tenant).await?;
        let (start, _end) = parse_month_period(&q.period)?;
        let from = start.to_rfc3339();
        let by_model = resources
            .repos
            .llm_usage
            .get_llm_usage_aggregates(&tenant_id, &from)
            .await?;
        let daily = resources
            .repos
            .llm_usage
            .get_llm_usage_daily_series(&tenant_id, &from)
            .await?;
        Ok((
            StatusCode::OK,
            Json(TenantInvoiceResponse {
                tenant_id,
                period: q.period,
                by_model,
                daily,
            }),
        )
            .into_response())
    }

    /// `GET /api/admin/billing/export?period=YYYY-MM&format=csv|json&limit=N`
    async fn handle_export_billing(
        State(resources): State<WebAdminContext>,
        headers: HeaderMap,
        Query(q): Query<BillingExportQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        // SECURITY (CWE-863): this export is platform-wide — it returns every
        // tenant's billing rows with no tenant filter — so restrict it to
        // super-admins. A tenant-scoped admin must not read other tenants'
        // billing data.
        let admin = resources
            .data
            .repos()
            .users
            .get_global(auth.user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Admin user not found"))?;
        if !admin.role.is_super_admin() {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Super-admin privileges required to export platform-wide billing",
            ));
        }
        let _ = parse_month_period(&q.period)?;
        let limit = q.limit.unwrap_or(1_000).clamp(1, 10_000);
        let records = resources
            .repos
            .llm_usage
            .get_recent_llm_calls_admin(limit)
            .await?;
        let format = q.format.as_deref().unwrap_or("csv").to_lowercase();
        if format == "json" {
            Ok((StatusCode::OK, Json(records)).into_response())
        } else {
            let mut body = String::from(
                "tenant_id,user_id,provider,model,call_type,prompt_tokens,completion_tokens,cached_tokens,total_tokens,cost_usd,created_at\n",
            );
            for r in &records {
                writeln!(
                    body,
                    "{},{},{},{},{},{},{},{},{},{:.6},{}",
                    r.tenant_id,
                    r.user_id,
                    r.provider,
                    r.model,
                    r.call_type,
                    r.prompt_tokens,
                    r.completion_tokens,
                    r.cached_tokens,
                    r.total_tokens,
                    r.cost_usd,
                    r.created_at,
                )
                .map_err(|e| AppError::internal(format!("failed to write CSV row: {e}")))?;
            }
            let mut resp = (StatusCode::OK, body).into_response();
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                "text/csv; charset=utf-8".parse().map_err(|_| {
                    AppError::internal("failed to build content-type header for export")
                })?,
            );
            resp.headers_mut().insert(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"billing-{}.csv\"", q.period)
                    .parse()
                    .map_err(|_| {
                        AppError::internal("failed to build content-disposition header")
                    })?,
            );
            Ok(resp)
        }
    }
}

fn resolve_start(from: Option<&str>) -> Result<DateTime<Utc>, AppError> {
    from.map_or_else(
        || {
            let now = Utc::now();
            Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
                .single()
                .ok_or_else(|| AppError::internal("failed to construct start-of-month timestamp"))
        },
        |s| {
            DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|e| AppError::invalid_input(format!("invalid 'from' timestamp: {e}")))
        },
    )
}

fn parse_month_period(period: &str) -> Result<(DateTime<Utc>, DateTime<Utc>), AppError> {
    let parts: Vec<&str> = period.split('-').collect();
    if parts.len() != 2 {
        return Err(AppError::invalid_input("period must be YYYY-MM"));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| AppError::invalid_input("period year must be a 4-digit integer"))?;
    let month: u32 = parts[1]
        .parse()
        .map_err(|_| AppError::invalid_input("period month must be a 2-digit integer"))?;
    if !(1..=12).contains(&month) {
        return Err(AppError::invalid_input("period month must be 1-12"));
    }
    let start = NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|dt| Utc.from_local_datetime(&dt).single())
        .ok_or_else(|| AppError::invalid_input("invalid period start"))?;
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let end = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|dt| Utc.from_local_datetime(&dt).single())
        .ok_or_else(|| AppError::invalid_input("invalid period end"))?;
    Ok((start, end))
}
